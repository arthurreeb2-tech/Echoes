//! Machine qui prête sa puissance : reçoit un dossier et une commande, renvoie le résultat.

use crate::proto::{Entry, Msg, Secure, Status, DEFAULT_PORT};
use crate::{config, discover, status, sync, update};
use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Tâches en cours.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
/// Connexions ouvertes (y compris celles qui n'ont pas encore prouvé qu'elles ont la clé).
static CONNS: AtomicUsize = AtomicUsize::new(0);
static UPDATING: AtomicBool = AtomicBool::new(false);
/// Dossiers de travail occupés : une seule tâche à la fois par projet.
static BUSY_WS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

const MAX_CONNS: usize = 64;

struct Count(&'static AtomicUsize);
impl Count {
    fn new(counter: &'static AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Count(counter)
    }
}
impl Drop for Count {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn bind(port: u16) -> Result<TcpListener> {
    // Après une mise à jour, l'ancienne version peut tenir le port quelques instants.
    let mut last = None;
    for _ in 0..20 {
        match TcpListener::bind(("0.0.0.0", port)) {
            Ok(l) => return Ok(l),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(last.unwrap()).with_context(|| format!("port {port} indisponible"))
}

pub fn serve(port: Option<u16>) -> Result<()> {
    config::require_key()?;
    let port = port.unwrap_or(DEFAULT_PORT);
    let listener = bind(port)?;
    let state = status::monitor()?;
    let (name, id) = {
        let s = state.lock().unwrap();
        (s.name.clone(), s.id.clone())
    };
    let _mdns = discover::advertise(&name, &id, port).map_err(|e| eprintln!("echoes: annonce réseau impossible ({e})")).ok();
    eprintln!("echoes {} : {name} prête sa puissance sur le port {port}", env!("CARGO_PKG_VERSION"));
    crate::dashboard::spawn_server();
    if update::enabled() {
        std::thread::spawn(auto_update);
    }
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            // Plus de descripteurs disponibles, par exemple : on laisse souffler au lieu de tourner à vide.
            std::thread::sleep(Duration::from_millis(200));
            continue;
        };
        if CONNS.load(Ordering::SeqCst) >= MAX_CONNS {
            continue; // connexion fermée aussitôt
        }
        let conn = Count::new(&CONNS);
        let state = state.clone();
        std::thread::spawn(move || {
            let _conn = conn;
            let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
            if let Err(e) = handle(stream, state) {
                eprintln!("echoes: {peer} : {e:#}");
            }
        });
    }
    Ok(())
}

/// Vérifie les nouvelles versions toutes les 6 h, et ne remplace le programme que lorsqu'aucune tâche ne tourne.
fn auto_update() {
    let delay = std::env::var("ECHOES_UPDATE_DELAY").ok().and_then(|s| s.parse().ok()).unwrap_or(90);
    std::thread::sleep(Duration::from_secs(delay));
    loop {
        match update::check() {
            Ok(Some(release)) => loop {
                UPDATING.store(true, Ordering::SeqCst);
                if can_update(ACTIVE.load(Ordering::SeqCst)) {
                    match update::install(&release) {
                        Ok(()) => {
                            eprintln!("echoes: mise à jour vers {} installée, redémarrage", release.version);
                            update::restart();
                        }
                        Err(e) => eprintln!("echoes: mise à jour impossible : {e:#}"),
                    }
                    UPDATING.store(false, Ordering::SeqCst);
                    break;
                }
                UPDATING.store(false, Ordering::SeqCst);
                std::thread::sleep(Duration::from_secs(30));
            },
            Ok(None) => {}
            Err(e) => eprintln!("echoes: vérification des mises à jour impossible : {e:#}"),
        }
        std::thread::sleep(Duration::from_secs(6 * 3600));
    }
}

fn handle(stream: TcpStream, state: Arc<Mutex<Status>>) -> Result<()> {
    stream.set_nodelay(true)?;
    // Sans la clé, on n'occupe pas le service plus de quelques secondes.
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    // Clé relue à chaque connexion : un nouvel appairage est pris en compte sans redémarrage.
    let key = config::require_key()?;
    let mut conn = Secure::accept(stream, &key)?;
    let request = conn.recv()?;
    conn.stream().set_read_timeout(Some(Duration::from_secs(60)))?;
    let snapshot = || {
        let mut st = state.lock().unwrap().clone();
        st.jobs = ACTIVE.load(Ordering::SeqCst) as u32;
        st
    };
    match request {
        Msg::StatusReq => conn.send(&Msg::Status(snapshot())),
        Msg::HistoryReq => conn.send(&Msg::History(crate::history::read(300))),
        Msg::Run { project, name, manifest, cwd, command, pull } => {
            let st = snapshot();
            // Admission : on ne sature pas une machine déjà prise par son propre travail.
            if let Err(reason) = admit(&st, UPDATING.load(Ordering::SeqCst)) {
                return conn.send(&Msg::Error(reason));
            }
            let _job = Count::new(&ACTIVE);
            let result = run(&mut conn, &project, &name, manifest, &cwd, command, pull, &st);
            if let Err(e) = &result {
                let _ = conn.send(&Msg::Error(format!("{e:#}")));
            }
            result
        }
        _ => bail!("demande inattendue"),
    }
}

/// Admission d'une tâche : refus (avec la raison) pendant une mise à jour ou quand la machine est pleine.
fn admit(st: &Status, updating: bool) -> Result<(), String> {
    if updating {
        return Err("mise à jour en cours".into());
    }
    if st.jobs >= st.max_jobs() {
        return Err(format!("occupée ({} tâche(s) en cours, {})", st.jobs, st.load_note()));
    }
    Ok(())
}

/// Le programme ne se remplace que lorsqu'aucune tâche ne tourne.
fn can_update(active_jobs: usize) -> bool {
    active_jobs == 0
}

/// Réserve le dossier de travail d'un projet ; libéré quand la valeur disparaît.
struct WsLock(String);
impl WsLock {
    /// `None` si une autre tâche tient déjà ce projet.
    fn try_acquire(tag: &str) -> Option<Self> {
        BUSY_WS.lock().unwrap().get_or_insert_with(HashSet::new).insert(tag.to_string()).then(|| WsLock(tag.to_string()))
    }

    /// Attend son tour si une autre tâche du même projet tourne, en donnant signe de vie.
    fn acquire(tag: &str, conn: &mut Secure) -> Result<Self> {
        loop {
            if let Some(lock) = Self::try_acquire(tag) {
                return Ok(lock);
            }
            conn.keepalive()?;
            std::thread::sleep(Duration::from_millis(300));
        }
    }
}
impl Drop for WsLock {
    fn drop(&mut self) {
        if let Some(set) = BUSY_WS.lock().unwrap().as_mut() {
            set.remove(&self.0);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run(conn: &mut Secure, project: &str, name: &str, manifest: Vec<Entry>, cwd: &str, command: Vec<String>, pull: Vec<String>, st: &Status) -> Result<()> {
    if command.is_empty() {
        bail!("commande vide");
    }
    let clean = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(40).collect::<String>();
    let base = config::data_dir()?.join("workspaces");
    let tag = format!("{}-{}", clean(name), clean(project));
    let ws = base.join(&tag);
    std::fs::create_dir_all(&ws)?;
    let _lock = WsLock::acquire(&tag, conn)?;
    let record = base.join(format!("{tag}.manifest"));
    let mut cache = sync::HashCache::load(base.join(format!("{tag}.cache")));

    // 1. Aligner le dossier de travail sur celui de la machine d'origine.
    for e in &manifest {
        sync::safe_dest(&ws, &e.path)?;
    }
    let wanted: HashMap<&str, &Entry> = manifest.iter().map(|e| (e.path.as_str(), e)).collect();
    let previous: Vec<Entry> = std::fs::read(&record).ok().and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default();
    for old in &previous {
        if !wanted.contains_key(old.path.as_str()) {
            if let Ok(p) = sync::safe_dest(&ws, &old.path) {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    let mut need = Vec::new();
    for e in &manifest {
        conn.keepalive()?;
        let path = sync::safe_dest(&ws, &e.path)?;
        match cache.hash(&e.path, &path) {
            Ok(h) if h == e.hash => sync::set_mode(&path, e.mode),
            _ => need.push(e.path.clone()),
        }
    }
    conn.send(&Msg::Need(need))?;
    let mut receiver = sync::Receiver::new();
    loop {
        match conn.recv()? {
            Msg::File { path, mode, first, last, data, .. } => {
                if !wanted.contains_key(path.as_str()) {
                    bail!("fichier non annoncé : {path}");
                }
                receiver.chunk(Some(sync::safe_dest(&ws, &path)?), mode, first, last, &data)?;
            }
            Msg::FilesDone => break,
            _ => bail!("message inattendu pendant l'envoi des fichiers"),
        }
    }
    drop(receiver);
    let tmp = record.with_extension("manifest.tmp");
    std::fs::write(&tmp, bincode::serialize(&manifest)?)?;
    std::fs::rename(&tmp, &record)?;

    // 2. Lancer la commande, dans le même sous-dossier qu'à l'origine, et renvoyer sa sortie en direct.
    let dir = sync::resolve(&ws, cwd)?;
    std::fs::create_dir_all(&dir)?;
    let code = execute(conn, &dir, &command, st)?;

    // 3. Renvoyer ce que la commande a modifié, puis les chemins demandés.
    let changed = sync::scan(&ws, &ws, false, &mut cache, &mut || conn.keepalive())?;
    for e in changed {
        if wanted.get(e.path.as_str()).map(|w| w.hash) != Some(e.hash) {
            sync::send_file(conn, &ws, &e, false)?;
        }
    }
    for p in &pull {
        let start = sync::resolve(&ws, p)?;
        if start.exists() {
            let found = sync::scan(&ws, &start, true, &mut cache, &mut || conn.keepalive())?;
            for e in found {
                sync::send_file(conn, &ws, &e, true)?;
            }
        }
    }
    cache.save();
    conn.send(&Msg::FilesDone)?;
    conn.send(&Msg::Exit(code))
}

/// Quand la machine est prise par un travail lourd, la tâche n'a droit qu'à la moitié des cœurs.
fn limited_jobs(st: &Status) -> Option<u32> {
    st.busy().then(|| (st.cores / 2).max(1))
}

fn spawn(dir: &Path, command: &[String], st: &Status) -> Result<Child> {
    #[cfg(unix)]
    let mut cmd = {
        use std::os::unix::process::CommandExt;
        // Shell de connexion : retrouve le même PATH que dans un terminal (cargo, node…).
        let fallback = if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/sh" };
        let shell = std::env::var("SHELL").ok().filter(|s| !s.ends_with("fish") && Path::new(s).exists()).unwrap_or_else(|| fallback.into());
        let mut c = Command::new(shell);
        c.args(["-lc", "exec \"$@\"", "echoes"]).args(command).process_group(0);
        // Priorité basse, et la plus basse possible si la machine est déjà prise : elle reste fluide.
        let nice = if st.busy() { 19 } else { 10 };
        unsafe {
            c.pre_exec(move || {
                libc::nice(nice);
                Ok(())
            });
        }
        c
    };
    #[cfg(not(unix))]
    let mut cmd = {
        let mut c = Command::new(&command[0]);
        c.args(&command[1..]);
        c
    };
    if let Some(n) = limited_jobs(st) {
        // Les outils de build courants lisent ces variables pour limiter leur parallélisme.
        cmd.env("CARGO_BUILD_JOBS", n.to_string())
            .env("MAKEFLAGS", format!("-j{n}"))
            .env("CMAKE_BUILD_PARALLEL_LEVEL", n.to_string())
            .env("ECHOES_MAX_JOBS", n.to_string());
    }
    cmd.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.spawn().with_context(|| format!("impossible de lancer `{}`", command[0]))
}

fn kill(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::killpg(child.id() as i32, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        // /T : avec tous les processus lancés par la commande.
        let _ = Command::new("taskkill").args(["/F", "/T", "/PID", &child.id().to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn exit_code(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return 128 + sig;
        }
    }
    status.code().unwrap_or(1)
}

fn execute(conn: &mut Secure, dir: &Path, command: &[String], st: &Status) -> Result<i32> {
    let mut child = spawn(dir, command, st)?;
    let (tx, rx) = mpsc::channel::<(bool, Vec<u8>)>();
    let pump = |mut src: Box<dyn Read + Send>, err: bool, tx: mpsc::Sender<(bool, Vec<u8>)>| {
        std::thread::spawn(move || {
            let mut buf = [0u8; 16384];
            while let Ok(n) = src.read(&mut buf) {
                if n == 0 || tx.send((err, buf[..n].to_vec())).is_err() {
                    break;
                }
            }
        });
    };
    pump(Box::new(child.stdout.take().unwrap()), false, tx.clone());
    pump(Box::new(child.stderr.take().unwrap()), true, tx);
    let mut exited: Option<(std::process::ExitStatus, Instant)> = None;
    loop {
        // Le signe de vie sert aussi à détecter un Ctrl-C côté origine : l'envoi échoue, on arrête tout.
        let sent = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok((err, data)) => {
                // Tant que de la sortie arrive, on ne coupe pas : le délai repart.
                if let Some((_, at)) = &mut exited {
                    *at = Instant::now();
                }
                conn.send(&Msg::Output { err, data })
            }
            Err(mpsc::RecvTimeoutError::Timeout) => conn.keepalive(),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if let Err(e) = sent {
            kill(&mut child);
            return Err(e).context("origine déconnectée, commande arrêtée");
        }
        // La tâche est finie quand la commande est finie, même si un processus d'arrière-plan
        // qu'elle a lancé garde sa sortie ouverte : on s'arrête après 600 ms sans nouvelle sortie.
        if exited.is_none() {
            if let Some(status) = child.try_wait()? {
                exited = Some((status, Instant::now()));
            }
        }
        if matches!(exited, Some((_, at)) if at.elapsed() > Duration::from_millis(600)) {
            break;
        }
    }
    let status = match exited {
        Some((s, _)) => s,
        None => child.wait()?,
    };
    Ok(exit_code(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(cores: u32, gpu: Option<f32>, mem_free_gb: f64, jobs: u32) -> Status {
        Status { id: "id".into(), name: "m".into(), os: "linux".into(), arch: "x86_64".into(), cores, bench: 1.0, free: cores as f64, mem_gb: 16.0, version: "0".into(), gpu, mem_free_gb, jobs }
    }

    #[test]
    fn admits_until_the_job_limit() {
        // 8 cœurs : 2 tâches en parallèle.
        assert!(admit(&status(8, None, 8.0, 0), false).is_ok());
        assert!(admit(&status(8, None, 8.0, 1), false).is_ok());
        let refused = admit(&status(8, None, 8.0, 2), false).unwrap_err();
        assert!(refused.starts_with("occupée (2 tâche(s) en cours"), "{refused}");
    }

    #[test]
    fn small_machine_takes_one_job() {
        assert!(admit(&status(2, None, 8.0, 0), false).is_ok());
        assert!(admit(&status(2, None, 8.0, 1), false).is_err());
    }

    #[test]
    fn busy_machine_takes_one_job_only() {
        let gpu_busy = status(16, Some(80.0), 8.0, 1);
        assert!(admit(&gpu_busy, false).is_err());
        let low_memory = status(16, None, 0.5, 1);
        assert!(admit(&low_memory, false).is_err());
        assert!(admit(&status(16, Some(80.0), 8.0, 0), false).is_ok());
    }

    #[test]
    fn refuses_during_update() {
        assert_eq!(admit(&status(8, None, 8.0, 0), true).unwrap_err(), "mise à jour en cours");
    }

    #[test]
    fn throttles_only_when_busy() {
        assert_eq!(limited_jobs(&status(8, None, 8.0, 0)), None);
        assert_eq!(limited_jobs(&status(8, Some(90.0), 8.0, 0)), Some(4));
        assert_eq!(limited_jobs(&status(8, None, 0.2, 0)), Some(4));
        assert_eq!(limited_jobs(&status(1, Some(90.0), 8.0, 0)), Some(1));
        assert_eq!(limited_jobs(&status(0, Some(90.0), 8.0, 0)), Some(1));
    }

    #[test]
    fn update_only_when_no_job_runs() {
        assert!(can_update(0));
        assert!(!can_update(1));
        assert!(!can_update(5));
    }

    #[test]
    fn one_task_at_a_time_per_project() {
        let first = WsLock::try_acquire("test-projet-a").expect("libre");
        assert!(WsLock::try_acquire("test-projet-a").is_none());
        let other = WsLock::try_acquire("test-projet-b").expect("autre projet libre");
        drop(first);
        let again = WsLock::try_acquire("test-projet-a").expect("libéré");
        drop((other, again));
        assert!(WsLock::try_acquire("test-projet-b").is_some());
    }

    #[test]
    fn counter_guard_counts_and_releases() {
        static N: AtomicUsize = AtomicUsize::new(0);
        let a = Count::new(&N);
        let b = Count::new(&N);
        assert_eq!(N.load(Ordering::SeqCst), 2);
        drop(a);
        assert_eq!(N.load(Ordering::SeqCst), 1);
        drop(b);
        assert_eq!(N.load(Ordering::SeqCst), 0);
    }

    #[cfg(unix)]
    #[test]
    fn exit_code_follows_shell_convention() {
        let run = |script: &str| Command::new("sh").args(["-c", script]).status().unwrap();
        assert_eq!(exit_code(run("exit 7")), 7);
        assert_eq!(exit_code(run("kill -9 $$")), 137);
    }
}
