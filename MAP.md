# MAP : Echoes

| Élément | Fichiers | État |
|---|---|---|
| Cœur (réseau, sync, exécution) | `src/client.rs`, `src/worker.rs`, `src/proto.rs`, `src/sync.rs`, `src/discover.rs`, `src/status.rs` | testé sous Linux (e2e) |
| Installation et service | `src/install.rs`, `install.sh`, `install.ps1` | testé sous Linux ; macOS et Windows non testés en réel |
| Mises à jour | `src/update.rs`, `.github/workflows/release.yml` | testé avec une version simulée |
| Tableau de bord | `src/dashboard.rs`, `src/dashboard.html`, `src/history.rs` | testé, épuré |
| App et DMG macOS | `packaging/macos/` | jamais construit (il faut un Mac) |
| Page de téléchargement | `docs/index.html`, `.github/workflows/pages.yml` | testée en local |
| Consignes Claude Code | `CLAUDE.md` | à jour |

**En attente d'Arthur** : dépôt public, branche `main`, GitHub Pages, iMac pour compiler le DMG.

**Prochaine étape** : revue en boucle (`contexte/loop.md`), puis compilation du DMG sur l'iMac.
