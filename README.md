# Echoes

Lance une commande ici, elle tourne sur la machine la plus disponible de ton réseau local, et le résultat revient comme si elle avait tourné ici.

```
echoes run -- cargo build --release
```

Echoes mesure ce que chaque machine appairée peut donner maintenant, envoie le projet à la meilleure, y lance la commande à priorité basse, affiche sa sortie en direct, puis rapatrie les fichiers modifiés et le code de sortie. Si cette machine-ci est la plus disponible, ou si aucune autre ne répond, la commande tourne simplement ici.

## Une délégation qui ménage les machines occupées

La machine la plus puissante n'est pas toujours la plus disponible : si elle fait un montage vidéo ou un rendu 3D, lui envoyer un build la ferait ramer. Echoes en tient compte.

- **Ce qui est mesuré**, en continu sur chaque machine : cœurs libres, occupation de la carte graphique, mémoire disponible, tâches Echoes en cours.
- **Puissance disponible** = vitesse d'un cœur × cœurs libres × réserve. La réserve vaut 100 % jusqu'à 30 % de carte graphique, puis baisse jusqu'à 40 % quand elle est saturée ; elle baisse aussi quand il reste moins de 2 Go de mémoire.
- **Choix** : la tâche part vers la machine dont la puissance disponible est la plus haute, si elle dépasse d'au moins 30 % celle de la machine qui demande. Une machine puissante mais occupée passe donc derrière une machine modeste mais libre.
- **Machine occupée** (carte graphique à 50 % ou plus, ou moins de 1 Go de mémoire libre) : elle n'accepte qu'une tâche à la fois, la lance à la priorité la plus basse et sur la moitié de ses cœurs (pour `cargo`, `make` et `cmake`). Les demandes en trop sont refusées et partent vers la machine suivante, ou tournent sur place.
- **Dans l'autre sens** : une machine occupée qui lance elle-même une tâche voit sa propre puissance disponible baisser, donc elle délègue plus volontiers.

`echoes ls` et le tableau de bord affichent ces mesures et signalent les machines « occupées ».

Limites : Echoes délègue des commandes (calcul sur processeur), il ne déplace pas le travail d'une carte graphique. La carte graphique est mesurée sur Mac (`ioreg`), et sur Windows et Linux avec une carte NVIDIA (`nvidia-smi`) ou AMD sous Linux ; ailleurs seuls le processeur et la mémoire comptent. La mesure sur Mac reste à confirmer sur de vraies machines.

## Installation

**Depuis la page de téléchargement** (`docs/index.html`, publiée sur GitHub Pages : https://arthurreeb2-tech.github.io/Echoes/), qui propose directement le bon fichier :

- **Mac** (Intel et Apple Silicon) : `Echoes-macOS.dmg`. Glisse Echoes dans Applications et lance-le : au premier lancement, il s'installe en arrière-plan et ouvre le tableau de bord. Ensuite, l'app « Echoes » (Spotlight, Launchpad) rouvre le tableau de bord ; elle n'a pas d'icône dans le Dock.
- **Windows** : `Echoes-Setup-Windows.exe`. Double-clic, puis le tableau de bord s'ouvre ; un raccourci « Echoes » est ajouté au menu Démarrer.

Pour relier une machine aux autres, colle le code d'appairage dans le tableau de bord (rubrique « Appairage »), ou passe-le en ligne de commande.

Ces installateurs ne sont pas encore signés par Apple ni par Microsoft. Au premier lancement, macOS refuse d'ouvrir Echoes : va dans Réglages Système > Confidentialité et sécurité > « Ouvrir quand même ». Windows affiche « Windows a protégé votre ordinateur » : « Informations complémentaires » > « Exécuter quand même ».

**Avec Claude Code** : ouvre ce dépôt et dis « installe » (voir `CLAUDE.md`).

**En ligne de commande** :

| Système | Première machine | Machines suivantes |
|---|---|---|
| macOS, Linux | `curl -fsSL https://raw.githubusercontent.com/arthurreeb2-tech/Echoes/main/install.sh \| sh` | `… \| sh -s -- <code>` |
| Windows | `irm https://raw.githubusercontent.com/arthurreeb2-tech/Echoes/main/install.ps1 \| iex` | `$env:ECHOES_CODE="<code>"` avant |
| Depuis le dépôt cloné | `./install.sh` | `./install.sh <code>` |

L'installation :

- place le programme dans `~/.local/bin` (Windows : `%LOCALAPPDATA%\Programs\Echoes`) et l'ajoute au PATH ;
- lance le service `echoes serve` à chaque ouverture de session et le relance s'il s'arrête (launchd sur macOS, systemd sur Linux, tâche planifiée sur Windows) ;
- ajoute à `~/.claude/CLAUDE.md` une consigne pour que Claude Code passe ses builds et ses tests par `echoes run --` ;
- affiche le code d'appairage sur la première machine (`echoes pair` le réaffiche).

À savoir sur Mac :

- macOS signale un nouvel « élément d'ouverture » à l'installation : c'est le service d'Echoes. Le désactiver dans Réglages Système > Général > Ouverture arrête Echoes.
- Le service ne tourne que lorsqu'une session est ouverte. Sur un Mac sans écran (Mac mini), active l'ouverture de session automatique pour qu'il reprenne après un redémarrage.
- Si macOS demande d'autoriser Echoes à accéder au réseau local ou à recevoir des connexions, accepte : sans cela les machines ne se voient pas.
- Si macOS dit l'app « endommagée » : `xattr -dr com.apple.quarantine /Applications/Echoes.app`.

`echoes uninstall` retire le tout (la clé d'appairage est conservée ; sur Mac, mets aussi `Echoes.app` à la corbeille).

## Tableau de bord

Echoes ne montre rien tant qu'on ne le demande pas. `echoes dashboard`, l'app « Echoes » sur Mac ou le raccourci du menu Démarrer ouvrent une page locale qui affiche :

- les tâches déléguées, le temps gagné (estimé), le calcul fait ailleurs et les données envoyées, sur 24 h, 7 jours, 30 jours ou depuis le début ;
- les machines du réseau, leur charge en direct, leur puissance et leur version (celles hors ligne restent affichées, grisées) ;
- la part du travail faite par chaque machine ;
- chaque tâche : commande, projet, trajet (de quelle machine vers laquelle, ou pourquoi elle est restée ici), durée, gain, résultat ;
- l'appairage : afficher le code du réseau, ou en coller un pour rejoindre un réseau existant.

Le journal de chaque machine est réuni sur la page, quelle que soit la machine qui a lancé la tâche. La page n'est accessible que depuis la machine elle-même (127.0.0.1). Chaque ouverture passe par un billet à usage unique, valable une minute, que seul ton compte peut fabriquer ; la session qui en résulte dure 12 heures et ne survit pas à la fermeture de l'onglet.

Le gain est une estimation : durée de la commande sur la machine distante × rapport de puissance disponible entre les deux machines, moins la durée des transferts.

Pour donner un nom lisible à une machine, écris-le dans le fichier `name` du dossier de configuration (`~/Library/Application Support/echoes/name` sur Mac, `~/.config/echoes/name` sous Linux, `%APPDATA%\echoes\name` sous Windows).

## Utilisation

```
echoes ls                                         # machines visibles, version et score
echoes run -- cargo test                          # choix automatique
echoes run --remote -- npm run build              # jamais ici
echoes run --host mac-mini.local -- make          # machine précise
echoes run --pull target/release -- cargo build --release   # rapatrie aussi un produit de build
echoes dashboard                                  # tableau de bord
echoes update                                     # mise à jour immédiate
```

## Mises à jour automatiques

Le service vérifie toutes les 6 heures s'il existe une version plus récente sur les versions publiées de GitHub, la télécharge, vérifie son empreinte SHA-256, s'assure qu'elle démarre sur cette machine, puis se relance avec elle, uniquement quand aucune tâche ne tourne. La version précédente est gardée à côté du programme (`echoes.prev`). `ECHOES_NO_UPDATE=1` désactive ce mécanisme.

Publier une version : augmenter `version` dans `Cargo.toml` sur `main`. Le workflow `release.yml` compile pour macOS (Apple Silicon et Intel), Linux et Windows et publie la version ; les machines la récupèrent d'elles-mêmes. Cela suppose que le dépôt soit **public**.

## Ce qui est synchronisé

- **Aller** : tout le dépôt git qui contient le dossier courant (ou le dossier courant hors dépôt), sauf `.git` et ce qu'excluent `.gitignore` et `.echoesignore`. Seuls les fichiers modifiés depuis la dernière fois sont envoyés. La commande tourne dans le même sous-dossier qu'ici.
- **Retour** : les fichiers suivis que la commande a modifiés ou créés, plus les chemins donnés à `--pull`. Un fichier modifié ici pendant l'exécution n'est jamais écrasé.
- Les caches de build ignorés par git (`target/`, `node_modules/`…) restent sur la machine distante : les builds suivants sont incrémentaux.
- Garde-fous : jamais le dossier personnel entier, ni plus de 50 000 fichiers ou 4 Go.

## Limites

- Délégation entre machines du **même système** seulement (Mac vers Mac, Windows vers Windows), sauf `--host` explicite. Entre un Mac Intel et un Mac Apple Silicon, la délégation est permise ; Echoes avertit quand `--pull` rapatrie des fichiers compilés pour l'autre architecture.
- Une seule tâche à la fois par projet sur une même machine : la suivante attend son tour.
- Les fichiers rapatriés ne sont jamais écrits dans `.git` ni à travers un lien symbolique.
- Un binaire compilé à distance a l'architecture de la machine distante : un build fait sur un Mac Apple Silicon ne s'exécute pas tel quel sur un Mac Intel. Sans importance pour les tests, le lint ou la vérification de types.
- La commande doit exister sur la machine distante (mêmes outils installés).
- Pas d'entrée clavier ni de terminal interactif pour la commande distante ; liens symboliques et `.git` non transmis.
- Découverte par Bonjour (mDNS) sur le même réseau local. Si le réseau la bloque : `ECHOES_HOSTS=192.168.1.20,192.168.1.21` ou `--host`.
- Vérifié automatiquement sous Linux et macOS (CI GitHub) ; l'installation comme service sous macOS et Windows reste à confirmer sur de vraies machines, de même que l'éventuelle demande d'accès au réseau local de macOS.

## Sécurité

Le code d'appairage est une clé secrète : toute machine qui la possède peut exécuter des commandes sur celles qui font tourner le service. Les échanges sont chiffrés (Noise, clé pré-partagée) ; les mises à jour sont vérifiées par empreinte SHA-256 et ne proviennent que des versions publiées de ce dépôt. N'installe le service que sur des machines qui t'appartiennent.
