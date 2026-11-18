// Styles MapLibre : exploration (streaming via le proxy Rust) et zone 3D (pack local).
import type { StyleSpecification } from 'maplibre-gl';
import { PROTOCOL_BASE as B, type OpenZoneResponse } from './api';
import { boundsAround, circlePolygon, featureCollection, orthoGroups, squarePolygon } from './geo';
import { ownerFillColor } from './ownerLayer';

export const IGN_ATTRIBUTION = '© IGN – Géoplateforme (Licence Ouverte)';

const EMPTY = featureCollection([]);

export const COLORS = {
  accent: '#ff7a1a',
  residential: '#ff4d2e',
  building: '#e9e4d8',
  lidar: '#3fb6ff',
};

/** Mode Exploration : ortho + Plan IGN + emprises LiDAR HD, aucun relief lourd. */
export function exploreStyle(): StyleSpecification {
  return {
    version: 8,
    sources: {
      ortho: {
        type: 'raster',
        tiles: [`${B}/wmts/ortho/{z}/{x}/{y}`],
        tileSize: 256,
        maxzoom: 19,
        attribution: IGN_ATTRIBUTION,
      },
      plan: {
        type: 'raster',
        tiles: [`${B}/wmts/plan/{z}/{x}/{y}`],
        tileSize: 256,
        maxzoom: 19,
        attribution: IGN_ATTRIBUTION,
      },
      names: {
        type: 'raster',
        tiles: [`${B}/wmts/names/{z}/{x}/{y}`],
        tileSize: 256,
        minzoom: 6,
        maxzoom: 18,
        attribution: IGN_ATTRIBUTION,
      },
      lidar: {
        type: 'vector',
        tiles: [`${B}/tms/lidar-coverage/{z}/{x}/{y}.pbf`],
        minzoom: 10,
        maxzoom: 16,
      },
      // Parcellaire cadastral (couche `parcelle` dès le zoom 15), identifiant `idu`
      // pour les états (catégorie du propriétaire, sélection).
      pci: {
        type: 'vector',
        tiles: [`${B}/tms/pci/{z}/{x}/{y}.pbf`],
        minzoom: 15,
        maxzoom: 16,
        promoteId: { parcelle: 'idu' },
        attribution: 'Cadastre © DGFiP, IGN',
      },
      zone: { type: 'geojson', data: EMPTY },
    },
    layers: [
      { id: 'bg', type: 'background', paint: { 'background-color': '#14161a' } },
      { id: 'ortho', type: 'raster', source: 'ortho' },
      { id: 'plan', type: 'raster', source: 'plan', layout: { visibility: 'none' } },
      {
        id: 'pci-fill',
        type: 'fill',
        source: 'pci',
        'source-layer': 'parcelle',
        minzoom: 15,
        layout: { visibility: 'none' },
        paint: {
          'fill-color': ownerFillColor(),
          'fill-opacity': ['case', ['boolean', ['feature-state', 'sel'], false], 0.7, 0.42],
        },
      },
      {
        id: 'pci-line',
        type: 'line',
        source: 'pci',
        'source-layer': 'parcelle',
        minzoom: 15,
        layout: { visibility: 'none' },
        paint: {
          'line-color': ['case', ['boolean', ['feature-state', 'sel'], false], '#ff7a1a', '#ffffff'],
          'line-width': ['case', ['boolean', ['feature-state', 'sel'], false], 3, 0.7],
          'line-opacity': ['case', ['boolean', ['feature-state', 'sel'], false], 1, 0.55],
        },
      },
      { id: 'names', type: 'raster', source: 'names', minzoom: 6 },
      {
        id: 'lidar-fill',
        type: 'fill',
        source: 'lidar',
        'source-layer': 'nuage_dalle',
        minzoom: 10,
        layout: { visibility: 'none' },
        paint: { 'fill-color': COLORS.lidar, 'fill-opacity': 0.12 },
      },
      {
        id: 'lidar-line',
        type: 'line',
        source: 'lidar',
        'source-layer': 'nuage_dalle',
        minzoom: 10,
        layout: { visibility: 'none' },
        paint: { 'line-color': COLORS.lidar, 'line-width': 0.8, 'line-opacity': 0.7 },
      },
      ...zoneLayers(),
    ],
  };
}

function zoneLayers(): StyleSpecification['layers'] {
  return [
    {
      id: 'zone-fill',
      type: 'fill',
      source: 'zone',
      filter: ['==', ['get', 'kind'], 'disk'],
      paint: { 'fill-color': COLORS.accent, 'fill-opacity': 0.08 },
    },
    {
      id: 'zone-rings',
      type: 'line',
      source: 'zone',
      filter: ['==', ['get', 'kind'], 'ring'],
      paint: { 'line-color': '#ffffff', 'line-width': 1, 'line-opacity': 0.55, 'line-dasharray': [3, 3] },
    },
    {
      id: 'zone-line',
      type: 'line',
      source: 'zone',
      filter: ['==', ['get', 'kind'], 'disk'],
      paint: { 'line-color': COLORS.accent, 'line-width': 2 },
    },
    {
      id: 'zone-center',
      type: 'circle',
      source: 'zone',
      filter: ['==', ['get', 'kind'], 'center'],
      paint: {
        'circle-radius': 7,
        'circle-color': COLORS.accent,
        'circle-stroke-color': '#14161a',
        'circle-stroke-width': 2,
      },
    },
  ];
}

/** Géométries de la zone : disque d'étude, anneaux de résolution, sono. */
export function zoneFeatures(
  lon: number,
  lat: number,
  radius: number,
  rings: { half_size_m: number }[] = [],
): GeoJSON.FeatureCollection {
  return featureCollection([
    { type: 'Feature', properties: { kind: 'disk' }, geometry: circlePolygon(lon, lat, radius) },
    ...rings
      .filter((r) => r.half_size_m < radius)
      .map(
        (r): GeoJSON.Feature => ({
          type: 'Feature',
          properties: { kind: 'ring' },
          geometry: squarePolygon(lon, lat, r.half_size_m),
        }),
      ),
    { type: 'Feature', properties: { kind: 'center' }, geometry: { type: 'Point', coordinates: [lon, lat] } },
  ]);
}

/** Vue 3D d'une zone : exclusivement le pack local (aucun appel réseau). */
export function zoneStyle(z: OpenZoneResponse): StyleSpecification {
  const id = z.summary.id;
  const [lon, lat] = z.products.center_lonlat;
  const R = z.spec.radius_m;
  const zoneBounds = boundsAround(lon, lat, R * 1.02);
  const sources: StyleSpecification['sources'] = {
    terrain: {
      type: 'raster-dem',
      tiles: [`${B}/zone/${id}/terrain/{z}/{x}/{y}.png`],
      tileSize: 256,
      encoding: 'terrarium',
      maxzoom: 17,
      bounds: zoneBounds,
    },
    hillshade: {
      type: 'raster-dem',
      tiles: [`${B}/zone/${id}/terrain/{z}/{x}/{y}.png`],
      tileSize: 256,
      encoding: 'terrarium',
      maxzoom: 16,
      bounds: zoneBounds,
    },
    buildings: { type: 'geojson', data: `${B}/zone/${id}/file/ui/buildings.geojson` },
    zone: { type: 'geojson', data: zoneFeatures(lon, lat, R, z.spec.rings.slice(0, -1)) },
  };
  const layers: StyleSpecification['layers'] = [
    { id: 'bg', type: 'background', paint: { 'background-color': '#14161a' } },
  ];
  orthoGroups(z.spec.ortho).forEach((g, i) => {
    sources[`ortho-${i}`] = {
      type: 'raster',
      tiles: [`${B}/zone/${id}/ortho/{z}/{x}/{y}.jpg`],
      tileSize: 256,
      minzoom: g.minzoom,
      maxzoom: g.maxzoom,
      bounds: boundsAround(lon, lat, Math.min(g.radius_m, R) * 1.02),
      attribution: i === 0 ? IGN_ATTRIBUTION : undefined,
    };
    layers.push({ id: `ortho-${i}`, type: 'raster', source: `ortho-${i}`, minzoom: i === 0 ? 0 : g.minzoom });
  });
  layers.push(
    {
      id: 'hillshade',
      type: 'hillshade',
      source: 'hillshade',
      paint: { 'hillshade-exaggeration': 0.25, 'hillshade-shadow-color': '#1d1a14' },
    },
    {
      id: 'buildings-3d',
      type: 'fill-extrusion',
      source: 'buildings',
      paint: {
        'fill-extrusion-color': ['case', ['get', 'res'], COLORS.residential, COLORS.building],
        'fill-extrusion-height': ['get', 'h'],
        'fill-extrusion-base': 0,
        'fill-extrusion-opacity': 0.92,
      },
    },
    ...zoneLayers().filter((l) => l.id !== 'zone-fill'),
  );
  return {
    version: 8,
    sources,
    layers,
    terrain: { source: 'terrain', exaggeration: 1 },
    sky: {
      'sky-color': '#0f1218',
      'horizon-color': '#2a2f3a',
      'fog-color': '#14161a',
      'sky-horizon-blend': 0.6,
      'horizon-fog-blend': 0.4,
      'fog-ground-blend': 0.85,
    },
  };
}
