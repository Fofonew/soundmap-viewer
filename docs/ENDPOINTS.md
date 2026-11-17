# Endpoints vérifiés en direct (17/11/2026)

Tous testés par requête réelle (curl) sur une zone test près de Millau (L93 ≈ 700 000–703 000 / 6 320 000–6 323 000).
Aucune clé API nécessaire pour l'ensemble de cette page.

---

## 1. Relief LiDAR HD — WMS-R Géoplateforme ✅ (source principale)

`https://data.geopf.fr/wms-r` — WMS 1.3.0. Formats disponibles : `image/x-bil;bits=32` (float32 brut), `image/geotiff`, `image/tiff`, `image/png`.

| Couche | Contenu | CRS |
|---|---|---|
| `IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93` | **MNT** (sol nu) 0,5 m | EPSG:2154 |
| `IGNF_LIDAR-HD_MNS_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93` | **MNS** (surface avec le sursol) | EPSG:2154 |
| `IGNF_LIDAR-HD_MNH_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93` | **MNH** (hauteur du sursol) | EPSG:2154 |
| `IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.WGS84G` (et MNS/MNH) | idem en WGS84 | EPSG:4326 |
| `IGNF_LIDAR-HD_MNT_ELEVATION.MIXED.WGS84G` | MNT LiDAR complété (mixte) | EPSG:4326 |
| `ELEVATION.ELEVATIONGRIDCOVERAGE.HIGHRES` | RGE ALTI (repli) | multiple |
| `IGNF_LIDAR-HD_CLASS`, `IGNF_LIDAR-HD_DENSITE-SOL`, `IGNF_LIDAR-HD_DENSITE-SURSOL` | classification et densités de points (rendus raster) | |

Styles : `normal` (valeurs brutes) et `hypso`.

**Test validé :**
```
GET https://data.geopf.fr/wms-r?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap
    &LAYERS=IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93&STYLES=normal
    &CRS=EPSG:2154&BBOX=700000,6320000,701000,6321000&WIDTH=1000&HEIGHT=1000
    &FORMAT=image/x-bil;bits=32
→ 200, 4 000 000 octets = 1000×1000 float32 little-endian, altitudes 494,7–830,4 m, aucune valeur nodata
```
→ **On obtient 1 km² à 1 m, ou n'importe quelle emprise et résolution, en une seule requête, sans télécharger de dalle.** C'est idéal pour la grille multi-résolution : 0,5–1 m près des sources, 5 m au loin.
Quota indiqué : ~40 req/s/IP.

## 2. Index des dalles LiDAR HD et nuages de points COPC ✅

**WFS** `IGNF_LIDAR-HD_METADONNEE:metadata` : un polygone par dalle de 1 km², avec les attributs suivants :
`url_mnt`, `url_mns`, `url_mnh` (GetMap GeoTIFF 2000×2000 à 0,5 m), **`url_npl` (nuage COPC)**, `nombre_points`, `date_debut_acquisition`, `date_fin_acquisition`, `capteur`, `procede_classement`, `systeme_altimetrique` (IGN69).

```
GET https://data.geopf.fr/wfs/ows?SERVICE=WFS&VERSION=2.0.0&REQUEST=GetFeature
    &TYPENAMES=IGNF_LIDAR-HD_METADONNEE:metadata&OUTPUTFORMAT=application/json
    &BBOX=700000,6320000,703000,6323000,EPSG:2154
```

**COPC en HTTP Range validé :**
```
GET https://data.geopf.fr/telechargement/download/LiDARHD-NUALID/NUALHD_1-0__LAZ_LAMB93_LP_2025-02-28/LHD_FXX_0701_6321_PTS_LAMB93_IGN69.copc.laz
Range: bytes=0-1023
→ 206 Partial Content, Accept-Ranges: bytes, taille totale 169,8 Mo
→ LASF 1.4, point format 6, 28 715 324 points, VLR « copc » présent
```
→ On peut lire l'octree et ne charger que les niveaux de détail et les zones utiles (avec `copc-rs` ou `copc-core` et un lecteur HTTP Range).

Les **tuiles vectorielles des emprises** existent aussi en TMS : `https://data.geopf.fr/tms/1.0.0/IGNF_NUAGES-DE-POINTS-LIDAR-HD-produit` (et `IGNF_MNT-LIDAR-HD-produit`, etc.), utiles pour afficher la couverture dans l'UI.

## 3. Fonds de carte — WMTS ✅

`https://data.geopf.fr/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER={layer}&STYLE=normal&TILEMATRIXSET={tms}&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}&FORMAT={fmt}`

| Usage | Couche | Format | TileMatrixSet | Zooms |
|---|---|---|---|---|
| **Vue « GPS » / ortho** | `ORTHOIMAGERY.ORTHOPHOTOS` | image/jpeg | PM_0_19 | 0–19 |
| Ortho haute résolution | `HR.ORTHOIMAGERY.ORTHOPHOTOS` | image/jpeg | PM_6_19 | 6–19 |
| Plan IGN | `GEOGRAPHICALGRIDSYSTEMS.PLANIGNV2` | image/png | PM_0_19 | 0–19 |
| Ombrage LiDAR | `IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.SHADOW` | image/png | PM_0_18 | 0–18 |
| Relief (RGE ALTI) | `ELEVATION.ELEVATIONGRIDCOVERAGE.HIGHRES` | **image/x-bil;bits=32** | WGS84G_6_14 | 6–14 |
| Occupation du sol | `OCSGE.COUVERTURE.2021-2023` (aussi `2024-2026`), `OCSGE.USAGE.*` | image/png | PM_6_16 | |
| CoSIA (IA) | `IGNF_COSIA_2021-2023`, `IGNF_COSIA_2024-2026` | image/png | PM_6_18 | |

**Terrain 3D dans MapLibre :** le WMTS d'élévation est en BIL float32, format que MapLibre ne lit pas. Le backend Rust doit donc **convertir à la volée en tuiles Terrarium PNG** servies via `soundmap://terrain/{z}/{x}/{y}.png`, de préférence depuis le WMS-R LiDAR HD pour la haute résolution.

**Vector tiles (TMS, pbf) :** `https://data.geopf.fr/tms/1.0.0/PLAN.IGN/{z}/{x}/{y}.pbf` (Plan IGN vectoriel), `.../BDTOPO/...` (BD TOPO personnalisable), `.../OCSGE-TMS.COUVERTURE.2021-2023/...`, `.../ISOHYPSE/...` (courbes de niveau), `.../LANDCOVER-TMS.FORESTINVENTORY.V2/...` (BD Forêt v2), `.../IGNF_CARTO-FORMATIONS-VEGETALES_2022_TMS/...`.
Raster : `.../PLANIGN.LIDAR.TERRAIN`.

## 4. Vecteurs — WFS ✅

`https://data.geopf.fr/wfs/ows?SERVICE=WFS&VERSION=2.0.0&REQUEST=GetFeature&TYPENAMES={type}&OUTPUTFORMAT=application/json&BBOX=xmin,ymin,xmax,ymax,EPSG:2154`
(2 couches maximum par requête)

| Type | Usage | Attributs utiles vérifiés |
|---|---|---|
| `BDTOPO_V3:batiment` | Récepteurs, écrans | `nature`, `usage_1`, `usage_2`, `nombre_de_logements`, `nombre_d_etages`, `hauteur`, `altitude_minimale_sol`, `altitude_maximale_toit`, `materiaux_des_murs`, `materiaux_de_la_toiture`, `construction_legere`, `etat_de_l_objet`, `identifiants_rnb` |
| `BDTOPO_V3:zone_de_vegetation` | Forêts, haies | |
| `BDTOPO_V3:surface_hydrographique`, `plan_d_eau` | Sol réfléchissant | |
| `BDTOPO_V3:troncon_de_route`, `troncon_de_voie_ferree` | Bruit résiduel | |
| `BDTOPO_V3:zone_d_activite_ou_d_interet` | Lieux sensibles (santé, enseignement, élevage…) | |
| `BDTOPO_V3:parc_ou_reserve` | Espaces protégés | |
| **`INSEE.FILOSOFI.INDICATORS:carreaux_200m`** | Population exposée | `ind` (habitants), `men` (ménages), `men_mais` (maisons), tranches d'âge, `log_av45` (logements anciens → isolation) |
| `patrinat_znieff1:znieff1`, `patrinat_znieff2:znieff2` | Couche faune | |
| `cbs2022_*` (selon le département) | Cartes de bruit stratégiques | disponibilité partielle |

## 5. API Géoplateforme utilitaires ✅

- **Géocodage :** `https://data.geopf.fr/geocodage/search?q=Millau&limit=1` → GeoJSON avec lon/lat **et x/y Lambert-93**.
- **Altimétrie (profils rapides) :** `https://data.geopf.fr/altimetrie/1.0/calcul/alti/rest/elevationLine.json?lon=a|b&lat=c|d&resource=ign_rge_alti_wld&sampling=N`
  Ressources (`/altimetrie/resources?format=json`) : `ign_lidar_hd_mnt_mono_wld`, `ign_lidar_hd_mnt_multi_wld`, `IGNF_LIDARHD_MNX_WLD`, `ign_rge_alti_wld`, etc.
  À réserver au prototypage : pour le calcul, le WMS-R en float32 est beaucoup plus efficace.

## 6. Météo ✅

### Open-Meteo (le plus simple, sans clé)
`https://api.open-meteo.com/v1/meteofrance?latitude=..&longitude=..&hourly=...&models=meteofrance_arome_france`
(et `meteofrance_arome_france_hd`, `meteofrance_seamless`)

**Variables AROME non nulles vérifiées :**
`temperature_2m`, `temperature_20m`, `temperature_50m`, `temperature_100m`, `temperature_150m`,
`wind_speed_10m`, `wind_direction_10m`, `wind_speed_20m`, `wind_speed_50m`, `wind_speed_100m`, `wind_speed_150m`, (`wind_direction_*m`),
`cloud_cover`, `relative_humidity_2m`, `temperature_1000hPa/950hPa/925hPa`.
**Nulles en AROME :** `boundary_layer_height`, `sensible_heat_flux`. Pour ces deux-là : modèle seamless ou ICON, ou déduction par la classe de stabilité.

→ On a **un profil vertical réel T(z) et U(z) sur 2–150 m** : on ajuste un profil de Monin-Obukhov (log-linéaire) sur ces 5 points, puis on l'extrapole jusqu'à 300 m. C'est exactement la tranche qui pilote la réfraction des basses.
Attention : le service renvoie parfois « The service is overloaded ». Prévoir des retries avec backoff et un cache.

### Météo-France brut (GRIB2, sans clé, S3 public)
`https://meteofrance-pnt.s3.rbx.io.cloud.ovh.net/pnt/{run}/arome/0025/{PAQUET}/arome__0025__{PAQUET}__{19H24H}__{run}.grib2`
- Paquets : `SP1–SP3` (surface), `IP1–IP5` (niveaux isobares), `HP1–HP3` (niveaux hauteur). Tranches horaires de 6 h, jusqu'à 51 h. Aussi en 0,01° (dataset « Paquets Arome - Résolution 0,01° »).
- `Accept-Ranges: bytes`, mais **~680 Mo par fichier HP1 pour 6 h sur toute la France**. À réserver à un mode avancé (décodage GRIB2 en Rust, extraction d'une petite zone).
- Catalogue : data.gouv.fr `paquets-arome-resolution-0-025deg` et `paquets-arome-resolution-0-01deg`.

## 7. Géologie BRGM ✅
`https://geoservices.brgm.fr/geologie?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`
Couches : `GEOLOGIE`, `SCAN_D_GEOL50` (carte au 1/50 000), `SCAN_F_GEOL250`, `SCAN_F_GEOL1M`.
Pour les attributs lithologiques (roche nue ou formations meubles), utiliser la BD Charm-50 vectorielle (téléchargement data.gouv), plus exploitable que le scan.

## 8. Sols (pédologie)
Des RRP (Référentiels Régionaux Pédologiques) commencent à apparaître en TMS sur la Géoplateforme (ex. `rrp_gironde_gpkg_03-09-2026_tms`, `rrp_landes_...`). La couverture reste **partielle** par département. À traiter comme une donnée optionnelle.

---

## 9. Compléments mesurés en session 1 (17/11/2026, implémentation `sm-data`)

| Service | Constat | Traitement dans le code |
|---|---|---|
| WMS-R | **nodata = −9999** hors couverture (testé hors France) | `sm_geo::raster::NODATA` |
| WMS-R | Tailles validées : 1000², 2048² (16 Mo, 3,2 s), **4096² (67 Mo, 13 s)** ; emprise 5 km à 1000 px (5 m) OK | blocs de 1000 px (reprise fine) |
| WMS-R | **502 et 400 transitoires** (même requête valide une seconde plus tard) | retry sur 5xx, et sur 400 pour le WMS-R uniquement |
| WMTS | **404 « No data found » transitoires sous charge** : sur un pack de 5 km, tous ont réussi au 1er retry (0 tuile réellement absente) | 1 retry sur 404 (WMTS/TMS) ; un 404 n'est jamais mis en cache |
| WFS | `CountDefault = 5000` (GetCapabilities) | pagination `COUNT`/`STARTINDEX` |
| TMS | Schéma **XYZ** (pas de y inversé). Source-layers : `nuage_dalle` (emprises LiDAR, z10–16), `occupation_sol` (OCS GE, z12–16, attribut `code_cs`) | `sm_data::tms` |
| Géocodage | `properties.x/y` (L93) présents pour les communes ; Millau = (707460,4 ; 6333667,33) | point de référence des tests de projection |
| Altimétrie | Réponse `{"elevations":[{"lon","lat","z","acc"}…]}` | `sm_data::altimetrie` |
| AROME S3 | Runs `pnt/AAAA-MM-JJTHH:00:00Z/` toutes les 3 h ; tranches `00H06H, 07H12H, 13H18H, 19H24H, … 49H51H` | `sm_data::arome_grib` |
| Débit global | Pack de 5 km : **260 Mo en 244 s ≈ 1,1 Mo/s** effectif (48 requêtes en vol). Une requête isolée : ~2 Mo/s. Avec un timeout total de 60 s, les gros blocs WMS-R expiraient. | timeout total 300 s + timeout d'inactivité 30 s ; débit de 1,5 Mo/s supposé pour l'estimation |
