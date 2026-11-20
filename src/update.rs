//! Mise à jour automatique depuis les versions publiées sur GitHub.

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::process::Command;

pub const REPO: &str = "arthurreeb2-tech/Echoes";
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
}

/// Adresse des fichiers de la dernière version publiée (modifiable pour un fork ou un test).
fn base() -> String {
    std::env::var("ECHOES_UPDATE_URL").unwrap_or_else(|_| format!("https://github.com/{REPO}/releases/latest/download"))
}

pub fn enabled() -> bool {
    std::env::var_os("ECHOES_NO_UPDATE").is_none()
}

fn asset() -> String {
    if cfg!(windows) {
        format!("echoes-{TARGET}.exe")
    } else {
        format!("echoes-{TARGET}")
    }
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
pub fn check() -> Result<Option<Release>> {
    let body = fetch(&format!("{}/version.txt", base())).map_err(|e| anyhow!("{e} (le dépôt doit être public et avoir au moins une version publiée)"))?;
    let tag = String::from_utf8_lossy(&body).trim().to_string();
    let tag = tag.as_str();
    let latest = parse(tag).ok_or_else(|| anyhow!("numéro de version illisible : {tag}"))?;
    let current = parse(env!("CARGO_PKG_VERSION")).unwrap();
    Ok((latest > current).then(|| Release { version: tag.trim_start_matches('v').to_string() }))
}

/// Télécharge, vérifie l'empreinte SHA-256, puis remplace le programme en place.
pub fn install(release: &Release) -> Result<()> {
    let url = format!("{}/{}", base(), asset());
    let binary = fetch(&url)?;
    let expected = String::from_utf8(fetch(&format!("{url}.sha256"))?)?;
    let expected = expected.split_whitespace().next().unwrap_or_default().to_lowercase();
    let actual = hex::encode(Sha256::digest(&binary));
    if expected != actual {
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
    #[test]
    fn versions() {
        assert_eq!(super::parse("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(super::parse("0.10.0\n"), Some((0, 10, 0)));
        assert!(super::parse("v0.10.0") > super::parse("v0.9.9"));
        assert_eq!(super::parse("latest"), None);
        assert!(super::newer("0.4.0", "0.3.9") && !super::newer("0.3.0", "0.3.0") && !super::newer("x", "0.1.0"));
    }
}
