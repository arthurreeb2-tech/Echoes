//! Ce que l'icône affiche : couleur, info-bulle, lignes du menu. Logique pure, sans interface.

use crate::dashboard::Machine;
use crate::history::Record;
use crate::proto::Status;

/// Couleur de la pastille.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Light {
    /// Au moins une autre machine en ligne et libre.
    Green,
    /// Aucune autre machine libre en ligne.
    Orange,
    /// Service local arrêté ou machine pas encore appairée.
    Grey,
}

/// Une ligne de machine dans le menu.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub label: String,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub light: Light,
    pub tooltip: String,
    pub machines: Vec<Row>,
    pub activity: String,
}

/// Nombre au format français, une décimale.
fn fr(x: f64) -> String {
    format!("{x:.1}").replace('.', ",")
}

fn state_label(s: &Status) -> String {
    if s.busy() {
        "occupée".into()
    } else {
        format!("libre · {} cœurs", fr(s.free))
    }
}

pub fn summarize(me: Option<&Status>, known: &[Machine], peers_online: &[Status], records: &[Record], service_up: bool, has_key: bool, now: u64) -> Summary {
    if !has_key {
        return Summary {
            light: Light::Grey,
            tooltip: "Echoes — pas encore appairée".into(),
            machines: vec![],
            activity: "Pas encore appairée : colle le code dans le tableau de bord".into(),
        };
    }

    let mut machines = Vec::new();
    if let Some(me) = me {
        machines.push(Row { label: format!("{} (cette machine) — {}", me.name, state_label(me)), online: true });
    }
    let mut peers: Vec<&Status> = peers_online.iter().filter(|p| me.map_or(true, |m| m.id != p.id)).collect();
    peers.sort_by(|a, b| b.score().total_cmp(&a.score()));
    for p in &peers {
        machines.push(Row { label: format!("{} — {}", p.name, state_label(p)), online: true });
    }
    for m in known {
        let id = &m.status.id;
        if m.me || me.is_some_and(|s| &s.id == id) || peers.iter().any(|p| &p.id == id) {
            continue;
        }
        machines.push(Row { label: format!("{} — hors ligne", m.status.name), online: false });
    }

    let best_free = peers.iter().find(|p| !p.busy());
    let light = match (service_up, best_free) {
        (false, _) => Light::Grey,
        (true, Some(_)) => Light::Green,
        (true, None) => Light::Orange,
    };
    let online = peers.len() + usize::from(me.is_some());
    let tooltip = match (service_up, best_free) {
        (false, _) => "Echoes — service arrêté".to_string(),
        (true, Some(p)) => format!("Echoes — {online} machines en ligne, {} libre", p.name),
        (true, None) => "Echoes — aucune autre machine libre".to_string(),
    };

    Summary { light, tooltip, machines, activity: activity(records, now) }
}

fn activity(records: &[Record], now: u64) -> String {
    let recent: Vec<&Record> = records.iter().filter(|r| r.remote && r.ts >= now.saturating_sub(86_400)).collect();
    if recent.is_empty() {
        return "Aucune tâche déléguée en 24 h".into();
    }
    let gained_ms: u64 = recent.iter().map(|r| r.est_local_ms.saturating_sub(r.total_ms)).sum();
    let n = recent.len();
    let tasks = if n == 1 { "1 tâche".to_string() } else { format!("{n} tâches") };
    let minutes = gained_ms / 60_000;
    let gain = match minutes {
        0 => "moins d'une minute gagnée".to_string(),
        1 => "1 min gagnée".to_string(),
        m => format!("{m} min gagnées"),
    };
    format!("{tasks} en 24 h · {gain}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::Machine;
    use crate::history::Record;
    use crate::proto::Status;

    fn status(id: &str, name: &str, free: f64, gpu: Option<f32>, mem_free: f64) -> Status {
        Status {
            id: id.into(),
            name: name.into(),
            os: "macos".into(),
            arch: "aarch64".into(),
            cores: 8,
            bench: 100.0,
            free,
            mem_gb: 8.0,
            version: "0.4.0".into(),
            gpu,
            mem_free_gb: mem_free,
            jobs: 0,
        }
    }

    fn machine(s: Status, me: bool) -> Machine {
        Machine { score: s.score(), busy: s.busy(), status: s, online: false, me, last_seen: 0 }
    }

    fn record(ts: u64, remote: bool, est_local_ms: u64, total_ms: u64) -> Record {
        Record { ts, remote, est_local_ms, total_ms, ..Default::default() }
    }

    const NOW: u64 = 1_000_000;

    fn imac() -> Status {
        status("imac", "imac", 2.1, Some(99.0), 10.0)
    }

    #[test]
    fn no_key_is_grey() {
        let me = imac();
        let s = summarize(Some(&me), &[], &[], &[], true, false, NOW);
        assert!(matches!(s.light, Light::Grey));
        assert!(s.machines.is_empty());
        assert!(s.activity.starts_with("Pas encore appairée"));
    }

    #[test]
    fn service_down_is_grey_even_with_peers() {
        let me = imac();
        let mini = status("mini", "mini", 7.8, Some(1.0), 5.8);
        let s = summarize(Some(&me), &[], &[mini], &[], false, true, NOW);
        assert!(matches!(s.light, Light::Grey));
        assert_eq!(s.tooltip, "Echoes — service arrêté");
    }

    #[test]
    fn free_peer_is_green() {
        let me = imac();
        let mini = status("mini", "mini", 7.8, Some(1.0), 5.8);
        let s = summarize(Some(&me), &[], &[mini], &[], true, true, NOW);
        assert!(matches!(s.light, Light::Green));
        assert_eq!(s.machines[0].label, "imac (cette machine) — occupée");
        assert_eq!(s.machines[1].label, "mini — libre · 7,8 cœurs");
        assert!(s.machines[1].online);
    }

    #[test]
    fn all_peers_busy_is_orange() {
        let me = imac();
        let mini = status("mini", "mini", 7.8, Some(99.0), 5.8);
        let s = summarize(Some(&me), &[], &[mini], &[], true, true, NOW);
        assert!(matches!(s.light, Light::Orange));
        assert!(s.machines[1].label.contains("occupée"));
    }

    #[test]
    fn offline_peer_never_counts_as_free() {
        let me = imac();
        let known = [machine(me.clone(), true), machine(status("mini", "mini", 7.8, Some(1.0), 5.8), false)];
        let s = summarize(Some(&me), &known, &[], &[], true, true, NOW);
        assert!(matches!(s.light, Light::Orange));
        let row = s.machines.iter().find(|r| r.label.starts_with("mini")).expect("ligne du mini");
        assert!(row.label.ends_with("hors ligne"));
        assert!(!row.online);
        assert_eq!(s.machines.len(), 2, "cette machine n'est pas répétée comme hors ligne");
    }

    #[test]
    fn activity_counts_last_24h_remote_only() {
        let me = imac();
        let recs = [
            record(NOW - 60, true, 150_000, 30_000),
            record(NOW - 3600, true, 100_000, 10_000),
            record(NOW - 7200, true, 40_000, 10_000),
            record(NOW - 100, false, 50_000, 1_000),
            record(NOW - 2 * 86_400, true, 900_000, 1_000),
        ];
        let s = summarize(Some(&me), &[], &[], &recs, true, true, NOW);
        assert_eq!(s.activity, "3 tâches en 24 h · 4 min gagnées");
    }
}
