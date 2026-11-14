mod client;
mod config;
mod discover;
mod proto;
mod status;
mod sync;
mod worker;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "echoes", version, about = "Lance une commande ici, elle tourne sur la machine la plus disponible de ton réseau.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Crée le code d'appairage (sans argument) ou rejoint les autres machines (avec le code)
    Pair { code: Option<String> },
    /// Prête la puissance de cette machine aux autres
    Serve {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Liste les machines disponibles et leur puissance
    Ls,
    /// Lance une commande sur la machine la plus disponible
    Run {
        /// Forcer une machine précise (nom ou adresse, port facultatif)
        #[arg(long)]
        host: Option<String>,
        /// Forcer l'exécution ici
        #[arg(long, conflicts_with_all = ["remote", "host"])]
        local: bool,
        /// Forcer l'exécution sur une autre machine
        #[arg(long)]
        remote: bool,
        /// Rapatrier ce chemin après l'exécution, même s'il est ignoré par git (répétable)
        #[arg(long)]
        pull: Vec<String>,
        /// Ne pas afficher les lignes d'information d'Echoes
        #[arg(short, long)]
        quiet: bool,
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,
    },
}

fn main() {
    let code = match real_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("echoes: {e:#}");
            1
        }
    };
    std::process::exit(code);
}

fn real_main() -> Result<i32> {
    match Cli::parse().cmd {
        Cmd::Pair { code: None } => {
            let key = match config::load_key()? {
                Some(k) => k,
                None => {
                    let k: [u8; 32] = rand::random();
                    config::save_key(&k)?;
                    k
                }
            };
            println!("Sur chaque autre machine, lance :\n\n  echoes pair {}\n", hex::encode(key));
            println!("Ce code donne le droit d'exécuter des commandes sur tes machines : ne le partage pas.");
            Ok(0)
        }
        Cmd::Pair { code: Some(code) } => {
            config::save_key(&config::parse_key(&code)?)?;
            println!("Machine appairée.");
            Ok(0)
        }
        Cmd::Serve { port } => worker::serve(port).map(|_| 0),
        Cmd::Ls => client::list().map(|_| 0),
        Cmd::Run { host, local, remote, pull, quiet, command } => client::run(client::RunOpts { host, local, remote, pull, quiet, command }),
    }
}
