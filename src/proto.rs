//! Messages échangés entre machines, et canal chiffré (Noise + clé partagée).

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpStream;

pub const VERSION: u32 = 4;
pub const DEFAULT_PORT: u16 = 47600;
pub const SERVICE: &str = "_echoes._tcp.local.";

const NOISE: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
const FRAME: usize = 60_000;
const MAX_MSG: usize = 64 << 20;
/// Taille des morceaux de fichier envoyés dans un message.
pub const CHUNK: usize = 1 << 20;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Entry {
    /// Chemin relatif, séparé par des `/`.
    pub path: String,
    pub hash: [u8; 32],
    pub mode: u32,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Status {
    pub id: String,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub cores: u32,
    /// Vitesse d'un cœur (unité arbitraire, comparable entre machines).
    pub bench: f64,
    /// Nombre de cœurs actuellement libres.
    pub free: f64,
    pub mem_gb: f64,
    pub version: String,
    /// Occupation de la carte graphique en %, si elle est mesurable.
    pub gpu: Option<f32>,
    /// Mémoire encore disponible, en Go.
    pub mem_free_gb: f64,
    /// Tâches Echoes en cours sur cette machine.
    pub jobs: u32,
}

/// Carte graphique à partir de laquelle la machine est considérée comme prise par un travail lourd.
pub const GPU_BUSY: f32 = 50.0;

impl Status {
    /// La personne fait tourner quelque chose de lourd (montage, 3D…) ou la mémoire est presque pleine.
    pub fn busy(&self) -> bool {
        self.gpu.unwrap_or(0.0) >= GPU_BUSY || self.mem_free_gb < 1.0
    }

    /// Part de la puissance qu'on s'autorise à prendre : 100 % jusqu'à 30 % de carte graphique,
    /// puis de moins en moins, jusqu'à 40 % quand elle est saturée.
    fn gpu_factor(&self) -> f64 {
        let gpu = self.gpu.unwrap_or(0.0) as f64;
        (1.0 - (gpu - 30.0).max(0.0) / 60.0 * 0.6).max(0.4)
    }

    /// Une machine sans mémoire libre ralentit tout le monde : pénalité sous 2 Go disponibles.
    fn mem_factor(&self) -> f64 {
        ((self.mem_free_gb - 0.5) / 1.5).clamp(0.2, 1.0)
    }

    /// Puissance qu'on peut lui demander maintenant sans la saturer :
    /// vitesse d'un cœur × cœurs libres × réserve laissée à la carte graphique et à la mémoire.
    pub fn score(&self) -> f64 {
        self.bench * self.free.max(0.25) * self.gpu_factor() * self.mem_factor()
    }

    /// Nombre de tâches qu'elle accepte en même temps.
    pub fn max_jobs(&self) -> u32 {
        if self.busy() {
            1
        } else {
            (self.cores / 4).max(1)
        }
    }

    /// Explication courte de l'état, pour les messages et le tableau de bord.
    pub fn load_note(&self) -> String {
        let mut parts = vec![format!("{:.1} cœurs libres", self.free)];
        if let Some(g) = self.gpu {
            parts.push(format!("carte graphique {g:.0} %"));
        }
        if self.mem_free_gb < 2.0 {
            parts.push(format!("{:.1} Go de mémoire libre", self.mem_free_gb));
        }
        parts.join(", ")
    }
}

#[derive(Serialize, Deserialize)]
pub enum Msg {
    Hello { version: u32 },
    StatusReq,
    Status(Status),
    Run { project: String, name: String, manifest: Vec<Entry>, cwd: String, command: Vec<String>, pull: Vec<String> },
    Need(Vec<String>),
    File { path: String, mode: u32, first: bool, last: bool, pulled: bool, data: Vec<u8> },
    FilesDone,
    Output { err: bool, data: Vec<u8> },
    Ping,
    Exit(i32),
    Error(String),
    HistoryReq,
    History(Vec<crate::history::Record>),
}

pub struct Secure {
    stream: TcpStream,
    noise: snow::TransportState,
    last_send: std::time::Instant,
}

impl Secure {
    pub fn connect(stream: TcpStream, key: &[u8; 32]) -> Result<Self> {
        let mut hs = snow::Builder::new(NOISE.parse()?).psk(0, key)?.build_initiator()?;
        let mut buf = [0u8; 1024];
        let n = hs.write_message(&[], &mut buf)?;
        write_frame(&stream, &buf[..n])?;
        let refused = || anyhow!("code d'appairage refusé par la machine distante (relance `echoes pair <code>` avec le bon code)");
        let reply = read_frame(&stream).map_err(|_| refused())?;
        hs.read_message(&reply, &mut buf).map_err(|_| refused())?;
        let mut s = Secure { stream, noise: hs.into_transport_mode()?, last_send: std::time::Instant::now() };
        s.send(&Msg::Hello { version: VERSION })?;
        Ok(s)
    }

    pub fn accept(stream: TcpStream, key: &[u8; 32]) -> Result<Self> {
        let mut hs = snow::Builder::new(NOISE.parse()?).psk(0, key)?.build_responder()?;
        let mut buf = [0u8; 1024];
        let first = read_frame(&stream)?;
        hs.read_message(&first, &mut buf).map_err(|_| anyhow!("clé d'appairage différente de celle de l'autre machine"))?;
        let n = hs.write_message(&[], &mut buf)?;
        write_frame(&stream, &buf[..n])?;
        let mut s = Secure { stream, noise: hs.into_transport_mode()?, last_send: std::time::Instant::now() };
        match s.recv()? {
            Msg::Hello { version } if version == VERSION => Ok(s),
            Msg::Hello { version } => {
                let m = format!("version de protocole {version}, attendu {VERSION} : mets Echoes à jour sur les deux machines");
                let _ = s.send(&Msg::Error(m.clone()));
                bail!(m)
            }
            _ => bail!("premier message inattendu (Hello attendu)"),
        }
    }

    pub fn stream(&self) -> &TcpStream {
        &self.stream
    }

    pub fn send(&mut self, msg: &Msg) -> Result<()> {
        let bytes = bincode::serialize(msg)?;
        let mut out = Vec::with_capacity(bytes.len() + 64);
        let mut plain = Vec::with_capacity(FRAME + 1);
        let mut cipher = vec![0u8; FRAME + 64];
        let mut chunks = bytes.chunks(FRAME).peekable();
        while let Some(c) = chunks.next() {
            plain.clear();
            plain.push(chunks.peek().is_none() as u8);
            plain.extend_from_slice(c);
            let n = self.noise.write_message(&plain, &mut cipher)?;
            out.extend_from_slice(&(n as u16).to_be_bytes());
            out.extend_from_slice(&cipher[..n]);
        }
        (&self.stream).write_all(&out)?;
        self.last_send = std::time::Instant::now();
        Ok(())
    }

    /// À appeler dans les traitements longs : prévient l'autre côté qu'on est toujours là.
    pub fn keepalive(&mut self) -> Result<()> {
        if self.last_send.elapsed() >= std::time::Duration::from_secs(2) {
            self.send(&Msg::Ping)?;
        }
        Ok(())
    }

    pub fn recv(&mut self) -> Result<Msg> {
        let mut bytes = Vec::new();
        let mut plain = vec![0u8; 65535];
        loop {
            let frame = read_frame(&self.stream)?;
            let n = self.noise.read_message(&frame, &mut plain).map_err(|_| anyhow!("message chiffré illisible (clé différente ou données altérées)"))?;
            if n == 0 {
                bail!("trame vide reçue");
            }
            bytes.extend_from_slice(&plain[1..n]);
            if bytes.len() > MAX_MSG {
                bail!("message trop volumineux (plus de 64 Mo)");
            }
            if plain[0] == 1 {
                break;
            }
        }
        bincode::deserialize(&bytes).context("message illisible (format inattendu)")
    }
}

fn write_frame(mut s: &TcpStream, data: &[u8]) -> Result<()> {
    s.write_all(&(data.len() as u16).to_be_bytes())?;
    s.write_all(data)?;
    Ok(())
}

fn read_frame(mut s: &TcpStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 2];
    s.read_exact(&mut len)?;
    let mut data = vec![0u8; u16::from_be_bytes(len) as usize];
    s.read_exact(&mut data)?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    const NB: &str = "\u{a0}";

    fn status() -> Status {
        Status { id: "a".into(), name: "a".into(), os: "linux".into(), arch: "x86_64".into(), cores: 8, bench: 100.0, free: 8.0, mem_gb: 16.0, version: "0".into(), gpu: Some(5.0), mem_free_gb: 8.0, jobs: 0 }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn free_machine_is_not_busy_and_scores_full_power() {
        let s = status();
        assert!(!s.busy());
        assert!(close(s.score(), 800.0));
        assert_eq!(s.max_jobs(), 2);
        assert_eq!(s.load_note(), format!("8.0 cœurs libres, carte graphique 5{NB}%"));
    }

    #[test]
    fn saturated_cpu_keeps_a_floor_of_a_quarter_core() {
        let mut s = status();
        s.free = 0.0;
        assert!(close(s.score(), 25.0));
        s.free = 0.1;
        assert!(close(s.score(), 25.0));
        s.free = 0.5;
        assert!(close(s.score(), 50.0));
    }

    #[test]
    fn gpu_thresholds() {
        let mut s = status();
        s.gpu = Some(49.9);
        assert!(!s.busy());
        s.gpu = Some(50.0);
        assert!(s.busy());
        assert_eq!(s.max_jobs(), 1);
        s.gpu = Some(30.0);
        assert!(close(s.score(), 800.0), "pas de pénalité jusqu'à 30 %");
        s.gpu = Some(60.0);
        assert!(close(s.score(), 800.0 * 0.7));
        s.gpu = Some(100.0);
        assert!(close(s.score(), 800.0 * 0.4));
        s.gpu = None;
        assert!(!s.busy() && close(s.score(), 800.0));
        assert_eq!(s.load_note(), "8.0 cœurs libres");
    }

    #[test]
    fn low_memory_is_busy_and_penalised() {
        let mut s = status();
        s.mem_free_gb = 1.0;
        assert!(!s.busy());
        s.mem_free_gb = 0.99;
        assert!(s.busy());
        s.mem_free_gb = 2.0;
        assert!(close(s.score(), 800.0));
        s.mem_free_gb = 1.25;
        assert!(close(s.score(), 800.0 * 0.5));
        s.mem_free_gb = 0.1;
        assert!(close(s.score(), 800.0 * 0.2), "pénalité plafonnée à 80 %");
        s.mem_free_gb = 1.5;
        assert_eq!(s.load_note(), format!("8.0 cœurs libres, carte graphique 5{NB}%, 1.5 Go de mémoire libre"));
    }

    #[test]
    fn max_jobs_follows_cores() {
        let mut s = status();
        s.cores = 2;
        assert_eq!(s.max_jobs(), 1);
        s.cores = 16;
        assert_eq!(s.max_jobs(), 4);
        s.gpu = Some(90.0);
        assert_eq!(s.max_jobs(), 1, "occupée : une seule tâche");
    }

    /// Serveur local : renvoie `Secure::accept` (ou l'erreur) via un thread.
    fn listen(key: [u8; 32]) -> (std::net::SocketAddr, std::thread::JoinHandle<Result<Secure>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            Secure::accept(stream, &key)
        });
        (addr, handle)
    }

    #[test]
    fn encrypted_round_trip() {
        let key = [7u8; 32];
        let (addr, server) = listen(key);
        let mut client = Secure::connect(TcpStream::connect(addr).unwrap(), &key).unwrap();
        let mut server = server.join().unwrap().unwrap();

        client.send(&Msg::StatusReq).unwrap();
        assert!(matches!(server.recv().unwrap(), Msg::StatusReq));

        // Plus grand qu'une trame : plusieurs morceaux chiffrés, réassemblés.
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        client.send(&Msg::Output { err: true, data: data.clone() }).unwrap();
        match server.recv().unwrap() {
            Msg::Output { err, data: got } => assert!(err && got == data),
            _ => panic!("message inattendu"),
        }

        server.send(&Msg::Exit(3)).unwrap();
        assert!(matches!(client.recv().unwrap(), Msg::Exit(3)));
        server.send(&Msg::Error("oups".into())).unwrap();
        assert!(matches!(client.recv().unwrap(), Msg::Error(m) if m == "oups"));
        assert!(client.stream().peer_addr().is_ok());
    }

    #[test]
    fn keepalive_sends_ping_only_after_two_seconds() {
        let key = [1u8; 32];
        let (addr, server) = listen(key);
        let mut client = Secure::connect(TcpStream::connect(addr).unwrap(), &key).unwrap();
        let mut server = server.join().unwrap().unwrap();
        client.keepalive().unwrap();
        client.last_send -= std::time::Duration::from_secs(3);
        client.keepalive().unwrap();
        assert!(matches!(server.recv().unwrap(), Msg::Ping));
        client.send(&Msg::Exit(0)).unwrap();
        assert!(matches!(server.recv().unwrap(), Msg::Exit(0)), "pas de second Ping en attente");
    }

    #[test]
    fn different_keys_are_refused_on_both_sides() {
        let (addr, server) = listen([1u8; 32]);
        let err = Secure::connect(TcpStream::connect(addr).unwrap(), &[2u8; 32]).err().expect("refus attendu");
        assert!(err.to_string().contains("refusé"), "{err}");
        let err = server.join().unwrap().err().expect("refus attendu");
        assert!(err.to_string().contains("clé d'appairage"), "{err}");
    }

    #[test]
    fn protocol_version_mismatch_is_refused_with_a_message() {
        let key = [9u8; 32];
        let (addr, server) = listen(key);
        // Client à la main : même poignée de main, mais un autre numéro de version.
        let stream = TcpStream::connect(addr).unwrap();
        let mut hs = snow::Builder::new(NOISE.parse().unwrap()).psk(0, &key).unwrap().build_initiator().unwrap();
        let mut buf = [0u8; 1024];
        let n = hs.write_message(&[], &mut buf).unwrap();
        write_frame(&stream, &buf[..n]).unwrap();
        let reply = read_frame(&stream).unwrap();
        hs.read_message(&reply, &mut buf).unwrap();
        let mut client = Secure { stream, noise: hs.into_transport_mode().unwrap(), last_send: std::time::Instant::now() };
        client.send(&Msg::Hello { version: VERSION + 1 }).unwrap();

        let err = server.join().unwrap().err().expect("refus attendu").to_string();
        assert!(err.contains(&format!("version de protocole {}", VERSION + 1)) && err.contains("mets Echoes à jour"), "{err}");
        assert!(matches!(client.recv().unwrap(), Msg::Error(m) if m == err), "l'autre côté est prévenu");
    }

    #[test]
    fn first_message_must_be_hello() {
        let key = [4u8; 32];
        let (addr, server) = listen(key);
        let stream = TcpStream::connect(addr).unwrap();
        let mut hs = snow::Builder::new(NOISE.parse().unwrap()).psk(0, &key).unwrap().build_initiator().unwrap();
        let mut buf = [0u8; 1024];
        let n = hs.write_message(&[], &mut buf).unwrap();
        write_frame(&stream, &buf[..n]).unwrap();
        let reply = read_frame(&stream).unwrap();
        hs.read_message(&reply, &mut buf).unwrap();
        let mut client = Secure { stream, noise: hs.into_transport_mode().unwrap(), last_send: std::time::Instant::now() };
        client.send(&Msg::StatusReq).unwrap();
        let err = server.join().unwrap().err().expect("refus attendu");
        assert!(err.to_string().contains("premier message inattendu"), "{err}");
    }

    #[test]
    fn recv_rejects_garbage_frames() {
        let key = [3u8; 32];
        let (addr, server) = listen(key);
        let client = Secure::connect(TcpStream::connect(addr).unwrap(), &key).unwrap();
        let mut server = server.join().unwrap().unwrap();
        write_frame(client.stream(), &[0u8; 40]).unwrap();
        let err = server.recv().err().expect("erreur attendue").to_string();
        assert!(err.contains("illisible"), "{err}");
    }
}
