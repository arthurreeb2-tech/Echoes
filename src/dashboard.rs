//! Tableau de bord : page locale (127.0.0.1 seulement), ouverte uniquement sur demande.
//!
//! Accès : `echoes dashboard` fabrique un billet à usage unique, valable 60 s, signé avec un secret
//! que seul ce compte peut lire. La page l'échange contre une session gardée en mémoire. Rien de
//! durable ne passe donc par la ligne de commande, la sortie standard ou l'historique du navigateur.

use crate::history::Record;
use crate::proto::Status;
use crate::{client, config, history, status};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PORT: u16 = 47601;
const PAGE: &str = include_str!("dashboard.html");
const MAX_CONNS: usize = 32;
const SESSION: Duration = Duration::from_secs(12 * 3600);
static CONNS: AtomicUsize = AtomicUsize::new(0);

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Secret du tableau de bord, propre à ce compte sur cette machine.
fn secret() -> Result<[u8; 32]> {
    let path = config::data_dir()?.join("dashboard-secret");
    if let Some(k) = std::fs::read_to_string(&path).ok().and_then(|t| hex::decode(t.trim()).ok()).and_then(|b| <[u8; 32]>::try_from(b).ok()) {
        return Ok(k);
    }
    let k: [u8; 32] = rand::random();
    config::private_write(&path, &hex::encode(k))?;
    Ok(k)
}

fn mac(secret: &[u8; 32], message: &str) -> String {
    blake3::keyed_hash(secret, message.as_bytes()).to_hex().to_string()
}

fn new_ticket(secret: &[u8; 32]) -> String {
    let body = format!("{}.{}", now() + 60, hex::encode(rand::random::<[u8; 8]>()));
    format!("{body}.{}", mac(secret, &format!("ticket:{body}")))
}

fn ticket_valid(secret: &[u8; 32], ticket: &str) -> bool {
    let Some((body, sig)) = ticket.rsplit_once('.') else { return false };
    let Some(expiry) = body.split('.').next().and_then(|e| e.parse::<u64>().ok()) else { return false };
    // Comparaison d'empreintes : sa durée ne dépend pas de l'endroit où deux chaînes diffèrent.
    expiry >= now() && blake3::hash(mac(secret, &format!("ticket:{body}")).as_bytes()) == blake3::hash(sig.as_bytes())
}

enum Port {
    Free,
    Ours,
    Other,
}

/// Qui écoute sur le port du tableau de bord ? Seul un vrai Echoes de ce compte connaît le secret.
fn probe(secret: &[u8; 32]) -> Port {
    let Ok(mut stream) = TcpStream::connect_timeout(&([127, 0, 0, 1], PORT).into(), Duration::from_millis(300)) else { return Port::Free };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let nonce = hex::encode(rand::random::<[u8; 16]>());
    let request = format!("GET /api/hello?n={nonce} HTTP/1.1\r\nHost: 127.0.0.1:{PORT}\r\nConnection: close\r\n\r\n");
    let mut reply = String::new();
    if stream.write_all(request.as_bytes()).is_ok() {
        let _ = stream.take(4096).read_to_string(&mut reply);
    }
    if reply.contains(&mac(secret, &format!("hello:{nonce}"))) {
        Port::Ours
    } else {
        Port::Other
    }
}

fn open_browser(url: &str) -> bool {
    if std::env::var_os("ECHOES_NO_BROWSER").is_some() {
        return false;
    }
    let child = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).stderr(std::process::Stdio::null()).spawn()
    };
    child.is_ok()
}

fn show(secret: &[u8; 32]) {
    let url = format!("http://127.0.0.1:{PORT}/#{}", new_ticket(secret));
    if !open_browser(&url) {
        // Machine sans navigateur (session distante) : l'adresse ne sert qu'une fois, pendant 60 s.
        println!("Ouvre cette adresse dans la minute, sur cette machine : {url}");
    }
}

/// `echoes dashboard` : ouvre la page ; si le service ne tourne pas, la sert le temps de la consulter.
pub fn open(wait_for_service: bool) -> Result<()> {
    let secret = secret()?;
    let tries = if wait_for_service { 60 } else { 1 };
    for attempt in 0..tries {
        match probe(&secret) {
            Port::Ours => {
                show(&secret);
                return Ok(());
            }
            Port::Other => bail!("le port {PORT} est occupé par un autre programme : le tableau de bord ne peut pas s'ouvrir en sécurité"),
            Port::Free if attempt + 1 < tries => std::thread::sleep(Duration::from_millis(250)),
            Port::Free => {}
        }
    }
    let listener = TcpListener::bind(("127.0.0.1", PORT)).with_context(|| format!("port {PORT} indisponible"))?;
    let shared = Arc::new(Shared::new(secret, false));
    show(&secret);
    println!("Le service Echoes ne tourne pas : cette page reste disponible tant qu'elle est utilisée (arrêt après 15 minutes sans activité).");
    serve_forever(listener, shared);
    Ok(())
}

/// Lancé par `echoes serve`.
pub fn spawn_server() {
    let Ok(secret) = secret() else { return };
    match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(listener) => {
            std::thread::spawn(move || serve_forever(listener, Arc::new(Shared::new(secret, true))));
        }
        Err(e) => eprintln!("echoes: tableau de bord indisponible (port {PORT} : {e})"),
    }
}

struct Shared {
    secret: [u8; 32],
    /// Sessions ouvertes (empreinte du jeton → expiration).
    sessions: Mutex<HashMap<[u8; 32], Instant>>,
    /// Billets déjà échangés.
    used: Mutex<HashSet<String>>,
    last_request: Mutex<Instant>,
    /// Vrai quand la page est servie par le service d'arrière-plan.
    in_service: bool,
}

impl Shared {
    fn new(secret: [u8; 32], in_service: bool) -> Self {
        Shared { secret, sessions: Mutex::new(HashMap::new()), used: Mutex::new(HashSet::new()), last_request: Mutex::new(Instant::now()), in_service }
    }

    fn authorized(&self, req: &Request) -> bool {
        let Some(token) = req.headers.get("x-echoes-token") else { return false };
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_, expiry| *expiry > Instant::now());
        sessions.contains_key(blake3::hash(token.as_bytes()).as_bytes())
    }
}

fn serve_forever(listener: TcpListener, shared: Arc<Shared>) {
    if !shared.in_service {
        // Lancé depuis l'app, ce processus est invisible : il ne doit pas rester indéfiniment.
        let shared = shared.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(30));
            if shared.last_request.lock().unwrap().elapsed() > Duration::from_secs(15 * 60) {
                std::process::exit(0);
            }
        });
    }
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            std::thread::sleep(Duration::from_millis(200));
            continue;
        };
        if CONNS.load(Ordering::SeqCst) >= MAX_CONNS {
            continue;
        }
        CONNS.fetch_add(1, Ordering::SeqCst);
        let shared = shared.clone();
        std::thread::spawn(move || {
            if let Err(e) = handle(&mut stream, &shared) {
                let body = serde_json::json!({ "error": format!("{e:#}") }).to_string();
                let _ = respond(&mut stream, "500 Internal Server Error", "application/json; charset=utf-8", body.as_bytes());
            }
            CONNS.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

struct Request {
    method: String,
    path: String,
    query: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

/// Lit une requête entière en 5 secondes au plus, quel que soit le rythme d'envoi.
fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let read = |stream: &mut TcpStream, chunk: &mut [u8]| -> Result<usize> {
        let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or_else(|| anyhow!("requête trop lente"))?;
        stream.set_read_timeout(Some(left))?;
        Ok(stream.read(chunk)?)
    };
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = read(stream, &mut chunk)?;
        if n == 0 {
            bail!("connexion fermée");
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 16 << 10 {
            bail!("en-tête trop long");
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or_default().split_whitespace();
    let method = first.next().unwrap_or_default().to_string();
    let target = first.next().unwrap_or_default().to_string();
    let (path, query) = target.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())).unwrap_or((target.clone(), String::new()));
    let headers: HashMap<String, String> =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0).min(8192);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = read(stream, &mut chunk)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    Ok(Request { method, path, query, headers, body })
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}

fn json<T: Serialize>(stream: &mut TcpStream, value: &T) -> Result<()> {
    respond(stream, "200 OK", "application/json; charset=utf-8", &serde_json::to_vec(value)?)
}

fn handle(stream: &mut TcpStream, shared: &Shared) -> Result<()> {
    let Ok(req) = read_request(stream) else { return Ok(()) };
    *shared.last_request.lock().unwrap() = Instant::now();
    // Contre le « DNS rebinding » : seule une page servie en local peut parler au tableau de bord.
    let host = req.headers.get("host").map(String::as_str).unwrap_or_default();
    if host != format!("127.0.0.1:{PORT}") && host != format!("localhost:{PORT}") {
        return respond(stream, "403 Forbidden", "text/plain", b"interdit");
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => return respond(stream, "200 OK", "text/html; charset=utf-8", PAGE.as_bytes()),
        // Preuve d'identité : permet à `echoes dashboard` de vérifier qu'il parle bien à Echoes.
        ("GET", "/api/hello") => {
            let nonce: String = req.query.strip_prefix("n=").unwrap_or_default().chars().filter(char::is_ascii_hexdigit).take(64).collect();
            return json(stream, &serde_json::json!({ "proof": mac(&shared.secret, &format!("hello:{nonce}")) }));
        }
        // Échange du billet à usage unique contre une session.
        ("POST", "/api/session") => {
            #[derive(Deserialize)]
            struct Body {
                ticket: String,
            }
            let Ok(body) = serde_json::from_slice::<Body>(&req.body) else { return respond(stream, "400 Bad Request", "text/plain", b"demande illisible") };
            if !ticket_valid(&shared.secret, &body.ticket) || !shared.used.lock().unwrap().insert(body.ticket) {
                return respond(stream, "401 Unauthorized", "text/plain", b"billet invalide");
            }
            let token = hex::encode(rand::random::<[u8; 32]>());
            shared.sessions.lock().unwrap().insert(*blake3::hash(token.as_bytes()).as_bytes(), Instant::now() + SESSION);
            return json(stream, &serde_json::json!({ "token": token }));
        }
        _ => {}
    }
    if !req.path.starts_with("/api/") {
        return respond(stream, "404 Not Found", "text/plain", b"introuvable");
    }
    if !shared.authorized(&req) {
        return respond(stream, "401 Unauthorized", "text/plain", b"session absente");
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/api/state") => json(stream, &state()?),
        ("GET", "/api/code") => {
            let code = config::load_key()?.map(hex::encode).unwrap_or_default();
            json(stream, &serde_json::json!({ "code": code }))
        }
        ("POST", "/api/pair") => {
            #[derive(Deserialize)]
            struct Pair {
                code: String,
            }
            let Ok(p) = serde_json::from_slice::<Pair>(&req.body) else { return respond(stream, "400 Bad Request", "text/plain", b"demande illisible") };
            match config::parse_key(&p.code) {
                // Le service relit la clé à chaque connexion : pas de redémarrage nécessaire.
                Ok(k) => {
                    config::save_key(&k)?;
                    json(stream, &serde_json::json!({ "ok": true }))
                }
                Err(e) => json(stream, &serde_json::json!({ "ok": false, "error": format!("{e}") })),
            }
        }
        _ => respond(stream, "404 Not Found", "text/plain", b"introuvable"),
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct Machine {
    pub(crate) status: Status,
    pub(crate) online: bool,
    pub(crate) me: bool,
    pub(crate) last_seen: u64,
    /// Puissance qu'on peut lui demander maintenant (même calcul que pour choisir où envoyer une tâche).
    #[serde(default)]
    pub(crate) score: f64,
    /// Prise par un travail lourd : Echoes lui envoie moins de tâches et les bride.
    #[serde(default)]
    pub(crate) busy: bool,
}

#[derive(Serialize)]
struct State {
    version: String,
    paired: bool,
    machines: Vec<Machine>,
    history: Vec<Record>,
}

fn state() -> Result<State> {
    let me = status::local()?;
    let key = config::load_key()?;
    let (peers, mut hist) = match &key {
        Some(k) => {
            let peers = client::find(k, &me.id, false);
            let handles: Vec<_> = peers
                .iter()
                .map(|(a, _)| {
                    let (a, k) = (*a, *k);
                    std::thread::spawn(move || client::fetch_history(a, &k))
                })
                .collect();
            let mut hist = history::read(2000);
            for h in handles {
                hist.extend(h.join().unwrap_or_default());
            }
            (peers, hist)
        }
        None => (vec![], history::read(2000)),
    };
    let mut seen = std::collections::HashSet::new();
    hist.retain(|r| seen.insert(r.id.clone()));
    hist.sort_by_key(|r| std::cmp::Reverse(r.ts));
    hist.truncate(2000);

    // Registre des machines déjà vues : celles hors ligne restent affichées.
    let file = config::data_dir()?.join("machines.json");
    let mut known: Vec<Machine> = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    for m in &mut known {
        m.online = false;
        m.me = false;
    }
    let t = now();
    let mut upsert = |status: Status, me: bool| {
        known.retain(|m| m.status.id != status.id);
        let (score, busy) = (status.score(), status.busy());
        known.push(Machine { status, online: true, me, last_seen: t, score, busy });
    };
    upsert(me, true);
    for (_, s) in peers {
        upsert(s, false);
    }
    let tmp = file.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, serde_json::to_vec(&known)?).is_ok() {
        let _ = std::fs::rename(&tmp, &file);
    }
    known.sort_by(|a, b| b.me.cmp(&a.me).then(b.online.cmp(&a.online)).then(b.score.total_cmp(&a.score)));
    Ok(State { version: env!("CARGO_PKG_VERSION").into(), paired: key.is_some(), machines: known, history: hist })
}
