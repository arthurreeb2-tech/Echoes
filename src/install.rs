//! Installation : copie du programme, démarrage automatique du service, consigne pour Claude Code.

use crate::config;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(any(target_os = "macos", test))]
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

    // 4. Consigne globale pour Claude Code.
    let claude = claude_instructions(true)?;

    println!("Echoes {} est installé : {}", env!("CARGO_PKG_VERSION"), dest.display());
    if active {
        println!("Service actif, lancé à chaque ouverture de session et mis à jour automatiquement.");
    } else if !cfg!(target_os = "macos") {
        println!("Démarrage automatique indisponible ici : lance `echoes serve` toi-même pour prêter cette machine.");
    }
    if let Some(note) = path_note {
        println!("{note}");
    }
    if claude {
        println!("Claude Code utilisera `echoes run --` pour les builds et les tests lourds (~/.claude/CLAUDE.md).");
    }
    firewall_hint();
    if let Some(code) = new_code {
        println!("\nPremière machine : sur chacune des autres, installe Echoes avec ce code :\n\n  {code}\n");
        println!("Ce code donne le droit d'exécuter des commandes sur tes machines : ne le partage avec personne.");
    }
    println!("Tableau de bord : `echoes dashboard`.");
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
fn domains() -> [String; 2] {
    let uid = unsafe { libc::getuid() };
    [format!("gui/{uid}"), format!("user/{uid}")]
}

#[cfg(any(target_os = "macos", test))]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Contenu de l'agent launchd : lance `bin serve` à l'ouverture de session et le relance s'il s'arrête.
#[cfg(any(target_os = "macos", test))]
fn launchd_plist(bin: &Path, path: &str, shell: &str, log: &Path) -> String {
    let log = xml_escape(&log.display().to_string());
    format!(
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
        xml_escape(&bin.display().to_string()),
        xml_escape(path),
        xml_escape(shell),
        log,
        log,
    )
}

#[cfg(target_os = "macos")]
fn start_service(bin: &Path) -> Result<bool> {
    let path = std::env::var("PATH").unwrap_or_default();
    let path = format!("{}:{path}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin", bin_dir()?.display());
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let log = home()?.join("Library/Logs/echoes.log");
    let plist = launchd_plist(bin, &path, &shell, &log);
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
    println!("Le service n'a pas pu démarrer maintenant (aucune session ouverte sur ce Mac ?). Il démarrera à la prochaine ouverture de session ; journal : {}", log.display());
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
            println!("Le pare-feu macOS est actif : s'il demande d'autoriser « echoes » à recevoir des connexions, accepte.");
        }
    }
}

// ---------- Linux : service systemd utilisateur ----------

#[cfg(target_os = "linux")]
fn unit_path() -> Result<PathBuf> {
    Ok(dirs::config_dir().ok_or_else(|| anyhow!("dossier de configuration introuvable"))?.join("systemd/user/echoes.service"))
}

/// Unité systemd utilisateur. Le chemin du programme est mis entre guillemets s'il contient un espace.
#[cfg(any(target_os = "linux", test))]
fn systemd_unit(bin: &Path, path: &str, shell: &str) -> String {
    let bin = bin.display().to_string();
    let exec = if bin.contains(char::is_whitespace) { format!("\"{bin}\"") } else { bin };
    format!(
        "[Unit]\nDescription=Echoes\nAfter=network-online.target\n\n[Service]\nExecStart={exec} serve\nRestart=always\nRestartSec=5\nEnvironment=\"PATH={path}\"\nEnvironment=\"SHELL={shell}\"\n\n[Install]\nWantedBy=default.target\n"
    )
}

#[cfg(target_os = "linux")]
fn start_service(bin: &Path) -> Result<bool> {
    let path = format!("{}:{}", bin_dir()?.display(), std::env::var("PATH").unwrap_or_default());
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let unit = systemd_unit(bin, &path, &shell);
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

/// Commande de la tâche planifiée ; `conhost --headless` évite la fenêtre de console.
#[cfg(any(windows, test))]
fn windows_task_action(bin: &Path) -> String {
    format!("conhost.exe --headless \"{}\" serve", bin.display())
}

#[cfg(windows)]
fn start_service(bin: &Path) -> Result<bool> {
    let action = windows_task_action(bin);
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
    println!("Si le pare-feu Windows demande d'autoriser « echoes » sur les réseaux privés, accepte.");
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
    let rc = home()?.join(profile_name(&std::env::var("SHELL").unwrap_or_default(), cfg!(target_os = "macos")));
    add_path_line(&rc)?;
    Ok(Some(format!("Chemin ajouté dans {} : ouvre un nouveau terminal pour utiliser la commande `echoes`.", rc.display())))
}

const PATH_LINE: &str = "export PATH=\"$HOME/.local/bin:$PATH\"  # echoes";

/// Fichier de profil du shell où ajouter le dossier du programme au PATH.
fn profile_name(shell: &str, macos: bool) -> &'static str {
    if shell.ends_with("zsh") {
        ".zshrc"
    } else if shell.ends_with("bash") && macos {
        ".bash_profile"
    } else if shell.ends_with("bash") {
        ".bashrc"
    } else if macos {
        // Lancée depuis le Finder, l'app ne connaît pas toujours le shell : zsh est celui de macOS.
        ".zshrc"
    } else {
        ".profile"
    }
}

/// Ajoute la ligne du PATH à la fin du profil, une seule fois.
fn add_path_line(rc: &Path) -> Result<()> {
    let current = fs::read_to_string(rc).unwrap_or_default();
    if !current.contains(PATH_LINE) {
        fs::write(rc, format!("{current}{}{PATH_LINE}\n", if current.is_empty() || current.ends_with('\n') { "" } else { "\n" }))?;
    }
    Ok(())
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
    claude_instructions_in(&home()?.join(".claude"), add)
}

/// Ajoute (ou retire) le bloc Echoes de `dir/CLAUDE.md`, sans toucher au reste du fichier.
/// Sans dossier `.claude`, Claude Code n'est pas utilisé ici : rien n'est créé.
fn claude_instructions_in(dir: &Path, add: bool) -> Result<bool> {
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

    /// Dossier temporaire unique, supprimé à la fin du test.
    struct Tmp(PathBuf);
    impl Tmp {
        fn new(name: &str) -> Tmp {
            let dir = std::env::temp_dir().join(format!("echoes-install-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn plist_runs_serve_with_right_binary() {
        let plist = launchd_plist(Path::new("/Users/a/.local/bin/echoes"), "/a:/b", "/bin/zsh", Path::new("/Users/a/Library/Logs/echoes.log"));
        assert!(plist.starts_with("<?xml"));
        assert!(plist.contains("<string>/Users/a/.local/bin/echoes</string><string>serve</string>"));
        assert!(plist.contains("<key>KeepAlive</key><true/>") && plist.contains("<key>RunAtLoad</key><true/>"));
        assert!(plist.contains("<key>PATH</key><string>/a:/b</string>"));
        assert!(plist.contains("<key>SHELL</key><string>/bin/zsh</string>"));
        assert_eq!(plist.matches("<string>/Users/a/Library/Logs/echoes.log</string>").count(), 2);
    }

    #[test]
    fn plist_escapes_spaces_and_xml_characters() {
        let plist = launchd_plist(Path::new("/Users/A & B/<bin>/echoes"), "/x y:/p&q", "/bin/a>b", Path::new("/Users/A & B/log.log"));
        assert!(plist.contains("<string>/Users/A &amp; B/&lt;bin&gt;/echoes</string><string>serve</string>"));
        assert!(plist.contains("<string>/x y:/p&amp;q</string>"));
        assert!(plist.contains("<string>/bin/a&gt;b</string>"));
        assert!(plist.contains("<string>/Users/A &amp; B/log.log</string>"));
        // Plus aucun caractère spécial brut dans les valeurs.
        assert!(!plist.contains("A & B"));
        assert!(!plist.contains("<bin>"));
    }

    #[test]
    fn systemd_unit_has_exec_and_environment() {
        let unit = systemd_unit(Path::new("/home/a/.local/bin/echoes"), "/home/a/.local/bin:/usr/bin", "/bin/bash");
        assert!(unit.contains("\nExecStart=/home/a/.local/bin/echoes serve\n"));
        assert!(unit.contains("\nRestart=always\n"));
        assert!(unit.contains("Environment=\"PATH=/home/a/.local/bin:/usr/bin\"\n"));
        assert!(unit.contains("Environment=\"SHELL=/bin/bash\"\n"));
        assert!(unit.contains("[Install]\nWantedBy=default.target\n"));
    }

    #[test]
    fn systemd_unit_quotes_path_with_spaces() {
        let unit = systemd_unit(Path::new("/home/jean dupont/.local/bin/echoes"), "/usr/bin", "/bin/sh");
        assert!(unit.contains("\nExecStart=\"/home/jean dupont/.local/bin/echoes\" serve\n"), "{unit}");
    }

    #[test]
    fn windows_task_runs_headless_serve() {
        assert_eq!(
            windows_task_action(Path::new(r"C:\Users\Jean Dupont\AppData\Local\Programs\Echoes\echoes.exe")),
            r#"conhost.exe --headless "C:\Users\Jean Dupont\AppData\Local\Programs\Echoes\echoes.exe" serve"#
        );
    }

    #[test]
    fn profile_follows_shell_and_system() {
        assert_eq!(profile_name("/bin/zsh", false), ".zshrc");
        assert_eq!(profile_name("/bin/zsh", true), ".zshrc");
        assert_eq!(profile_name("/bin/bash", true), ".bash_profile");
        assert_eq!(profile_name("/usr/bin/bash", false), ".bashrc");
        assert_eq!(profile_name("", true), ".zshrc");
        assert_eq!(profile_name("/usr/bin/fish", false), ".profile");
    }

    #[test]
    fn path_line_is_added_once() {
        let tmp = Tmp::new("path");
        let rc = tmp.0.join(".zshrc");
        add_path_line(&rc).unwrap();
        add_path_line(&rc).unwrap();
        let text = fs::read_to_string(&rc).unwrap();
        assert_eq!(text, format!("{PATH_LINE}\n"));
    }

    #[test]
    fn path_line_keeps_existing_profile() {
        let tmp = Tmp::new("path-keep");
        let rc = tmp.0.join(".bashrc");
        fs::write(&rc, "alias ll='ls -l'").unwrap();
        add_path_line(&rc).unwrap();
        add_path_line(&rc).unwrap();
        let text = fs::read_to_string(&rc).unwrap();
        assert_eq!(text, format!("alias ll='ls -l'\n{PATH_LINE}\n"));
        assert_eq!(text.matches("# echoes").count(), 1);
    }

    #[test]
    fn claude_block_is_not_created_without_claude_dir() {
        let tmp = Tmp::new("claude-none");
        let dir = tmp.0.join(".claude");
        assert!(!claude_instructions_in(&dir, true).unwrap());
        assert!(!dir.exists());
    }

    #[test]
    fn claude_block_is_idempotent_and_keeps_user_text() {
        let tmp = Tmp::new("claude-add");
        let dir = tmp.0.join(".claude");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("CLAUDE.md");
        fs::write(&file, "# Mes règles\n\nRépondre en français.\n").unwrap();
        assert!(claude_instructions_in(&dir, true).unwrap());
        let once = fs::read_to_string(&file).unwrap();
        assert!(once.starts_with("# Mes règles\n\nRépondre en français.\n\n"));
        assert!(once.contains(MARK_BEGIN) && once.contains("echoes run --") && once.ends_with(&format!("{MARK_END}\n")));
        // Deuxième installation : fichier identique, un seul bloc.
        assert!(claude_instructions_in(&dir, true).unwrap());
        let twice = fs::read_to_string(&file).unwrap();
        assert_eq!(once, twice);
        assert_eq!(twice.matches(MARK_BEGIN).count(), 1);
    }

    #[test]
    fn claude_block_is_replaced_in_place_and_removed_cleanly() {
        let tmp = Tmp::new("claude-remove");
        let dir = tmp.0.join(".claude");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("CLAUDE.md");
        fs::write(&file, format!("avant\n\n{MARK_BEGIN}\nancien bloc\n{MARK_END}\n")).unwrap();
        claude_instructions_in(&dir, true).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(!text.contains("ancien bloc") && text.contains("délégation de calcul") && text.starts_with("avant\n\n"));
        // Retrait : il ne reste que le texte de l'utilisateur.
        assert!(!claude_instructions_in(&dir, false).unwrap());
        assert_eq!(fs::read_to_string(&file).unwrap(), "avant\n");
        // Retirer deux fois ne change rien.
        claude_instructions_in(&dir, false).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "avant\n");
    }

    #[test]
    fn claude_block_removal_without_file_creates_nothing_useful() {
        let tmp = Tmp::new("claude-nofile");
        let dir = tmp.0.join(".claude");
        fs::create_dir_all(&dir).unwrap();
        claude_instructions_in(&dir, false).unwrap();
        assert!(!dir.join("CLAUDE.md").exists());
    }
}
