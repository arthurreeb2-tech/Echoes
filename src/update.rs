//! Mise à jour automatique depuis les versions publiées sur GitHub.

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::process::Command;

/// Dépôt des versions publiées : les machines y cherchent leurs mises à jour.
pub const REPO: &str = "arthurreeb2-tech/Echoes-Maj";
/// Ancien emplacement, gardé en secours tant que `Echoes-Maj` n'a pas encore de version.
pub const LEGACY_REPO: &str = "arthurreeb2-tech/Echoes";
const TARGET: &str = env!("ECHOES_TARGET");

static EXE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// À appeler au démarrage : une fois le fichier remplacé, le système ne donne plus le bon chemin.
pub fn remember_exe() {
    if let Ok(p) = std::env::current_exe() {
        let _ = EXE.set(p);
    }
}

pub struct Release {
    pub version: String,
    /// Adresse où cette version a été trouvée : le programme est téléchargé au même endroit.
    base: String,
}

/// Adresses des fichiers de la dernière version publiée, dans l'ordre d'essai
/// (`ECHOES_UPDATE_URL`, une ou plusieurs adresses séparées par des espaces, les remplace
/// pour un fork ou un test).
fn bases() -> Vec<String> {
    match std::env::var("ECHOES_UPDATE_URL") {
        Ok(urls) => urls.split_whitespace().map(str::to_string).collect(),
        Err(_) => [REPO, LEGACY_REPO].iter().map(|r| format!("https://github.com/{r}/releases/latest/download")).collect(),
    }
}

pub fn enabled() -> bool {
    std::env::var_os("ECHOES_NO_UPDATE").is_none()
}

fn asset() -> String {
    asset_name(TARGET, cfg!(windows))
}

fn asset_name(target: &str, windows: bool) -> String {
    format!("echoes-{target}{}", if windows { ".exe" } else { "" })
}

/// `curl` est présent sur macOS, Windows 10+ et presque tous les Linux : pas de pile TLS à embarquer.
fn fetch(url: &str) -> Result<Vec<u8>> {
    let out = Command::new("curl")
        .args(["-fsSL", "--proto", if url.starts_with("file://") { "=file" } else { "=https" }, "--max-time", "300", url])
        .output()
        .context("curl introuvable")?;
    if !out.status.success() {
        bail!("{url} : {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

/// Vrai si la version `a` est strictement plus récente que `b`.
pub fn newer(a: &str, b: &str) -> bool {
    matches!((parse(a), parse(b)), (Some(a), Some(b)) if a > b)
}

fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next()??, it.next()??))
}

/// Renvoie la dernière version publiée si elle est plus récente que celle-ci.
///
/// Tous les dépôts sont consultés et la version la plus récente l'emporte : si la
/// publication sur l'un échoue un jour, les machines suivent quand même l'autre.
pub fn check() -> Result<Option<Release>> {
    let answers = bases().into_iter().map(|base| {
        let tag = fetch(&format!("{base}/version.txt")).map(|b| String::from_utf8_lossy(&b).trim().to_string());
        (base, tag)
    });
    select(answers, env!("CARGO_PKG_VERSION"))
}

/// Parmi les réponses des dépôts (adresse, numéro de version ou erreur), garde la plus récente
/// si elle dépasse `current`. Une erreur n'est rapportée que si aucun dépôt n'a répondu.
fn select(answers: impl IntoIterator<Item = (String, Result<String>)>, current: &str) -> Result<Option<Release>> {
    let mut best: Option<((u64, u64, u64), Release)> = None;
    let mut last_err = None;
    for (base, tag) in answers {
        let tag = match tag {
            Ok(t) => t,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        let Some(v) = parse(&tag) else {
            last_err = Some(anyhow!("numéro de version illisible : {tag}"));
            continue;
        };
        if best.as_ref().is_none_or(|(b, _)| v > *b) {
            best = Some((v, Release { version: tag.trim_start_matches('v').to_string(), base }));
        }
    }
    let Some((latest, release)) = best else {
        let e = last_err.unwrap_or_else(|| anyhow!("aucune adresse de mise à jour"));
        bail!("{e} (le dépôt {REPO} doit être public et avoir au moins une version publiée)");
    };
    let current = parse(current).context("version courante illisible")?;
    Ok((latest > current).then_some(release))
}

/// Vrai si `binary` a l'empreinte SHA-256 annoncée (premier mot de `expected`, majuscules admises).
fn checksum_matches(binary: &[u8], expected: &str) -> bool {
    let expected = expected.split_whitespace().next().unwrap_or_default().to_lowercase();
    !expected.is_empty() && expected == hex::encode(Sha256::digest(binary))
}

/// Télécharge, vérifie l'empreinte SHA-256, puis remplace le programme en place.
pub fn install(release: &Release) -> Result<()> {
    let url = format!("{}/{}", release.base, asset());
    let binary = fetch(&url)?;
    let expected = String::from_utf8(fetch(&format!("{url}.sha256"))?)?;
    if !checksum_matches(&binary, &expected) {
        bail!("empreinte incorrecte pour {} : fichier refusé", asset());
    }
    let tmp = std::env::temp_dir().join(format!("echoes-update-{}", std::process::id()));
    std::fs::write(&tmp, &binary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    // Essai avant mise en place : un programme qui ne démarre pas ici ne remplace jamais celui qui marche.
    let trial = Command::new(&tmp).arg("--version").output();
    let starts = matches!(&trial, Ok(o) if o.status.success() && String::from_utf8_lossy(&o.stdout).contains(&release.version));
    if !starts {
        let _ = std::fs::remove_file(&tmp);
        bail!("la version {} ne démarre pas sur cette machine : mise à jour abandonnée, la version actuelle est conservée", release.version);
    }
    // Copie de secours de la version actuelle (`echoes.prev`, à côté du programme).
    if let Some(exe) = EXE.get().cloned().or_else(|| std::env::current_exe().ok()) {
        let _ = std::fs::copy(&exe, exe.with_extension("prev"));
    }
    let result = self_replace::self_replace(&tmp);
    let _ = std::fs::remove_file(&tmp);
    result.context("remplacement du programme impossible")
}

/// Relance le service avec la nouvelle version.
pub fn restart() -> ! {
    let exe = EXE.get().cloned().or_else(|| std::env::current_exe().ok()).expect("chemin du programme");
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = Command::new(&exe).args(&args).exec();
        eprintln!("echoes: redémarrage impossible : {err}");
        std::process::exit(1);
    }
    #[cfg(not(unix))]
    {
        let _ = Command::new(&exe).args(&args).spawn();
        std::process::exit(0);
    }
}

pub fn run_cli(check_only: bool) -> Result<()> {
    println!("Version installée : {} ({TARGET})", env!("CARGO_PKG_VERSION"));
    match check()? {
        None => println!("Déjà à jour"),
        Some(r) if check_only => println!("Nouvelle version disponible : {}", r.version),
        Some(r) => {
            install(&r)?;
            println!("Mis à jour vers {}", r.version);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(base: &str, tag: &str) -> (String, Result<String>) {
        (base.to_string(), Ok(tag.to_string()))
    }

    #[test]
    fn versions() {
        assert_eq!(parse("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse("0.10.0\n"), Some((0, 10, 0)));
        assert!(parse("v0.10.0") > parse("v0.9.9"));
        assert_eq!(parse("latest"), None);
        assert!(newer("0.4.0", "0.3.9") && !newer("0.3.0", "0.3.0") && !newer("x", "0.1.0"));
    }

    #[test]
    fn numeric_comparison_not_textual() {
        assert!(newer("0.4.10", "0.4.9"));
        assert!(!newer("0.4.9", "0.4.10"));
        assert!(newer("1.0.0", "0.99.99"));
    }

    #[test]
    fn prerelease_and_short_versions_are_unreadable() {
        assert_eq!(parse("1.2.3-beta"), None);
        assert_eq!(parse("1.2"), None);
        assert!(!newer("1.2.4-beta", "1.2.3"));
    }

    #[test]
    fn newest_repo_wins_whatever_the_order() {
        let r = select([answer("maj", "v0.5.0"), answer("legacy", "v0.5.2")], "0.4.0").unwrap().unwrap();
        assert_eq!((r.version.as_str(), r.base.as_str()), ("0.5.2", "legacy"));
        let r = select([answer("maj", "v0.5.2"), answer("legacy", "v0.5.0")], "0.4.0").unwrap().unwrap();
        assert_eq!((r.version.as_str(), r.base.as_str()), ("0.5.2", "maj"));
    }

    #[test]
    fn tie_keeps_first_repo() {
        let r = select([answer("maj", "0.5.0"), answer("legacy", "v0.5.0")], "0.4.0").unwrap().unwrap();
        assert_eq!(r.base, "maj");
    }

    #[test]
    fn nothing_when_up_to_date_or_older() {
        assert!(select([answer("maj", "0.4.0")], "0.4.0").unwrap().is_none());
        assert!(select([answer("maj", "0.3.0")], "0.4.0").unwrap().is_none());
    }

    #[test]
    fn one_repo_down_is_not_an_error() {
        let r = select([("maj".to_string(), Err(anyhow!("503"))), answer("legacy", "0.5.0")], "0.4.0").unwrap().unwrap();
        assert_eq!(r.base, "legacy");
    }

    #[test]
    fn all_repos_down_or_unreadable_is_an_error() {
        let e = select([("maj".to_string(), Err(anyhow!("503")))], "0.4.0").err().unwrap().to_string();
        assert!(e.contains("503") && e.contains(REPO), "{e}");
        let e = select([answer("maj", "latest")], "0.4.0").err().unwrap().to_string();
        assert!(e.contains("illisible"), "{e}");
        assert!(select([], "0.4.0").is_err());
    }

    #[test]
    fn checksum_right_wrong_missing() {
        let data = b"echoes";
        let good = hex::encode(Sha256::digest(data));
        assert!(checksum_matches(data, &good));
        assert!(checksum_matches(data, &format!("{} echoes-x86_64\n", good.to_uppercase())));
        assert!(!checksum_matches(data, &"0".repeat(64)));
        assert!(!checksum_matches(b"autre", &good));
        assert!(!checksum_matches(data, ""));
        assert!(!checksum_matches(data, "  \n"));
    }

    #[test]
    fn asset_name_depends_on_platform() {
        assert_eq!(asset_name("aarch64-apple-darwin", false), "echoes-aarch64-apple-darwin");
        assert_eq!(asset_name("x86_64-pc-windows-msvc", true), "echoes-x86_64-pc-windows-msvc.exe");
    }

    #[test]
    fn bases_follow_override_variable() {
        let _guard = crate::config::test_support::env_lock();
        std::env::set_var("ECHOES_UPDATE_URL", "file:///a  file:///b");
        assert_eq!(bases(), ["file:///a", "file:///b"]);
        std::env::remove_var("ECHOES_UPDATE_URL");
        let defaults = bases();
        assert!(defaults[0].contains(REPO) && defaults[1].contains(LEGACY_REPO));
    }
}
