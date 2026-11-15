# Echoes dans la barre des menus — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** une icône Echoes permanente (barre des menus macOS, zone de notification Windows) qui montre l'état du réseau et rouvre le tableau de bord, plus une vraie icône pour `Echoes.app`.

**Architecture:** nouvelle sous-commande cachée `echoes tray` dans le binaire existant. Un fil rafraîchit toutes les 15 s un modèle pur (`tray::model`) à partir des fonctions existantes (`status::local`, `client::find`, `history::read`, `machines.json`) ; la boucle d'événements principale affiche l'icône et le menu. Démarrage par un second agent launchd (`io.echoes.tray`, sessions graphiques seulement) ou une seconde tâche planifiée Windows (« Echoes Tray »).

**Tech Stack:** Rust 2021, `tray-icon` + `muda` (+ boucle `tao` ou équivalent, au choix du constructeur), launchd, schtasks.

**Spec:** `contexte/2026-11-15-barre-des-menus-design.md`

## Global Constraints

- Messages et libellés en français ; code et noms techniques en anglais.
- Dépendances d'interface **uniquement** sous `[target.'cfg(any(target_os = "macos", windows))'.dependencies]` ; module `tray` compilé sous la même condition. Linux : `echoes tray` affiche « L'icône de la barre des menus n'est pas disponible sous Linux. » et sort avec 0.
- Rafraîchissement toutes les **15 s**, dans un fil séparé ; aucun appel `client::fetch_history` vers les pairs depuis l'icône.
- `machines.json` est lu, jamais écrit par l'icône.
- Aucun nouveau port, aucune modification de `proto::VERSION`.
- Ouvrir le tableau de bord = lancer le processus `<bin_path> dashboard`, jamais `dashboard::open` dans le processus de l'icône.
- macOS : agent `io.echoes.tray`, domaine `gui/<uid>` uniquement, `LimitLoadToSessionType` = `Aqua`, `KeepAlive` = `{SuccessfulExit: false}`, `ThrottleInterval` = 60. Processus en politique « accessoire » (pas d'icône Dock).
- Windows : tâche « Echoes Tray », ONLOGON, `/RL LIMITED`, sans console.
- Taille : viser moins de +1,5 Mo par binaire release.
- Critères du SPEC : `cargo build --locked` et `cargo clippy --locked --all-targets -- -D warnings` sans avertissement, `cargo test --locked` vert (macOS ici ; Linux et Windows via la CI), `cargo audit` propre.
- Commits : `type: description courte` en français, ligne de fin `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Ne jamais pousser sur `main`.

## Review Focus

1. Pas de clé d'appairage (machine fraîchement installée) → icône grise, menu « Pas encore appairée », pas de plantage. → test `no_key_is_grey` (Task 1).
2. Le service local arrêté pendant que l'icône tourne → passe au gris au rafraîchissement suivant, ne reste pas vert. → test `service_down_is_grey_even_with_peers` (Task 1).
3. Une machine hors ligne depuis longtemps dans `machines.json` → affichée « hors ligne », jamais comptée comme libre. → test `offline_peer_never_counts_as_free` (Task 1).
4. Sous Windows, arrêter le service (installation, mise à jour) ne doit pas tuer l'icône. → test `serve_kill_filter_spares_tray` (Task 4).
5. Session sans affichage (SSH) → `echoes tray` sort avec 0 sans boucle de relance ; l'agent n'est jamais chargé hors session Aqua. → test `tray_plist_is_gui_only` (Task 4) + vérification manuelle (Task 5).

---

### Task 1 : modèle de l'icône (logique pure)

**Files:**
- Create: `src/tray/model.rs` (tests dans le même fichier, `#[cfg(test)]`)
- Create: `src/tray/mod.rs` (pour l'instant : `pub mod model;`)
- Modify: `src/main.rs` (`mod tray;`, non conditionnel : le modèle compile partout)
- Modify: `src/dashboard.rs:319-331` — rendre `Machine` et ses champs `pub(crate)` pour réutilisation.

**Interfaces:**
- Produces:
  - `pub enum Light { Green, Orange, Grey }`
  - `pub struct Row { pub label: String, pub online: bool }`
  - `pub struct Summary { pub light: Light, pub tooltip: String, pub machines: Vec<Row>, pub activity: String }`
  - `pub fn summarize(me: Option<&Status>, known: &[Machine], peers_online: &[Status], records: &[Record], service_up: bool, has_key: bool, now: u64) -> Summary`

Règles (de la spec) :
- `Grey` si `!has_key` ou `!service_up`. Sinon `Green` si au moins un pair en ligne avec `!busy()` ; sinon `Orange`.
- Lignes : cette machine d'abord (« imac-de-arthur-1 (cette machine) — occupée » / « — libre · 2,1 cœurs »), puis pairs en ligne triés par `score()` décroissant (« arthurs-mac-mini-1 — libre · 7,8 cœurs »), puis machines de `known` absentes de `peers_online` (« … — hors ligne », `online: false`). Nombres au format français (virgule, une décimale).
- `activity` : tâches avec `ts >= now - 86400` et `remote == true` ; gain = Σ `est_local_ms.saturating_sub(total_ms)`. Texte : « 3 tâches en 24 h · 4 min gagnées » ; 0 tâche → « Aucune tâche déléguée en 24 h » ; gain < 1 min → « · moins d'une minute gagnée ». (*« en 24 h » plutôt qu'« aujourd'hui » : décision de Claude, évite le fuseau horaire.*)
- Sans clé : `machines` vide, `activity` = « Pas encore appairée : colle le code dans le tableau de bord ».
- `tooltip` : « Echoes — N machines en ligne, <nom du meilleur pair libre> libre » ; si aucun libre « Echoes — aucune autre machine libre » ; gris « Echoes — service arrêté » ou « Echoes — pas encore appairée ».

- [ ] **Step 1 : écrire les tests** dans `src/tray/model.rs`, avec un constructeur de test `status(id, name, free, gpu, mem_free) -> Status` :
  - `no_key_is_grey` : `has_key=false` → `Light::Grey`, `machines.is_empty()`, `activity` commence par « Pas encore appairée ».
  - `service_down_is_grey_even_with_peers` : un pair libre, `service_up=false` → `Grey`, tooltip « Echoes — service arrêté ».
  - `free_peer_is_green` : un pair `gpu=Some(1.0)`, `mem_free=5.8` → `Green` ; ligne pair = « mini — libre · 7,8 cœurs ».
  - `all_peers_busy_is_orange` : pair `gpu=Some(99.0)` → `Orange` ; ligne contient « occupée ».
  - `offline_peer_never_counts_as_free` : `known` contient un pair libre absent de `peers_online` → `Orange`, sa ligne finit par « hors ligne », `online == false`.
  - `activity_counts_last_24h_remote_only` : 3 enregistrements distants récents (gains 120 000 + 90 000 + 30 000 ms), 1 local récent, 1 distant vieux de 2 jours → « 3 tâches en 24 h · 4 min gagnées ».
- [ ] **Step 2 :** `cargo test --locked tray::model` → échec (fonction absente).
- [ ] **Step 3 :** implémenter `summarize` et les types.
- [ ] **Step 4 :** `cargo test --locked tray::model` → 6 tests PASS ; `cargo clippy --locked --all-targets -- -D warnings` propre.
- [ ] **Step 5 :** commit `feat: modèle d'état pour l'icône de la barre des menus`.

### Task 2 : dessin de l'icône et icône d'app

**Files:**
- Create: `src/icon.rs` (tests dans le fichier)
- Modify: `src/main.rs` — sous-commande cachée `#[command(hide = true)] Icon { dir: PathBuf }` qui écrit le jeu `Echoes.iconset` (16, 32, 128, 256, 512 px et leurs @2x, noms conformes à `iconutil`).
- Create: `packaging/macos/Echoes.icns` (généré une fois : `cargo run -- icon /tmp/x/Echoes.iconset && iconutil -c icns /tmp/x/Echoes.iconset -o packaging/macos/Echoes.icns`, puis commité).
- Modify: `packaging/macos/Info.plist` — `CFBundleIconFile` = `Echoes`.
- Modify: `packaging/macos/build-dmg.sh` — `mkdir -p "$app/Contents/Resources" && cp "$here/Echoes.icns" "$app/Contents/Resources/"`.

**Interfaces:**
- Produces: `pub fn render(size: u32, dot: Option<[u8; 4]>, app_style: bool) -> Vec<u8>` — RGBA, `size*size*4` octets. Glyphe : trois arcs concentriques (ondes) partant du coin bas-gauche. `app_style=false` (barre des menus) : fond transparent, glyphe gris neutre `#8E8E93` lisible en clair et en sombre (*pas d'image « template » : elle effacerait la pastille de couleur — décision de Claude*), pastille de rayon `size/5` en bas à droite si `dot` est fourni. `app_style=true` : carré arrondi plein (fond encre `#1F2430`, glyphe cuivre `#C8794A`), sans pastille. Bords lissés (couverture par sur-échantillonnage 4×4).
- Couleurs de pastille (consommées en Task 3) : `GREEN = [52,199,89,255]`, `ORANGE = [255,149,0,255]`, `GREY = [142,142,147,255]`.

- [ ] **Step 1 : tests** : `render_has_expected_len` (36 px → 5184 octets) ; `menu_icon_corners_transparent` (pixel (0,0) alpha 0 pour `app_style=false`) ; `dot_is_coloured` (pixel au centre de la pastille = `GREEN` à ±8 près) ; `app_icon_is_opaque_inside` (pixel central alpha 255).
- [ ] **Step 2 :** `cargo test --locked icon` → échec.
- [ ] **Step 3 :** implémenter `render` et la sous-commande `icon`.
- [ ] **Step 4 :** tests PASS ; générer `Echoes.icns` ; reconstruire le DMG (commande du README de la CI, chemins `target/*/debug`) ; `plutil -p` montre `CFBundleIconFile` ; ouvrir l'image et vérifier à l'œil l'icône dans le Finder.
- [ ] **Step 5 :** commit `feat: icône d'app Echoes et dessin de l'icône de barre des menus`.

### Task 3 : processus `echoes tray`

**Files:**
- Modify: `Cargo.toml` — `tray-icon`, `muda` (+ boucle d'événements choisie ; sur macOS l'API `objc2-app-kit` pour `setActivationPolicy(Accessory)` si la boucle ne l'offre pas) sous la cible `cfg(any(target_os = "macos", windows))`.
- Modify: `src/tray/mod.rs` — `pub fn run() -> Result<i32>` ; UI sous `#[cfg(any(target_os = "macos", windows))]`, variante Linux qui affiche le message de la contrainte globale.
- Modify: `src/client.rs` — `pub fn service_up(key: &[u8; 32]) -> bool` (sonde `127.0.0.1:DEFAULT_PORT` avec `probe`, délai court).
- Modify: `src/main.rs` — `#[command(hide = true)] Tray { #[arg(long)] ensure: bool }` ; `ensure` est branché en Task 4 (ici : `bail!` « non implémenté » si `true`).

**Interfaces:**
- Consumes: `tray::model::summarize`, `icon::render`, `install::bin_path`, `config::load_key`, `config::data_dir`.
- Produces: `tray::run() -> Result<i32>` ; `client::service_up`.

Comportement :
- Au démarrage : si aucun affichage n'est disponible (création de la boucle d'événements ou de l'icône en échec), écrire une ligne dans stderr et `return Ok(0)`.
- Fil de rafraîchissement : immédiatement puis toutes les 15 s, calcule `Summary` et l'envoie à la boucle (proxy d'événement utilisateur). La boucle remplace icône (`render(36, Some(couleur), false)` = 18 pt @2x), info-bulle et menu.
- Menu : lignes machines désactivées, ligne activité désactivée, séparateur, « Ouvrir le tableau de bord », « Quitter ».
- « Ouvrir le tableau de bord » : `Command::new(install::bin_path()?).arg("dashboard").spawn()` (repli `current_exe()` si `bin_path` n'existe pas).
- « Quitter » : sortie avec code 0 (launchd ne relance pas grâce à `SuccessfulExit: false`).
- Une erreur de rafraîchissement ne tue pas l'icône : elle passe au gris (`service_up=false`).

- [ ] **Step 1 :** ajouter les dépendances ; `cargo build --locked` (mettre à jour `Cargo.lock`) sur macOS ; noter la taille release avant/après (`cargo build --release` puis `ls -l`).
- [ ] **Step 2 :** implémenter `client::service_up` et `tray::run`.
- [ ] **Step 3 :** `cargo clippy --locked --all-targets -- -D warnings` propre ; `cargo test --locked` vert.
- [ ] **Step 4 : vérification manuelle sur l'iMac** : `cargo run -- tray` → icône visible en haut à droite, pas d'icône dans le Dock ; le menu liste l'iMac (occupée) et le Mac mini (libre) ; couleur verte ; « Ouvrir le tableau de bord » ouvre le navigateur ; `launchctl bootout gui/$(id -u)/io.echoes.serve` → gris en ≤ 15 s, puis `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/io.echoes.serve.plist` → redevient coloré ; « Quitter » ferme l'icône, code de sortie 0.
- [ ] **Step 5 :** commit `feat: icône Echoes dans la barre des menus`.

### Task 4 : démarrage automatique, migration, mises à jour

**Files:**
- Modify: `src/install.rs` — agent / tâche de l'icône, désinstallation, filtre `taskkill`.
- Modify: `src/main.rs` — branche sans argument (machine installée) : lancer `<bin_path> tray --ensure` (attendre sa fin, ignorer l'échec) avant `dashboard` ; `Tray { ensure: true }` → `install::ensure_tray()`.
- Modify: `src/worker.rs:90-101` — dans `auto_update`, après `update::install` réussi et avant `update::restart()`, appeler `install::restart_tray_if_running()`.

**Interfaces:**
- Produces (dans `install.rs`) :
  - `fn tray_plist(bin: &Path, log: &Path) -> String` (macOS)
  - `pub fn ensure_tray() -> Result<()>` — crée l'agent/la tâche s'il manque, puis le démarre s'il ne tourne pas. No-op sous Linux.
  - `pub fn restart_tray_if_running()` — relance seulement si l'icône tourne (macOS : `launchctl print gui/<uid>/io.echoes.tray` contient `state = running` → `launchctl kickstart -k`; Windows : `schtasks /Query /TN "Echoes Tray" /FO LIST` indique `Running` → `/End` puis `/Run`).
  - `fn serve_kill_filter() -> String` (Windows, testable partout) — commande PowerShell qui arrête les `echoes.exe` dont la ligne de commande se termine par ` serve`, sauf le PID courant.
- `install::run` appelle `ensure_tray()` après `start_service` (erreur affichée, pas fatale) ; `uninstall` retire agent/tâche de l'icône (`bootout` + suppression du plist ; `schtasks /End` + `/Delete`) ; `stop_service` Windows remplace le `taskkill /IM echoes.exe` par `serve_kill_filter()`.
- macOS : bootstrap **uniquement** dans `gui/<uid>` ; s'il n'existe pas (SSH), ne rien faire et ne rien afficher d'alarmant. Log : `~/Library/Logs/echoes-tray.log`.
- Windows : action `conhost.exe --headless "<bin>" tray`, `/SC ONLOGON /RL LIMITED /TN "Echoes Tray"`.

- [ ] **Step 1 : tests** (dans `install.rs`, `#[cfg(test)]`) :
  - `tray_plist_is_gui_only` (macOS) : contient `<key>LimitLoadToSessionType</key><string>Aqua</string>`, `<key>SuccessfulExit</key><false/>`, `<key>ThrottleInterval</key><integer>60</integer>`, `io.echoes.tray`, et l'argument `tray`.
  - `serve_kill_filter_spares_tray` (toutes plateformes, fonction non conditionnée) : la chaîne contient `serve` et l'exclusion du PID, et ne contient pas `/IM echoes.exe`.
- [ ] **Step 2 :** `cargo test --locked install` → échec.
- [ ] **Step 3 :** implémenter.
- [ ] **Step 4 :** tests PASS, clippy propre.
- [ ] **Step 5 : vérification sur l'iMac** : `cargo build --release && ./target/release/echoes install` → `launchctl print gui/$(id -u)/io.echoes.tray | grep state` = `running`, icône visible ; « Quitter » → l'icône reste fermée (`launchctl print` ne montre plus `running`) ; `open -a /Applications/Echoes.app` (après reconstruction et copie de l'app) → l'icône revient et le tableau de bord s'ouvre ; `echoes uninstall` puis `echoes install` remettent tout en place.
- [ ] **Step 6 :** commit `feat: démarrage automatique de l'icône et migration au lancement de l'app`.

### Task 5 : documentation et vérification finale

**Files:**
- Modify: `README.md` — section « Tableau de bord » : l'icône de la barre des menus (couleurs, menu, « Quitter », retour via l'app) ; « À savoir sur Mac » : un second élément d'ouverture « Echoes » apparaît.
- Modify: `CLAUDE.md` — « Repères dans le code » : `src/tray/`, `src/icon.rs` ; étape 4 de « installe » : vérifier aussi `io.echoes.tray`.

- [ ] **Step 1 :** mettre à jour les deux fichiers.
- [ ] **Step 2 :** `cargo build --release --locked --target x86_64-apple-darwin` et `--target aarch64-apple-darwin` sans avertissement ; `cargo clippy --locked --all-targets -- -D warnings` ; `cargo test --locked` ; `cargo audit` (installer `cargo-audit --locked` si absent).
- [ ] **Step 3 :** reconstruire `dist/Echoes-macOS.dmg`, l'installer sur l'iMac (copie dans `/Applications`, lancement) et refaire la vérification de Task 4 Step 5 depuis l'app.
- [ ] **Step 4 :** session sans affichage simulée : `ssh localhost '~/.local/bin/echoes tray; echo $?'` (si Session à distance est active ; sinon noter « non vérifié ») → sortie 0 immédiate.
- [ ] **Step 5 :** commit `docs: icône de la barre des menus`. Pousser la branche seulement avec l'accord d'Arthur (Windows et Linux ne sont vérifiés que par la CI après la poussée).

**Hors plan (reporté)** : icône intégrée à l'exécutable Windows.
