//! Deux « machines » sur la même : une prête sa puissance, l'autre lance une commande.
#![cfg(unix)]

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::RwLock;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_echoes");

/// Le service lance aussi le tableau de bord sur 127.0.0.1:47601 : le test du tableau de bord
/// (qui a besoin de ce port) attend que plus aucun service de test ne tourne.
static DASHBOARD_PORT: RwLock<()> = RwLock::new(());

/// Dossier temporaire propre à un test, supprimé à la fin.
struct Tmp(PathBuf);

impl Deref for Tmp {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tmp(name: &str) -> Tmp {
    let d = std::env::temp_dir().join(format!("echoes-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    Tmp(d)
}

fn echoes_env(home: &Path, dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    Command::new(BIN).args(args).current_dir(dir).env("ECHOES_HOME", home).env("ECHOES_NO_UPDATE", "1").envs(env.iter().copied()).output().unwrap()
}

fn echoes(home: &Path, dir: &Path, args: &[&str]) -> Output {
    echoes_env(home, dir, &[], args)
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Un port libre : on le réserve un instant pour que le système nous le donne, puis on le relâche.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Attend (par sondage) que le service accepte les connexions.
fn wait_listening(port: u16) {
    let end = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < end, "le service n'écoute pas sur le port {port}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn pair_code(home: &Path) -> String {
    let out = echoes(home, home, &["pair"]);
    String::from_utf8_lossy(&out.stdout).split_whitespace().find(|w| w.len() == 64).unwrap().to_string()
}

/// Deux machines appairées : `worker_home` sert (`echoes serve`), `client_home` lance les commandes depuis `proj`.
struct Lab {
    // Ordre des champs : le service est arrêté avant de rendre le verrou et d'effacer les dossiers.
    _worker: Kill,
    _lock: std::sync::RwLockReadGuard<'static, ()>,
    _worker_home: Tmp,
    client_home: Tmp,
    proj: Tmp,
    host: String,
}

impl Lab {
    fn new(name: &str) -> Lab {
        Lab::with_worker_env(name, &[])
    }

    fn with_worker_env(name: &str, worker_env: &[(&str, &str)]) -> Lab {
        let lock = DASHBOARD_PORT.read().unwrap_or_else(|e| e.into_inner());
        let (worker_home, client_home, proj) = (tmp(&format!("{name}-w")), tmp(&format!("{name}-c")), tmp(&format!("{name}-p")));
        let code = pair_code(&worker_home);
        assert!(echoes(&client_home, &client_home, &["pair", &code]).status.success());
        let port = free_port();
        let worker = Command::new(BIN)
            .args(["serve", "--port", &port.to_string()])
            .env("ECHOES_HOME", &*worker_home)
            .env("ECHOES_NO_UPDATE", "1")
            .envs(worker_env.iter().copied())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let worker = Kill(worker);
        wait_listening(port);
        Lab { _worker: worker, _lock: lock, _worker_home: worker_home, client_home, proj, host: format!("127.0.0.1:{port}") }
    }

    /// `echoes run --host <worker> <args> -- <command>` depuis `dir` (dossier du projet par défaut).
    fn run_in(&self, dir: &Path, opts: &[&str], command: &[&str]) -> Output {
        let mut args = vec!["run", "--host", &self.host];
        args.extend_from_slice(opts);
        args.push("--");
        args.extend_from_slice(command);
        echoes(&self.client_home, dir, &args)
    }

    fn run(&self, opts: &[&str], command: &[&str]) -> Output {
        self.run_in(&self.proj, opts, command)
    }

    fn sh(&self, script: &str) -> Output {
        self.run(&[], &["sh", "-c", script])
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn run_on_another_machine() {
    let lab = Lab::new("run");
    let proj = &lab.proj;
    fs::create_dir_all(proj.join(".git")).unwrap();
    fs::create_dir_all(proj.join("sub")).unwrap();
    fs::write(proj.join(".gitignore"), "build/\n").unwrap();
    fs::write(proj.join("top.txt"), "top\n").unwrap();
    fs::write(proj.join("sub/in.txt"), "in\n").unwrap();

    let script = "cat ../top.txt in.txt; echo changed >> in.txt; mkdir -p ../build; echo bin > ../build/out; echo oops >&2; exit 7";
    let out = lab.run_in(&proj.join("sub"), &["--pull", "../build"], &["sh", "-c", script]);
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(7), "stderr: {stderr}");
    assert_eq!(stdout, "top\nin\n");
    assert!(stderr.contains("oops"));
    assert_eq!(fs::read_to_string(proj.join("sub/in.txt")).unwrap(), "in\nchanged\n");

    // --pull est relatif au dossier courant : `../build` depuis `sub`.
    assert!(proj.join("build/out").exists(), "stderr: {stderr}");

    // La tâche figure dans le journal du tableau de bord, avec sa destination.
    let log = fs::read_to_string(lab.client_home.join("echoes/history.jsonl")).unwrap();
    assert!(log.contains("\"remote\":true") && log.contains("\"exit\":7"), "{log}");

    // Mauvais code d'appairage : refus net.
    let other = tmp("run-other");
    echoes(&other, &other, &["pair"]);
    let out = echoes(&other, proj, &["run", "--host", &lab.host, "--", "true"]);
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("refusé"));
}

#[test]
fn exit_code_and_streams_are_propagated() {
    let lab = Lab::new("exit");
    let out = lab.sh("echo out; echo err >&2; exit 3");
    assert_eq!(out.status.code(), Some(3), "stderr: {}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "out\n");
    assert!(text(&out.stderr).contains("err\n"));
    assert_eq!(lab.sh("true").status.code(), Some(0));
    assert_eq!(lab.run(&[], &["false"]).status.code(), Some(1));
    // Tuée par un signal : 128 + numéro du signal, comme un shell.
    assert_eq!(lab.sh("kill -9 $$").status.code(), Some(137));
    // Les tâches sont bien parties sur l'autre machine.
    let log = fs::read_to_string(lab.client_home.join("echoes/history.jsonl")).unwrap();
    assert!(log.lines().all(|l| l.contains("\"remote\":true")), "{log}");
}

#[test]
fn produced_files_come_back() {
    let lab = Lab::new("files");
    let proj = &lab.proj;
    fs::write(proj.join(".gitignore"), "ignored/\n").unwrap();
    fs::write(proj.join("keep.txt"), "avant\n").unwrap();

    let script = "echo apres > keep.txt; echo neuf > new.txt; mkdir -p deep/er; echo d > deep/er/f.txt; \
                  printf '#!/bin/sh\\necho hi\\n' > tool.sh; chmod 755 tool.sh; mkdir -p ignored; echo i > ignored/x";
    let out = lab.sh(script);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    assert_eq!(fs::read_to_string(proj.join("keep.txt")).unwrap(), "apres\n");
    assert_eq!(fs::read_to_string(proj.join("new.txt")).unwrap(), "neuf\n");
    assert_eq!(fs::read_to_string(proj.join("deep/er/f.txt")).unwrap(), "d\n");
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(proj.join("tool.sh")).unwrap().permissions().mode() & 0o777, 0o755);
    }
    // Ce que .gitignore exclut ne revient que sur demande.
    assert!(!proj.join("ignored").exists());
    let out = lab.run(&["--pull", "ignored"], &["sh", "-c", "mkdir -p ignored; echo i > ignored/x"]);
    assert!(out.status.success(), "stderr: {}", text(&out.stderr));
    assert_eq!(fs::read_to_string(proj.join("ignored/x")).unwrap(), "i\n");
}

#[test]
fn local_changes_are_synced_between_runs() {
    let lab = Lab::new("resync");
    let proj = &lab.proj;
    fs::write(proj.join("a.txt"), "un\n").unwrap();
    fs::write(proj.join("b.txt"), "b\n").unwrap();
    assert_eq!(text(&lab.run(&[], &["cat", "a.txt"]).stdout), "un\n");
    // Fichier modifié ici : la nouvelle version est envoyée ; fichier supprimé ici : il disparaît là-bas.
    fs::write(proj.join("a.txt"), "deux\n").unwrap();
    fs::remove_file(proj.join("b.txt")).unwrap();
    let out = lab.sh("cat a.txt; test ! -e b.txt && echo absent");
    assert_eq!(text(&out.stdout), "deux\nabsent\n", "stderr: {}", text(&out.stderr));
}

#[test]
fn symlinks_are_neither_sent_nor_followed() {
    let lab = Lab::new("symlink");
    let proj = &lab.proj;
    let outside = tmp("symlink-out");
    fs::write(outside.join("secret.txt"), "secret\n").unwrap();
    fs::write(outside.join("target.txt"), "intact\n").unwrap();
    std::os::unix::fs::symlink(outside.join("secret.txt"), proj.join("link.txt")).unwrap();
    std::os::unix::fs::symlink(outside.join("target.txt"), proj.join("victim.txt")).unwrap();
    fs::write(proj.join("plain.txt"), "plain\n").unwrap();

    // Le lien n'est pas envoyé : la commande ne voit pas le fichier qu'il désigne.
    let out = lab.sh("cat plain.txt; test ! -e link.txt && echo pas-de-lien");
    assert_eq!(text(&out.stdout), "plain\npas-de-lien\n", "stderr: {}", text(&out.stderr));

    // Un fichier créé là-bas à l'emplacement d'un lien local ne l'écrase pas et n'écrit pas à travers.
    let out = lab.sh("echo pirate > victim.txt");
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out.stderr).contains("victim.txt n'a pas été rapatrié"), "stderr: {}", text(&out.stderr));
    assert_eq!(fs::read_to_string(outside.join("target.txt")).unwrap(), "intact\n");
    assert!(fs::symlink_metadata(proj.join("victim.txt")).unwrap().file_type().is_symlink());
}

#[test]
fn git_directory_is_neither_sent_nor_written() {
    let lab = Lab::new("git");
    let proj = &lab.proj;
    fs::create_dir_all(proj.join(".git/hooks")).unwrap();
    fs::write(proj.join(".git/hooks/pre-commit"), "#!/bin/sh\necho local\n").unwrap();
    fs::write(proj.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(proj.join("f.txt"), "f\n").unwrap();

    // Rien de .git n'est envoyé.
    let out = lab.sh("test ! -e .git/HEAD && echo sans-git");
    assert_eq!(text(&out.stdout), "sans-git\n", "stderr: {}", text(&out.stderr));

    // Ce que la commande y écrit, même demandé par --pull, n'arrive pas dans le .git local.
    let script = "mkdir -p .git/hooks; echo evil > .git/hooks/pre-commit; echo evil > .git/config";
    let out = lab.run(&["--pull", ".git"], &["sh", "-c", script]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    assert_eq!(fs::read_to_string(proj.join(".git/hooks/pre-commit")).unwrap(), "#!/bin/sh\necho local\n");
    assert!(!proj.join(".git/config").exists());
    assert_eq!(fs::read_to_string(proj.join(".git/HEAD")).unwrap(), "ref: refs/heads/main\n");
}

#[test]
fn large_output_is_never_truncated() {
    let lab = Lab::new("big");
    let out = lab.sh("seq 1 400000; seq 1 100000 >&2");
    assert_eq!(out.status.code(), Some(0));
    let expected: String = (1..=400_000).map(|n| format!("{n}\n")).collect();
    assert!(out.stdout == expected.as_bytes(), "sortie standard tronquée ou altérée : {} octets au lieu de {}", out.stdout.len(), expected.len());
    let err = text(&out.stderr);
    assert!(err.ends_with("100000\n"), "erreur standard tronquée");
    assert_eq!(err.lines().filter(|l| l.parse::<u32>().is_ok()).count(), 100_000);
}

#[test]
fn unpaired_machine_is_ignored_and_command_runs_here() {
    let lab = Lab::new("unpaired");
    // Autre clé d'appairage : le service est joignable mais refuse, la commande tourne ici.
    let stranger = tmp("unpaired-s");
    echoes(&stranger, &stranger, &["pair"]);
    fs::write(lab.proj.join("f.txt"), "f\n").unwrap();
    let out = echoes_env(&stranger, &lab.proj, &[("ECHOES_HOSTS", &lab.host)], &["run", "--", "sh", "-c", "cat f.txt; exit 4"]);
    assert_eq!(out.status.code(), Some(4));
    assert_eq!(text(&out.stdout), "f\n");
    assert!(text(&out.stderr).contains("exécution ici"), "stderr: {}", text(&out.stderr));
    let log = fs::read_to_string(stranger.join("echoes/history.jsonl")).unwrap();
    assert!(log.contains("\"remote\":false") && log.contains("\"exit\":4"), "{log}");

    // En visant la machine à la main : refus net, rien ne tourne.
    let out = echoes(&stranger, &lab.proj, &["run", "--host", &lab.host, "--", "touch", "marque"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("refusé"), "stderr: {}", text(&out.stderr));
    assert!(!lab.proj.join("marque").exists());
}

#[test]
fn without_worker_the_command_runs_locally() {
    let (home, proj) = (tmp("alone-h"), tmp("alone-p"));
    pair_code(&home);
    let dead = format!("127.0.0.1:{}", free_port());
    let env = [("ECHOES_HOSTS", dead.as_str())];
    let out = echoes_env(&home, &proj, &env, &["run", "--", "sh", "-c", "pwd; exit 5"]);
    assert_eq!(out.status.code(), Some(5), "stderr: {}", text(&out.stderr));
    assert_eq!(fs::canonicalize(text(&out.stdout).trim()).unwrap(), fs::canonicalize(&*proj).unwrap(), "la commande tourne dans le dossier courant");
    assert!(text(&out.stderr).contains("exécution ici"), "stderr: {}", text(&out.stderr));
    assert!(echoes_env(&home, &proj, &env, &["run", "-q", "--", "touch", "fait"]).status.success());
    assert!(proj.join("fait").exists());

    // Machine visée absente, ou exécution distante exigée sans machine : erreur claire, rien ne tourne ici.
    let out = echoes_env(&home, &proj, &env, &["run", "--host", &dead, "--", "touch", "non"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("injoignable"), "stderr: {}", text(&out.stderr));
    let out = echoes_env(&home, &proj, &env, &["run", "--remote", "--", "touch", "non"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("aucune machine disponible"), "stderr: {}", text(&out.stderr));
    assert!(!proj.join("non").exists());

    let out = echoes_env(&home, &proj, &env, &["run", "--local", "--", "sh", "-c", "exit 2"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn ls_lists_the_worker() {
    let lab = Lab::new("ls");
    let out = echoes_env(&lab.client_home, &lab.client_home, &[("ECHOES_HOSTS", &lab.host)], &["ls"]);
    assert!(out.status.success(), "stderr: {}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("cette machine"), "{stdout}");
    assert!(stdout.contains(&lab.host) && stdout.contains("puissance"), "{stdout}");
    assert!(!stdout.contains("Aucune autre machine"), "{stdout}");
    assert_eq!(stdout.lines().count(), 2, "{stdout}");

    // Sans appairage commun, l'autre machine n'apparaît pas.
    let stranger = tmp("ls-s");
    echoes(&stranger, &stranger, &["pair"]);
    let out = echoes_env(&stranger, &stranger, &[("ECHOES_HOSTS", &lab.host)], &["ls"]);
    let stdout = text(&out.stdout);
    assert!(!stdout.contains(&lab.host) && stdout.contains("Aucune autre machine"), "{stdout}");
}

#[test]
fn busy_worker_is_not_chosen_and_is_throttled_when_forced() {
    let lab = Lab::with_worker_env("busy", &[("ECHOES_FAKE_GPU", "100")]);
    let out = echoes_env(&lab.client_home, &lab.client_home, &[("ECHOES_HOSTS", &lab.host)], &["ls"]);
    assert!(text(&out.stdout).contains("(occupée)"), "{}", text(&out.stdout));

    // Sans --host : la machine occupée n'est pas choisie, la commande tourne ici.
    let out = echoes_env(&lab.client_home, &lab.proj, &[("ECHOES_HOSTS", &lab.host)], &["run", "--", "sh", "-c", "echo ici-$ECHOES_MAX_JOBS-"]);
    // (Si la machine de test est elle-même très chargée, la distante peut rester la meilleure : alors elle bride.)
    if text(&out.stderr).contains("→") {
        assert!(text(&out.stdout).trim_start_matches("ici-").trim_end().trim_end_matches('-').parse::<u32>().is_ok(), "tâche non bridée : {}", text(&out.stdout));
    } else {
        assert_eq!(text(&out.stdout), "ici--\n", "stderr: {}", text(&out.stderr));
        assert!(text(&out.stderr).contains("occupée"), "stderr: {}", text(&out.stderr));
    }

    // Forcée : elle accepte une tâche, bridée.
    let out = lab.sh("echo $ECHOES_MAX_JOBS");
    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    let jobs: u32 = text(&out.stdout).trim().parse().unwrap_or_else(|_| panic!("ECHOES_MAX_JOBS absent : {}", text(&out.stdout)));
    assert!(jobs >= 1);
}

fn http(request: &str) -> String {
    use std::io::{Read, Write};
    // Sous forte charge, la première connexion peut être coupée sans réponse : on réessaie les lectures (GET).
    let attempts = if request.starts_with("GET") { 5 } else { 1 };
    let mut out = String::new();
    for _ in 0..attempts {
        let mut s = std::net::TcpStream::connect("127.0.0.1:47601").unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(request.as_bytes()).unwrap();
        out.clear();
        let _ = s.read_to_string(&mut out);
        if !out.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    out
}

fn post(path: &str, token: &str, body: &str) -> String {
    http(&format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1:47601\r\nX-Echoes-Token: {token}\r\nContent-Length: {}\r\n\r\n{body}", body.len()))
}

#[test]
fn dashboard_requires_a_one_time_ticket() {
    use std::io::{BufRead, BufReader};
    let _port = DASHBOARD_PORT.write().unwrap_or_else(|e| e.into_inner());
    let home = tmp("dash");
    assert!(echoes(&home, &home, &["pair"]).status.success());
    let mut child = Command::new(BIN)
        .arg("dashboard")
        .env("ECHOES_HOME", &*home)
        .env("ECHOES_NO_UPDATE", "1")
        .env("ECHOES_NO_BROWSER", "1")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let _server = Kill(child);
    let ticket = line.trim().rsplit('#').next().unwrap().to_string();
    assert!(ticket.contains('.'), "adresse inattendue : {line}");

    // Sans session : refus. Avec un autre nom d'hôte (DNS rebinding) : refus.
    let first = http("GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:47601\r\n\r\n");
    assert!(first.starts_with("HTTP/1.1 401"), "réponse : {first:?}");
    assert!(http("GET / HTTP/1.1\r\nHost: evil.example:47601\r\n\r\n").starts_with("HTTP/1.1 403"));
    assert!(post("/api/session", "", "{\"ticket\":\"1.aa.bb\"}").starts_with("HTTP/1.1 401"));

    // Le billet ouvre une session, une seule fois.
    let reply = post("/api/session", "", &format!("{{\"ticket\":\"{ticket}\"}}"));
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    let token = reply.rsplit("\"token\":\"").next().unwrap().trim_end_matches("\"}").to_string();
    assert!(post("/api/session", "", &format!("{{\"ticket\":\"{ticket}\"}}")).starts_with("HTTP/1.1 401"));

    let state = http(&format!("GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:47601\r\nX-Echoes-Token: {token}\r\n\r\n"));
    assert!(state.starts_with("HTTP/1.1 200") && state.contains("\"machines\""), "{state}");
    assert!(post("/api/pair", &token, "pas du json").starts_with("HTTP/1.1 400"));
}
