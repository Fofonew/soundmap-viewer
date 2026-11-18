// Conversions fiche technique → modèle SoundMap, et géométrie des enceintes.

/** Ce qu'annonce la fiche technique pour le SPL max. */
export interface DatasheetSpl {
  /** Valeur lue sur la fiche (dB). */
  spl_db: number;
  /** « Peak » (crête) : le régime continu est ~6 dB plus bas (facteur de crête du bruit rose). */
  peak: boolean;
  /** Mesuré ou calculé en demi-espace (posé au sol) : −6 dB pour le champ libre. */
  half_space: boolean;
  f_lo: number;
  f_hi: number;
}

/**
 * SPL max continu, champ libre, **par bande d'octave** (ce qu'attend le modèle) :
 * L_oct = L − 6·[crête] − 6·[demi-espace] − 10·lg(n), n = nombre d'octaves de la
 * bande passante (≥ 1), car la fiche donne un niveau large bande.
 */
export function octaveSplFromDatasheet(d: DatasheetSpl): number {
  const octaves = Math.max(1, Math.log2(Math.max(d.f_hi, d.f_lo * 1.0001) / d.f_lo));
  return d.spl_db - (d.peak ? 6 : 0) - (d.half_space ? 6 : 0) - 10 * Math.log10(octaves);
}

/** SPL max thermique (large bande) à partir de la sensibilité 1 W / 1 m et de la puissance. */
export function thermalSpl(sensitivity_db: number, power_w: number): number {
  return sensitivity_db + 10 * Math.log10(Math.max(power_w, 1e-3));
}

/**
 * Fréquence de coupure à −3 dB à partir d'une coupure donnée à −10 dB (fiches
 * « −10 dB »), avec la pente du modèle (dB/octave).
 */
export function cutoffMinus3(f: number, atDb: 3 | 10, slope: number, side: 'lo' | 'hi'): number {
  if (atDb === 3) return f;
  const oct = 7 / Math.max(slope, 1);
  return side === 'lo' ? f * 2 ** oct : f / 2 ** oct;
}

/** Lit « Hz dB » par ligne (séparateurs espace, tabulation, `;` ; virgule décimale acceptée). */
export function parseResponse(text: string): [number, number][] {
  const out: [number, number][] = [];
  for (const line of text.split(/\r?\n/)) {
    const parts = line
      .trim()
      .split(/[\s;]+/)
      .map((s) => Number(s.replace(',', '.')));
    if (parts.length >= 2 && parts[0] > 0 && Number.isFinite(parts[0]) && Number.isFinite(parts[1])) out.push([parts[0], parts[1]]);
  }
  return out.sort((a, b) => a[0] - b[0]);
}

/**
 * Empreinte au sol d'une boîte (L × P) centrée sur (lon, lat), tournée selon
 * l'azimut (face avant vers l'azimut).
 */
export function boxFootprint(lon: number, lat: number, width_m: number, depth_m: number, azimuth_deg: number): [number, number][] {
  const a = (azimuth_deg * Math.PI) / 180;
  const fwd = [Math.sin(a), Math.cos(a)];
  const right = [Math.cos(a), -Math.sin(a)];
  const k = 111_320;
  const kx = k * Math.cos((lat * Math.PI) / 180);
  const pts: [number, number][] = [
    [-1, -1],
    [1, -1],
    [1, 1],
    [-1, 1],
    [-1, -1],
  ].map(([r, f]) => {
    const dx = (r * width_m) / 2 * right[0] + (f * depth_m) / 2 * fwd[0];
    const dy = (r * width_m) / 2 * right[1] + (f * depth_m) / 2 * fwd[1];
    return [lon + dx / kx, lat + dy / k];
  });
  return pts;
}

/** Rotation de points (lon, lat) autour d'un centre, angle en degrés (sens horaire). */
export function rotateAround(lon: number, lat: number, clon: number, clat: number, deg: number): [number, number] {
  const k = 111_320;
  const kx = k * Math.cos((clat * Math.PI) / 180);
  const r = (deg * Math.PI) / 180;
  const x = (lon - clon) * kx;
  const y = (lat - clat) * k;
  const xr = x * Math.cos(r) + y * Math.sin(r);
  const yr = -x * Math.sin(r) + y * Math.cos(r);
  return [clon + xr / kx, clat + yr / k];
}

/** Azimut (°, 0 = nord, horaire) du point b vu depuis a. */
export function bearing(a: [number, number], b: [number, number]): number {
  const k = 111_320;
  const kx = k * Math.cos((a[1] * Math.PI) / 180);
  const dx = (b[0] - a[0]) * kx;
  const dy = (b[1] - a[1]) * k;
  return ((Math.atan2(dx, dy) * 180) / Math.PI + 360) % 360;
}
