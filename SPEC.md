# SPEC : revue avant la première compilation du DMG

**Objectif** : s'assurer qu'Echoes est prêt à être compilé en `Echoes-macOS.dmg` sur l'iMac et installé sur les Mac d'Arthur (MacBook Intel 2019, Mac mini M1, futur MacBook Pro M5).

**Critères de validation**
- Compilation sans avertissement pour macOS Intel, macOS Apple Silicon, Windows et Linux ; tests et clippy au vert.
- Aucune faille connue dans les dépendances (`cargo audit`).
- Scripts shell et workflows GitHub sans erreur d'analyse.
- Relecture en contexte neuf : aucune erreur GRAVE sur la sécurité (clé d'appairage, chemins, mises à jour), sur l'installation macOS (app, launchd, DMG) ou sur le tableau de bord.

**Hors périmètre** : signature et notarisation Apple (pas de compte développeur) ; tests sur machines réelles (après la compilation sur l'iMac).

**Livrable** : branche `claude/exciting-albattani-rv0f3m` corrigée, journal `contexte/loop.md`, verdict.
