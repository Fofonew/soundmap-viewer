# ADR 0012 — Interface : organisation, identité visuelle, traductions

**Statut :** accepté (retours d'usage du 18/11/2026 : « pas intuitif », « pas d'identité »,
« tout est en français en dur »).

## 1. Organisation
- **Panneau de gauche = le déroulé du travail**, en étapes numérotées : 01 Trouver un spot,
  02 Zone de travail, 03 Zones téléchargées. La progression du téléchargement s'affiche dans
  l'étape 02.
- **Calques sur la carte** (panneau flottant repliable) : fond satellite ou Plan IGN, noms de
  lieux, propriétés, gendarmeries, dalles LiDAR. Le détail d'un calque (légende des
  propriétaires, 3 gendarmeries les plus proches) apparaît sous sa ligne quand il est allumé.
- **Une seule manière de connaître un propriétaire :** calque « Propriétés » allumé, clic sur
  une parcelle → fiche flottante à droite. Le bouton « Qui possède ce point ? » est supprimé.
  Un clic sur un spot ouvre aussi la fiche.
- **Aide contextuelle :** icône ⓘ (survol, focus clavier ou clic), portail fixe borné à la
  fenêtre. Les explications longues quittent le panneau et vont dans ces infobulles.
- **Niveau de la sono :** « Comment connaître ce niveau ? » calcule le niveau à 1 m depuis une
  mesure (L + 20·lg d − 3) ou depuis la fiche technique (sensibilité + 10·lg P − 6 si
  demi-espace − 3 de compression + 20·lg N). Ce n'est pas la sensibilité. Repères corrigés :
  125 dB(C) = 2 subs 18", pas 4.

## 2. Identité
- **Sujet :** un instrument de terrain, entre carte topographique et appareil de mesure.
- **Signature :** l'échelle des dB de la carte de bruit (vert → jaune → orange → rouge →
  violet) sert de marque : bande en tête du panneau, logo en barres de vumètre, candidats des
  spots. L'orange 60 dB est la couleur d'action.
- **Typographie** (embarquée, l'app marche hors ligne, `@fontsource`) :
  - Barlow, dérivée de la signalétique routière, pour le texte ;
  - Barlow Condensed en capitales pour les titres et les boutons principaux ;
  - JetBrains Mono pour toutes les valeurs mesurées, en chiffres tabulaires.
- **Matière :** sections à plat séparées par des filets, sans cartes dans des cartes ;
  panneaux flottants seulement sur la carte.
- **Transitions :** propriétés explicites, jamais `transition: all`.

## 3. Traductions (`ui/src/i18n`, `ui/src/locales/*.json`)
- **Un fichier JSON par langue**, chargé par `import.meta.glob` : ajouter une langue = déposer
  `de.json`, elle apparaît dans le sélecteur (nom lu dans `_meta.name`).
- **Format compatible i18next** (outils de traduction courants) : clés imbriquées, `{{var}}`,
  pluriels `_zero` / `_one` / `_other` résolus par `Intl.PluralRules`.
- **Référence :** `fr.json`. Une clé absente d'une autre langue retombe sur le français.
- **Typage :** les clés sont typées depuis `fr.json` (`TKey`), une clé inexistante ne compile
  pas.
- **Nombres :** formatés par `Intl.NumberFormat` selon la langue (1,5 / 1.5 ; 7 558 / 7,558).
- **Test :** `i18n.test.ts` vérifie que chaque langue a toutes les clés du français, avec les
  mêmes variables.
- **Non traduit à ce stade :** les textes produits par le moteur Rust (messages de
  progression, erreurs, noms des modèles d'enceintes fournis, nom des scénarios GFPE,
  description de la méthode). Ils devront passer par des codes traduits côté interface.
