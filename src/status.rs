//! Mesure de la puissance de la machine : vitesse d'un cœur et cœurs libres.

use crate::config;
use crate::proto::Status;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::System;

/// Petit calcul chronométré, mis en cache : sert à comparer les machines.
fn bench() -> f64 {
    let path = config::data_dir().ok().map(|d| d.join("bench"));
    if let Some(v) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| s.trim().parse().ok()) {
        return v;
    }
    let mut best = 0f64;
    for _ in 0..3 {
        let start = Instant::now();
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut acc = 0u64;
        for _ in 0..20_000_000u32 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            acc = acc.wrapping_add(x % 1000);
        }
        std::hint::black_box(acc);
        best = best.max(20.0 / start.elapsed().as_secs_f64());
    }
    let v = (best * 10.0).round() / 10.0;
    if let Some(p) = path {
        let _ = std::fs::write(p, v.to_string());
    }
    v
}

/// Nom affiché : celui du fichier `name` de la configuration s'il existe, sinon le nom réseau.
pub fn host_name() -> String {
    if let Some(n) = config::config_dir().ok().and_then(|d| std::fs::read_to_string(d.join("name")).ok()) {
        if !n.trim().is_empty() {
            return n.trim().chars().take(40).collect();
        }
    }
    System::host_name().unwrap_or_else(|_| "machine".into()).trim_end_matches(".local").to_string()
}

fn fixed() -> Result<Status> {
    let mut sys = System::new()?;
    sys.refresh_memory();
    sys.refresh_cpu_usage();
    Ok(Status {
        id: config::machine_id()?,
        name: host_name(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        cores: sys.cpus().len().max(1) as u32,
        bench: bench(),
        free: 0.0,
        mem_gb: sys.total_memory() as f64 / GB,
        version: env!("CARGO_PKG_VERSION").into(),
        gpu: None,
        mem_free_gb: sys.available_memory() as f64 / GB,
        jobs: 0,
    })
}

const GB: f64 = (1u64 << 30) as f64;

fn free_cores(sys: &System) -> f64 {
    sys.cpus().len() as f64 * (1.0 - sys.global_cpu_usage() as f64 / 100.0)
}

fn quiet(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // pas de fenêtre de console
    }
    let out = cmd.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Plus forte valeur `"Device Utilization %"=N` dans la sortie de `ioreg` (une entrée par carte graphique).
fn parse_ioreg(text: &str) -> Option<f32> {
    const KEY: &str = "\"Device Utilization %\"=";
    text.match_indices(KEY)
        .filter_map(|(i, _)| {
            let rest = &text[i + KEY.len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse::<f32>().ok()
        })
        .reduce(f32::max)
}

/// Plus forte valeur parmi des lignes contenant chacune un pourcentage (une ligne par carte).
fn parse_lines(text: &str) -> Option<f32> {
    text.lines().filter_map(|l| l.trim().trim_end_matches('%').trim().parse::<f32>().ok()).reduce(f32::max)
}

/// Occupation de la carte graphique en %, ou `None` si elle n'est pas mesurable sur cette machine.
pub fn gpu_percent() -> Option<f32> {
    // Pour les tests : simule une carte graphique occupée.
    if let Some(v) = std::env::var("ECHOES_FAKE_GPU").ok().and_then(|v| v.parse::<f32>().ok()) {
        return Some(v.clamp(0.0, 100.0));
    }
    let value = if cfg!(target_os = "macos") {
        parse_ioreg(&quiet("ioreg", &["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"])?)
    } else {
        quiet("nvidia-smi", &["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"]).and_then(|t| parse_lines(&t)).or_else(|| {
            // Pilotes AMD sous Linux.
            let cards = std::fs::read_dir("/sys/class/drm").ok()?;
            cards.flatten().filter_map(|c| std::fs::read_to_string(c.path().join("device/gpu_busy_percent")).ok()).filter_map(|t| parse_lines(&t)).reduce(f32::max)
        })
    };
    value.map(|v| v.clamp(0.0, 100.0))
}

/// Mesure ponctuelle (côté machine qui lance la commande).
pub fn local() -> Result<Status> {
    let gpu = std::thread::spawn(gpu_percent);
    let mut st = fixed()?;
    let mut sys = System::new()?;
    sys.refresh_cpu_usage();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();
    st.free = free_cores(&sys);
    st.gpu = gpu.join().ok().flatten();
    Ok(st)
}

/// Mesure continue (côté machine qui prête sa puissance) : processeur chaque seconde,
/// carte graphique et mémoire toutes les 4 secondes.
pub fn monitor() -> Result<Arc<Mutex<Status>>> {
    let mut st = fixed()?;
    st.free = st.cores as f64;
    st.gpu = gpu_percent();
    let shared = Arc::new(Mutex::new(st));
    let out = shared.clone();
    let mut sys = System::new()?;
    std::thread::spawn(move || {
        sys.refresh_cpu_usage();
        let mut previous_gpu: Option<f32> = None;
        let mut tick: u64 = 0;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            sys.refresh_cpu_usage();
            let free = free_cores(&sys);
            if tick.is_multiple_of(4) {
                sys.refresh_memory();
                let gpu = gpu_percent();
                // On retient la plus forte des deux dernières mesures : un rendu qui travaille par à-coups reste « occupé ».
                let held = match (gpu, previous_gpu) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, _) => a,
                };
                previous_gpu = gpu;
                let mut s = out.lock().unwrap();
                s.gpu = held;
                s.mem_free_gb = sys.available_memory() as f64 / GB;
            }
            out.lock().unwrap().free = free;
            tick = tick.wrapping_add(1);
        }
    });
    Ok(shared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_parsers() {
        let ioreg = r#"| "PerformanceStatistics" = {"In use system memory"=412,"Device Utilization %"=87,"Renderer Utilization %"=80}
          | "PerformanceStatistics" = {"Device Utilization %"=3}"#;
        assert_eq!(parse_ioreg(ioreg), Some(87.0));
        assert_eq!(parse_ioreg("rien"), None);
        assert_eq!(parse_lines("12\n 64 %\n"), Some(64.0));
        assert_eq!(parse_lines("[N/A]\n"), None);
    }

    #[test]
    fn score_reserves_power_for_the_user() {
        let mut s = Status { id: "a".into(), name: "a".into(), os: "macos".into(), arch: "aarch64".into(), cores: 8, bench: 100.0, free: 8.0, mem_gb: 16.0, version: "0".into(), gpu: Some(5.0), mem_free_gb: 8.0, jobs: 0 };
        let idle = s.score();
        s.gpu = Some(95.0);
        assert!(s.busy() && s.max_jobs() == 1);
        assert!((s.score() / idle - 0.4).abs() < 0.01, "carte graphique saturée : 40 % de la puissance au plus");
        s.gpu = None;
        s.mem_free_gb = 0.6;
        assert!(s.busy() && s.score() < idle * 0.3, "mémoire presque pleine : forte pénalité");
    }
}
