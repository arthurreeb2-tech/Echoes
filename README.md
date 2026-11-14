# Echoes

Lance une commande ici, elle tourne sur la machine la plus disponible de ton réseau local, et le résultat revient comme si elle avait tourné ici.

```
echoes run -- cargo build --release
```

Echoes mesure la puissance libre de chaque machine appairée (vitesse d'un cœur × cœurs libres), envoie le dossier courant à la meilleure, y lance la commande, affiche sa sortie en direct, puis rapatrie les fichiers modifiés et le code de sortie. Si cette machine-ci est la plus disponible, ou si aucune autre ne répond, la commande tourne simplement ici.

## Installation

Sur chaque machine (Rust requis) :

```
cargo install --path .
```

## Mise en route

Sur une première machine :

```
echoes pair            # affiche le code d'appairage
```

Sur chaque autre machine :

```
echoes pair <code>
```

Sur chaque machine qui prête sa puissance (par exemple le Mac mini) :

```
echoes serve
```

Puis, depuis n'importe quelle machine appairée :

```
echoes ls                                   # machines visibles et leur score
echoes run -- cargo test                    # choix automatique
echoes run --remote -- npm run build        # jamais ici
echoes run --host mac-mini.local -- make    # machine précise
echoes run --pull target/release -- cargo build --release   # rapatrie aussi les produits de build
```

## Ce qui est synchronisé

- **Aller** : tous les fichiers du dossier courant, sauf `.git` et ce qu'excluent `.gitignore` et `.echoesignore`. Seuls les fichiers qui ont changé depuis la dernière fois sont envoyés.
- **Retour** : les fichiers suivis que la commande a modifiés ou créés (formatage, fichiers de verrouillage…), plus les chemins donnés à `--pull`. Un fichier modifié ici pendant l'exécution n'est jamais écrasé.
- Les caches de build ignorés par git (`target/`, `node_modules/`…) restent sur la machine distante d'une exécution à l'autre : les builds suivants sont incrémentaux.

## Limites actuelles

- Les machines sont trouvées par Bonjour (mDNS) sur le même réseau local. Si le réseau le bloque : `ECHOES_HOSTS=192.168.1.20,192.168.1.21` ou `--host`.
- Un binaire compilé à distance a l'architecture de la machine distante : un build fait sur un Mac Apple Silicon ne s'exécute pas tel quel sur un Mac Intel (il faut alors cibler `x86_64-apple-darwin`). Sans importance pour les tests, le lint ou la vérification de types.
- La commande doit exister sur la machine distante (mêmes outils installés).
- Pas d'entrée clavier ni de terminal interactif pour la commande distante.
- Les liens symboliques et le dossier `.git` ne sont pas transmis.
- `echoes serve` ne se lance pas encore tout seul au démarrage.
- Testé sous Linux uniquement pour l'instant ; macOS et Windows sont prévus par le code mais pas encore vérifiés sur de vraies machines.

## Sécurité

Le code d'appairage est une clé secrète : toute machine qui la possède peut exécuter des commandes sur celles qui font tourner `echoes serve`. Les échanges sont chiffrés (Noise, clé pré-partagée). N'installe `echoes serve` que sur des machines qui t'appartiennent.
