# Echoes : consignes pour Claude Code

Echoes exécute une commande sur la machine la plus disponible du réseau local et rapatrie le résultat. Un seul binaire Rust ; réponds et écris les messages de l'outil en français.

## Quand l'utilisateur dit « installe »

Installer Echoes sur la machine où tourne cette session, sans poser de question inutile :

1. **Code d'appairage.** S'il en donne un, l'utiliser. Sinon, s'il existe déjà une clé sur cette machine, `echoes install` la réutilise. Sinon, cette machine est la première : `echoes install` sans code en crée un.
2. **Installer**, depuis la racine du dépôt : `./install.sh [code]`. Le script télécharge la dernière version publiée et vérifie son empreinte ; s'il n'y en a pas, il compile depuis les sources.
3. **Si la compilation est nécessaire et que Rust manque** :
   - macOS : `xcode-select -p` ; s'il échoue, `xcode-select --install` (une fenêtre s'ouvre, l'utilisateur doit la valider), attendre la fin.
   - puis `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y` et `. "$HOME/.cargo/env"`.
4. **Vérifier** :
   - `~/.local/bin/echoes ls` liste la machine (et les autres déjà installées) ;
   - macOS : `launchctl print gui/$(id -u)/io.echoes.serve | grep state` doit indiquer `running` ; journal dans `~/Library/Logs/echoes.log` ;
   - Linux : `systemctl --user status echoes`.
5. **Ouvrir le tableau de bord** si l'utilisateur est devant la machine : `~/.local/bin/echoes dashboard`.
6. **Rendre compte** en quelques lignes : installé ou non, le code d'appairage s'il vient d'être créé (secret, à garder pour les autres machines), et les éventuelles validations à faire à la main (pare-feu, accès au réseau local).

Windows : `irm https://raw.githubusercontent.com/arthurreeb2-tech/Echoes/main/install.ps1 | iex` (variable `ECHOES_CODE` pour le code). Ne jamais l'installer sur un poste professionnel sans l'accord de l'utilisateur, qui doit avoir validé ce point avec son service informatique.

### Installer sur une autre machine depuis celle-ci

Utile pour le Mac mini sans écran. Il faut « Session à distance » activée sur la machine cible (Réglages Système > Général > Partage) ; le code s'obtient avec `echoes pair`.

- Si une version est publiée : `ssh <hôte> 'curl -fsSL https://raw.githubusercontent.com/arthurreeb2-tech/Echoes/main/install.sh | sh -s -- <code>'`
- Sinon, compiler ici pour l'architecture de la cible (`ssh <hôte> uname -m`) : pour `arm64`, `rustup target add aarch64-apple-darwin && cargo build --release --target aarch64-apple-darwin`, puis `scp target/aarch64-apple-darwin/release/echoes <hôte>:/tmp/echoes && ssh <hôte> '/tmp/echoes install <code>'`.

## Veille (maintenance périodique)

Chaque version publiée est installée automatiquement sur toutes les machines : une erreur ici se propage partout. Prudence d'abord.

1. Partir de `main` à jour, sur une branche dédiée. Ne jamais pousser sur `main`.
2. Lire les tickets GitHub ouverts du dépôt ; corriger les bugs avérés.
3. `cargo update`, puis vérifier les nouvelles versions majeures des dépendances de `Cargo.toml` ; ne les adopter que si elles apportent quelque chose (sécurité, correctif, performance) et que les tests passent.
4. Failles connues : `cargo install cargo-audit --locked` puis `cargo audit` ; corriger.
5. `cargo build --locked`, `cargo test --locked`, `cargo clippy` : tout doit passer.
6. S'il y a un changement : augmenter la version dans `Cargo.toml` (correctif : 0.2.x → 0.2.x+1 ; nouvelle fonction : mineure). Si le format des messages réseau change, augmenter aussi `VERSION` dans `src/proto.rs` et le signaler dans la PR (les machines ne se parlent plus tant qu'elles n'ont pas toutes la même version).
7. Ouvrir une pull request qui résume les changements et leur raison. La publication part toute seule quand la PR est fusionnée sur `main` (`.github/workflows/release.yml`).
8. S'il n'y a rien à faire, ne rien ouvrir et le dire.

Ne jamais affaiblir la sécurité : clé d'appairage, chiffrement Noise, refus des chemins qui sortent du dossier de travail (`sync::resolve`), vérification SHA-256 des mises à jour.

## Repères dans le code

- `src/client.rs` : choix de la machine, envoi du projet, sortie en direct, rapatriement, repli local.
- `src/worker.rs` : service `echoes serve`, exécution à priorité basse, mise à jour automatique quand aucune tâche ne tourne.
- `src/proto.rs` : messages et canal chiffré. `src/sync.rs` : inventaire et transfert de fichiers.
- `src/install.rs` : installation (launchd, systemd, tâche planifiée Windows), PATH, consigne globale Claude Code.
- `src/update.rs` : mise à jour depuis les versions GitHub.
- `src/history.rs` : journal des tâches. `src/dashboard.rs` + `src/dashboard.html` : tableau de bord local (127.0.0.1:47601, jeton dans l'adresse après `#`, contrôle de l'en-tête Host).
- `packaging/macos/` : app `Echoes.app` (sans icône dans le Dock) et installateur `.pkg`. Sous Windows, l'installateur est le programme lui-même : lancé sans argument, il s'installe puis ouvre le tableau de bord.
- `tests/e2e.rs` : deux « machines » sur la même, de bout en bout.
