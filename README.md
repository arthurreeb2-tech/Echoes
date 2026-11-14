# Echoes

Lance une commande ici, elle tourne sur la machine la plus disponible de ton réseau local, et le résultat revient comme si elle avait tourné ici.

```
echoes run -- cargo build --release
```

Echoes mesure la puissance libre de chaque machine appairée (vitesse d'un cœur × cœurs libres), envoie le projet à la meilleure, y lance la commande à priorité basse, affiche sa sortie en direct, puis rapatrie les fichiers modifiés et le code de sortie. Si cette machine-ci est la plus disponible, ou si aucune autre ne répond, la commande tourne simplement ici.

## Installation

Avec Claude Code, ouvre ce dépôt et dis « installe » (voir `CLAUDE.md`). Sinon :

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

`echoes uninstall` retire le tout (la clé d'appairage est conservée).

## Utilisation

```
echoes ls                                         # machines visibles, version et score
echoes run -- cargo test                          # choix automatique
echoes run --remote -- npm run build              # jamais ici
echoes run --host mac-mini.local -- make          # machine précise
echoes run --pull target/release -- cargo build --release   # rapatrie aussi un produit de build
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
