//! Deux « machines » sur la même : une prête sa puissance, l'autre lance une commande.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_echoes");

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("echoes-test-{}-{name}", std::process::id()));
    fs::create_dir_all(&d).unwrap();
    d
}

fn echoes(home: &Path, dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN).args(args).current_dir(dir).env("ECHOES_HOME", home).env("ECHOES_NO_UPDATE", "1").output().unwrap()
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

#[test]
fn run_on_another_machine() {
    let (a, b, proj) = (tmp("a"), tmp("b"), tmp("proj"));
    let out = echoes(&a, &a, &["pair"]);
    let code = String::from_utf8_lossy(&out.stdout).split_whitespace().find(|w| w.len() == 64).unwrap().to_string();
    assert!(echoes(&b, &b, &["pair", &code]).status.success());

    let port = 40000 + (std::process::id() % 20000) as u16;
    let _worker = Kill(
        Command::new(BIN)
            .args(["serve", "--port", &port.to_string()])
            .env("ECHOES_HOME", &a)
            .env("ECHOES_NO_UPDATE", "1")
            .spawn()
            .unwrap(),
    );
    std::thread::sleep(Duration::from_millis(1500));

    fs::create_dir_all(proj.join(".git")).unwrap();
    fs::create_dir_all(proj.join("sub")).unwrap();
    fs::write(proj.join(".gitignore"), "build/\n").unwrap();
    fs::write(proj.join("top.txt"), "top\n").unwrap();
    fs::write(proj.join("sub/in.txt"), "in\n").unwrap();

    let host = format!("127.0.0.1:{port}");
    let script = "cat ../top.txt in.txt; echo changed >> in.txt; mkdir -p ../build; echo bin > ../build/out; echo oops >&2; exit 7";
    let out = echoes(&b, &proj.join("sub"), &["run", "--host", &host, "--pull", "../build", "--", "sh", "-c", script]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(7), "stderr: {stderr}");
    assert_eq!(stdout, "top\nin\n");
    assert!(stderr.contains("oops"));
    assert_eq!(fs::read_to_string(proj.join("sub/in.txt")).unwrap(), "in\nchanged\n");

    // --pull est relatif au dossier courant : `../build` depuis `sub`.
    assert!(proj.join("build/out").exists(), "stderr: {stderr}");

    // Mauvais code d'appairage : refus net.
    let c = tmp("c");
    echoes(&c, &c, &["pair"]);
    let out = echoes(&c, &proj, &["run", "--host", &host, "--", "true"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("refusé"));
}
