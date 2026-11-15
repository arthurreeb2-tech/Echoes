# Design : Echoes dans la barre des menus

## Intention

Aujourd'hui, une fois l'onglet du tableau de bord fermé, rien à l'écran ne rappelle Echoes : `Echoes.app` n'a ni icône ni présence dans le Dock, et il faut penser à la chercher dans Spotlight. Arthur veut un point d'entrée **toujours visible** qui donne l'état du réseau d'un coup d'œil et rouvre le tableau de bord en un clic.

## Objectifs

- Une icône Echoes permanente dans la barre des menus (macOS) et dans la zone de notification (Windows).
- Sa couleur résume l'état, sans cliquer.
- Un clic ouvre un menu : machines avec leur état, résumé du jour, « Ouvrir le tableau de bord », « Quitter ».
- Elle démarre seule à l'ouverture de session, comme le service.
- `Echoes.app` reçoit enfin une vraie icône (Finder, Launchpad, Spotlight) — décision de Claude, coût marginal puisque le motif de l'icône est dessiné de toute façon.

## Non-objectifs

- Refaire le tableau de bord en natif : il reste la page web locale.
- Lancer des tâches, appairer ou régler quoi que ce soit depuis le menu.
- Linux (pas de barre des menus standard ; on n'y ajoute pas GTK).
- Notifications.

## Anti-objectifs (échec même si ça marche)

- Ralentir la machine : l'icône ne doit rien coûter de visible (CPU au repos ≈ 0, pas de trafic réseau en rafale).
- Affaiblir la sécurité : aucun nouveau port, aucun accès au tableau de bord sans billet.
- Casser la CI Linux ou les mises à jour automatiques.
- Une icône qui ment : un état périmé doit se voir (gris), pas rester vert.

## Approches considérées

- **A. Icône dans la barre des menus — retenue par Arthur.** État visible en permanence, le tableau de bord reste la page web. Deux petites bibliothèques, un processus léger de plus.
- B. Seulement une icône d'app dans le Dock : plus petit, mais aucun état visible sans cliquer. Son icône d'app est reprise dans A.
- C. Fenêtre native complète (egui) : beaucoup de travail pour dupliquer la page, sans mieux régler l'accès.

## Comportement

**Icône** : un glyphe Echoes monochrome (ondes concentriques), au format « template » sur macOS pour suivre le mode clair/sombre, avec une pastille de couleur :
- vert : au moins une autre machine en ligne et non occupée ;
- orange : en ligne mais toutes les autres sont occupées, ou aucune autre machine en ligne ;
- gris : le service local `echoes serve` ne répond pas, ou pas de clé d'appairage.
Info-bulle : « Echoes — 2 machines, Mac mini libre ».

**Menu** (reconstruit à chaque rafraîchissement) :
```
● arthurs-mac-mini-1 — libre · 7,8 cœurs
● imac-de-arthur-1 (cette machine) — occupée
  3 tâches aujourd'hui · 4 min gagnées
──────────────
Ouvrir le tableau de bord
Quitter
```
Les lignes de machines sont inactives (information). Une machine déjà vue mais hors ligne apparaît grisée, « hors ligne ».

**Rafraîchissement** : toutes les 15 s dans un fil séparé, sans bloquer l'interface. Les données viennent du code existant, sans passer par HTTP : `status::local()`, `client::find(key, id, false)` (sonde les pairs connus, ne relance Bonjour que si nécessaire), `history::read` pour le résumé du jour (journal local seulement : pas d'appel `fetch_history` vers les pairs, trop cher toutes les 15 s). Le registre `machines.json` est lu pour afficher les machines hors ligne, jamais réécrit par l'icône.

**Ouvrir le tableau de bord** : même chemin que `echoes dashboard` (`dashboard::open(false)`, billet à usage unique), lancé dans un fil.

**Quitter** : ferme l'icône jusqu'à la prochaine ouverture de session ; le service continue de tourner.

## Démarrage et installation

- Nouvelle sous-commande cachée `echoes tray`, dans le même binaire.
- macOS : second agent launchd `io.echoes.tray` (`RunAtLoad`, `KeepAlive` = relancer seulement en cas de plantage, pas après « Quitter »), lançant `~/.local/bin/echoes tray`. Le processus se déclare « accessoire » (pas d'icône dans le Dock).
- Windows : seconde tâche planifiée « Echoes Tray » à l'ouverture de session.
- `echoes install` les crée, `echoes uninstall` les retire, la mise à jour automatique relance aussi l'icône (sinon elle garde l'ancienne version).
- Lancer `Echoes.app` alors que l'icône tourne ouvre simplement le tableau de bord, comme aujourd'hui. Si l'icône avait été fermée par « Quitter », l'app la relance aussi (c'est le geste naturel pour la retrouver). *Décision de Claude.*
- **Machines déjà installées** (choix d'Arthur) : lancer `Echoes.app` (ou `echoes` sans argument) sur une machine installée crée l'agent ou la tâche de l'icône s'il manque, puis la démarre. Rien n'est créé par la mise à jour automatique elle-même.
- **Pas de session graphique** (installation par SSH, Mac mini sans écran) : l'agent `io.echoes.tray` n'est enregistré que dans le domaine `gui/<uid>` ; s'il est lancé sans affichage, `echoes tray` sort aussitôt avec le code 0, et `KeepAlive` ne relance qu'en cas de sortie anormale, avec `ThrottleInterval` de 60 s : pas de boucle de relance. *Décision de Claude.*
- **Respect de « Quitter »** : une mise à jour ne relance l'icône que si elle tournait au moment de la mise à jour (`launchctl print` / requête de la tâche planifiée). *Décision de Claude.*
- **Ouvrir le tableau de bord depuis l'icône** : lance le processus `echoes dashboard` séparé, jamais `dashboard::open` dans le processus de l'icône (cette fonction peut servir la page elle-même puis terminer le processus).
- **Windows** : les arrêts du service ciblent la tâche « Echoes » seule ; tout `taskkill` par nom d'image doit épargner l'icône (filtrer par ligne de commande ou PID).

## Contraintes techniques

- Bibliothèques : `tray-icon` + `muda` (menus), boucle d'événements fournie par `tao` ou équivalent minimal ; dépendances déclarées **uniquement** pour macOS et Windows (`[target.'cfg(...)'.dependencies]`), et le module `tray` compilé sous les mêmes conditions. Linux : `echoes tray` affiche « non disponible sous Linux ».
- Icônes intégrées au binaire (`include_bytes!`) : glyphe barre des menus en PNG 2x, quatre variantes ou pastille composée à l'exécution.
- Icône d'app : `packaging/macos/Echoes.icns` copié par `build-dmg.sh`, déclaré par `CFBundleIconFile` dans `Info.plist`. Windows : icône intégrée à l'exe si simple (`winres`/`embed-resource`), sinon reporté.
- Critères du SPEC maintenus : compilation sans avertissement sur les quatre cibles, tests et clippy au vert, `cargo audit` propre.
- Pas de changement de `proto::VERSION` (aucun message réseau nouveau).

## Laissé au constructeur

- Choix précis de la boucle d'événements (tao, winit, ou boucle native) tant que la taille du binaire reste raisonnable (viser < +1,5 Mo).
- Dessin du glyphe (simple, lisible en 18 pt), génération du `.icns` (`iconutil`).
- Formulation exacte des libellés du menu, en français.
