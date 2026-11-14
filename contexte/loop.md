# Journal de boucle : revue avant DMG

| Tour | Point | Gravité | Action | Statut |
|---|---|---|---|---|
| 1 (3a) | build/test/clippy, 4 cibles (Linux, Windows, macOS x86_64 et arm64) | — | contrôle | vert |
| 1 (3a) | shellcheck install.sh, build-dmg.sh | — | contrôle | vert |
| 1 (3a) | actionlint : SC2193 faux positif sur `${{ matrix.target }}` dans release.yml | MINEUR | cible passée par variable d'environnement | corrigé |
| 1 (3a) | cargo audit : 0 faille ; `bincode` 1.3.3 non maintenu (RUSTSEC-2025-0141) | MINEUR | à remplacer | ouvert |
| 1 (3b-C) | Jeton du tableau de bord permanent, visible dans `ps` (argument de `open`), la sortie standard et l'historique du navigateur → accès à la clé d'appairage | GRAVE | à traiter | ouvert |
| 1 (3b-C) | Port 47601 squattable : `echoes dashboard` ouvre le navigateur sur un faux serveur sans vérifier son identité | GRAVE | à traiter | ouvert |
| 1 (3b-C) | Serveur HTTP : un thread par connexion sans limite, délai par lecture et non par requête, boucle CPU si `accept` échoue | GRAVE | à traiter | ouvert |
| 1 (3b-C) | Page publique : boutons vers des fichiers absents tant qu'aucune version n'est publiée | GRAVE | à traiter | ouvert |
| 1 (3b-C) | Fichiers secrets créés lisibles puis chmod (fenêtre) ; comparaison du jeton non constante ; erreurs API sans réponse ni message ; message « redémarre » faux et clé remplacée sans confirmation ; totaux plafonnés sans le dire ; gain compté sur les échecs ; NaN si 0 cœur ; écritures non atomiques ; historique non protégé ; détection iPad/Linux ; promesses inexactes de la page | MINEUR | à traiter | ouvert |
