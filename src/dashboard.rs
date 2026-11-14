//! Tableau de bord : page locale (127.0.0.1 seulement), ouverte uniquement sur demande.

use crate::history::Record;
use crate::proto::Status;
use crate::{client, config, history, status};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

pub const PORT: u16 = 47601;
const PAGE: &str = include_str!("dashboard.html");

fn token() -> Result<String> {
    let path = config::data_dir()?.join("dashboard-token");
    if let Ok(t) = std::fs::read_to_string(&path) {
        if t.trim().len() == 32 {
            return Ok(t.trim().to_string());
        }
    }
    let t = hex::encode(rand::random::<[u8; 16]>());
    std::fs::write(&path, &t)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(t)
}

fn url() -> Result<String> {
    Ok(format!("http://127.0.0.1:{PORT}/#{}", token()?))
}

fn running() -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], PORT).into(), Duration::from_millis(300)).is_ok()
}

fn open_browser(url: &str) {
    let _ = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
}

/// `echoes dashboard` : ouvre la page ; si le service ne tourne pas, la sert le temps de la consulter.
pub fn open(wait_for_service: bool) -> Result<()> {
    let url = url()?;
    if wait_for_service {
        for _ in 0..40 {
            if running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    if running() {
        open_browser(&url);
        return Ok(());
    }
    if wait_for_service {
        println!("Tableau de bord : {url}");
        return Ok(());
    }
    let listener = TcpListener::bind(("127.0.0.1", PORT)).with_context(|| format!("port {PORT} indisponible"))?;
    println!("Tableau de bord : {url}\n(service arrêté : la page reste disponible tant que cette fenêtre est ouverte, Ctrl-C pour fermer)");
    open_browser(&url);
    serve_forever(listener);
    Ok(())
}

/// Lancé par `echoes serve` ; silencieux si le port est déjà pris.
pub fn spawn_server() {
    if let Ok(listener) = TcpListener::bind(("127.0.0.1", PORT)) {
        std::thread::spawn(move || serve_forever(listener));
    }
}

fn serve_forever(listener: TcpListener) {
    let Ok(token) = token() else { return };
    for stream in listener.incoming().flatten() {
        let token = token.clone();
        std::thread::spawn(move || {
            let _ = handle(stream, &token);
        });
    }
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Result<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = stream.read(&mut chunk)?;
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
    let path = first.next().unwrap_or_default().to_string();
    let headers: HashMap<String, String> =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0).min(8192);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    Ok(Request { method, path, headers, body })
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}

fn json<T: Serialize>(stream: &mut TcpStream, value: &T) -> Result<()> {
    respond(stream, "200 OK", "application/json; charset=utf-8", &serde_json::to_vec(value)?)
}

fn handle(mut stream: TcpStream, token: &str) -> Result<()> {
    let req = read_request(&mut stream)?;
    // Contre le « DNS rebinding » : seule une page servie en local peut parler au tableau de bord.
    let host = req.headers.get("host").map(String::as_str).unwrap_or_default();
    if host != format!("127.0.0.1:{PORT}") && host != format!("localhost:{PORT}") {
        return respond(&mut stream, "403 Forbidden", "text/plain", b"interdit");
    }
    let path = req.path.split('?').next().unwrap_or_default();
    if req.method == "GET" && path == "/" {
        return respond(&mut stream, "200 OK", "text/html; charset=utf-8", PAGE.as_bytes());
    }
    if !path.starts_with("/api/") {
        return respond(&mut stream, "404 Not Found", "text/plain", b"introuvable");
    }
    // Les données exigent le jeton secret (passé dans l'adresse de la page, jamais envoyé ailleurs).
    if req.headers.get("x-echoes-token").map(String::as_str) != Some(token) {
        return respond(&mut stream, "401 Unauthorized", "text/plain", b"jeton manquant");
    }
    match (req.method.as_str(), path) {
        ("GET", "/api/state") => json(&mut stream, &state()?),
        ("GET", "/api/code") => {
            let code = config::load_key()?.map(hex::encode).unwrap_or_default();
            json(&mut stream, &serde_json::json!({ "code": code }))
        }
        ("POST", "/api/pair") => {
            #[derive(Deserialize)]
            struct Pair {
                code: String,
            }
            let p: Pair = serde_json::from_slice(&req.body)?;
            match config::parse_key(&p.code) {
                Ok(k) => {
                    config::save_key(&k)?;
                    json(&mut stream, &serde_json::json!({ "ok": true }))?;
                    // Le service repart avec la nouvelle clé.
                    if std::env::args().nth(1).as_deref() == Some("serve") {
                        std::thread::sleep(Duration::from_millis(300));
                        crate::update::restart();
                    }
                    Ok(())
                }
                Err(e) => json(&mut stream, &serde_json::json!({ "ok": false, "error": format!("{e}") })),
            }
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", b"introuvable"),
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Machine {
    status: Status,
    online: bool,
    me: bool,
    last_seen: u64,
}

#[derive(Serialize)]
struct State {
    version: String,
    paired: bool,
    machines: Vec<Machine>,
    history: Vec<Record>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
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
            let mut hist = history::read(300);
            for h in handles {
                hist.extend(h.join().unwrap_or_default());
            }
            (peers, hist)
        }
        None => (vec![], history::read(300)),
    };
    let mut seen = std::collections::HashSet::new();
    hist.retain(|r| seen.insert(r.id.clone()));
    hist.sort_by_key(|r| std::cmp::Reverse(r.ts));
    hist.truncate(500);

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
        known.push(Machine { status, online: true, me, last_seen: t });
    };
    upsert(me, true);
    for (_, s) in peers {
        upsert(s, false);
    }
    let _ = std::fs::write(&file, serde_json::to_vec(&known)?);
    known.sort_by(|a, b| b.me.cmp(&a.me).then(b.online.cmp(&a.online)).then(b.status.score().total_cmp(&a.status.score())));
    Ok(State { version: env!("CARGO_PKG_VERSION").into(), paired: key.is_some(), machines: known, history: hist })
}
