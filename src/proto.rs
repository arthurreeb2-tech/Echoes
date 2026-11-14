//! Messages échangés entre machines, et canal chiffré (Noise + clé partagée).

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpStream;

pub const VERSION: u32 = 3;
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
}

impl Status {
    /// Puissance disponible maintenant : vitesse d'un cœur × cœurs libres.
    pub fn score(&self) -> f64 {
        self.bench * self.free.max(0.25)
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
        let mut s = Secure { stream, noise: hs.into_transport_mode()? };
        s.send(&Msg::Hello { version: VERSION })?;
        Ok(s)
    }

    pub fn accept(stream: TcpStream, key: &[u8; 32]) -> Result<Self> {
        let mut hs = snow::Builder::new(NOISE.parse()?).psk(0, key)?.build_responder()?;
        let mut buf = [0u8; 1024];
        let first = read_frame(&stream)?;
        hs.read_message(&first, &mut buf).map_err(|_| anyhow!("clé invalide"))?;
        let n = hs.write_message(&[], &mut buf)?;
        write_frame(&stream, &buf[..n])?;
        let mut s = Secure { stream, noise: hs.into_transport_mode()? };
        match s.recv()? {
            Msg::Hello { version } if version == VERSION => Ok(s),
            Msg::Hello { version } => {
                let m = format!("version de protocole {version}, attendu {VERSION} : mets Echoes à jour des deux côtés");
                let _ = s.send(&Msg::Error(m.clone()));
                bail!(m)
            }
            _ => bail!("premier message inattendu"),
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
        Ok(())
    }

    pub fn recv(&mut self) -> Result<Msg> {
        let mut bytes = Vec::new();
        let mut plain = vec![0u8; 65535];
        loop {
            let frame = read_frame(&self.stream)?;
            let n = self.noise.read_message(&frame, &mut plain).map_err(|_| anyhow!("message corrompu"))?;
            if n == 0 {
                bail!("trame vide");
            }
            bytes.extend_from_slice(&plain[1..n]);
            if bytes.len() > MAX_MSG {
                bail!("message trop gros");
            }
            if plain[0] == 1 {
                break;
            }
        }
        bincode::deserialize(&bytes).context("message illisible")
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
