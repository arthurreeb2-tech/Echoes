//! Journal des tâches lancées depuis cette machine (pour le tableau de bord).

use crate::config;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;

const KEEP: usize = 5000;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Record {
    pub id: String,
    /// Horodatage Unix, en secondes.
    pub ts: u64,
    pub origin: String,
    pub origin_id: String,
    /// Machine qui a exécuté la commande (l'origine elle-même si `remote` est faux).
    pub target: String,
    pub remote: bool,
    /// Pourquoi la commande est restée ici.
    pub reason: String,
    pub command: String,
    pub project: String,
    /// Durée totale vue depuis l'origine (transferts compris).
    pub total_ms: u64,
    /// Durée de la commande elle-même.
    pub command_ms: u64,
    /// Estimation de la durée qu'aurait prise la commande ici.
    pub est_local_ms: u64,
    pub exit: Option<i32>,
    pub sent_bytes: u64,
    pub files_sent: u32,
    pub files_back: u32,
}

impl Record {
    pub fn new(origin: &str, origin_id: &str, command: &[String], project: &str) -> Self {
        let mut cmd = command.join(" ");
        if cmd.chars().count() > 200 {
            cmd = cmd.chars().take(200).collect::<String>() + "…";
        }
        Record {
            id: hex::encode(rand::random::<[u8; 8]>()),
            ts: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            origin: origin.into(),
            origin_id: origin_id.into(),
            target: origin.into(),
            command: cmd,
            project: project.into(),
            ..Default::default()
        }
    }
}

fn file() -> Option<std::path::PathBuf> {
    config::data_dir().ok().map(|d| d.join("history.jsonl"))
}

pub fn append(rec: &Record) {
    let Some(path) = file() else { return };
    let Ok(line) = serde_json::to_string(rec) else { return };
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        // Les commandes peuvent contenir des informations sensibles : journal privé.
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    if let Ok(mut f) = opts.open(&path) {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
    // Élagage occasionnel : on garde les KEEP dernières tâches.
    if fs::metadata(&path).map(|m| m.len() > 4 << 20).unwrap_or(false) {
        let all = read(usize::MAX);
        let keep: Vec<String> = all.iter().rev().take(KEEP).rev().filter_map(|r| serde_json::to_string(r).ok()).collect();
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        if fs::write(&tmp, keep.join("\n") + "\n").is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
}

/// Les `limit` dernières tâches, de la plus ancienne à la plus récente.
pub fn read(limit: usize) -> Vec<Record> {
    let Some(path) = file() else { return vec![] };
    let Ok(text) = fs::read_to_string(path) else { return vec![] };
    let all: Vec<Record> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let skip = all.len().saturating_sub(limit);
    all.into_iter().skip(skip).collect()
}
