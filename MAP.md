# MAP : Echoes

| Élément | Fichiers | État |
|---|---|---|
| Cœur (réseau, sync, exécution) | `src/client.rs`, `src/worker.rs`, `src/proto.rs`, `src/sync.rs`, `src/discover.rs` | testé sous Linux (e2e), revu |
| Délégation selon la charge (carte graphique, mémoire) | `src/status.rs`, `Status::score` dans `src/proto.rs` | testé avec une carte graphique simulée ; mesure réelle à confirmer sur Mac |
| Installation et service | `src/install.rs`, `install.sh`, `install.ps1` | testé sous Linux ; macOS et Windows non testés en réel |
| Mises à jour | `src/update.rs`, `.github/workflows/release.yml` | testé avec une version simulée |
| Tableau de bord | `src/dashboard.rs`, `src/dashboard.html`, `src/history.rs` | testé, épuré |
| App et DMG macOS | `packaging/macos/` | jamais construit (il faut un Mac) |
| Page de téléchargement | `docs/index.html`, `.github/workflows/pages.yml` | testée en local |
| Consignes Claude Code | `CLAUDE.md` | à jour |

**En attente d'Arthur** : dépôt public, branche `main`, GitHub Pages, iMac pour compiler le DMG.

**À confirmer sur un vrai Mac** : mesure de la carte graphique (`ioreg`), accès au réseau local (macOS 15+), démarrage du service, Gatekeeper.

**Prochaine étape** : compilation du DMG sur l'iMac, puis essai entre deux Mac.
