//! Inventaire d'un dossier et transfert de fichiers.

use crate::proto::{Entry, Msg, Secure, CHUNK};
use anyhow::{bail, Result};
use ignore::WalkBuilder;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Refuse tout chemin qui sortirait du dossier de travail.
pub fn resolve(root: &Path, rel: &str) -> Result<PathBuf> {
    let mut out = root.to_path_buf();
    if rel.is_empty() {
        bail!("chemin vide");
    }
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\\', ':', '\0']) {
            bail!("chemin refusé : {rel}");
        }
        out.push(part);
    }
    Ok(out)
}

fn rel(root: &Path, path: &Path) -> Option<String> {
    let parts: Option<Vec<&str>> = path.strip_prefix(root).ok()?.components().map(|c| c.as_os_str().to_str()).collect();
    Some(parts?.join("/"))
}

pub fn hash(path: &Path) -> Result<[u8; 32]> {
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut File::open(path)?, &mut hasher)?;
    Ok(*hasher.finalize().as_bytes())
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
pub fn scan(root: &Path, start: &Path, everything: bool) -> Result<Vec<Entry>> {
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
        out.push(Entry { path, hash: hash(item.path())?, mode: mode(&meta) });
    }
    Ok(out)
}

pub fn send_file(conn: &mut Secure, root: &Path, entry: &Entry, pulled: bool) -> Result<()> {
    let mut file = File::open(resolve(root, &entry.path)?)?;
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
