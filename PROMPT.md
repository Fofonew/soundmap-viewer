# PROMPT — Construire « SoundMap », simulateur 3D de propagation sonore sur LiDAR HD IGN

> À donner tel quel à un agent de code (Claude Code, etc.) à la racine du dépôt `soundmap-viewer`.
> **Lis d'abord en entier** `docs/RECHERCHE.md` (contexte, physique, choix techniques) et `docs/ENDPOINTS.md` (endpoints **déjà vérifiés en direct** : couches, formats, requêtes testées).
> Projet de **recherche et d'usage personnel** : aucune réglementation n'est à intégrer. Les seuils de nuisance sont de simples paramètres utilisateur.

---

## Rôle
Tu es un ingénieur senior Rust, spécialiste du calcul scientifique haute performance et de l'acoustique environnementale (propagation en extérieur, ISO 9613-2, CNOSSOS-EU, Nord2000, équation parabolique GFPE). Tu construis un logiciel solide : code testé, mesuré, documenté.

## Objectif produit
Une application desktop qui permet de :
1. **Explorer la France en vue satellite** (ortho IGN) et Plan IGN, sans charger de données lourdes.
2. **Choisir une zone de travail** (emplacement de la sono + rayon). Toutes les données de cette zone sont alors **téléchargées en local** : LiDAR HD multi-résolution, bâtiments, occupation du sol, population, météo, ortho.
3. **Visualiser la zone en 3D** (relief LiDAR, bâtiments extrudés, vue satellite drapée) et **placer une sono** : caissons, têtes, line arrays ; position, hauteur, azimut, site, délais, polarité ; réseaux cardioïdes et end-fire.
4. **Simuler la propagation** selon le relief, le sol, la végétation, les bâtiments et la **météo** (vent, gradient de température, inversion nocturne), par bande de fréquence de 25 Hz à 8 kHz.
5. **Évaluer l'impact** : niveau en façade de chaque bâtiment d'habitation, émergence par rapport au bruit résiduel, habitants exposés, bâtiment le plus exposé, marge en dB par rapport à un seuil **choisi par l'utilisateur**.
6. **Dimensionner la sono** : en direct (sono → niveaux) et en inverse (dancefloor + niveau cible → nombre de boîtes et puissance), avec un **optimiseur** de position, d'orientation et de configuration des subs.

Cas d'usage principal : un sound system de free party en milieu rural, de nuit. Les **basses fréquences (30–120 Hz)** et l'**inversion de température nocturne** dominent l'impact à plusieurs km. Elles doivent être traitées par un solveur onde (GFPE), pas seulement par une méthode d'ingénierie.

---

## Contraintes non négociables
- **Chargement en deux temps (RECHERCHE.md §2.4).**
  - *Mode Exploration* : uniquement des tuiles WMTS en streaming (ortho, Plan IGN, emprises LiDAR en option), avec un cache disque. **Aucun appel LiDAR HD, aucun calcul.**
  - *Zone de travail* : un **pack local** est téléchargé une fois (reprenable, avec progression et estimation du volume avant validation). Ensuite, rendu et calcul lisent **exclusivement le local**, hors ligne. Ne pas hésiter à stocker en local tout ce qui améliore la performance.
  - LiDAR en **anneaux de résolution** : 0,5 m jusqu'à ~500 m de la sono, 1 m jusqu'à 2 km, 5 m au-delà. Tous les seuils sont configurables.
  - Nuages de points COPC : **uniquement à la demande**, par HTTP Range, et seulement pour les dalles proches.
- **Performance.**
  - Aperçu interactif en < 300 ms (méthode d'ingénierie, recalcul pendant le déplacement d'une enceinte).
  - GFPE basses fréquences en < 60 s sur 8 cœurs pour un rayon de 5 km.
  - Rasters locaux memory-mappés (`memmap2`).
  - Benchmark `criterion` pour chaque étage.
- **Pas de dépendance C lourde** sous Windows si une alternative pure Rust existe : `proj4rs` plutôt que `proj`, `tiff` plutôt que GDAL.
- `sm-core` est **sans I/O**, compilable en `wasm32`.
- **Respect des services publics** : limitation à ~40 req/s/IP sur le WMS-R, 2 couches max par requête WFS, retry avec backoff (Open-Meteo est parfois « overloaded »), User-Agent générique.
- **Honnêteté des résultats** : afficher l'incertitude (enveloppe de scénarios météo) et la méthode utilisée.
- Documentation à jour des bibliothèques (Tauri 2, MapLibre GL JS, deck.gl, wgpu, rustfft, copc…) **via Context7** avant de les utiliser.

---

## Données — endpoints vérifiés (détails dans `docs/ENDPOINTS.md`)
| Donnée | Accès |
|---|---|
| MNT, MNS, MNH LiDAR HD | WMS-R `https://data.geopf.fr/wms-r`, couches `IGNF_LIDAR-HD_{MNT,MNS,MNH}_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93`, `CRS=EPSG:2154`, `FORMAT=image/x-bil;bits=32` (float32 LE, emprise et résolution libres) |
| Index des dalles + URL COPC | WFS `IGNF_LIDAR-HD_METADONNEE:metadata` (champ `url_npl`) ; COPC lisible par HTTP Range (206 vérifié) |
| Repli relief | `ELEVATION.ELEVATIONGRIDCOVERAGE.HIGHRES` (RGE ALTI), WMS-R et WMTS en BIL |
| Vue satellite | WMTS `ORTHOIMAGERY.ORTHOPHOTOS` (jpeg, PM, z0–19) |
| Plan | WMTS `GEOGRAPHICALGRIDSYSTEMS.PLANIGNV2` ; vector tiles TMS `PLAN.IGN` |
| Bâtiments | WFS `BDTOPO_V3:batiment` (`usage_1`, `nombre_de_logements`, `hauteur`, `materiaux_des_murs`, etc.) |
| Végétation, eau, routes, lieux d'intérêt | WFS `BDTOPO_V3:zone_de_vegetation`, `surface_hydrographique`, `troncon_de_route`, `zone_d_activite_ou_d_interet` |
| Occupation du sol | WMTS `OCSGE.COUVERTURE.2021-2023` / `IGNF_COSIA_2024-2026`, ou TMS pbf `OCSGE-TMS.COUVERTURE.*` |
| Population | WFS `INSEE.FILOSOFI.INDICATORS:carreaux_200m` (champ `ind`) |
| Faune | WFS `patrinat_znieff1:znieff1`, `patrinat_znieff2:znieff2` |
| Géocodage | `https://data.geopf.fr/geocodage/search?q=...` (renvoie aussi x/y Lambert-93) |
| Météo | Open-Meteo `.../v1/meteofrance?models=meteofrance_arome_france` : T à 2/20/50/100/150 m, vent à 10/20/50/100/150 m, nébulosité. Repli GRIB2 sur le S3 public Météo-France |
| Géologie (option) | WMS BRGM `https://geoservices.brgm.fr/geologie` ; BD Charm-50 vectorielle |

---

## Architecture cible

```
soundmap-viewer/
├─ Cargo.toml                 (workspace)
├─ crates/
│  ├─ sm-core/     physique pure : bandes, pondérations A/C, sources & directivité complexe,
│  │               atmosphère (ISO 9613-1, Monin-Obukhov ajusté sur le profil AROME, Pasquill),
│  │               sol (Delany-Bazley/Miki, classes σ), méthode d'ingénierie, GFPE avec relief,
│  │               sommes cohérente/incohérente
│  ├─ sm-geo/      L93↔WGS84 (proj4rs), rasters multi-résolution mmap, profils de terrain,
│  │               R-tree bâtiments, rasterisation de l'occupation du sol, isophones, encodeur Terrarium
│  ├─ sm-data/     clients Géoplateforme (WMTS/WMS-R/WFS/géocodage/COPC Range), Open-Meteo,
│  │               BRGM ; cache de tuiles ; **gestionnaire de packs de zone** (plan de téléchargement,
│  │               estimation du volume, reprise, manifeste versionné)
│  ├─ sm-compute/  grille polaire, parallélisme rayon, jobs annulables avec progression,
│  │               (phase 5) backend wgpu
│  └─ sm-app/      Tauri 2 : commandes IPC ; protocole `soundmap://` qui sert les tuiles ortho
│                  en cache, le terrain Terrarium et les résultats SPL depuis le pack local
├─ ui/             Vite + TypeScript + React (ou Svelte) + MapLibre GL JS + deck.gl
├─ zones/          (hors git) packs locaux : manifest.json, rasters float32 tuilés, *.fgb, tuiles
├─ docs/           RECHERCHE.md, ENDPOINTS.md, PHYSIQUE.md, ADR/
└─ tests/data/     petits extraits MNT/bâtiments et cas tests de référence
```

---

## Feuille de route (chaque phase se termine par une démo fonctionnelle et des tests verts)

### Phase 0 — Fondations
- Workspace cargo, CI (fmt, clippy `-D warnings`, tests, bench smoke), `tracing`.
- `sm-data` : clients typés pour chaque endpoint d'ENDPOINTS.md, avec un test d'intégration chacun (marqués `#[ignore]` pour la CI hors ligne) et une limitation de débit.
- **Critère de sortie :** `cargo run -p sm-data --example probe` télécharge un MNT de 1 km² en float32, les bâtiments de la zone, une tuile ortho et le profil météo, puis affiche un résumé.

### Phase 1 — Exploration et pack de zone
- Fenêtre Tauri avec MapLibre, **vue satellite par défaut**, bascule Plan IGN, recherche d'adresse, couche d'emprise LiDAR HD en option. Aucune donnée lourde.
- Outil « Zone de travail » : clic sur la sono et rayon. Estimation du volume et du temps → validation → téléchargement en tâche de fond (anneaux de résolution, vecteurs, occupation du sol, Filosofi, météo, ortho haut zoom) → pack dans `zones/{id}/`. Reprise après coupure et liste des zones déjà téléchargées.
- Passage en **vue 3D de la zone** : terrain Terrarium généré depuis le pack, ortho drapée, bâtiments extrudés, habitations mises en évidence.
- Placement et édition des enceintes (gizmo de position et d'azimut), presets (sub 18", 2×18", top 15", line array générique) et saisie manuelle. Polygone dancefloor.
- **Critères de sortie :** une zone de 5 km de rayon se télécharge, se rouvre hors ligne en < 2 s, et le projet `.smproj` se sauvegarde et se recharge.

### Phase 2 — Méthode d'ingénierie (aperçu rapide)
- `sm-core` : bandes d'octave et de 1/3 d'octave de 25 Hz à 8 kHz, pondérations A/C, ISO 9613-1, divergence, effet de sol (G par segment), diffraction multi-arêtes sur le profil MNT, conditions homogènes et favorables, feuillage (MNH), écrans bâtis.
- Somme **cohérente** pour les subs (amplitude complexe, délai, polarité), **incohérente** pour les têtes.
- `sm-compute` : grille polaire, profils extraits une fois par azimut et réutilisés, recalcul incrémental quand une source bouge.
- UI : heatmap dB(A), dB(C) ou par bande, isophones, clic → coupe du terrain avec ligne de visée.
- **Critères de sortie :** tests analytiques verts (champ libre, Weyl-van der Pol, Maekawa, tables ISO 9613-1) ; aperçu < 300 ms ; comparaison à NoiseModelling sur 2 cas documentée.

### Phase 3 — Basses fréquences par GFPE
- GFPE N×2D (`rustfft` + `rayon`) : célérité effective par azimut, impédance par segment (Delany-Bazley ou Miki, classes σ Nordtest), couche absorbante, relief (GTPE ou transformation conforme ; décision dans un ADR), starter pondéré par la directivité complexe du réseau de subs.
- Atmosphère : **ajustement de Monin-Obukhov sur le profil AROME** (T et vent à 2–150 m), extrapolation au-dessus ; repli par classe de Pasquill ; scénarios manuels (neutre, inversion forte, vent portant vers les habitations).
- Fusion PE (< 200 Hz) et méthode d'ingénierie (> 200 Hz) avec transition progressive.
- Enveloppe d'incertitude : plusieurs scénarios en parallèle, carte médiane et carte P90.
- **Critères de sortie :** PE = analytique sur sol plat homogène (±1 dB) ; reproduction qualitative des benchmarks de Salomons (2001) ; < 60 s sur 8 cœurs pour 5 km × 360 azimuts × 1/3 d'octave 25–200 Hz.

### Phase 4 — Impact, dimensionnement, optimisation
- Récepteurs en façade des habitations (1,5 m et 4 m), bruit résiduel (défaut rural réglable, proximité des routes, saisie de mesure), émergence globale et par octave, **seuils libres**.
- Habitants exposés (Filosofi), lieux sensibles, couche faune (ZNIEFF).
- Dimensionnement inverse : niveau cible sur le dancefloor → nombre de boîtes, puissance, headroom, homogénéité.
- Optimiseur : position, azimut et configuration cardioïde ou end-fire ; recherche grossière avec la méthode d'ingénierie, puis GFPE sur les 3 meilleurs candidats.
- Export HTML/PDF, GeoTIFF, GeoJSON.

### Phase 5 — Raffinements (en option, par ordre de valeur)
1. Backend **wgpu** pour le GFPE (FFT Stockham en WGSL), avec bascule automatique CPU/GPU.
2. **Calage sur mesures** (CSV/GPX de sonomètre) : ajustement bayésien de σ, de la stabilité et du vent.
3. COPC à la demande : densité de végétation par voxel et rugosité du sol.
4. Géologie BRGM (roche nue → sol dur, falaises), humidité du sol après pluie.
5. Lacs d'air froid et brises catabatiques en vallée.
6. Assistant « choix de spot » sur 30 km, et auralisation (`cpal`).

---

## Règles de travail
- Commence chaque phase par un plan court (fichiers, interfaces, tests). TDD pour `sm-core` : test analytique d'abord.
- Toutes les équations sont documentées dans `docs/PHYSIQUE.md` avec leur référence.
- Unités explicites dans les types (newtypes `Db`, `Hz`, `Meters`). Grilles en Lambert-93 côté calcul, WGS84 côté UI uniquement.
- Calculs et téléchargements longs **annulables**, avec progression ; jamais de blocage du thread UI.
- Pas d'`unwrap()` hors des tests ; `thiserror` dans les bibliothèques, `anyhow` dans l'app.
- Signaler toute régression de benchmark supérieure à 10 %.
- Ne pose une question que si la décision change réellement le résultat. Sinon, choisis et note-le dans un ADR.

## Livrable de la première session
Phase 0 complète et phase 1 jusqu'au pack de zone : exploration satellite, sélection d'une zone, téléchargement local, vue 3D de la zone. Termine par un résumé : ce qui marche, les mesures de performance, les écarts avec la doc et le plan de la phase suivante.
