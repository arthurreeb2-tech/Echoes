# Journal de boucle : revue avant DMG

| Tour | Point | Gravité | Action | Statut |
|---|---|---|---|---|
| 1 (3a) | build/test/clippy, 4 cibles (Linux, Windows, macOS x86_64 et arm64) | — | contrôle | vert |
| 1 (3a) | shellcheck install.sh, build-dmg.sh | — | contrôle | vert |
| 1 (3a) | actionlint : SC2193 faux positif sur `${{ matrix.target }}` dans release.yml | MINEUR | cible passée par variable d'environnement | corrigé |
| 1 (3a) | cargo audit : 0 faille ; `bincode` 1.3.3 non maintenu (RUSTSEC-2025-0141) | MINEUR | à remplacer | ouvert |
