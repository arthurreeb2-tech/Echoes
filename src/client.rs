//! Machine qui lance la commande : choisit où elle tourne et rapatrie le résultat.

use crate::proto::{Entry, Msg, Secure, Status};
use crate::history::{self, Record};
use crate::{config, discover, status, sync};
use std::path::PathBuf;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// Une machine distante doit être nettement meilleure pour compenser le transfert.
const REMOTE_ADVANTAGE: f64 = 1.3;

pub struct RunOpts {
    pub host: Option<String>,
    pub local: bool,
    pub remote: bool,
    pub pull: Vec<String>,
    pub quiet: bool,
    pub command: Vec<String>,
}

fn connect(addr: SocketAddr, key: &[u8; 32]) -> Result<Secure> {
    let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(600))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    Secure::connect(stream, key)
}

fn probe(addr: SocketAddr, key: &[u8; 32]) -> Result<Status> {
    let mut conn = connect(addr, key)?;
    conn.send(&Msg::StatusReq)?;
    match conn.recv()? {
        Msg::Status(s) => Ok(s),
        Msg::Error(e) => bail!(e),
        _ => bail!("réponse inattendue"),
    }
}

pub fn fetch_history(addr: SocketAddr, key: &[u8; 32]) -> Vec<Record> {
    let Ok(mut conn) = connect(addr, key) else { return vec![] };
    if conn.send(&Msg::HistoryReq).is_err() {
        return vec![];
    }
    match conn.recv() {
        Ok(Msg::History(h)) => h,
        _ => vec![],
    }
}

fn probe_all(addrs: &[SocketAddr], key: &[u8; 32], me: &str) -> Vec<(SocketAddr, Status)> {
    let handles: Vec<_> = addrs.iter().map(|&a| (a, { let k = *key; std::thread::spawn(move || probe(a, &k)) })).collect();
    let mut out: Vec<(SocketAddr, Status)> = Vec::new();
    for (addr, h) in handles {
        if let Ok(Ok(st)) = h.join() {
            if st.id != me && !out.iter().any(|(_, o)| o.id == st.id) {
                out.push((addr, st));
            }
        }
    }
    out
}

fn static_hosts() -> Vec<SocketAddr> {
    let list = std::env::var("ECHOES_HOSTS").unwrap_or_default();
    list.split(',').filter(|s| !s.trim().is_empty()).filter_map(|s| parse_host(s).ok()).collect()
}

fn parse_host(s: &str) -> Result<SocketAddr> {
    let s = s.trim();
    let full = if s.contains(':') { s.to_string() } else { format!("{s}:{}", crate::proto::DEFAULT_PORT) };
    full.to_socket_addrs()?.next().ok_or_else(|| anyhow!("machine introuvable : {s}"))
}

/// Machines joignables. `fresh` force une recherche réseau au lieu des dernières machines connues.
pub fn find(key: &[u8; 32], me: &str, fresh: bool) -> Vec<(SocketAddr, Status)> {
    let mut known = static_hosts();
    known.extend(config::load_peers());
    let mut found = if fresh { vec![] } else { probe_all(&known, key, me) };
    if found.is_empty() {
        known.extend(discover::browse(Duration::from_millis(if fresh { 1500 } else { 800 })));
        known.dedup();
        found = probe_all(&known, key, me);
        config::save_peers(&found.iter().map(|(a, _)| *a).collect::<Vec<_>>());
    }
    found.sort_by(|a, b| b.1.score().total_cmp(&a.1.score()));
    found
}

pub fn list() -> Result<()> {
    let key = config::require_key()?;
    let me = status::local()?;
    let line = |s: &Status, place: &str| {
        println!("{:<22} {:<16} v{:<7} {:>2} cœurs  {:>4.1} libres  {:>3.0} Go  score {:>6.0}  {}", s.name, format!("{}/{}", s.os, s.arch), s.version, s.cores, s.free, s.mem_gb, s.score(), place)
    };
    line(&me, "cette machine");
    let others = find(&key, &me.id, true);
    for (addr, s) in &others {
        line(s, &addr.to_string());
    }
    if others.is_empty() {
        println!("Aucune autre machine trouvée. Sur les autres : `echoes pair <code>` puis `echoes serve`.");
    }
    Ok(())
}

fn run_local(command: &[String], rec: &mut Record, reason: &str) -> Result<i32> {
    rec.remote = false;
    rec.reason = reason.into();
    let start = Instant::now();
    let status = Command::new(&command[0]).args(&command[1..]).status().with_context(|| format!("impossible de lancer `{}`", command[0]))?;
    rec.command_ms = start.elapsed().as_millis() as u64;
    rec.est_local_ms = rec.command_ms;
    Ok(status.code().unwrap_or(1))
}

pub fn run(opts: RunOpts) -> Result<i32> {
    let start = Instant::now();
    let cwd = std::env::current_dir()?;
    let project = project_root(&cwd).file_name().and_then(|n| n.to_str()).unwrap_or("projet").to_string();
    let me_name = status::host_name();
    let mut rec = Record::new(&me_name, &config::machine_id()?, &opts.command, &project);
    let result = choose_and_run(&opts, &mut rec);
    rec.total_ms = start.elapsed().as_millis() as u64;
    rec.exit = result.as_ref().ok().copied();
    if !rec.remote {
        rec.est_local_ms = rec.total_ms;
    }
    history::append(&rec);
    result
}

fn choose_and_run(opts: &RunOpts, rec: &mut Record) -> Result<i32> {
    if opts.local {
        return run_local(&opts.command, rec, "exécution locale demandée");
    }
    let key = config::require_key()?;
    let note = |m: &str| {
        if opts.quiet {
        } else if std::io::stderr().is_terminal() {
            eprintln!("\x1b[2mechoes: {m}\x1b[0m");
        } else {
            eprintln!("echoes: {m}");
        }
    };
    let me = std::thread::spawn(status::local);
    let my_id = config::machine_id()?;
    let candidates = match &opts.host {
        Some(h) => {
            let addr = parse_host(h)?;
            vec![(addr, probe(addr, &key).with_context(|| format!("{h} injoignable"))?)]
        }
        // Mêmes outils, mêmes chemins, mêmes binaires : on ne délègue qu'à une machine du même système.
        None => find(&key, &my_id, false).into_iter().filter(|(_, s)| s.os == std::env::consts::OS).collect(),
    };
    let me = me.join().map_err(|_| anyhow!("mesure locale impossible"))??;
    let forced = opts.remote || opts.host.is_some();
    let Some((addr, target)) = candidates.into_iter().next() else {
        if forced {
            bail!("aucune machine disponible sur le réseau");
        }
        note("aucune autre machine disponible, exécution ici");
        return run_local(&opts.command, rec, "aucune autre machine disponible");
    };
    if !forced && target.score() < me.score() * REMOTE_ADVANTAGE {
        note(&format!("cette machine est la plus disponible (score {:.0} contre {:.0} pour {}), exécution ici", me.score(), target.score(), target.name));
        return run_local(&opts.command, rec, &format!("plus disponible ici que {}", target.name));
    }
    match remote(addr, &target, &key, opts, &me, &note, rec)? {
        Some(code) => Ok(code),
        None if forced => bail!("{} n'accepte pas de tâche pour l'instant", target.name),
        None => {
            note(&format!("{} indisponible pour l'instant, exécution ici", target.name));
            run_local(&opts.command, rec, &format!("{} indisponible", target.name))
        }
    }
}

/// Dossier à synchroniser : la racine du dépôt git si on est dedans, sinon le dossier courant.
fn project_root(cwd: &Path) -> PathBuf {
    let home = dirs::home_dir();
    cwd.ancestors()
        .take_while(|d| Some(*d) != home.as_deref())
        .find(|d| d.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

/// Chemin donné à `--pull` (relatif au dossier courant) → chemin relatif à la racine du projet.
fn pull_path(root: &Path, cwd: &Path, p: &str) -> Result<String> {
    let mut out = PathBuf::new();
    for c in cwd.join(p).components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    sync::rel(root, &out).ok_or_else(|| anyhow!("--pull {p} : chemin hors du projet"))
}

/// Garde-fous : jamais le dossier personnel entier, ni un projet démesuré.
fn too_big(root: &Path, manifest: &[Entry]) -> Option<String> {
    if Some(root) == dirs::home_dir().as_deref() || root.parent().is_none() {
        return Some("le dossier courant est le dossier personnel ou la racine du disque".into());
    }
    let size: u64 = manifest.iter().map(|e| e.size).sum();
    if manifest.len() > 50_000 || size > 4 << 30 {
        return Some(format!("projet trop volumineux ({} fichiers, {:.1} Go) : ajoute un .echoesignore", manifest.len(), size as f64 / (1u64 << 30) as f64));
    }
    None
}

/// `Ok(None)` : la machine a refusé la tâche avant de la commencer (on peut la faire ici à la place).
fn remote(addr: SocketAddr, target: &Status, key: &[u8; 32], opts: &RunOpts, me: &Status, note: &impl Fn(&str), rec: &mut Record) -> Result<Option<i32>> {
    let cwd = std::env::current_dir()?;
    let root = project_root(&cwd);
    let project = blake3::hash(format!("{}:{}", me.id, root.display()).as_bytes()).to_hex()[..12].to_string();
    let mut cache = sync::HashCache::load(config::data_dir()?.join(format!("cache-{project}")));
    let manifest = sync::scan(&root, &root, false, &mut cache)?;
    cache.save();
    if let Some(reason) = too_big(&root, &manifest) {
        if opts.remote || opts.host.is_some() {
            bail!("{reason}");
        }
        note(&format!("{reason}, exécution ici"));
        return run_local(&opts.command, rec, &reason).map(Some);
    }
    let rel_cwd = sync::rel(&root, &cwd).unwrap_or_default();
    let pull = opts.pull.iter().map(|p| pull_path(&root, &cwd, p)).collect::<Result<Vec<_>>>()?;
    let by_path: HashMap<&str, &Entry> = manifest.iter().map(|e| (e.path.as_str(), e)).collect();
    let name = root.file_name().and_then(|n| n.to_str()).unwrap_or("projet").to_string();

    let Ok(mut conn) = connect(addr, key) else { return Ok(None) };
    conn.stream().set_read_timeout(Some(Duration::from_secs(120)))?;
    conn.send(&Msg::Run { project, name, manifest: manifest.clone(), cwd: rel_cwd, command: opts.command.clone(), pull })?;
    let need = match conn.recv()? {
        Msg::Need(n) => n,
        Msg::Error(e) => {
            note(&format!("{} : {e}", target.name));
            return Ok(None);
        }
        _ => bail!("réponse inattendue"),
    };
    rec.remote = true;
    rec.target = target.name.clone();
    note(&format!("→ {} ({}/{}, {:.1} cœurs libres), {} fichier(s) à envoyer sur {}", target.name, target.os, target.arch, target.free, need.len(), manifest.len()));
    for path in &need {
        let entry = by_path.get(path.as_str()).ok_or_else(|| anyhow!("fichier inconnu demandé : {path}"))?;
        sync::send_file(&mut conn, &root, entry, false)?;
        rec.files_sent += 1;
        rec.sent_bytes += entry.size;
    }
    conn.send(&Msg::FilesDone)?;
    conn.stream().set_read_timeout(Some(Duration::from_secs(30)))?;
    let started = Instant::now();
    let mut finished: Option<Instant> = None;

    let mut receiver = sync::Receiver::new();
    let (mut updated, mut skipped) = (0usize, Vec::new());
    loop {
        let msg = conn.recv().with_context(|| format!("connexion perdue avec {}", target.name))?;
        if finished.is_none() && matches!(msg, Msg::File { .. } | Msg::FilesDone | Msg::Exit(_)) {
            finished = Some(Instant::now());
        }
        match msg {
            Msg::Output { err: false, data } => {
                let mut out = std::io::stdout().lock();
                out.write_all(&data)?;
                out.flush()?;
            }
            Msg::Output { err: true, data } => std::io::stderr().write_all(&data)?,
            Msg::Ping | Msg::FilesDone => {}
            Msg::File { path, mode, first, last, pulled, data } => {
                let dest = sync::resolve_file(&root, &path)?;
                if first {
                    let keep = pulled || safe_to_write(&dest, by_path.get(path.as_str()).copied());
                    if keep {
                        updated += 1;
                    } else {
                        skipped.push(path.clone());
                    }
                    receiver.chunk(keep.then_some(dest), mode, first, last, &data)?;
                } else {
                    receiver.chunk(None, mode, first, last, &data)?;
                }
            }
            Msg::Exit(code) => {
                rec.files_back = updated as u32;
                rec.command_ms = finished.unwrap_or_else(Instant::now).duration_since(started).as_millis() as u64;
                // Estimation : la même commande ici, au prorata de la puissance disponible de part et d'autre.
                let ratio = (target.score() / me.score().max(1.0)).clamp(0.25, 8.0);
                rec.est_local_ms = (rec.command_ms as f64 * ratio) as u64;
                if updated > 0 {
                    note(&format!("← {updated} fichier(s) rapatrié(s)"));
                }
                for p in skipped {
                    eprintln!("echoes: {p} a changé ici pendant l'exécution, version distante ignorée");
                }
                return Ok(Some(code));
            }
            Msg::Error(e) => bail!("{} : {e}", target.name),
            _ => bail!("message inattendu"),
        }
    }
}

/// Un fichier modifié à distance ne remplace le fichier local que si celui-ci n'a pas bougé depuis l'envoi.
fn safe_to_write(dest: &Path, sent: Option<&Entry>) -> bool {
    match (sent, sync::hash(dest)) {
        (Some(e), Ok(h)) => h == e.hash,
        (Some(_), Err(_)) => false,
        (None, Ok(_)) => false,
        (None, Err(_)) => !dest.exists(),
    }
}
