# Journal de boucle : revue avant DMG

| Tour | Point | Gravité | Action | Statut |
|---|---|---|---|---|
| 1 (3a) | build/test/clippy, 4 cibles (Linux, Windows, macOS x86_64 et arm64) | — | contrôle | vert |
| 1 (3a) | shellcheck install.sh, build-dmg.sh | — | contrôle | vert |
| 1 (3a) | actionlint : SC2193 faux positif sur `${{ matrix.target }}` dans release.yml | MINEUR | cible passée par variable d'environnement | corrigé |
| 1 (3a) | cargo audit : 0 faille ; `bincode` 1.3.3 non maintenu (RUSTSEC-2025-0141) | MINEUR | à remplacer | ouvert |
| 1 (3b-C) | Jeton du tableau de bord permanent, visible dans `ps` (argument de `open`), la sortie standard et l'historique du navigateur → accès à la clé d'appairage | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-C) | Port 47601 squattable : `echoes dashboard` ouvre le navigateur sur un faux serveur sans vérifier son identité | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-C) | Serveur HTTP : un thread par connexion sans limite, délai par lecture et non par requête, boucle CPU si `accept` échoue | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-C) | Page publique : boutons vers des fichiers absents tant qu'aucune version n'est publiée | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-C) | Fichiers secrets créés lisibles puis chmod (fenêtre) ; comparaison du jeton non constante ; erreurs API sans réponse ni message ; message « redémarre » faux et clé remplacée sans confirmation ; totaux plafonnés sans le dire ; gain compté sur les échecs ; NaN si 0 cœur ; écritures non atomiques ; historique non protégé ; détection iPad/Linux ; promesses inexactes de la page | MINEUR | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-B) | `fs::copy` peut propager la quarantaine (xattr) vers ~/.local/bin/echoes → service bloqué par Gatekeeper (incertain) | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-B) | Mise à jour sans test de démarrage ni retour arrière : une version cassée immobilise toutes les machines | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-B) | `launchctl bootstrap gui/UID` échoue sans session graphique (installation SSH du Mac mini) ; agent arrêté hors session | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-B) | Accès au réseau local macOS 15+ pour un agent launchd non signé (incertain) | GRAVE | non vérifiable ici ; à tester sur Mac | ouvert |
| 1 (3b-B) | install.ps1 : `.Content` en octets → vérification SHA-256 toujours en échec | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-B) | DMG plus récent ignoré si déjà installé ; repli `.profile` sur macOS ; serveur de tableau de bord orphelin invisible ; aucun retour si le service tarde ; ordre stop/start dans install ; Gatekeeper « endommagé » non documenté ; journal sans rotation ; CI sans clippy/audit ; tranche Intel jamais exécutée ; e2e bloquant la publication | MINEUR | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-A) | Écoute du service : connexions illimitées sans clé, délai par lecture, boucle CPU sur erreur d'accept | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-A) | Retour de fichiers : écriture à travers un lien symbolique, `pulled` décidé par le pair, `.git` non protégé | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-A) | Un fichier illisible fait échouer tout l'inventaire (perte du résultat) | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-A) | Délais 30 s / 120 s dépassés pendant les phases silencieuses ; pas de délai d'écriture | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-A) | Commande laissant un démon : tubes ouverts, blocage sans fin | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-A) | Deux exécutions simultanées du même projet sans verrou | GRAVE | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 1 (3b-A) | Architecture Intel / Apple Silicon non comparée (binaire rapatrié inutilisable) | GRAVE, arbitrage | avertissement quand `--pull` croise les architectures ; choix à valider par Arthur | arbitrage |
| 1 (3b-A) | Clé relue seulement au démarrage ; SHELL absent sous launchd ; découverte figée sur les anciens pairs ; `.echoes-part` résiduels ; code de sortie sur signal ; inventaire avant garde-fou | MINEUR | corrigé au tour 2 (détail ci-dessous) | corrigé |
| 2 | Nouvelle demande d'Arthur : délégation tenant compte de la carte graphique et de la charge réelle | — | réalisé | fait |

## Tour 2 : corrections

- **Service** : 64 connexions au plus, 5 s pour prouver la clé, pause sur erreur d'accept, délai d'écriture, clé relue à chaque connexion.
- **Fichiers** : `sync::safe_dest` refuse `.git` et les liens symboliques des deux côtés ; `pulled` n'est admis que sous un chemin demandé par `--pull` ; inventaire tolérant aux fichiers illisibles ; fichiers partiels nettoyés.
- **Délais** : signe de vie toutes les 2 s pendant les inventaires et l'attente ; fin de tâche sur la fin du processus (démons) ; code 128+signal.
- **Concurrence** : une tâche à la fois par projet (file d'attente) ; manifeste, registre des machines et journal écrits de façon atomique.
- **Tableau de bord** : billet à usage unique (60 s) échangé contre une session en mémoire, preuve d'identité du serveur avant d'ouvrir le navigateur, 32 connexions au plus, 5 s par requête, CSP, erreurs renvoyées et affichées, arrêt du serveur de secours après 15 min.
- **Installation** : copie sans marque de quarantaine, repli `user/UID` sans session graphique, échec du service non bloquant, `.zshrc` par défaut sur Mac, DMG plus récent réinstallé.
- **Mise à jour** : essai de démarrage avant remplacement, copie de secours `echoes.prev`.
- **Divers** : install.ps1 (lecture de l'empreinte), page de téléchargement (iPad, Linux, promesses), CI (clippy, signature, démarrage des deux moitiés du programme), page republiée après chaque version.
- **Délégation selon la charge** : carte graphique, mémoire, tâches en cours ; admission et bridage sur machine occupée ; essai des machines dans l'ordre.

Contrôles 3a après corrections : build et clippy (`-D warnings`) sur 4 cibles, 7 tests (dont sécurité du tableau de bord), actionlint, shellcheck : verts. Mise à jour testée (réussite, refus d'un programme cassé). Délégation testée avec carte graphique simulée à 92 %.

Restent ouverts (MINEUR) : `bincode` non maintenu ; rotation du journal launchd ; shell `fish` (repli sur zsh/sh) ; noms de fichiers avec `:` refusés.
