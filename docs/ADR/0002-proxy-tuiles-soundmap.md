# ADR 0002 — Tout le trafic de tuiles passe par le protocole `soundmap://`

**Statut :** accepté (session 1)

## Contexte
Le mode Exploration doit streamer l'ortho et le Plan IGN avec un cache disque, un User-Agent générique et la limitation de débit. La vue 3D doit lire exclusivement le pack local.

## Décision
- Protocole asynchrone Tauri `soundmap` (`crates/sm-app/src/protocol.rs`). Sous Windows, l'URL est `http://soundmap.localhost/…` ; ailleurs, `soundmap://localhost/…`.
- Routes :
  - `/wmts/{couche}/{z}/{x}/{y}` et `/tms/{couche}/{z}/{x}/{y}.pbf` : proxy avec cache disque `cache/{couche}/{z}/{x}/{y}.{ext}` ;
  - `/zone/{id}/terrain|ortho|ocsge|cosia/…` et `/zone/{id}/file/…` : lecture du pack uniquement.
- En-tête `Access-Control-Allow-Origin: *` sur les réponses : l'UI est servie depuis une autre origine (`localhost:5173` en dev, `tauri.localhost` en production).
- Seules les tuiles présentes sont mises en cache. Un 404 n'est jamais mémorisé, car le WMTS renvoie des 404 transitoires sous charge (voir ENDPOINTS.md).
- `SOUNDMAP_OFFLINE=1` : le proxy ne sert que le cache (test hors ligne).
