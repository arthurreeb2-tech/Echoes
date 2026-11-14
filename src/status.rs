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
        mem_gb: sys.total_memory() as f64 / (1u64 << 30) as f64,
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

fn free_cores(sys: &System) -> f64 {
    sys.cpus().len() as f64 * (1.0 - sys.global_cpu_usage() as f64 / 100.0)
}

/// Mesure ponctuelle (côté machine qui lance la commande).
pub fn local() -> Result<Status> {
    let mut st = fixed()?;
    let mut sys = System::new()?;
    sys.refresh_cpu_usage();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();
    st.free = free_cores(&sys);
    Ok(st)
}

/// Mesure continue (côté machine qui prête sa puissance).
pub fn monitor() -> Result<Arc<Mutex<Status>>> {
    let mut st = fixed()?;
    st.free = st.cores as f64;
    let shared = Arc::new(Mutex::new(st));
    let out = shared.clone();
    let mut sys = System::new()?;
    std::thread::spawn(move || {
        sys.refresh_cpu_usage();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            sys.refresh_cpu_usage();
            out.lock().unwrap().free = free_cores(&sys);
        }
    });
    Ok(shared)
}
