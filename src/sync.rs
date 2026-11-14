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
}
