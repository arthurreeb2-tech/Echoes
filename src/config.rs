//! Clé d'appairage, identité de la machine et petits fichiers d'état.

use anyhow::{anyhow, Context, Result};
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;

fn dir(base: Option<PathBuf>) -> Result<PathBuf> {
    let d = base.ok_or_else(|| anyhow!("dossier de configuration de l'utilisateur introuvable"))?.join("echoes");
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
    let raw = hex::decode(code.trim().replace(['-', ' '], "")).context("code d'appairage invalide (chiffres 0-9 et lettres a-f uniquement)")?;
    raw.try_into().map_err(|_| anyhow!("code d'appairage invalide (64 chiffres hexadécimaux attendus)"))
}

pub fn load_key() -> Result<Option<[u8; 32]>> {
    let path = config_dir()?.join("key");
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(parse_key(&fs::read_to_string(path)?)?))
}

pub fn require_key() -> Result<[u8; 32]> {
    load_key()?.ok_or_else(|| anyhow!("aucune clé d'appairage sur cette machine : lance `echoes pair` (ou `echoes pair <code>` pour rejoindre les autres)"))
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

/// Aides communes aux tests : les variables d'environnement sont globales au processus,
/// donc tout test qui touche à `ECHOES_HOME` ou `ECHOES_FAKE_GPU` passe par `env_lock`.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, MutexGuard};

    static ENV: Mutex<()> = Mutex::new(());
    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub fn env_lock() -> MutexGuard<'static, ()> {
        ENV.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Dossier temporaire neuf et unique.
    pub fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("echoes-test-{name}-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Pointe `ECHOES_HOME` sur un dossier neuf tant que la garde vit.
    pub fn home(name: &str) -> (MutexGuard<'static, ()>, PathBuf) {
        let guard = env_lock();
        let dir = tmp_dir(name);
        std::env::set_var("ECHOES_HOME", &dir);
        (guard, dir)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn parse_key_accepts_64_hex_digits() {
        let code = "ab".repeat(32);
        assert_eq!(parse_key(&code).unwrap(), [0xab; 32]);
        assert_eq!(parse_key(&code.to_uppercase()).unwrap(), [0xab; 32]);
    }

    #[test]
    fn parse_key_tolerates_spaces_and_dashes() {
        let code = "0f".repeat(32);
        assert_eq!(parse_key(&format!("  {code}\n")).unwrap(), [0x0f; 32]);
        let grouped: Vec<&str> = (0..8).map(|i| &code[i * 8..i * 8 + 8]).collect();
        assert_eq!(parse_key(&grouped.join("-")).unwrap(), [0x0f; 32]);
        assert_eq!(parse_key(&grouped.join(" ")).unwrap(), [0x0f; 32]);
    }

    #[test]
    fn parse_key_refuses_bad_codes() {
        assert!(parse_key("").is_err());
        assert!(parse_key(&"ab".repeat(31)).is_err(), "trop court");
        assert!(parse_key(&"ab".repeat(33)).is_err(), "trop long");
        assert!(parse_key(&"zz".repeat(32)).is_err(), "caractères non hexadécimaux");
        assert!(parse_key(&format!("{}g", "a".repeat(63))).is_err());
        let e = parse_key(&"ab".repeat(31)).unwrap_err().to_string();
        assert!(e.contains("64"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn private_write_is_0600_and_replaces() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp_dir("private");
        let path = dir.join("secret");
        private_write(&path, "un").unwrap();
        private_write(&path, "deux").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "deux");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "aucun fichier temporaire laissé");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn state_files_round_trip_in_echoes_home() {
        let (_guard, home) = home("state");

        assert_eq!(load_key().unwrap(), None);
        assert!(require_key().is_err());
        let key = [0x5a; 32];
        save_key(&key).unwrap();
        assert_eq!(load_key().unwrap(), Some(key));
        assert_eq!(require_key().unwrap(), key);
        assert_eq!(fs::read_to_string(home.join("echoes/key")).unwrap(), hex::encode(key));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(home.join("echoes/key")).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(fs::metadata(home.join("echoes")).unwrap().permissions().mode() & 0o777, 0o700);
        }

        fs::write(home.join("echoes/key"), "pas une clé").unwrap();
        assert!(load_key().is_err());

        let id = machine_id().unwrap();
        assert_eq!(id.len(), 16);
        assert_eq!(machine_id().unwrap(), id);

        assert!(load_peers().is_empty());
        assert!(peers_stale(), "pas de fichier : à rafraîchir");
        let peers: Vec<SocketAddr> = vec!["192.168.1.10:47600".parse().unwrap(), "10.0.0.2:47600".parse().unwrap()];
        save_peers(&peers);
        assert_eq!(load_peers(), peers);
        assert!(!peers_stale());
        fs::write(home.join("echoes/peers"), "192.168.1.10:47600\nn'importe quoi\n\n10.0.0.2:1\n").unwrap();
        assert_eq!(load_peers().len(), 2, "les lignes illisibles sont ignorées");
    }
}
