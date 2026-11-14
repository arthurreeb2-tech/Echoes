//! Clé d'appairage, identité de la machine et petits fichiers d'état.

use anyhow::{anyhow, Context, Result};
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;

fn dir(base: Option<PathBuf>) -> Result<PathBuf> {
    let d = base.ok_or_else(|| anyhow!("dossier utilisateur introuvable"))?.join("echoes");
    fs::create_dir_all(&d)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&d, fs::Permissions::from_mode(0o700));
    }
    Ok(d)
}

pub fn config_dir() -> Result<PathBuf> {
    match std::env::var_os("ECHOES_HOME") {
        Some(h) => dir(Some(PathBuf::from(h))),
        None => dir(dirs::config_dir()),
    }
}

pub fn data_dir() -> Result<PathBuf> {
    match std::env::var_os("ECHOES_HOME") {
        Some(h) => dir(Some(PathBuf::from(h))),
        None => dir(dirs::data_local_dir()),
    }
}

/// Écrit un fichier secret : créé illisible pour les autres comptes, puis mis en place d'un coup.
pub fn private_write(path: &std::path::Path, content: &str) -> Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(&tmp)?.write_all(content.as_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn save_key(key: &[u8; 32]) -> Result<()> {
    private_write(&config_dir()?.join("key"), &hex::encode(key))
}

pub fn parse_key(code: &str) -> Result<[u8; 32]> {
    let raw = hex::decode(code.trim().replace(['-', ' '], "")).context("code d'appairage invalide")?;
    raw.try_into().map_err(|_| anyhow!("code d'appairage invalide (64 caractères attendus)"))
}

pub fn load_key() -> Result<Option<[u8; 32]>> {
    let path = config_dir()?.join("key");
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(parse_key(&fs::read_to_string(path)?)?))
}

pub fn require_key() -> Result<[u8; 32]> {
    load_key()?.ok_or_else(|| anyhow!("aucune clé d'appairage : lance `echoes pair` sur cette machine"))
}

/// Identifiant stable de cette machine (pour ne pas se déléguer à soi-même).
pub fn machine_id() -> Result<String> {
    let path = config_dir()?.join("id");
    if let Ok(id) = fs::read_to_string(&path) {
        return Ok(id.trim().to_string());
    }
    let id = hex::encode(rand::random::<[u8; 8]>());
    fs::write(path, &id)?;
    Ok(id)
}

/// Dernières machines vues : évite une recherche réseau à chaque commande.
pub fn load_peers() -> Vec<SocketAddr> {
    let Ok(d) = data_dir() else { return vec![] };
    let Ok(s) = fs::read_to_string(d.join("peers")) else { return vec![] };
    s.lines().filter_map(|l| l.trim().parse().ok()).collect()
}

/// Vrai si la liste des machines connues date de plus de dix minutes (ou n'existe pas).
pub fn peers_stale() -> bool {
    let age = data_dir().ok().and_then(|d| fs::metadata(d.join("peers")).ok()).and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok());
    age.map(|a| a.as_secs() > 600).unwrap_or(true)
}

pub fn save_peers(peers: &[SocketAddr]) {
    if let Ok(d) = data_dir() {
        let s: Vec<String> = peers.iter().map(|p| p.to_string()).collect();
        let _ = fs::write(d.join("peers"), s.join("\n"));
    }
}
