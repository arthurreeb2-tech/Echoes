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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_support::home;

    fn rec(n: u64) -> Record {
        let mut r = Record::new("mac", "abc", &["make".into(), format!("t{n}")], "/p");
        r.ts = n;
        r
    }

    fn stamps(v: Vec<Record>) -> Vec<u64> {
        v.iter().map(|r| r.ts).collect()
    }

    #[test]
    fn new_fills_identity_and_defaults() {
        let r = Record::new("mac", "id1", &["cargo".into(), "build".into(), "--release".into()], "/proj");
        assert_eq!(r.command, "cargo build --release");
        assert_eq!((r.origin.as_str(), r.origin_id.as_str(), r.project.as_str()), ("mac", "id1", "/proj"));
        assert_eq!(r.target, "mac", "tant qu'on ne sait pas mieux, la tâche tourne ici");
        assert!(!r.remote && r.exit.is_none() && r.total_ms == 0);
        assert_eq!(r.id.len(), 16);
        assert!(r.ts > 1_700_000_000);
        assert_ne!(r.id, Record::new("mac", "id1", &[], "/proj").id);
    }

    #[test]
    fn new_truncates_long_commands_on_char_boundaries() {
        let r = Record::new("m", "i", &["é".repeat(300)], "/p");
        assert_eq!(r.command.chars().count(), 201);
        assert!(r.command.ends_with('…'));
        let exact = "x".repeat(200);
        assert_eq!(Record::new("m", "i", std::slice::from_ref(&exact), "/p").command, exact);
    }

    #[test]
    fn append_then_read_keeps_order_and_limit() {
        let (_guard, dir) = home("history");
        assert!(read(10).is_empty());
        for n in 1..=5 {
            append(&rec(n));
        }
        assert_eq!(stamps(read(usize::MAX)), [1, 2, 3, 4, 5]);
        assert_eq!(stamps(read(2)), [4, 5], "les plus récentes, de l'ancienne à la récente");
        assert!(read(0).is_empty());
        assert_eq!(read(99).len(), 5);
        let path = dir.join("echoes/history.jsonl");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }

        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("{pas du json\n\n");
        fs::write(&path, text).unwrap();
        append(&rec(6));
        assert_eq!(stamps(read(usize::MAX)), [1, 2, 3, 4, 5, 6], "ligne corrompue ignorée");
    }

    #[test]
    fn oversized_journal_is_pruned_to_keep() {
        let (_guard, dir) = home("history-prune");
        let path = dir.join("echoes/history.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = String::new();
        for n in 1..=20_000u64 {
            text.push_str(&serde_json::to_string(&rec(n)).unwrap());
            text.push('\n');
        }
        assert!(text.len() > 4 << 20);
        fs::write(&path, text).unwrap();
        append(&rec(20_001));
        let all = read(usize::MAX);
        assert_eq!(all.len(), KEEP);
        assert_eq!(all.last().unwrap().ts, 20_001);
        assert_eq!(all.first().unwrap().ts, 20_001 - KEEP as u64 + 1);
    }
}
