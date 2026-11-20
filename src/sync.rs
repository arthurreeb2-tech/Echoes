//! Inventaire d'un dossier et transfert de fichiers.

use crate::proto::{Entry, Msg, Secure, CHUNK};
use anyhow::{bail, Result};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Refuse tout chemin qui sortirait du dossier de travail. Un chemin vide désigne la racine.
pub fn resolve(root: &Path, rel: &str) -> Result<PathBuf> {
    let mut out = root.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        if part == "." || part == ".." || part.contains(['\\', ':', '\0']) {
            bail!("chemin refusé : {rel}");
        }
        out.push(part);
    }
    Ok(out)
}

/// Comme `resolve`, mais pour un fichier : le chemin vide est refusé.
pub fn resolve_file(root: &Path, rel: &str) -> Result<PathBuf> {
    if rel.split('/').all(|p| p.is_empty()) {
        bail!("chemin vide");
    }
    resolve(root, rel)
}

/// Destination d'un fichier reçu de l'autre machine. En plus de `resolve_file`, refuse `.git`
/// (des scripts y sont exécutés par git) et tout lien symbolique sur le chemin, qui ferait
/// écrire hors du projet.
pub fn safe_dest(root: &Path, rel: &str) -> Result<PathBuf> {
    let dest = resolve_file(root, rel)?;
    let mut cur = root.to_path_buf();
    for (i, part) in rel.split('/').filter(|p| !p.is_empty()).enumerate() {
        if i == 0 && part.eq_ignore_ascii_case(".git") {
            bail!("écriture refusée dans .git : {rel}");
        }
        cur.push(part);
        if let Ok(meta) = fs::symlink_metadata(&cur) {
            if meta.file_type().is_symlink() {
                bail!("écriture refusée à travers un lien symbolique : {rel}");
            }
        }
    }
    Ok(dest)
}

pub fn rel(root: &Path, path: &Path) -> Option<String> {
    let parts: Option<Vec<&str>> = path.strip_prefix(root).ok()?.components().map(|c| c.as_os_str().to_str()).collect();
    Some(parts?.join("/"))
}

pub fn hash(path: &Path) -> Result<[u8; 32]> {
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut File::open(path)?, &mut hasher)?;
    Ok(*hasher.finalize().as_bytes())
}

/// Empreintes déjà calculées, réutilisées tant que taille et date de modification n'ont pas changé.
#[derive(Serialize, Deserialize, Default)]
pub struct HashCache {
    #[serde(skip)]
    file: Option<PathBuf>,
    entries: HashMap<String, (u64, u128, [u8; 32])>,
}

impl HashCache {
    pub fn load(file: PathBuf) -> Self {
        let mut c: HashCache = fs::read(&file).ok().and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default();
        c.file = Some(file);
        c
    }

    pub fn save(&self) {
        if let Some(f) = &self.file {
            if let Ok(b) = bincode::serialize(self) {
                let _ = fs::write(f, b);
            }
        }
    }

    /// Empreinte d'un fichier, relue seulement s'il a changé depuis la dernière fois.
    pub fn hash(&mut self, rel: &str, path: &Path) -> Result<[u8; 32]> {
        let meta = fs::metadata(path)?;
        self.get(rel, path, &meta)
    }

    fn get(&mut self, rel: &str, path: &Path, meta: &fs::Metadata) -> Result<[u8; 32]> {
        let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
        if let Some((size, m, h)) = self.entries.get(rel) {
            if *size == meta.len() && *m == mtime && mtime != 0 {
                return Ok(*h);
            }
        }
        let h = hash(path)?;
        self.entries.insert(rel.to_string(), (meta.len(), mtime, h));
        Ok(h)
    }
}

fn mode(meta: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0
    }
}

pub fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    if mode != 0 {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

/// Fichiers du projet : tout sauf `.git` et ce que `.gitignore` / `.echoesignore` excluent.
/// Avec `everything`, aucune règle d'exclusion (sert à rapatrier des produits de build).
/// Les fichiers illisibles ou disparus en cours de route sont ignorés, pas fatals.
/// `tick` est appelé à chaque fichier (pour donner signe de vie pendant un long inventaire).
pub fn scan(root: &Path, start: &Path, everything: bool, cache: &mut HashCache, tick: &mut dyn FnMut() -> Result<()>) -> Result<Vec<Entry>> {
    let mut walk = WalkBuilder::new(start);
    walk.hidden(false).parents(false).git_global(false).git_exclude(false).require_git(false);
    if everything {
        walk.git_ignore(false).ignore(false);
    } else {
        walk.add_custom_ignore_filename(".echoesignore");
    }
    walk.filter_entry(|e| e.file_name() != ".git" && !e.file_name().to_string_lossy().ends_with(PART));
    let mut out = Vec::new();
    for item in walk.build() {
        tick()?;
        let Ok(item) = item else { continue };
        let Ok(meta) = item.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Some(path) = rel(root, item.path()) else { continue };
        let Ok(hash) = cache.get(&path, item.path(), &meta) else { continue };
        out.push(Entry { path, hash, mode: mode(&meta), size: meta.len() });
    }
    Ok(out)
}

pub fn send_file(conn: &mut Secure, root: &Path, entry: &Entry, pulled: bool) -> Result<()> {
    let mut file = File::open(resolve_file(root, &entry.path)?)?;
    let mut first = true;
    loop {
        let mut data = vec![0u8; CHUNK];
        let mut n = 0;
        while n < CHUNK {
            let r = file.read(&mut data[n..])?;
            if r == 0 {
                break;
            }
            n += r;
        }
        data.truncate(n);
        let last = n < CHUNK;
        conn.send(&Msg::File { path: entry.path.clone(), mode: entry.mode, first, last, pulled, data })?;
        first = false;
        if last {
            return Ok(());
        }
    }
}

/// Suffixe des fichiers en cours de réception.
const PART: &str = ".echoes-part";

/// Écrit un fichier reçu morceau par morceau, via un fichier temporaire.
pub struct Receiver {
    current: Option<(PathBuf, PathBuf, File)>,
}

impl Drop for Receiver {
    fn drop(&mut self) {
        if let Some((_, tmp, _)) = self.current.take() {
            let _ = fs::remove_file(tmp);
        }
    }
}

impl Receiver {
    pub fn new() -> Self {
        Receiver { current: None }
    }

    /// `dest` vaut `None` quand le fichier doit être ignoré. Un transfert interrompu ne laisse rien derrière lui.
    pub fn chunk(&mut self, dest: Option<PathBuf>, mode: u32, first: bool, last: bool, data: &[u8]) -> Result<()> {
        if first {
            if let Some((_, tmp, _)) = self.current.take() {
                let _ = fs::remove_file(tmp);
            }
            if let Some(dest) = dest {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut name = dest.file_name().unwrap_or_default().to_os_string();
                name.push(PART);
                let tmp = dest.with_file_name(name);
                self.current = Some((dest, tmp.clone(), File::create(tmp)?));
            }
        }
        if let Some((_, _, file)) = &mut self.current {
            file.write_all(data)?;
        }
        if last {
            if let Some((dest, tmp, file)) = self.current.take() {
                drop(file);
                fs::rename(&tmp, &dest)?;
                set_mode(&dest, mode);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_refuses_escapes() {
        let root = Path::new("/ws");
        assert!(resolve_file(root, "src/main.rs").is_ok());
        assert!(resolve_file(root, "../etc/passwd").is_err());
        assert!(resolve_file(root, "a/../../b").is_err());
        assert!(resolve_file(root, "c:/x").is_err());
        assert!(resolve_file(root, "").is_err());
        assert_eq!(resolve(root, "").unwrap(), root);
    }

    #[cfg(unix)]
    #[test]
    fn safe_dest_refuses_git_and_symlinks() {
        let root = std::env::temp_dir().join(format!("echoes-safe-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!("echoes-out-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("vendor")).unwrap();
        assert!(safe_dest(&root, "src/new.rs").is_ok());
        assert!(safe_dest(&root, "vendor/out").is_err());
        assert!(safe_dest(&root, ".git/hooks/pre-commit").is_err());
        assert!(safe_dest(&root, "src/.git/x").is_ok());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    use crate::config::test_support::tmp_dir;

    fn names(entries: &[Entry]) -> Vec<String> {
        let mut v: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
        v.sort();
        v
    }

    fn scan_all(root: &Path, everything: bool) -> Vec<Entry> {
        let mut cache = HashCache::default();
        scan(root, root, everything, &mut cache, &mut || Ok(())).unwrap()
    }

    #[test]
    fn resolve_rejects_each_dangerous_component() {
        let root = Path::new("/ws");
        assert_eq!(resolve(root, "a/b").unwrap(), Path::new("/ws/a/b"));
        assert_eq!(resolve(root, "a//b/").unwrap(), Path::new("/ws/a/b"), "séparateurs en double ignorés");
        for bad in ["..", "a/..", "a/../b", "./a", "a/./b", "a\\b", "C:/x", "a:b", "a\0b"] {
            assert!(resolve(root, bad).is_err(), "{bad:?} doit être refusé");
        }
        // Un chemin absolu est relu comme relatif : il reste dans le dossier de travail.
        assert_eq!(resolve(root, "/etc/passwd").unwrap(), Path::new("/ws/etc/passwd"));
        assert!(resolve_file(root, "/").is_err());
        assert!(resolve_file(root, "//").is_err());
        assert_eq!(resolve_file(root, "x").unwrap(), Path::new("/ws/x"));
    }

    #[cfg(unix)]
    #[test]
    fn safe_dest_edge_cases() {
        let root = tmp_dir("safe");
        let outside = tmp_dir("outside");
        fs::create_dir_all(root.join("a/b")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("a/link")).unwrap();
        std::os::unix::fs::symlink(outside.join("cible"), root.join("a/b/fichier")).unwrap();

        // `.git` : refusé en racine, quelle que soit la casse, et comme fichier seul.
        assert!(safe_dest(&root, ".git").is_err());
        assert!(safe_dest(&root, ".GIT/config").is_err());
        assert!(safe_dest(&root, ".Git/hooks/x").is_err());
        // Comportement actuel : seul le `.git` de la racine est protégé (sous-modules et dépôts imbriqués restent écrits).
        assert!(safe_dest(&root, "a/.git/x").is_ok());
        assert!(safe_dest(&root, ".gitignore").is_ok());
        assert!(safe_dest(&root, ".github/workflows/x.yml").is_ok());

        // Lien symbolique dans un dossier parent, ou comme destination elle-même.
        assert!(safe_dest(&root, "a/link/x").is_err());
        assert!(safe_dest(&root, "a/link/deep/er/x").is_err());
        assert!(safe_dest(&root, "a/link").is_err());
        assert!(safe_dest(&root, "a/b/fichier").is_err());
        assert!(safe_dest(&root, "a/b/autre").is_ok());

        // Sorties de dossier et chemins absolus.
        assert!(safe_dest(&root, "a/../../x").is_err());
        assert!(safe_dest(&root, "a/b/../x").is_err());
        assert!(safe_dest(&root, "").is_err());
        let abs = safe_dest(&root, "/etc/passwd").unwrap();
        assert_eq!(abs, root.join("etc/passwd"), "un chemin absolu reste dans le dossier de travail");

        // Chemin inexistant : accepté (le dossier sera créé à la réception).
        assert!(safe_dest(&root, "nouveau/dossier/f.txt").is_ok());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn rel_uses_forward_slashes_and_stays_inside_root() {
        let root = Path::new("/ws/p");
        assert_eq!(rel(root, Path::new("/ws/p/a/b.txt")).as_deref(), Some("a/b.txt"));
        assert_eq!(rel(root, Path::new("/ws/p")).as_deref(), Some(""));
        assert_eq!(rel(root, Path::new("/ws/autre/a")), None);
        assert_eq!(rel(root, Path::new("/ws")), None);
    }

    #[test]
    fn hash_matches_blake3() {
        let dir = tmp_dir("hash");
        let empty = dir.join("vide");
        fs::write(&empty, b"").unwrap();
        assert_eq!(hex::encode(hash(&empty).unwrap()), "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262");
        let f = dir.join("f");
        fs::write(&f, b"hello").unwrap();
        assert_eq!(hash(&f).unwrap(), *blake3::hash(b"hello").as_bytes());
        assert!(hash(&dir.join("absent")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hash_cache_reuses_until_file_changes() {
        let dir = tmp_dir("cache");
        let f = dir.join("f");
        let store = dir.join("cache.bin");
        fs::write(&f, "aaaa").unwrap();
        let mtime = fs::metadata(&f).unwrap().modified().unwrap();

        let mut cache = HashCache::load(store.clone());
        let h1 = cache.hash("f", &f).unwrap();
        assert_eq!(h1, hash(&f).unwrap());
        cache.save();

        // Même taille et même date : l'empreinte enregistrée est reprise sans relire le fichier.
        fs::write(&f, "bbbb").unwrap();
        File::options().write(true).open(&f).unwrap().set_modified(mtime).unwrap();
        let mut cache = HashCache::load(store.clone());
        assert_eq!(cache.hash("f", &f).unwrap(), h1);

        // Taille différente : recalcul.
        fs::write(&f, "cccccc").unwrap();
        let h2 = cache.hash("f", &f).unwrap();
        assert_eq!(h2, hash(&f).unwrap());
        assert_ne!(h2, h1);

        // Même taille, date différente : recalcul aussi.
        fs::write(&f, "dddddd").unwrap();
        File::options().write(true).open(&f).unwrap().set_modified(mtime + std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(cache.hash("f", &f).unwrap(), hash(&f).unwrap());

        // Cache absent ou corrompu : on repart de zéro sans erreur.
        fs::write(&store, b"n'importe quoi").unwrap();
        let mut cache = HashCache::load(store);
        assert_eq!(cache.hash("f", &f).unwrap(), hash(&f).unwrap());
        assert!(HashCache::load(dir.join("absent")).hash("f", &dir.join("absent")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_skips_git_ignored_and_partial_files() {
        let root = tmp_dir("scan");
        for (path, body) in [
            ("a.txt", "a"),
            ("sub/b.txt", "bb"),
            (".env", "x"),
            (".git/config", "g"),
            (".gitignore", "ignore.txt\ntarget/\n"),
            ("ignore.txt", "i"),
            ("target/out", "o"),
            (".echoesignore", "gros/\n"),
            ("gros/data", "d"),
            ("sub/reçu.bin.echoes-part", "p"),
        ] {
            let p = root.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        }
        let found = scan_all(&root, false);
        assert_eq!(names(&found), [".echoesignore", ".env", ".gitignore", "a.txt", "sub/b.txt"]);
        let b = found.iter().find(|e| e.path == "sub/b.txt").unwrap();
        assert_eq!((b.size, b.hash), (2, *blake3::hash(b"bb").as_bytes()));

        // `everything` lève les règles d'exclusion, mais jamais `.git` ni les fichiers en cours de réception.
        let all = scan_all(&root, true);
        assert_eq!(names(&all), [".echoesignore", ".env", ".gitignore", "a.txt", "gros/data", "ignore.txt", "sub/b.txt", "target/out"]);

        // Inventaire d'un sous-dossier : les chemins restent relatifs à la racine.
        let mut cache = HashCache::default();
        let sub = scan(&root, &root.join("sub"), false, &mut cache, &mut || Ok(())).unwrap();
        assert_eq!(names(&sub), ["sub/b.txt"]);

        // `tick` est appelé à chaque élément, et une erreur interrompt l'inventaire.
        let mut ticks = 0;
        scan(&root, &root, false, &mut cache, &mut || {
            ticks += 1;
            Ok(())
        })
        .unwrap();
        assert!(ticks >= 5);
        assert!(scan(&root, &root, false, &mut cache, &mut || bail!("stop")).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn scan_ignores_symlinks_and_records_mode() {
        use std::os::unix::fs::PermissionsExt;
        let root = tmp_dir("scan-mode");
        let script = root.join("run.sh");
        fs::write(&script, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(&script, root.join("lien")).unwrap();
        std::os::unix::fs::symlink("/nonexistent", root.join("casse")).unwrap();
        let found = scan_all(&root, false);
        assert_eq!(names(&found), ["run.sh"]);
        assert_eq!(found[0].mode, 0o755);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn set_mode_applies_permissions_and_ignores_zero() {
        let dir = tmp_dir("mode");
        let f = dir.join("f");
        fs::write(&f, "x").unwrap();
        set_mode(&f, 0o640);
        assert_eq!(mode(&fs::metadata(&f).unwrap()), 0o640);
        set_mode(&f, 0);
        assert_eq!(mode(&fs::metadata(&f).unwrap()), 0o640, "0 = mode inconnu, on ne touche à rien");
        set_mode(&dir.join("absent"), 0o600);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn receiver_assembles_chunks_atomically() {
        let dir = tmp_dir("recv");
        let dest = dir.join("deep/dir/out.bin");
        let tmp = dir.join(format!("deep/dir/out.bin{PART}"));
        let mut rx = Receiver::new();
        rx.chunk(Some(dest.clone()), 0o600, true, false, b"abc").unwrap();
        assert!(!dest.exists() && tmp.exists(), "pendant le transfert, seul le temporaire existe");
        rx.chunk(Some(dest.clone()), 0o600, false, false, b"def").unwrap();
        rx.chunk(Some(dest.clone()), 0o600, false, true, b"g").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"abcdefg");
        assert!(!tmp.exists());
        #[cfg(unix)]
        assert_eq!(mode(&fs::metadata(&dest).unwrap()), 0o600);

        // Un fichier d'un seul morceau, vide, remplace l'ancien contenu.
        rx.chunk(Some(dest.clone()), 0o644, true, true, b"").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn receiver_leaves_nothing_behind_on_abort_or_skip() {
        let dir = tmp_dir("recv-abort");
        let a = dir.join("a");
        let b = dir.join("b");
        {
            let mut rx = Receiver::new();
            rx.chunk(Some(a.clone()), 0, true, false, b"partiel").unwrap();
            assert!(dir.join(format!("a{PART}")).exists());
        } // abandon : Drop nettoie
        assert!(!a.exists() && !dir.join(format!("a{PART}")).exists());

        // Un nouveau `first` abandonne le précédent transfert inachevé.
        let mut rx = Receiver::new();
        rx.chunk(Some(a.clone()), 0, true, false, b"1").unwrap();
        rx.chunk(Some(b.clone()), 0, true, true, b"2").unwrap();
        assert!(!a.exists() && !dir.join(format!("a{PART}")).exists());
        assert_eq!(fs::read(&b).unwrap(), b"2");

        // Fichier ignoré (`dest` = None) : les morceaux sont jetés, rien n'est écrit.
        let before = fs::read_dir(&dir).unwrap().count();
        rx.chunk(None, 0, true, false, b"x").unwrap();
        rx.chunk(None, 0, false, true, b"y").unwrap();
        assert_eq!(fs::read_dir(&dir).unwrap().count(), before);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn send_file_streams_chunks_to_receiver_over_secure_channel() {
        use std::net::{TcpListener, TcpStream};
        let key = [8u8; 32];
        let src = tmp_dir("send-src");
        let dst = tmp_dir("send-dst");
        let sizes = [("vide", 0usize), ("petit", 10), ("pile", CHUNK), ("grand", CHUNK * 2 + 123)];
        let mut entries = Vec::new();
        for (name, size) in sizes {
            let body: Vec<u8> = (0..size).map(|i| (i % 253) as u8).collect();
            fs::write(src.join(name), &body).unwrap();
            entries.push(Entry { path: name.into(), hash: *blake3::hash(&body).as_bytes(), mode: 0, size: size as u64 });
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let dst2 = dst.clone();
        let count = entries.len();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut conn = Secure::accept(stream, &key).unwrap();
            let mut rx = Receiver::new();
            let mut messages = 0;
            let mut done = 0;
            while done < count {
                match conn.recv().unwrap() {
                    Msg::File { path, mode, first, last, data, .. } => {
                        messages += 1;
                        rx.chunk(Some(safe_dest(&dst2, &path).unwrap()), mode, first, last, &data).unwrap();
                        done += last as usize;
                    }
                    _ => panic!("message inattendu"),
                }
            }
            messages
        });
        let mut conn = Secure::connect(TcpStream::connect(addr).unwrap(), &key).unwrap();
        for e in &entries {
            send_file(&mut conn, &src, e, false).unwrap();
        }
        // vide 1 + petit 1 + pile 2 (un morceau plein puis un morceau vide) + grand 3
        assert_eq!(server.join().unwrap(), 7);
        for e in &entries {
            assert_eq!(hash(&dst.join(&e.path)).unwrap(), e.hash, "{}", e.path);
        }
        assert!(send_file(&mut conn, &src, &Entry { path: "../x".into(), hash: [0; 32], mode: 0, size: 0 }, false).is_err());
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&dst);
    }
}
