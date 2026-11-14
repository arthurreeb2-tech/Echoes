//! Machine qui prête sa puissance : reçoit un dossier et une commande, renvoie le résultat.

use crate::proto::{Entry, Msg, Secure, Status, DEFAULT_PORT};
use crate::{config, discover, status, sync};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn serve(port: Option<u16>) -> Result<()> {
    let key = config::require_key()?;
    let port = port.unwrap_or(DEFAULT_PORT);
    let listener = TcpListener::bind(("0.0.0.0", port)).with_context(|| format!("port {port} indisponible"))?;
    let state = status::monitor()?;
    let (name, id) = {
        let s = state.lock().unwrap();
        (s.name.clone(), s.id.clone())
    };
    let _mdns = discover::advertise(&name, &id, port).map_err(|e| eprintln!("echoes: annonce réseau impossible ({e})")).ok();
    eprintln!("echoes: {name} prête sa puissance sur le port {port}");
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let state = state.clone();
        std::thread::spawn(move || {
            let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
            if let Err(e) = handle(stream, &key, state) {
                eprintln!("echoes: {peer} : {e:#}");
            }
        });
    }
    Ok(())
}

fn handle(stream: TcpStream, key: &[u8; 32], state: Arc<Mutex<Status>>) -> Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    let mut conn = Secure::accept(stream, key)?;
    match conn.recv()? {
        Msg::StatusReq => {
            let st = state.lock().unwrap().clone();
            conn.send(&Msg::Status(st))
        }
        Msg::Run { project, name, manifest, command, pull } => {
            let result = run(&mut conn, &project, &name, manifest, command, pull);
            if let Err(e) = &result {
                let _ = conn.send(&Msg::Error(format!("{e:#}")));
            }
            result
        }
        _ => bail!("demande inattendue"),
    }
}

fn run(conn: &mut Secure, project: &str, name: &str, manifest: Vec<Entry>, command: Vec<String>, pull: Vec<String>) -> Result<()> {
    if command.is_empty() {
        bail!("commande vide");
    }
    let clean = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(40).collect::<String>();
    let base = config::data_dir()?.join("workspaces");
    let ws = base.join(format!("{}-{}", clean(name), clean(project)));
    std::fs::create_dir_all(&ws)?;
    let record = base.join(format!("{}-{}.manifest", clean(name), clean(project)));

    // 1. Aligner le dossier de travail sur celui de la machine d'origine.
    for e in &manifest {
        sync::resolve(&ws, &e.path)?;
    }
    let wanted: HashMap<&str, &Entry> = manifest.iter().map(|e| (e.path.as_str(), e)).collect();
    let previous: Vec<Entry> = std::fs::read(&record).ok().and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default();
    for old in &previous {
        if !wanted.contains_key(old.path.as_str()) {
            if let Ok(p) = sync::resolve(&ws, &old.path) {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    let mut need = Vec::new();
    for e in &manifest {
        let path = sync::resolve(&ws, &e.path)?;
        match sync::hash(&path) {
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
                    bail!("fichier non annoncé : {path}");
                }
                receiver.chunk(Some(sync::resolve(&ws, &path)?), mode, first, last, &data)?;
            }
            Msg::FilesDone => break,
            _ => bail!("message inattendu pendant l'envoi des fichiers"),
        }
    }
    std::fs::write(&record, bincode::serialize(&manifest)?)?;

    // 2. Lancer la commande et renvoyer sa sortie en direct.
    let code = execute(conn, &ws, &command)?;

    // 3. Renvoyer ce que la commande a modifié, puis les chemins demandés.
    for e in sync::scan(&ws, &ws, false)? {
        if wanted.get(e.path.as_str()).map(|w| w.hash) != Some(e.hash) {
            sync::send_file(conn, &ws, &e, false)?;
        }
    }
    for p in &pull {
        let start = sync::resolve(&ws, p.trim_end_matches('/'))?;
        if start.exists() {
            for e in sync::scan(&ws, &start, true)? {
                sync::send_file(conn, &ws, &e, true)?;
            }
        }
    }
    conn.send(&Msg::FilesDone)?;
    conn.send(&Msg::Exit(code))
}

fn spawn(ws: &Path, command: &[String]) -> Result<Child> {
    #[cfg(unix)]
    let mut cmd = {
        use std::os::unix::process::CommandExt;
        // Shell de connexion : retrouve le même PATH que dans un terminal (cargo, node…).
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut c = Command::new(shell);
        c.args(["-lc", "exec \"$@\"", "echoes"]).args(command).process_group(0);
        c
    };
    #[cfg(not(unix))]
    let mut cmd = {
        let mut c = Command::new(&command[0]);
        c.args(&command[1..]);
        c
    };
    cmd.current_dir(ws).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.spawn().with_context(|| format!("impossible de lancer `{}`", command[0]))
}

fn kill(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::killpg(child.id() as i32, libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn execute(conn: &mut Secure, ws: &Path, command: &[String]) -> Result<i32> {
    let mut child = spawn(ws, command)?;
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
    loop {
        // Le Ping sert aussi à détecter un Ctrl-C côté origine : l'envoi échoue, on arrête tout.
        let msg = match rx.recv_timeout(Duration::from_secs(1)) {
            Ok((err, data)) => Msg::Output { err, data },
            Err(mpsc::RecvTimeoutError::Timeout) => Msg::Ping,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if let Err(e) = conn.send(&msg) {
            kill(&mut child);
            return Err(e).context("origine déconnectée, commande arrêtée");
        }
    }
    Ok(child.wait()?.code().unwrap_or(1))
}
