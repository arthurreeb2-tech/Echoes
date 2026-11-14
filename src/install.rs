//! Installation : copie du programme, démarrage automatique du service, consigne pour Claude Code.

use crate::config;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "macos")]
const LABEL: &str = "io.echoes.serve";
const MARK_BEGIN: &str = "<!-- echoes:begin -->";
const MARK_END: &str = "<!-- echoes:end -->";

fn home() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow!("dossier personnel introuvable"))
}

pub fn bin_dir() -> Result<PathBuf> {
    if cfg!(windows) {
        Ok(dirs::data_local_dir().ok_or_else(|| anyhow!("LOCALAPPDATA introuvable"))?.join("Programs").join("Echoes"))
    } else {
        Ok(home()?.join(".local").join("bin"))
    }
}

pub fn bin_path() -> Result<PathBuf> {
    Ok(bin_dir()?.join(if cfg!(windows) { "echoes.exe" } else { "echoes" }))
}

fn quiet(cmd: &mut Command) -> bool {
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

pub fn is_installed() -> bool {
    bin_path().map(|p| p.exists()).unwrap_or(false) && config::load_key().ok().flatten().is_some()
}

pub fn run(code: Option<String>, open: bool) -> Result<()> {
    // 1. Clé d'appairage.
    let new_code = match (code, config::load_key()?) {
        (Some(c), _) => {
            config::save_key(&config::parse_key(&c)?)?;
            None
        }
        (None, Some(_)) => None,
        (None, None) => {
            let k: [u8; 32] = rand::random();
            config::save_key(&k)?;
            Some(hex::encode(k))
        }
    };

    // 2. Programme à un emplacement stable (celui que le service lance et que les mises à jour remplacent).
    stop_service();
    let dest = bin_path()?;
    fs::create_dir_all(dest.parent().unwrap())?;
    let me = std::env::current_exe()?;
    if fs::canonicalize(&me).ok() != fs::canonicalize(&dest).ok() {
        let tmp = dest.with_extension("new");
        fs::copy(&me, &tmp).context("copie du programme impossible")?;
        fs::rename(&tmp, &dest).or_else(|_| fs::copy(&me, &dest).map(|_| ()))?;
    }
    let path_note = ensure_path()?;

    // 3. Service lancé à chaque ouverture de session, relancé s'il s'arrête.
    let active = start_service(&dest)?;
    status::warm_up(&dest);

    // 4. Consigne globale pour Claude Code.
    let claude = claude_instructions(true)?;

    println!("Echoes {} est installé : {}", env!("CARGO_PKG_VERSION"), dest.display());
    if active {
        println!("Service actif, lancé automatiquement à chaque ouverture de session, mis à jour automatiquement.");
    } else {
        println!("Démarrage automatique indisponible ici : lance `echoes serve` toi-même pour prêter cette machine.");
    }
    if let Some(note) = path_note {
        println!("{note}");
    }
    if claude {
        println!("Claude Code utilisera `echoes run --` pour les builds et les tests lourds (~/.claude/CLAUDE.md).");
    }
    firewall_hint();
    if let Some(code) = new_code {
        println!("\nPremière machine : sur chacune des autres, installe Echoes avec ce code :\n\n  {code}\n");
        println!("Ce code donne le droit d'exécuter des commandes sur tes machines : ne le partage avec personne.");
    }
    println!("Tableau de bord : `echoes dashboard`.");
    if open && active {
        crate::dashboard::open(true)?;
    }
    Ok(())
}

pub fn uninstall() -> Result<()> {
    stop_service();
    remove_service_files()?;
    claude_instructions(false)?;
    println!("Service arrêté et retiré, consigne Claude Code retirée.");
    let dest = bin_path()?;
    if dest.exists() {
        if fs::canonicalize(std::env::current_exe()?).ok() == fs::canonicalize(&dest).ok() {
            self_replace::self_delete()?;
        } else {
            fs::remove_file(&dest)?;
        }
        println!("Programme supprimé. La clé d'appairage reste dans {}.", config::config_dir()?.display());
    }
    Ok(())
}

/// Relance le service (après une mise à jour manuelle).
pub fn restart_service() -> Result<()> {
    let dest = bin_path()?;
    if dest.exists() {
        stop_service();
        start_service(&dest)?;
    }
    Ok(())
}

mod status {
    use std::path::Path;
    /// Mesure la vitesse de la machine une fois pour toutes, hors du chemin critique.
    pub fn warm_up(bin: &Path) {
        let _ = std::process::Command::new(bin).arg("ls").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    }
}

// ---------- macOS : agent launchd ----------

#[cfg(target_os = "macos")]
fn plist_path() -> Result<PathBuf> {
    Ok(home()?.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
}

#[cfg(target_os = "macos")]
fn domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

#[cfg(target_os = "macos")]
fn start_service(bin: &Path) -> Result<bool> {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let path = std::env::var("PATH").unwrap_or_default();
    let path = format!("{}:{path}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin", bin_dir()?.display());
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let log = home()?.join("Library/Logs/echoes.log");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>{}</string><string>serve</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>EnvironmentVariables</key><dict>
    <key>PATH</key><string>{}</string>
    <key>SHELL</key><string>{}</string>
  </dict>
  <key>StandardOutPath</key><string>{}</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
        esc(&bin.display().to_string()),
        esc(&path),
        esc(&shell),
        esc(&log.display().to_string()),
        esc(&log.display().to_string()),
    );
    let file = plist_path()?;
    fs::create_dir_all(file.parent().unwrap())?;
    fs::write(&file, plist)?;
    // `bootout` est asynchrone : on réessaie quelques fois le temps que l'ancien service s'arrête.
    let mut ok = false;
    for _ in 0..5 {
        if quiet(Command::new("launchctl").args(["bootstrap", &domain()]).arg(&file)) {
            ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    if !ok {
        anyhow::bail!("launchctl n'a pas pu démarrer le service (journal : {})", log.display());
    }
    Ok(true)
}

#[cfg(target_os = "macos")]
fn stop_service() {
    let _ = quiet(Command::new("launchctl").args(["bootout", &format!("{}/{LABEL}", domain())]));
}

#[cfg(target_os = "macos")]
fn remove_service_files() -> Result<()> {
    let _ = fs::remove_file(plist_path()?);
    Ok(())
}

#[cfg(target_os = "macos")]
fn firewall_hint() {
    let out = Command::new("/usr/libexec/ApplicationFirewall/socketfilterfw").arg("--getglobalstate").output();
    if let Ok(o) = out {
        if String::from_utf8_lossy(&o.stdout).contains("enabled") {
            println!("Le pare-feu macOS est actif : s'il demande d'autoriser « echoes » à recevoir des connexions, accepte.");
        }
    }
}

// ---------- Linux : service systemd utilisateur ----------

#[cfg(target_os = "linux")]
fn unit_path() -> Result<PathBuf> {
    Ok(dirs::config_dir().ok_or_else(|| anyhow!("dossier de configuration introuvable"))?.join("systemd/user/echoes.service"))
}

#[cfg(target_os = "linux")]
fn start_service(bin: &Path) -> Result<bool> {
    let path = format!("{}:{}", bin_dir()?.display(), std::env::var("PATH").unwrap_or_default());
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let unit = format!(
        "[Unit]\nDescription=Echoes\nAfter=network-online.target\n\n[Service]\nExecStart={} serve\nRestart=always\nRestartSec=5\nEnvironment=\"PATH={path}\"\nEnvironment=\"SHELL={shell}\"\n\n[Install]\nWantedBy=default.target\n",
        bin.display()
    );
    let file = unit_path()?;
    fs::create_dir_all(file.parent().unwrap())?;
    fs::write(&file, unit)?;
    let ok = quiet(Command::new("systemctl").args(["--user", "daemon-reload"]))
        && quiet(Command::new("systemctl").args(["--user", "enable", "--now", "echoes.service"]));
    Ok(ok)
}

#[cfg(target_os = "linux")]
fn stop_service() {
    let _ = quiet(Command::new("systemctl").args(["--user", "disable", "--now", "echoes.service"]));
}

#[cfg(target_os = "linux")]
fn remove_service_files() -> Result<()> {
    let _ = fs::remove_file(unit_path()?);
    let _ = quiet(Command::new("systemctl").args(["--user", "daemon-reload"]));
    Ok(())
}

#[cfg(target_os = "linux")]
fn firewall_hint() {}

// ---------- Windows : tâche planifiée à l'ouverture de session ----------

#[cfg(windows)]
fn start_service(bin: &Path) -> Result<bool> {
    // conhost --headless : pas de fenêtre de console visible.
    let action = format!("conhost.exe --headless \"{}\" serve", bin.display());
    let ok = Command::new("schtasks").args(["/Create", "/F", "/SC", "ONLOGON", "/RL", "LIMITED", "/TN", "Echoes", "/TR", &action]).status()?.success();
    if !ok {
        anyhow::bail!("création de la tâche planifiée impossible");
    }
    quiet(Command::new("schtasks").args(["/Run", "/TN", "Echoes"]));
    // Raccourci « Echoes » dans le menu Démarrer : ouvre le tableau de bord.
    let ps = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut([Environment]::GetFolderPath('Programs')+'\\Echoes.lnk'); $s.TargetPath='{}'; $s.Arguments='dashboard'; $s.WindowStyle=7; $s.Description='Tableau de bord Echoes'; $s.Save()",
        bin.display()
    );
    quiet(Command::new("powershell").args(["-NoProfile", "-Command", &ps]));
    Ok(true)
}

#[cfg(windows)]
fn stop_service() {
    let _ = quiet(Command::new("schtasks").args(["/End", "/TN", "Echoes"]));
    let _ = quiet(Command::new("taskkill").args(["/F", "/IM", "echoes.exe", "/FI", &format!("PID ne {}", std::process::id())]));
}

#[cfg(windows)]
fn remove_service_files() -> Result<()> {
    let _ = quiet(Command::new("schtasks").args(["/Delete", "/F", "/TN", "Echoes"]));
    if let Some(start) = dirs::data_dir() {
        let _ = fs::remove_file(start.join("Microsoft/Windows/Start Menu/Programs/Echoes.lnk"));
    }
    Ok(())
}

#[cfg(windows)]
fn firewall_hint() {
    println!("Si le pare-feu Windows demande d'autoriser « echoes » sur les réseaux privés, accepte.");
}

// ---------- PATH ----------

fn ensure_path() -> Result<Option<String>> {
    let dir = bin_dir()?;
    let in_path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d == dir)).unwrap_or(false);
    if in_path {
        return Ok(None);
    }
    if cfg!(windows) {
        let ps = format!(
            "$p=[Environment]::GetEnvironmentVariable('Path','User'); if(-not ($p -split ';' -contains '{0}')){{[Environment]::SetEnvironmentVariable('Path', ($p.TrimEnd(';') + ';{0}'), 'User')}}",
            dir.display()
        );
        quiet(Command::new("powershell").args(["-NoProfile", "-Command", &ps]));
        return Ok(Some("Ouvre un nouveau terminal pour utiliser la commande `echoes`.".into()));
    }
    let shell = std::env::var("SHELL").unwrap_or_default();
    let rc = home()?.join(if shell.ends_with("zsh") {
        ".zshrc"
    } else if shell.ends_with("bash") && cfg!(target_os = "macos") {
        ".bash_profile"
    } else if shell.ends_with("bash") {
        ".bashrc"
    } else {
        ".profile"
    });
    let line = "export PATH=\"$HOME/.local/bin:$PATH\"  # echoes";
    let current = fs::read_to_string(&rc).unwrap_or_default();
    if !current.contains(line) {
        fs::write(&rc, format!("{current}{}{line}\n", if current.is_empty() || current.ends_with('\n') { "" } else { "\n" }))?;
    }
    Ok(Some(format!("Chemin ajouté dans {} : ouvre un nouveau terminal pour utiliser la commande `echoes`.", rc.display())))
}

// ---------- Claude Code ----------

const CLAUDE_BLOCK: &str = "## Echoes (délégation de calcul)

Echoes est installé sur cette machine : il exécute une commande sur la machine la plus disponible du réseau local et rapatrie le résultat, ou l'exécute ici si cette machine est la meilleure.

- Préfixe par `echoes run --` les commandes longues de build, de test et de lint lancées dans un projet : `cargo build/test/clippy`, `swift build/test`, `xcodebuild`, `npm|pnpm|yarn run build/test`, `tsc`, `make`, `cmake --build`, `go build/test`, `pytest`, `gradle`, rendus `blender -b`.
- Ne l'utilise pas pour les commandes rapides, interactives ou qui touchent des chemins hors du projet (serveurs de dev, `git`, `ls`, installations globales).
- Les fichiers suivis par git modifiés à distance reviennent automatiquement ; ajoute `--pull <chemin>` pour rapatrier un produit de build ignoré par git (ex. `--pull target/release`).
- Un binaire compilé à distance a l'architecture de la machine distante : pour un exécutable à lancer ici, compile ici (`echoes run --local --` ou sans Echoes).
- Si `echoes` signale une erreur réseau, relance la commande sans Echoes.
";

fn claude_instructions(add: bool) -> Result<bool> {
    let dir = home()?.join(".claude");
    if add && !dir.exists() {
        return Ok(false);
    }
    let file = dir.join("CLAUDE.md");
    let current = fs::read_to_string(&file).unwrap_or_default();
    let stripped = match (current.find(MARK_BEGIN), current.find(MARK_END)) {
        (Some(a), Some(b)) if b > a => format!("{}{}", &current[..a], current[b + MARK_END.len()..].trim_start_matches('\n')),
        _ => current.clone(),
    };
    let next = if add {
        let sep = if stripped.is_empty() || stripped.ends_with("\n\n") {
            ""
        } else if stripped.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        format!("{stripped}{sep}{MARK_BEGIN}\n{CLAUDE_BLOCK}{MARK_END}\n")
    } else {
        stripped.trim_end().to_string() + if stripped.trim().is_empty() { "" } else { "\n" }
    };
    if next != current {
        fs::write(&file, next)?;
    }
    Ok(add)
}
