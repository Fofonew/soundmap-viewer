# ADR 0005 — Anneaux de résolution en carrés concentriques

**Statut :** accepté (session 1)

## Décision
- Chaque anneau est un **carré complet** centré sur la sono : 0,5 m sur ±500 m, 1 m sur ±2 km, 5 m sur ±R.
  - Les anneaux se recouvrent ; l'échantillonnage prend l'anneau le plus fin qui couvre le point (`MultiResDem::sample`).
  - Les demi-côtés sont bornés au rayon ; un anneau qui ne dépasse pas le précédent est supprimé.
- **Téléchargement** :
  - blocs WMS-R de 1000×1000 px (format validé ; 2048² et 4096² fonctionnent aussi, mais une erreur coûte plus cher) ;
  - les blocs qui ne touchent pas le disque de rayon R sont ignorés ;
  - le centre est calé sur une grille de 5 m pour aligner tous les anneaux.
- Tout est configurable dans `ZoneSpec` : `rings`, `block_px`, `ortho`, `ocsge_zoom`.

## Écart avec RECHERCHE.md §2.4
RECHERCHE estimait ~70 Mo par couche pour 5 km avec des anneaux annulaires. Les carrés recouvrants donnent 96 Mo par couche, soit 192 Mo pour MNT + MNS.

On accepte ce surcoût de 37 % : le recouvrement donne un raster continu par anneau, sans trou, ce qui simplifie les profils de terrain de la phase 2.
