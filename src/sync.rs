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
pub fn scan(root: &Path, start: &Path, everything: bool, cache: &mut HashCache) -> Result<Vec<Entry>> {
    let mut walk = WalkBuilder::new(start);
    walk.hidden(false).parents(false).git_global(false).git_exclude(false).require_git(false);
    if everything {
        walk.git_ignore(false).ignore(false);
    } else {
        walk.add_custom_ignore_filename(".echoesignore");
    }
    walk.filter_entry(|e| e.file_name() != ".git");
    let mut out = Vec::new();
    for item in walk.build() {
        let item = item?;
        let meta = item.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let Some(path) = rel(root, item.path()) else { continue };
        let hash = cache.get(&path, item.path(), &meta)?;
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

/// Écrit un fichier reçu morceau par morceau, via un fichier temporaire.
pub struct Receiver {
    current: Option<(PathBuf, PathBuf, File)>,
}

impl Receiver {
    pub fn new() -> Self {
        Receiver { current: None }
    }

    /// `dest` vaut `None` quand le fichier doit être ignoré.
    pub fn chunk(&mut self, dest: Option<PathBuf>, mode: u32, first: bool, last: bool, data: &[u8]) -> Result<()> {
        if first {
            self.current = None;
            if let Some(dest) = dest {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut name = dest.file_name().unwrap_or_default().to_os_string();
                name.push(".echoes-part");
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
}
