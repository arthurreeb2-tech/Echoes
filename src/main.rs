mod client;
mod config;
mod dashboard;
mod discover;
mod history;
mod install;
mod proto;
mod status;
mod sync;
mod update;
mod worker;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "echoes", version, about = "Lance une commande ici, elle tourne sur la machine la plus disponible de ton réseau.")]
struct Cli {
    /// Sans commande : installe Echoes si besoin, puis ouvre le tableau de bord.
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Crée le code d'appairage (sans argument) ou rejoint les autres machines (avec le code)
    Pair { code: Option<String> },
    /// Installe Echoes : service au démarrage, mises à jour automatiques, consigne Claude Code.
    /// Sans code sur la première machine (un code est créé), avec le code sur les suivantes.
    Install {
        code: Option<String>,
        /// Ouvrir le tableau de bord une fois l'installation terminée
        #[arg(long)]
        open: bool,
    },
    /// Ouvre le tableau de bord (tâches déléguées, machines, efficacité)
    Dashboard,
    /// Retire le service et le programme (la clé d'appairage est conservée)
    Uninstall,
    /// Met Echoes à jour depuis GitHub
    Update {
        /// Vérifier seulement, sans installer
        #[arg(long)]
        check: bool,
    },
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
            alert(&format!("{e:#}"));
            1
        }
    };
    std::process::exit(code);
}

fn real_main() -> Result<i32> {
    update::remember_exe();
    let Some(cmd) = Cli::parse().cmd else {
        // Double-clic sur le programme : installation si nécessaire, puis tableau de bord.
        if install::is_installed() {
            // Lancé depuis l'app ou l'installateur : on passe la main à la copie installée, tenue à jour.
            let installed = install::bin_path()?;
            let me = std::env::current_exe()?;
            if std::fs::canonicalize(&me).ok() != std::fs::canonicalize(&installed).ok() {
                let status = std::process::Command::new(installed).arg("dashboard").status()?;
                return Ok(status.code().unwrap_or(1));
            }
            dashboard::open(false)?;
        } else {
            install::run(None, true)?;
        }
        return Ok(0);
    };
    match cmd {
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
        Cmd::Install { code, open } => install::run(code, open).map(|_| 0),
        Cmd::Dashboard => dashboard::open(false).map(|_| 0),
        Cmd::Uninstall => install::uninstall().map(|_| 0),
        Cmd::Update { check } => {
            update::run_cli(check)?;
            if !check {
                install::restart_service()?;
            }
            Ok(0)
        }
        Cmd::Serve { port } => worker::serve(port).map(|_| 0),
        Cmd::Ls => client::list().map(|_| 0),
        Cmd::Run { host, local, remote, pull, quiet, command } => client::run(client::RunOpts { host, local, remote, pull, quiet, command }),
    }
}

/// Lancé depuis le Finder (app Echoes), une erreur n'a pas de terminal où s'afficher : fenêtre d'alerte.
fn alert(message: &str) {
    use std::io::IsTerminal;
    if cfg!(target_os = "macos") && std::env::args().len() == 1 && !std::io::stderr().is_terminal() {
        let text = message.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!("display alert \"Echoes\" message \"{text}\" as critical");
        let _ = std::process::Command::new("osascript").args(["-e", &script]).status();
    }
}
