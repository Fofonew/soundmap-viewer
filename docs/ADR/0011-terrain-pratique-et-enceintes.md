# ADR 0011 — Terrain pratique, cadastre, enceintes personnelles

**Statut :** accepté (retours d'usage après la v0.1.0).

## 1. Assistant de spot : clairière et accès (`sm_compute::site`)
- **Problème :** le criblage (relief, population) proposait des spots en forêt ou sans
  chemin.
- **Méthode :** les candidats sont vérifiés dans l'ordre du classement, 6 en parallèle, 80 au
  plus. Sur chacun, on lit la BD TOPO `zone_de_vegetation` et `troncon_de_route` dans ±600 m.
- **Points testés :** grille de 25 m dans un rayon de 250 m autour du candidat.
- **Clairière :** 13 points dans un disque de 50 m de rayon, aucun en bois, forêt, lande
  ligneuse, peupleraie, verger, vigne ou haie.
- **Accès :** une voie à moins de 300 m. Seules comptent la route, la route empierrée et le
  chemin. Sont exclus : sentier, escalier, type autoroutier, et « physiquement impossible »
  pour un véhicule léger.
- **Coût d'un point :** distance d'accès + pénalité (route 0, piste 50 m, chemin 120 m,
  accès restreint 100 m) + 0,3 × le déplacement. Le meilleur point remplace le candidat.
- **Critères réglables** (`SiteSpec`, mémorisés dans l'interface) :
  - rayon de la clairière, distance d'accès maximale, déplacement maximal ;
  - voies admises : route revêtue, route empierrée, chemin ; accès restreint accepté ou non ;
  - forêt ouverte (clairsemée), vignes et vergers, landes ligneuses et haies : excluent le
    site ou non ;
  - bois et forêts fermées excluent toujours.
- **Échantillonnage de la clairière :** centre, puis cercles tous les 25 m au plus, points
  espacés de 25 m au plus (21 points pour 50 m, 66 pour 100 m).
- **Bornes :** clairière 10 à 300 m, accès 0 à 2 km, déplacement 0 à 1 km.
- **Mesure (Larzac, 30 km, clairière de 100 m, sans chemins) :** 80 candidats vérifiés (le
  plafond), 65 boisés, 6 sans accès, 8 spots en 36 s. Des critères stricts donnent moins de
  spots.
- **Mesure (Larzac, 30 km) :**
  - 28 candidats vérifiés : 17 écartés car boisés, 1 sans accès ;
  - durée 13 à 21 s, contre 4,5 s sans vérification. Option décochable.

## 2. Cadastre (`sm_data::cadastre`)
- **Parcelle :** Parcellaire Express IGN (WFS `CADASTRALPARCELS.PARCELLAIRE_EXPRESS:parcelle`).
  Donne la référence, la commune et la contenance.
- **Forêt publique :** BD TOPO `foret_publique` (domaniale, communale, sectionale ; ONF).
- **Propriétaires :** seules les **personnes morales** sont publiées (fichiers DGFiP, Licence
  Ouverte, millésime 2024) : communes, sections de commune, État, GFA, SCI… Les particuliers
  relèvent du secret fiscal : relevé de propriété au service de publicité foncière.
- **Accès aux fichiers :** les fichiers DGFiP sont deux zip nationaux de 250 Mo. On lit leur
  répertoire central par HTTP Range (zip64 géré), puis on télécharge et décompresse
  (`miniz_oxide`, Rust pur) le seul CSV du département.
  - Volume : environ 3 Mo compressé, 30 Mo en clair.
  - Mesure : 2,3 s pour l'Aveyron.
  - Cache sur disque (`cache/cadastre/pm_2024_<dep>.csv`) et index en mémoire.
- **Jointure :** clé commune (3) + préfixe (3) + section (2, complétée de 0) + numéro (4).
  - IGN : `0C`, `000` ; DGFiP : ` C`, vide.
  - Les subdivisions fiscales d'une parcelle sont fusionnées (une ligne par ayant droit).

## 2 bis. Carte des propriétés
- **Géométrie :** tuiles vectorielles PCI (couche `parcelle`, zoom ≥ 15) via le proxy Rust,
  donc en cache disque.
- **Propriétaires :** après chaque déplacement, l'interface envoie les parcelles visibles
  encore inconnues (`parcel_owners`) ; Rust les joint à l'index DGFiP du département. Le
  résultat est mis en cache par `idu` et posé en état MapLibre (`feature-state`).
- **Catégories :** commune, section de commune, État, collectivité ou établissement public,
  groupement agricole, groupement forestier, société ou association ; sinon particulier (non
  publié). Calculées depuis le groupe de personne et la forme juridique DGFiP.
- **Propriété :** le droit principal est le propriétaire (« P »), sinon le premier droit. Les
  parcelles d'un même SIREN (sinon d'une même dénomination) forment une propriété. Un clic
  l'entoure et donne son nombre de parcelles et sa surface à l'écran.
- **Mesure (Nant) :** 1 413 parcelles à l'écran, 165 de personnes morales ; commune de Nant :
  86 parcelles, 11,95 ha à l'écran.

## 3. Gendarmeries et commissariats
- BD TOPO `zone_d_activite_ou_d_interet` filtrée en CQL (`nature IN ('Gendarmerie', 'Poste ou
  hôtel de police') AND BBOX(...)`) dans 40 km. Distances à vol d'oiseau.

## 4. Enceintes personnelles (`sm_core::source`, bibliothèque `speakers.json`)
- **Nouveaux champs du modèle :** `dims` (L × H × P, dessin 3D), `response` (réponse mesurée
  Hz/dB, interpolée en log f, prolongée par la pente), `notes`, `custom`.
- **Directivité cardioïde** du premier ordre : p(θ) = A + B·cos θ, A − B = 10^(−R/20).
- **Saisie depuis la fiche technique** (`ui/src/sim/speakerMath.ts`) :
  - L_oct = L − 6·[crête] − 6·[demi-espace] − 10·lg(n_octaves) ;
  - coupures à −10 dB ramenées à −3 dB avec la pente : f·2^(±7/pente) ;
  - contrôle par sensibilité + 10·lg P.
- **Dessin :** boîtes `fill-extrusion` à l'échelle, posées sur le relief, de
  max(0, h − H/2) à + H.
- **Rotation dans la vue 3D :** une poignée accrochée au stack fait pivoter tout le système
  autour de son barycentre (Maj : pas de 15°).
  - La poignée est placée à 72 px de l'écran, distance convertie en mètres, car
    `map.unproject` est faux en vue inclinée avec relief.

## 5. Habitants hors Filosofi
- **Constat :** une habitation hors de tout carreau Filosofi peuplé avait 0 habitant
  (résidence secondaire sans ménage fiscal, carreau absent).
- **Correction :** 2,2 habitants par logement (taille moyenne des ménages INSEE), marqués
  « estimé » dans l'interface. La fête est souvent l'été, quand les résidences secondaires
  sont occupées.

## 6. Lecture de la carte
- **Survol :** niveau au point (`result_value_at`, raster en mémoire) et nature du bâtiment
  (usage, nature, logements, hauteur).
- **Légende :** couleur → dB sur la carte.
- **Toponymes :** surcouche `GEOGRAPHICALNAMES.NAMES` (PNG transparent, `PM_6_18`) sur le
  satellite.
