# ADR 0001 — UI : React + TypeScript + Vite + MapLibre GL JS (sans wrapper)

**Statut :** accepté (session 1, 17/11/2026)

## Contexte
PROMPT.md laisse le choix entre React et Svelte. Il faut une carte 2D/3D (terrain `raster-dem`, extrusions), puis en phase 2 des couches deck.gl.

## Décision
- React 19 + TypeScript + Vite 8, MapLibre GL JS 6 utilisé directement (pas de `react-map-gl`) : une seule instance de carte gérée par `useRef`, styles construits en TypeScript (`ui/src/mapStyles.ts`).
- deck.gl n'est pas installé tant qu'il n'y a pas de résultat SPL à afficher (phase 2).

## Conséquences
- MapLibre v6 + Vite 8 (Rolldown) exige `setWorkerUrl()` avec `maplibre-gl/dist/maplibre-gl-worker.mjs?url` (`ui/src/main.tsx`). Sans cela, les sources GeoJSON et vectorielles ne se chargent jamais (constaté puis corrigé).
- `map.isStyleLoaded()` reste faux tant que des tuiles chargent : on teste la présence des couches avant de les modifier.
