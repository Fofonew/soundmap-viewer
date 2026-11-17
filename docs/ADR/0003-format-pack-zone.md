# ADR 0003 — Format du pack de zone

**Statut :** accepté (session 1)

## Décision
- `zones/{id}/manifest.json` contient le schéma versionné (`schema_version = 1`), la spec, le plan complet, l'état, les produits et le hash des sources.
  - `id` = `{slug}-{8 hex}`, un hash du centre et du rayon : stable, donc une même zone n'est jamais re-téléchargée.
- **Reprise.** `progress.log` est un journal append-only `id\toctets`, une ligne par item terminé. Réécrire le manifeste après chacune des ~3 000 tuiles coûterait O(n²).
  - Chaque fichier est écrit en `.part` puis renommé.
  - Un item est sauté si son id figure dans le journal **et** que le fichier existe.
- **Rasters `.smr`** : en-tête de 64 octets, puis float32 LE tuilé 256×256, lu par `memmap2` (`sm-geo/src/raster.rs`). Pourquoi pas GeoTIFF : pas de dépendance, accès aléatoire O(1), et l'export GeoTIFF reste prévu en phase 4.
- **Vecteurs** : FlatGeobuf en Lambert-93 (calcul) et `ui/buildings.geojson` en WGS84, réduit aux attributs utiles (affichage).
- **Tuiles** : un fichier par tuile (`ortho/`, `ocsge/`, `terrain/`), pas de MBTiles. SQLite serait une dépendance C, et la réouverture n'énumère pas les tuiles.
- **Terrain Terrarium** : pré-généré jusqu'à z15 à la finalisation (183 tuiles pour 5 km, < 1 s), puis généré à la demande et mis en cache jusqu'à z17.
- Les blocs BIL bruts sont supprimés après assemblage.

## Mesures (zone de 5 km, 17/11/2026)
- 403 Mo sur disque, dont 289 Mo de rasters.
- Réouverture (manifeste + mmap) : 3 à 8 ms.
- Premier rendu 3D complet : 331 à 448 ms.
