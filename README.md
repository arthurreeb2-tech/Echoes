# Echoes

Lance une commande ici, elle tourne sur la machine la plus disponible de ton réseau local, et le résultat revient comme si elle avait tourné ici.

```
echoes run -- cargo build --release
```

Echoes mesure la puissance libre de chaque machine appairée (vitesse d'un cœur × cœurs libres), envoie le projet à la meilleure, y lance la commande à priorité basse, affiche sa sortie en direct, puis rapatrie les fichiers modifiés et le code de sortie. Si cette machine-ci est la plus disponible, ou si aucune autre ne répond, la commande tourne simplement ici.

## Installation

**En double-clic**, depuis la page [Releases](https://github.com/arthurreeb2-tech/Echoes/releases/latest) :

- **Mac** (Intel et Apple Silicon) : `Echoes-macOS.pkg`. Ouvre-le et suis l'installateur. Le tableau de bord s'ouvre à la fin. Ensuite, l'app « Echoes » (Spotlight, Launchpad) le rouvre ; elle n'a pas d'icône dans le Dock.
- **Windows** : `Echoes-Setup-Windows.exe`. Double-clic, puis le tableau de bord s'ouvre ; un raccourci « Echoes » est ajouté au menu Démarrer.

Pour relier une machine aux autres, colle le code d'appairage dans le tableau de bord (rubrique « Appairage »), ou passe-le en ligne de commande.

Ces installateurs ne sont pas encore signés par Apple ni par Microsoft. Au premier lancement, macOS refuse d'ouvrir le `.pkg` : va dans Réglages Système > Confidentialité et sécurité > « Ouvrir quand même ». Windows affiche « Windows a protégé votre ordinateur » : « Informations complémentaires » > « Exécuter quand même ».

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

`echoes uninstall` retire le tout (la clé d'appairage est conservée ; sur Mac, supprime aussi `/Applications/Echoes.app` à la main).

## Tableau de bord

Echoes ne montre rien tant qu'on ne le demande pas. `echoes dashboard`, l'app « Echoes » sur Mac ou le raccourci du menu Démarrer ouvrent une page locale qui affiche :

- les tâches déléguées, le temps gagné (estimé), le calcul fait ailleurs et les données envoyées, sur 24 h, 7 jours, 30 jours ou depuis le début ;
- les machines du réseau, leur charge en direct, leur puissance et leur version (celles hors ligne restent affichées, grisées) ;
- la part du travail faite par chaque machine ;
- chaque tâche : commande, projet, trajet (de quelle machine vers laquelle, ou pourquoi elle est restée ici), durée, gain, résultat ;
- l'appairage : afficher le code du réseau, ou en coller un pour rejoindre un réseau existant.

Le journal de chaque machine est réuni sur la page, quelle que soit la machine qui a lancé la tâche. La page n'est accessible que depuis la machine elle-même (127.0.0.1) et exige un jeton secret contenu dans son adresse.

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

Le service vérifie toutes les 6 heures s'il existe une version plus récente sur les versions publiées de GitHub, la télécharge, vérifie son empreinte SHA-256 et se relance avec elle, uniquement quand aucune tâche ne tourne. `ECHOES_NO_UPDATE=1` désactive ce mécanisme.

Publier une version : augmenter `version` dans `Cargo.toml` sur `main`. Le workflow `release.yml` compile pour macOS (Apple Silicon et Intel), Linux et Windows et publie la version ; les machines la récupèrent d'elles-mêmes. Cela suppose que le dépôt soit **public**.

## Ce qui est synchronisé

- **Aller** : tout le dépôt git qui contient le dossier courant (ou le dossier courant hors dépôt), sauf `.git` et ce qu'excluent `.gitignore` et `.echoesignore`. Seuls les fichiers modifiés depuis la dernière fois sont envoyés. La commande tourne dans le même sous-dossier qu'ici.
- **Retour** : les fichiers suivis que la commande a modifiés ou créés, plus les chemins donnés à `--pull`. Un fichier modifié ici pendant l'exécution n'est jamais écrasé.
- Les caches de build ignorés par git (`target/`, `node_modules/`…) restent sur la machine distante : les builds suivants sont incrémentaux.
- Garde-fous : jamais le dossier personnel entier, ni plus de 50 000 fichiers ou 4 Go.

## Limites

- Délégation entre machines du **même système** seulement (Mac vers Mac, Windows vers Windows), sauf `--host` explicite.
- Un binaire compilé à distance a l'architecture de la machine distante : un build fait sur un Mac Apple Silicon ne s'exécute pas tel quel sur un Mac Intel. Sans importance pour les tests, le lint ou la vérification de types.
- La commande doit exister sur la machine distante (mêmes outils installés).
- Pas d'entrée clavier ni de terminal interactif pour la commande distante ; liens symboliques et `.git` non transmis.
- Découverte par Bonjour (mDNS) sur le même réseau local. Si le réseau la bloque : `ECHOES_HOSTS=192.168.1.20,192.168.1.21` ou `--host`.
- Vérifié automatiquement sous Linux et macOS (CI GitHub) ; l'installation comme service sous macOS et Windows reste à confirmer sur de vraies machines, de même que l'éventuelle demande d'accès au réseau local de macOS.

## Sécurité

Le code d'appairage est une clé secrète : toute machine qui la possède peut exécuter des commandes sur celles qui font tourner le service. Les échanges sont chiffrés (Noise, clé pré-partagée) ; les mises à jour sont vérifiées par empreinte SHA-256 et ne proviennent que des versions publiées de ce dépôt. N'installe le service que sur des machines qui t'appartiennent.
