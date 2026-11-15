//! Installation : copie du programme, démarrage automatique du service, consigne pour Claude Code.

use crate::config;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "macos")]
const LABEL: &str = "io.echoes.serve";
#[cfg(target_os = "macos")]
const TRAY_LABEL: &str = "io.echoes.tray";
#[cfg(windows)]
const TRAY_TASK: &str = "Echoes Tray";
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

/// Version de la copie installée, lue en la lançant.
pub fn installed_version() -> Option<String> {
    let out = Command::new(bin_path().ok()?).arg("--version").output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).split_whitespace().last()?.to_string())
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
    // Sous Windows, l'icône en cours d'exécution verrouille le programme : on l'arrête, `ensure_tray` la relance.
    #[cfg(windows)]
    remove_tray_process();
    let dest = bin_path()?;
    fs::create_dir_all(dest.parent().unwrap())?;
    let me = std::env::current_exe()?;
    if fs::canonicalize(&me).ok() != fs::canonicalize(&dest).ok() {
        // Copie du contenu seul : `fs::copy` recopierait aussi la marque de quarantaine que macOS
        // pose sur ce qui vient d'un navigateur, et le service serait alors bloqué au lancement.
        let tmp = dest.with_extension("new");
        fs::write(&tmp, fs::read(&me).context("lecture du programme impossible")?).context("copie du programme impossible")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
        }
        if fs::rename(&tmp, &dest).is_err() {
            fs::copy(&tmp, &dest)?;
            let _ = fs::remove_file(&tmp);
        }
        if cfg!(target_os = "macos") {
            quiet(Command::new("xattr").args(["-d", "com.apple.quarantine"]).arg(&dest));
        }
    }
    let path_note = ensure_path()?;

    // 3. Service lancé à chaque ouverture de session, relancé s'il s'arrête.
    let active = start_service(&dest)?;
    status::warm_up(&dest);
    // Icône de la barre des menus : relancée sur le nouveau programme si elle tournait, créée sinon.
    restart_tray_if_running();
    if let Err(e) = ensure_tray() {
        eprintln!("echoes: icône de la barre des menus non installée : {e:#}");
    }

    // 4. Consigne globale pour Claude Code.
    let claude = claude_instructions(true)?;

    println!("Echoes {} est installé : {}", env!("CARGO_PKG_VERSION"), dest.display());
    if active {
        println!("Service actif, lancé automatiquement à chaque ouverture de session, mis à jour automatiquement.");
    } else if !cfg!(target_os = "macos") {
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
    remove_tray();
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
fn domains() -> [String; 2] {
    let uid = unsafe { libc::getuid() };
    [format!("gui/{uid}"), format!("user/{uid}")]
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
    // Sans session graphique (installation par SSH), le domaine `gui` n'existe pas : repli sur `user`.
    for _ in 0..4 {
        for domain in domains() {
            if quiet(Command::new("launchctl").args(["bootstrap", &domain]).arg(&file)) {
                return Ok(true);
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    println!("Le service n'a pas pu démarrer maintenant (aucune session ouverte sur ce Mac ?). Il démarrera à la prochaine ouverture de session ; journal : {}", log.display());
    Ok(false)
}

#[cfg(target_os = "macos")]
fn stop_service() {
    for domain in domains() {
        let _ = quiet(Command::new("launchctl").args(["bootout", &format!("{domain}/{LABEL}")]));
    }
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
    // Seulement le service : l'icône (`echoes tray`) continue de tourner.
    let _ = quiet(Command::new("powershell").args(["-NoProfile", "-Command", &kill_filter(std::process::id(), "serve")]));
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

/// Commande PowerShell qui arrête les processus `echoes.exe … <sub>` (`serve` ou `tray`), sauf `except_pid`.
#[cfg_attr(not(windows), allow(dead_code))]
fn kill_filter(except_pid: u32, sub: &str) -> String {
    format!(
        "Get-CimInstance Win32_Process -Filter \"Name='echoes.exe'\" | Where-Object {{ $_.ProcessId -ne {except_pid} -and $_.CommandLine -and $_.CommandLine.TrimEnd().EndsWith(' {sub}') }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force }}"
    )
}

// ---------- Icône de la barre des menus (`echoes tray`) ----------

#[cfg(target_os = "macos")]
fn tray_plist_path() -> Result<PathBuf> {
    Ok(home()?.join("Library/LaunchAgents").join(format!("{TRAY_LABEL}.plist")))
}

/// Agent launchd de l'icône : sessions graphiques seulement (`Aqua`), relancé seulement après un plantage
/// (« Quitter » sort avec 0 et n'est pas relancé), au plus une fois par minute.
#[cfg(target_os = "macos")]
fn tray_plist(bin: &Path, log: &Path) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{TRAY_LABEL}</string>
  <key>ProgramArguments</key><array><string>{}</string><string>tray</string></array>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>60</integer>
  <key>ProcessType</key><string>Interactive</string>
  <key>AssociatedBundleIdentifiers</key><string>io.echoes.app</string>
  <key>StandardOutPath</key><string>{}</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
        esc(&bin.display().to_string()),
        esc(&log.display().to_string()),
        esc(&log.display().to_string()),
    )
}

/// Programme lancé pour l'icône : celui de `Echoes.app` s'il est installé. macOS n'autorise l'accès au
/// réseau local qu'à une app (bundle) ; un programme nu lancé par launchd ne verrait aucune autre machine.
/// L'icône tourne donc dans le programme de l'app : elle est mise à jour par un nouveau DMG, pas par la
/// mise à jour automatique (qui ne remplace que la copie installée).
#[cfg(target_os = "macos")]
fn tray_program() -> Result<PathBuf> {
    let app = PathBuf::from("/Applications/Echoes.app/Contents/MacOS/Echoes");
    if app.exists() {
        Ok(app)
    } else {
        bin_path()
    }
}

#[cfg(target_os = "macos")]
fn gui_domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

#[cfg(target_os = "macos")]
fn tray_state() -> Option<String> {
    let out = Command::new("launchctl").args(["print", &format!("{}/{TRAY_LABEL}", gui_domain())]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Crée le démarrage automatique de l'icône s'il manque, puis la lance si elle ne tourne pas.
/// Sans session graphique (installation par SSH), ne fait rien.
#[cfg(target_os = "macos")]
pub fn ensure_tray() -> Result<()> {
    if !quiet(Command::new("launchctl").args(["print", &gui_domain()])) {
        return Ok(());
    }
    let file = tray_plist_path()?;
    let plist = tray_plist(&tray_program()?, &home()?.join("Library/Logs/echoes-tray.log"));
    if fs::read_to_string(&file).ok().as_deref() != Some(plist.as_str()) {
        fs::create_dir_all(file.parent().unwrap())?;
        fs::write(&file, &plist)?;
        let _ = quiet(Command::new("launchctl").args(["bootout", &format!("{}/{TRAY_LABEL}", gui_domain())]));
    }
    match tray_state() {
        None => {
            // `bootout` est asynchrone : quelques essais.
            for _ in 0..4 {
                if quiet(Command::new("launchctl").args(["bootstrap", &gui_domain()]).arg(&file)) {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            anyhow::bail!("launchctl bootstrap {TRAY_LABEL} a échoué")
        }
        Some(state) if !state.contains("state = running") => {
            quiet(Command::new("launchctl").args(["kickstart", &format!("{}/{TRAY_LABEL}", gui_domain())]));
            Ok(())
        }
        Some(_) => Ok(()),
    }
}

/// Après une mise à jour : relance l'icône sur le nouveau programme, seulement si elle tournait
/// (une icône fermée par « Quitter » le reste).
#[cfg(target_os = "macos")]
pub fn restart_tray_if_running() {
    if tray_state().is_some_and(|s| s.contains("state = running")) {
        quiet(Command::new("launchctl").args(["kickstart", "-k", &format!("{}/{TRAY_LABEL}", gui_domain())]));
    }
}

#[cfg(target_os = "macos")]
fn remove_tray() {
    let _ = quiet(Command::new("launchctl").args(["bootout", &format!("{}/{TRAY_LABEL}", gui_domain())]));
    if let Ok(f) = tray_plist_path() {
        let _ = fs::remove_file(f);
    }
}

#[cfg(windows)]
fn tray_task_state() -> Option<String> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!("(Get-ScheduledTask -TaskName '{TRAY_TASK}' -ErrorAction Stop).State")])
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(windows)]
pub fn ensure_tray() -> Result<()> {
    let state = match tray_task_state() {
        Some(s) => s,
        None => {
            let action = format!("conhost.exe --headless \"{}\" tray", bin_path()?.display());
            let ok = Command::new("schtasks").args(["/Create", "/F", "/SC", "ONLOGON", "/RL", "LIMITED", "/TN", TRAY_TASK, "/TR", &action]).status()?.success();
            if !ok {
                anyhow::bail!("création de la tâche planifiée « {TRAY_TASK} » impossible");
            }
            String::new()
        }
    };
    if state != "Running" {
        quiet(Command::new("schtasks").args(["/Run", "/TN", TRAY_TASK]));
    }
    Ok(())
}

/// Arrête l'icône : la tâche planifiée, puis le programme lui-même (`schtasks /End` n'arrête que `conhost`).
#[cfg(windows)]
fn remove_tray_process() {
    quiet(Command::new("schtasks").args(["/End", "/TN", TRAY_TASK]));
    quiet(Command::new("powershell").args(["-NoProfile", "-Command", &kill_filter(std::process::id(), "tray")]));
}

#[cfg(windows)]
pub fn restart_tray_if_running() {
    if tray_task_state().as_deref() == Some("Running") {
        remove_tray_process();
        quiet(Command::new("schtasks").args(["/Run", "/TN", TRAY_TASK]));
    }
}

#[cfg(windows)]
fn remove_tray() {
    remove_tray_process();
    quiet(Command::new("schtasks").args(["/Delete", "/F", "/TN", TRAY_TASK]));
}

#[cfg(target_os = "linux")]
pub fn ensure_tray() -> Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn restart_tray_if_running() {}

#[cfg(target_os = "linux")]
fn remove_tray() {}

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
    } else if cfg!(target_os = "macos") {
        // Lancée depuis le Finder, l'app ne connaît pas toujours le shell : zsh est celui de macOS.
        ".zshrc"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn tray_plist_is_gui_only() {
        let p = tray_plist(Path::new("/Users/a/.local/bin/echoes"), Path::new("/Users/a/Library/Logs/echoes-tray.log"));
        assert!(p.contains("<key>LimitLoadToSessionType</key><string>Aqua</string>"));
        assert!(p.contains("<key>SuccessfulExit</key><false/>"));
        assert!(p.contains("<key>ThrottleInterval</key><integer>60</integer>"));
        assert!(p.contains("<string>io.echoes.tray</string>"));
        assert!(p.contains("<string>/Users/a/.local/bin/echoes</string><string>tray</string>"));
    }

    #[test]
    fn serve_kill_filter_spares_tray() {
        let f = kill_filter(4242, "serve");
        assert!(f.contains("' serve'"), "ne vise que les processus `serve` : {f}");
        assert!(f.contains("4242"), "épargne le processus courant : {f}");
        assert!(!f.contains("/IM echoes.exe"));
    }

    #[test]
    fn tray_kill_filter_targets_tray_only() {
        let f = kill_filter(7, "tray");
        assert!(f.contains("' tray'") && !f.contains("' serve'"), "{f}");
    }
}
