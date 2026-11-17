# SoundMap — Rapport de recherche

> Simulateur 3D de propagation du son en extérieur (sound systems, free parties, festivals) sur les données LiDAR HD de l'IGN.
> Recherche du 17/11/2026. **Usage : recherche et personnel**, la réglementation est volontairement hors périmètre.
> Tous les endpoints ont été **vérifiés en direct** : voir `docs/ENDPOINTS.md` (couches exactes, requêtes testées, formats).

---

## 1. Ce qu'on cherche vraiment à simuler (et pourquoi c'est difficile)

Une sono de free party, c'est surtout des **basses fréquences (30–120 Hz)** à très fort niveau. C'est ce qui porte à plusieurs km et ce qui génère les plaintes. Les conséquences sur la physique :

| Phénomène | Impact en BF (subs) | Impact en HF (têtes) |
|---|---|---|
| Absorption atmosphérique (ISO 9613-1) | quasi nulle (< 0,1 dB/km à 63 Hz) | forte (> 10 dB/km à 4 kHz) |
| Effet de sol (impédance) | **fort**, interférences source/image sur des centaines de m | modéré |
| Réfraction (gradient de température, vent) | **dominant la nuit** : une inversion par ciel clair et vent faible ramène le son au sol et peut ajouter **+10 à +20 dB** à 2–5 km | fort aussi, zones d'ombre au vent contraire |
| Diffraction par le relief (crêtes, talus) | faible, les grandes longueurs d'onde (3–10 m) contournent les obstacles | **fort**, une crête protège bien |
| Végétation / forêt | faible | modéré à fort |
| Bâtiments (écrans) | faible | fort |

**Conclusion clé :** une méthode d'ingénierie classique (ISO 9613-2, CNOSSOS-EU) suffit pour une prévisualisation rapide et pour les médiums/aigus. En revanche, elle **sous-estime gravement** la portée des basses par nuit d'inversion, qui est justement la configuration typique d'une free party (nuit d'été claire, vent calme, souvent en vallée ou sur un causse). Il faut donc un **solveur onde de type équation parabolique (PE)** pour les basses. C'est d'ailleurs l'approche publiée par L-Acoustics en 2026 pour la pollution sonore BF des systèmes de sonorisation (PE, 20–160 Hz, atmosphère inhomogène et nature du sol).

---

## 2. Choix technologiques

### 2.1 Recommandation : application desktop **Tauri 2** avec un cœur de calcul **Rust** et une carte **MapLibre GL JS + deck.gl**

```
┌──────────────────────── Tauri 2 (fenêtre native, Windows/Linux/macOS) ─────────────────────────┐
│  Frontend (WebView)  TypeScript + Vite + React (ou Svelte)                                    │
│   ├─ MapLibre GL JS : fond PLAN IGN / ortho (« vue GPS »), terrain 3D raster-dem, bâtiments 3D │
│   ├─ deck.gl : heatmap SPL, isophones, enceintes 3D, rayons, profils                          │
│   └─ UI : placement des enceintes, presets sono, scénarios météo, rapport de nuisances        │
│              ▲ IPC binaire / protocole custom  soundmap://tiles/...                           │
├──────────────┴────────────────────────────────────────────────────────────────────────────────┤
│  Backend Rust (workspace cargo)                                                              │
│   ├─ sm-core      : physique pure, sans I/O (sources, sol, atmosphère, PE, méthode ingé.)    │
│   ├─ sm-geo       : projections L93↔WGS84, rasters, profils de terrain, R-tree bâtiments     │
│   ├─ sm-data      : clients Géoplateforme/Météo-France/BRGM, cache disque, tuiles            │
│   ├─ sm-compute   : orchestration rayon (CPU), puis wgpu (GPU) en option                     │
│   └─ sm-app       : commandes Tauri, serveur de tuiles résultat                              │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```

**Pourquoi pas tout en natif (Bevy/wgpu) ?** Il faudrait réécrire un moteur cartographique complet : tuiles WMTS, projections, labels, vector tiles, LOD terrain. Les crates Rust de rendu de terrain à grande échelle (Terra, plugins Bevy) restent des projets expérimentaux. La valeur du logiciel est dans la **simulation**, pas dans le moteur de carte. MapLibre gère déjà nativement le terrain 3D (`raster-dem`), les tuiles IGN et l'extrusion des bâtiments.

**Pourquoi pas une app 100 % web (WASM) ?** Il y aurait trois limites : le multi-thread WASM exige les en-têtes COOP/COEP, la mémoire est plafonnée à 4 Go, et le LiDAR est lourd (un nuage de points COPC fait 0,5 à 2 Go par km²). Cela dit, si `sm-core` reste **sans I/O**, il pourra être compilé plus tard en WASM pour une version web allégée.

### 2.2 Crates Rust proposées

| Besoin | Crate(s) | Remarque |
|---|---|---|
| Parallélisme CPU | `rayon` | parallélisation par azimut × fréquence |
| FFT (solveur PE) | `rustfft`, `realfft`, `num-complex` | SIMD AVX/NEON auto |
| Algèbre / grilles | `ndarray` ou `glam`, `nalgebra` | |
| GPU compute (phase 2) | `wgpu` + WGSL | FFT Stockham à écrire en WGSL (pas de cuFFT) |
| Géométrie | `geo`, `geo-types`, `rstar` (R-tree) | intersections rayons/bâtiments |
| Projections | `proj4rs` (pur Rust) | EPSG:2154 Lambert-93 ↔ EPSG:4326. Éviter `proj` (dépendance C pénible sous Windows) |
| GeoTIFF | `tiff` (+ lecture des tags GeoKey) | éviter GDAL (dépendance C lourde) sauf besoin réel |
| LiDAR COPC/LAZ | `las`, `laz`, `copc-rs` ou `copc-core`/`copc-reader` | lecture par octree avec HTTP Range. Comparer les deux familles de crates sur crates.io |
| HTTP | `reqwest` + `tokio` | respecter les quotas Géoplateforme |
| Cache | `rusqlite` (MBTiles) ou PMTiles | |
| Vecteur | `geojson`, `flatgeobuf` | WFS en GeoJSON |
| Isophones | `contour` (marching squares) | → GeoJSON pour le front |
| Tests / bench | `approx`, `proptest`, `criterion`, `insta` | |
| Profilage | `tracing`, `puffin` ou `tracy` | |

### 2.3 Cibles de performance

- **Aperçu interactif (< 300 ms)** avec la méthode d'ingénierie sur une grille polaire (360 azimuts × pas de 10 m × rayon de 3 km), en bandes d'octave de 31,5 Hz à 8 kHz. Recalcul pendant qu'on déplace une enceinte.
- **Calcul fin des basses (< 30–60 s sur 8 cœurs)** avec le solveur GFPE en N×2D : 360 azimuts × ~20 fréquences en 1/3 d'octave (25–200 Hz) × portée de 5 km. Ordre de grandeur : ~10⁷ FFT de 2048 points, soit ~10¹¹–10¹² flops. C'est faisable sur CPU et accélérable ×10 sur GPU.
- **Mémoire :** ne jamais charger le MNT 0,5 m sur 10 km de rayon (~5 Go). Utiliser une **résolution multi-échelle** : 0,5–1 m près des sources et des récepteurs, 5 m au-delà. Le PE n'a pas besoin de mieux que λ/4 horizontalement pour le terrain.

### 2.4 Chargement des données en deux temps (exploration → zone de travail)

**Principe :** aucune donnée LiDAR n'est chargée tant que la zone précise n'est pas choisie. Une fois la zone fixée, **tout est téléchargé en local** et la simulation tourne ensuite hors ligne, sans aucune latence réseau.

1. **Mode Exploration** (léger, en streaming) : vue satellite (`ORTHOIMAGERY.ORTHOPHOTOS`) et Plan IGN en 2D, recherche d'adresse, couche d'emprise des dalles LiDAR disponibles (TMS pbf). En option, un relief 3D grossier (RGE ALTI via WMTS, z ≤ 14) pour se repérer. **Aucun appel LiDAR HD, aucun calcul.** Seules les tuiles affichées sont mises en cache.
2. **Choix de la zone de travail** : clic sur l'emplacement de la sono, puis rayon d'étude (par exemple 1 à 10 km) ou rectangle. L'outil affiche une **estimation du volume à télécharger** et du temps nécessaire avant de valider.
3. **Téléchargement du « pack zone »** (en tâche de fond, avec barre de progression, reprenable après coupure) :
   - **MNT, MNS et MNH LiDAR** en float32 via WMS-R, **en anneaux de résolution** : 0,5 m sur ~500 m autour de la sono, 1 m jusqu'à 2 km, 5 m au-delà. Exemple pour 5 km de rayon : ~70 Mo par couche au lieu de ~1,3 Go à 0,5 m partout (calcul : 0,8 km² × 16 Mo + 11,8 km² × 4 Mo + 66 km² × 0,16 Mo). Le MNH peut être recalculé localement (MNS − MNT) pour économiser une couche. Requêtes par blocs de 1–2 km², parallélisées sous le quota de 40 req/s.
   - Vecteurs WFS de la zone : bâtiments, végétation, eau, routes, Filosofi, ZNIEFF.
   - Occupation du sol (OCS GE / CoSIA) rasterisée sur la grille.
   - Météo de la date choisie (Open-Meteo), rafraîchissable.
   - Tuiles ortho à haut zoom de la zone, pour une **vue satellite fluide hors ligne**.
   - Nuages COPC : **seulement à la demande** (étage d'affinage végétation), et seulement pour les dalles proches de la sono, en lisant uniquement les niveaux de l'octree utiles.
4. **Stockage local** : un dossier par zone (`zones/{id}/`) contenant un manifeste JSON, les rasters en binaire float32 tuilé et **memory-mappé** (`memmap2`) pour un accès instantané depuis le solveur, les vecteurs en FlatGeobuf, les tuiles en MBTiles/PMTiles, et un hash de version des sources. Recharger une zone déjà téléchargée est immédiat.
5. **Mode Simulation** : le rendu 3D haute résolution (terrain Terrarium généré depuis le pack local, bâtiments extrudés) et tous les calculs lisent **exclusivement le pack local**.

---

## 3. Modèle physique proposé (architecture hybride)

### 3.1 Sources (la sono)
- **Bibliothèque d'enceintes** : SPL max à 1 m, sensibilité, puissance, réponse en fréquence et directivité. Le format GLL (AFMG) est propriétaire. Le **CLF** (Common Loudspeaker Format) est lisible. Sinon, prévoir des presets génériques (sub 18" et 2×18", top 15"+2", line array) et une saisie manuelle.
- **Réseaux de subs = somme cohérente.** Chaque caisson est une source ponctuelle complexe (amplitude, phase, délai, polarité). La directivité d'un montage **cardioïde ou end-fire** émerge naturellement de la somme. C'est un levier majeur pour réduire les nuisances (−15 à −20 dB à l'arrière).
- Les têtes et les médiums sont sommés de façon incohérente (énergie).
- **Spectre de programme** : gabarits « tekno/hardtek », « techno », « DnB », « rock » en 1/3 d'octave. On peut aussi importer un spectre mesuré (fichier audio → analyse 1/3 d'octave en Rust).
- Hauteur des sources : posées au sol (couplage au sol, +6 dB en BF) ou en hauteur.

### 3.2 Propagation, deux étages
1. **Méthode d'ingénierie** (type ISO 9613-2:2024 / CNOSSOS-EU), toutes bandes :
   `Lp = Lw + D − A_div − A_atm − A_ground − A_diff (− A_foliage − A_bâti)`
   - A_atm : ISO 9613-1 selon température et humidité.
   - A_ground : facteur G par segment de profil, issu de l'occupation du sol.
   - A_diff : diffraction multi-arêtes (Deygout ou chemin « élastique » CNOSSOS) sur le profil MNT.
   - Conditions **homogènes** vs **favorables** (méthode CNOSSOS), pour encadrer le résultat.
2. **Équation parabolique GFPE (Green's Function PE)** en N×2D pour 20–200 Hz :
   - Profil de célérité effective par azimut : `c_eff(z) = c(T(z)) + u(z)·cos(θ_vent − θ_azimut)`.
   - Impédance de sol par segment : Delany-Bazley ou Miki, avec une résistivité au passage de l'air σ par classe (classes Nordtest / Nord2000).
   - **Relief** : GTPE (Generalized Terrain PE) ou transformation conforme pour les pentes modérées. Pour les falaises et les fortes pentes, se rabattre sur la diffraction.
   - Couche absorbante en haut du domaine. Pas vertical λ/10, pas horizontal de quelques λ (atout du GFPE).
   - Starter : champ de départ pondéré par la **directivité complexe du réseau** dans l'azimut, ce qui évite un run PE par caisson.
   - Turbulence en option (perte de cohérence, remplissage des zones d'ombre).
3. **Fusion** : PE sous ~200 Hz, méthode d'ingénierie au-dessus, avec une transition progressive. Les niveaux A et C globaux sont recombinés à partir des bandes.

### 3.3 Atmosphère (le paramètre le plus influent la nuit)
- Profils verticaux log-linéaires de vent et de température (similitude de **Monin-Obukhov**), paramétrés par u*, T* et la longueur d'Obukhov L.
- Si les flux ne sont pas disponibles, déduire une **classe de stabilité de Pasquill** de la nébulosité, du vent à 10 m et de l'heure (méthode de Turner). Nuit claire et vent < 2 m/s : stable (classe F), inversion forte.
- **Scénarios** : « défavorable » (inversion + vent portant vers les habitations), « neutre », « favorable », et « prévision AROME du soir J ». Afficher l'**enveloppe** (min/médiane/max) plutôt qu'une seule carte.
- Effets de relief (idées avancées) : brise de pente catabatique la nuit, lac d'air froid en fond de vallée (inversion piégée → canalisation du son le long de la vallée).

### 3.4 Récepteurs et nuisances
- Récepteurs en façade des bâtiments d'habitation (BD TOPO, `usage_1 = Résidentiel`), à 1,5 m et 4 m de hauteur.
- **Bruit résiduel** estimé : défaut rural de nuit ~25–30 dB(A), majoré près des routes ou voies ferrées (cartes de bruit stratégiques), ou saisi par l'utilisateur à partir d'une mesure.
- **Émergence** = niveau avec la sono − niveau résiduel, en global dB(A) et dB(C) et par bande d'octave. Les seuils sont **libres et configurables** par l'utilisateur (par exemple +3, +5, +10 dB) ; aucun référentiel réglementaire n'est intégré.
- Indicateurs : nombre de bâtiments et d'habitants exposés par classe d'émergence (INSEE Filosofi 200 m, champ `ind`), bâtiment le plus exposé, et **marge disponible** (de combien de dB on peut monter avant d'atteindre le seuil choisi).

### 3.5 Dimensionnement de la sono
- **Direct** : sono donnée → carte de SPL, niveau sur le dancefloor, nuisances.
- **Inverse** : zone dancefloor (polygone) + niveau cible (par ex. 100 dB(A) / 115 dB(C) au centre) → nombre de subs et de têtes, puissance d'amplification, marge (headroom), homogénéité de couverture.
- **Optimiseur de placement et d'orientation** : on maximise le niveau sur le dancefloor et on minimise l'émergence au bâtiment le plus exposé. Variables : position, azimut, configuration des subs (cardioïde, end-fire, délais). Recherche grossière avec la méthode d'ingénierie, puis vérification PE des 3 meilleurs candidats.

---

## 4. Données disponibles et pistes pour affiner la simulation

### 4.1 Données IGN (Géoplateforme, `data.geopf.fr`), gratuites (Licence Ouverte)

| Donnée | Usage dans la simulation | Accès |
|---|---|---|
| **MNT LiDAR HD** (0,5 m) | Profils de terrain, diffraction, PE | ✅ WMS-R `IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93` en **float32 (BIL)** pour n'importe quelle emprise et résolution. Testé : 1 km² à 1 m en une requête |
| **MNS LiDAR HD** (0,5 m) | Surface avec le sursol (bâtiments, arbres) : écrans, réflexions | ✅ WMS-R `IGNF_LIDAR-HD_MNS_…LAMB93` |
| **MNH LiDAR HD** (= MNS − MNT) | Hauteur de canopée → forêts, densité de végétation | ✅ WMS-R `IGNF_LIDAR-HD_MNH_…LAMB93` |
| **Nuages de points LiDAR HD** (COPC.LAZ, 1 km², ~170 Mo et ~29 M de points par dalle) | Classes sol/végétation/bâti/eau, densité de végétation par voxel, rugosité du sol | ✅ Index WFS `IGNF_LIDAR-HD_METADONNEE:metadata` (champ `url_npl`) + **HTTP Range validé** (206) |
| **RGE ALTI 1 m / 5 m** | Repli si le LiDAR HD manque, et terrain lointain | ✅ WMS-R et WMTS `ELEVATION.ELEVATIONGRIDCOVERAGE.HIGHRES` (BIL float32) |
| **API Altimétrie** (`/altimetrie/1.0/calcul/alti/rest/elevationLine.json`) | Profils ponctuels rapides (prototype) | ✅ ressources `ign_lidar_hd_mnt_*`, `ign_rge_alti_wld` |
| **Ortho** | **Vue « GPS » / satellite** | ✅ WMTS `ORTHOIMAGERY.ORTHOPHOTOS` (jpeg, z0–19), `HR.ORTHOIMAGERY.ORTHOPHOTOS` |
| **PLAN IGN** (raster et vector tiles) | Fond de carte | ✅ WMTS `GEOGRAPHICALGRIDSYSTEMS.PLANIGNV2` ; TMS pbf `PLAN.IGN` |
| **BD TOPO – bâtiments** | Hauteur, usage, nombre de logements, nombre d'étages, matériaux des murs → récepteurs, écrans, isolation | ✅ WFS `BDTOPO_V3:batiment` (attributs vérifiés). **2 couches max par requête WFS** |
| **BD TOPO – routes, voies ferrées, hydrographie, zones de végétation** | Bruit résiduel, surfaces d'eau (sol dur et réfléchissant), forêts | WFS |
| **OCS GE / CoSIA** (occupation du sol par IA, 15 classes) | Facteur G et résistivité σ du sol par polygone | WMTS / téléchargement |

**Quotas :** le WMS-Raster est limité à ~40 requêtes/s par IP. Il faut donc un cache disque agressif et un client avec limitation de débit.

### 4.2 Météo
| Donnée | Usage | Accès |
|---|---|---|
| **AROME via Open-Meteo** ✅ | **Profil vertical réel** : T à 2/20/50/100/150 m et vent à 10/20/50/100/150 m, nébulosité, humidité. Ajustement de Monin-Obukhov sur ces points. (`boundary_layer_height` et `sensible_heat_flux` sont nuls en AROME.) | `api.open-meteo.com/v1/meteofrance?models=meteofrance_arome_france`, sans clé. Parfois surchargé → retries |
| **AROME GRIB2 brut** ✅ | Mêmes champs, grille complète, paquets HP (hauteurs), IP (isobares), SP (surface) | S3 public `meteofrance-pnt.s3.rbx.io.cloud.ovh.net`, sans clé, mais ~680 Mo par paquet et par tranche de 6 h → mode avancé |
| **Observations des stations** (vent, T) | Calage en temps réel le soir de l'événement | API Météo-France « Données d'observation » |
| **Climatologie / roses des vents** | Planification : « quel risque un samedi soir d'août ici ? » | ERA5, normales Météo-France |
| **Pluie récente / neige** | Sol humide (σ plus élevé) ou neige (sol très absorbant) | Observations / AROME |

### 4.3 Géologie et sols (gain réel mais secondaire en BF)
- **BRGM BD Charm-50** (géologie vectorielle au 1/50 000, WMS et téléchargement) : roche affleurante (causses calcaires, karst, granite) → sol dur (G ≈ 0), falaises réfléchissantes. Très pertinent pour les spots typiques de free party (Causses, Cévennes, carrières).
- **Gis Sol** (RRP, BDGSF) : texture et porosité des sols → σ plus réaliste que la seule occupation du sol.
- **Carrières et gravières** (BD TOPO, BRGM) : parois réfléchissantes et effets d'amphithéâtre.
- À noter honnêtement : à 30–100 Hz, l'impédance dépend surtout des premiers centimètres de sol (végétation, humidité, labour), pas de la géologie profonde. La géologie sert surtout à **détecter la roche nue et les parois**.

### 4.4 Données humaines
- **INSEE Filosofi carroyé à 200 m** ✅ (WFS `INSEE.FILOSOFI.INDICATORS:carreaux_200m`, champ `ind`) : population par carreau → nombre d'habitants exposés.
- **BDNB** (Base de données nationale des bâtiments, CSTB) : usage, logements, matériaux des murs et menuiseries → **isolation de façade**, donc estimation du niveau **à l'intérieur** (point clé : les basses traversent les vitrages).
- **Établissements sensibles** : hôpitaux, EHPAD, écoles (BD TOPO `zone_d_activite_ou_d_interet`), **élevages** (stress animal).
- **Faune / zones protégées** : ZNIEFF ✅ (WFS `patrinat_znieff1:znieff1`, `patrinat_znieff2:znieff2`), `BDTOPO_V3:parc_ou_reserve` → couche de « nuisance faune ». Utile pour choisir un site de moindre impact.
- **Cartes de bruit stratégiques** (directive 2002/49/CE) → bruit résiduel près des grands axes.
- **Cadastre** (parcelles, propriétaires publics ou privés) et **OSM** (chemins d'accès, repli pour les bâtiments).

### 4.5 Idées pour aller plus loin
1. **Calage terrain** : importer des mesures géolocalisées de sonomètre ou d'appli smartphone (CSV ou GPX + dB) prises le soir J. Un ajustement bayésien de σ, de la classe de stabilité et du vent rend la carte de plus en plus juste au fil de la nuit.
2. **Mode « live »** : brancher un sonomètre USB ou une mesure en façade, et afficher l'écart prévu/mesuré en temps réel avec une alerte quand on approche du seuil.
3. **Carte d'incertitude** : une carte P50/P90 sur un ensemble de scénarios météo, plutôt qu'une précision trompeuse.
4. **Assistant « choix de spot »** : sur une zone donnée, classer les sites candidats (clairières, carrières, plateaux) par niveau de nuisance minimal à SPL égal. On peut le faire en lot la nuit sur un rayon de 30 km.
5. **Auralisation** : à un point cliqué (maison du voisin), écouter le rendu filtré (réponse impulsionnelle simplifiée du chemin : retards, filtres de sol et d'absorption) en Rust via `cpal`.
6. **Export** : rapport PDF/HTML (étude d'impact simplifiée), GeoTIFF/GeoJSON pour QGIS, fichier projet `.smproj`.
7. **Comparaison avec la référence** : valider la méthode d'ingénierie contre **NoiseModelling** (Java, open source, CNOSSOS-EU, Univ. Gustave Eiffel) sur des cas communs.

---

## 5. Validation (indispensable pour qu'on fasse confiance aux cartes)
- **Tests analytiques** : champ libre (−6 dB par doublement de distance), réflexion sur sol plan d'impédance finie (Weyl-van der Pol), écran mince (Maekawa), absorption ISO 9613-1 (valeurs tabulées).
- **Cas normalisés** : ISO/TR 17534-4 (CNOSSOS), cas tests Nord2000 (terrain plat, forêt, réfraction).
- **PE vs analytique** sur sol plat en atmosphère homogène, puis PE vs littérature (benchmarks de Salomons, *Computational Atmospheric Acoustics*, 2001).
- **Bench `criterion`** sur chaque étage (profil, GFPE, somme), avec des budgets de temps en CI.

---

## 6. Risques et points ouverts
- ~~Disponibilité des couches LiDAR HD~~ → **résolu** : WMS-R float32 et COPC en HTTP Range fonctionnent (voir ENDPOINTS.md).
- ~~Niveaux verticaux AROME~~ → **résolu** : T et vent à 20/50/100/150 m via Open-Meteo. Le flux de chaleur et la hauteur de couche limite manquent : estimer la stabilité par ajustement du profil (nombre de Richardson) ou par la méthode de Pasquill.
- Résolution AROME de 1,3–2,5 km : les lacs d'air froid de fond de vallée sont mal résolus. Prévoir une correction topographique ou des scénarios manuels.
- MapLibre ne lit pas le BIL float32 : conversion Terrarium côté Rust.
- Open-Meteo parfois surchargé : cache et retries, avec repli possible sur le GRIB2 brut.
- Directivités constructeurs : le GLL est fermé. Partir de presets et du CLF, et laisser la saisie manuelle possible.
- Précision attendue : ±3 dB à moins de 500 m, ±5–10 dB à plusieurs km la nuit. Il faut l'afficher clairement.

---

## Sources
- [MNT LiDAR HD – data.gouv.fr](https://www.data.gouv.fr/datasets/mnt-lidar-hd) · [MNS LiDAR HD](https://www.data.gouv.fr/datasets/mns-lidar-hd/) · [Nuages de points LiDAR HD](https://www.data.gouv.fr/datasets/nuages-de-points-lidar-hd)
- [Offre produit LiDAR IGN (2025-08)](https://geoservices.ign.fr/sites/default/files/2025-09/Offre_Produit_LiDAR_2025-08.pdf) · [Fiche produit Nuages de points LiDAR](https://geoservices.ign.fr/sites/default/files/2024-11/Fiche_produit_Nuages-de-points-LiDAR.pdf)
- [Tuto Géoplateforme](https://geoservices.ign.fr/sites/default/files/2024-06/Tuto_Geoplateforme.pdf) · [Note IGN sur le WFS (limite de 2 couches)](https://geoservices.ign.fr/node/1640) · [Services OCS GE](https://geoservices.ign.fr/node/938) · [CoSIA](https://www.data.gouv.fr/datasets/cosia)
- [BD TOPO](https://www.data.gouv.fr/datasets/bd-topo) · [BDNB](https://www.data.gouv.fr/datasets/base-de-donnees-nationale-des-batiments/discussions)
- [API Géoplateforme WMS-Raster (quotas)](https://www.data.gouv.fr/dataservices/api-geoplateforme-diffusion-dimages-wms-raster/reuses)
- [BRGM BD Charm-50](https://www.data.gouv.fr/datasets/66631d4a67b5b112fafd0f8a)
- [Portail API Météo-France – AROME](https://portail-api.meteofrance.fr/web/en/api/AROME) · [Open-Meteo Météo-France API](https://open-meteo.com/en/docs/meteofrance-api) · [Météo-France sur AWS Open Data](https://registry.opendata.aws/meteo-france-models/)
- [NoiseModelling – documentation](https://noisemodelling.readthedocs.io/) · [Modèle numérique NoiseModelling](https://noisemodelling.readthedocs.io/en/latest/Numerical_Model.html)
- [L-Acoustics (2026) – Simulating low frequency noise pollution using the parabolic equations](https://www.l-acoustics.com/wp-content/uploads/2026/06/Simulating_low_frequency_noise_pollution_using_the_parabolic_equations_in_sound_reinforcement_loudspeaker_systems.pdf)
- [L-Acoustics / AES 151 – Noise pollution: ground vs flown subwoofers](https://www.l-acoustics.com/wp-content/uploads/2021/10/AES151_MouterdeEtAl._noisePollutionGroundVsFlown.pdf)
- [Nord2000 – propagation en atmosphère réfractive](https://forcetechnology.com/da/-/media/force-technology-media/pdf-files/projects/nord2000/nord2000-comprehensive-outdoor-sound-propagation-model---part-2-propagation-in-an-atmosphere-with-re) · [Nordtest NT ACOU 104 (impédance des sols)](https://www.nordtest.info/wp/1999/11/01/617/) · [Attenborough – Ground effect](https://www.ioa.org.uk/system/files/publications/K%20ATTENBOROUGH%20GROUND%20EFFECT%20IN%20OUTDOOR%20NOISE%20PROPAGATION.pdf)
- [ISO 9613-2:2024](https://www.iso.org/standard/74047.html?browse=tc) · [EAA 2025 – révision ISO 9613-2](https://dael.euracoustics.org/confs/fa2025/data/articles/000511.pdf)
- [copc-rs](https://docs.rs/crate/copc-rs/latest) · [copc-core](https://docs.rs/crate/copc-core/latest) · [Terra (terrain wgpu)](https://redirect.github.com/fintelia/terra)
