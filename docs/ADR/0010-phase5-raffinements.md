# ADR 0010 — Phase 5 : raffinements

**Statut :** accepté (phase 5). Le GFPE sur GPU fait l'objet de l'ADR 0009.

## 1. Calage sur mesures (`sm_compute::calibrate`)
- **Entrées :**
  - CSV : séparateur `,`, `;` ou tabulation, virgule décimale acceptée ; colonnes lat/lon et
    LAeq et/ou LCeq ;
  - GPX : points `wpt`/`trkpt` ; niveaux pris dans les balises d'extension ou dans
    `name`/`desc`/`cmt`, par exemple « 62,5 dB(A) » ou « LCeq=78 ».
- **Paramètres calés** sur la méthode d'ingénierie, au point de mesure exact :
  - facteur sur σ (9 valeurs de ×0,25 à ×4) ;
  - gradient de célérité thermique g_T (0 à 0,2 s⁻¹) ;
  - gradient dû au vent g_W (0 à 0,1 s⁻¹), dans la direction du vent prévu par AROME ;
  - décalage global de niveau δ.
- **Méthode :** grille bayésienne. δ suit un a priori N(0, 3²) et est intégré analytiquement ;
  a priori log2(facteur σ) ~ N(0, 1) ; bruit de 3 dB par valeur.
  Résultat : moyenne, écart-type et maximum a posteriori (MAP) de chaque paramètre.
- **Rayon de courbure** : Rc(az) = c/g(az) avec g = g_T + g_W·cos(az − az_vent).
  - Le rayon est borné à 200 m ou plus.
  - Si g ≤ 0, les rayons restent droits : la méthode d'ingénierie ne modélise pas les zones
    d'ombre. Le rapport le signale quand les mesures sont plus basses que le modèle même sans
    réfraction.
- **Application** : `Settings.calibration`. Le facteur σ et la réfraction passent par `PathConfig`,
  le décalage s'ajoute au gain général. Le GFPE reçoit aussi le facteur σ.
- **Validation** : `examples/calib_check`, 30 mesures synthétiques aux habitations du Larzac,
  bruit de 1 dB.
  - Paramètres imposés : σ × 2, g_T = 0,05 s⁻¹, +2 dB.
  - Écart quadratique : 6,6 dB avant, 1,0 dB après.
  - g_T retrouvé à 0,050 et δ à +2,17 dB. Le facteur σ est peu contraint (MAP × 1,41,
    ± 0,76 octave), et le rapport le dit.

## 2. Végétation et rugosité LiDAR HD à la demande (`sm_data::copc`, `sm_compute::vegetation`)
- **Lecture COPC par HTTP Range :**
  - VLR `copc`, VLR LAZ, pages de hiérarchie de l'octree ;
  - nœuds limités à l'emprise et au niveau de détail utile ;
  - décodage LAZ avec le crate `laz`, en Rust pur.
- **Niveau de détail :** profondeur choisie pour un espacement d'environ 2 m (`depth_for_spacing`).
  Mesures sur le Larzac, 1 km de rayon, 9 dalles :
  - 2 m → 8,8 M points, 124 Mo, 47 s ;
  - 1 m → 29 M points, 327 Mo, 204 s, pour des statistiques quasi identiques.
- **Voxels :** 5 m × 5 m × 2 m, couches de 1 à 31 m au-dessus du MNT. Densité d'un voxel =
  fraction des impulsions qui l'atteignent et y sont interceptées : n_l / Σ_{k≤l} n_k.
- **Facteur de feuillage :** f = ln P / ln 0,2, borné à [0 ; 1,5].
  - P = n(< 1 m) / n(< 11 m) est le taux de passage (Beer-Lambert).
  - f = 1 correspond au « feuillage dense » de l'ISO 9613-2, rapporté à P = 0,2.
  - Il pondère la longueur de trajet sous la canopée : `PathSample.foliage`.
  - Sans LiDAR, le forfait est conservé (MNH ≥ 3 m = feuillage dense).
- **Rugosité :** écart-type des points sol (classe 2) par rapport au MNT. Elle est calculée et
  affichée, mais pas encore utilisée dans la propagation : aucun modèle de sol rugueux assez
  simple et validé n'a été retenu.

## 3. Sol : géologie, falaises, état du sol (`sm_compute::ground_state`)
- **Falaises :** pente du MNT ≥ 45° (hors forêt et eau) → roche (classe G).
- **Géologie :**
  - Source : lithologie BRGM au 1/1 000 000 (WFS `LITHO_1M_SIMPLIFIEE`, GeoJSON en
    Lambert-93), classée par mots-clés en roche dure, mixte ou meuble.
  - Sur roche dure : matériaux minéraux → roche ; sols en pente > 25° durcis d'une classe (sols
    minces). Sur roche mixte, même règle au-delà de 35°.
  - Le scan au 1/50 000 n'est pas interrogeable en attributs, et la BD Charm-50 vectorielle
    n'existe qu'en téléchargement départemental : l'échelle 1/1 000 000 est une limite assumée.
- **État du sol (Open-Meteo) :** humidité 0–1 cm, neige, pluie des 24 h, pour la veille et le
  jour J.
  - σ × 1 jusqu'à 0,25 m³/m³, puis linéaire jusqu'à × 2,5 à 0,45 m³/m³ : les pores se remplissent
    d'eau, le sol devient plus résistif. C'est une heuristique, désactivable dans l'interface.
  - Neige ≥ 5 cm → classe A (σ = 12,5 kPa·s/m²) partout.
- **Mesures (Larzac) :** 0,27 km² de falaises et 0,85 km² de sols durcis. Avec la pluie du
  17/11 (30 mm), σ × 2,47 à 23 h : +0,8 dB(A) et +1,6 dB(C) à l'habitation la plus exposée.

## 4. Vallées : lac d'air froid et vent catabatique (`sm_core::atmosphere::ValleyFlow`, `sm_compute::valley`)
- **Détection :** TPI = z(sono) − z̄(anneau de 300 à 1 500 m) ; vallée si TPI < −10 m.
  La direction aval vient d'un plan ajusté au MNT sur 1,5 km.
- **Profil :** superposé à un profil Pasquill F à 1 m/s.
  - Lac d'air froid : ΔT·(1 − z/h)².
  - Vent catabatique : jet de Prandtl u₀·e^(−z/l)·sin(z/l), maximum u_max à 10 m.
- **Paramètres :** h = 0,6·|TPI| (20–150 m), ΔT = 0,04·h (2–8 K), u_max = 1 + 15·sin(pente)
  (1–4 m/s). Ordres de grandeur tirés de Whiteman (2000).
- **Usage :** 5ᵉ scénario du calcul basses fréquences, ajouté seulement si la sono est en vallée.
  Le Larzac est sur le plateau (TPI +22 m) ; la zone de test de 1 km est en creux (TPI −22 m).

## 5. Assistant de choix de spot (`sm_compute::spots`)
- **Données :** MNT RGE ALTI à 100 m (une requête) et carreaux Filosofi 200 m (WFS paginé), sur
  le rayon de recherche (≤ 30 km) plus 6 km de marge.
- **Criblage à 63 Hz :** L = L₁ − 20 lg d + 3 − A_écran.
  - A_écran suit Maekawa sur le profil du MNT, avec des rayons courbes de conditions favorables
    (même abaissement que la méthode d'ingénierie), plafonné à 20 dB.
  - Candidats tous les 500 m, pente ≤ 8 %.
- **Classement :** habitants au-delà du seuil, puis distance au premier habitant, puis pente.
  Les 10 meilleurs spots sont espacés d'au moins 2 km.
- **Mesure :** 30 km autour du Larzac en 4,5 s ; 3 307 candidats, 7 084 carreaux, 88 918
  habitants.
- **Limite :** c'est un criblage. Le spot retenu doit être téléchargé et calculé finement.

## 6. Auralisation (`sm_compute::auralize`)
- **Choix :** pas de `cpal`, qui demande ALSA, une bibliothèque C, sous Linux. Le WAV est généré
  en Rust (16 bits, 22 050 Hz) et lu par `<audio>` dans la webview, ce qui est portable.
- **Son :** boucle techno de synthèse à 138 BPM : kick glissant, basse en contretemps,
  charleston, clap.
- **Filtrage :** égaliseur à phase nulle par FFT. Le gain ΔL_b(point − dancefloor) est interpolé
  en dB sur l'échelle log des fréquences ; les bandes sont celles du GFPE si un résultat basses
  fréquences existe.
- **Bruit résiduel :** bruit rose ajouté au niveau relatif L_résiduel − L_point (dB(A), rapport
  des valeurs RMS non pondérées).
- **Niveau d'écoute :** ramené pour entendre le timbre ; l'écart réel (ex. −62 dB à 780 m) est
  affiché.
