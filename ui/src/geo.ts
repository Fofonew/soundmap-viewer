// Petits utilitaires géographiques côté UI (WGS84 uniquement).

const M_PER_DEG_LAT = 111_320;

/** Polygone approché d'un cercle de rayon `r` mètres (sphère locale). */
export function circlePolygon(lon: number, lat: number, r: number, steps = 128): GeoJSON.Polygon {
  const coords: [number, number][] = [];
  const kLon = M_PER_DEG_LAT * Math.cos((lat * Math.PI) / 180);
  for (let i = 0; i <= steps; i++) {
    const a = (i / steps) * 2 * Math.PI;
    coords.push([lon + (r * Math.cos(a)) / kLon, lat + (r * Math.sin(a)) / M_PER_DEG_LAT]);
  }
  return { type: 'Polygon', coordinates: [coords] };
}

/** Carré de demi-côté `half` mètres autour d'un point. */
export function squarePolygon(lon: number, lat: number, half: number): GeoJSON.Polygon {
  const [w, s, e, n] = boundsAround(lon, lat, half);
  return { type: 'Polygon', coordinates: [[[w, s], [e, s], [e, n], [w, n], [w, s]]] };
}

/** [ouest, sud, est, nord] d'un carré de demi-côté `half` mètres. */
export function boundsAround(lon: number, lat: number, half: number): [number, number, number, number] {
  const dLat = half / M_PER_DEG_LAT;
  const dLon = half / (M_PER_DEG_LAT * Math.cos((lat * Math.PI) / 180));
  return [lon - dLon, lat - dLat, lon + dLon, lat + dLat];
}

export function featureCollection(features: GeoJSON.Feature[]): GeoJSON.FeatureCollection {
  return { type: 'FeatureCollection', features };
}

/** Regroupe les niveaux ortho de même rayon en plages de zoom contiguës. */
export function orthoGroups(levels: { zoom: number; radius_m: number }[]) {
  const sorted = [...levels].sort((a, b) => a.zoom - b.zoom);
  const groups: { minzoom: number; maxzoom: number; radius_m: number }[] = [];
  for (const l of sorted) {
    const g = groups[groups.length - 1];
    if (g && g.radius_m === l.radius_m && g.maxzoom === l.zoom - 1) g.maxzoom = l.zoom;
    else groups.push({ minzoom: l.zoom, maxzoom: l.zoom, radius_m: l.radius_m });
  }
  return groups;
}
