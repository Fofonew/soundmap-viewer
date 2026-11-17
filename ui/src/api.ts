// Accès au backend Rust : commandes IPC et URL du protocole soundmap://.
import { Channel, invoke } from '@tauri-apps/api/core';

/** Base du protocole personnalisé (Windows/Android : http://soundmap.localhost). */
export const PROTOCOL_BASE = /Windows|Android/i.test(navigator.userAgent)
  ? 'http://soundmap.localhost'
  : 'soundmap://localhost';

export interface AppInfo {
  version: string;
  zones_dir: string;
  cache_dir: string;
  offline: boolean;
}

export interface GeocodeResult {
  label: string;
  context?: string;
  kind?: string;
  lon: number;
  lat: number;
  x?: number;
  y?: number;
}

export type Category = 'relief' | 'vecteurs' | 'occupation_sol' | 'meteo' | 'ortho';

export interface Estimate {
  categories: { category: Category; items: number; bytes: number }[];
  total_items: number;
  total_bytes: number;
  disk_bytes: number;
  est_seconds: number;
}

export interface Ring {
  half_size_m: number;
  cell_m: number;
}
export interface OrthoLevel {
  zoom: number;
  radius_m: number;
}

export type ZoneStatus = 'partial' | 'downloaded' | 'complete';

export interface EstimateResponse {
  id: string;
  existing: ZoneStatus | null;
  estimate: Estimate;
  rings: Ring[];
  ortho: OrthoLevel[];
}

export interface ZoneRequest {
  name: string;
  lon: number;
  lat: number;
  radius_m: number;
  meteo_date?: string;
}

export type Stage = 'downloading' | 'finalizing' | 'complete' | 'cancelled' | 'failed';

export interface PackProgress {
  stage: Stage;
  done_items: number;
  total_items: number;
  bytes: number;
  est_total_bytes: number;
  failed_items: number;
  elapsed_s: number;
  message: string;
}

export interface ZoneSummary {
  id: string;
  name: string;
  status: ZoneStatus;
  radius_m: number;
  center_l93: [number, number];
  center_lonlat: [number, number] | null;
  done_items: number;
  total_items: number;
  downloaded_bytes: number;
  updated_at: number;
}

export interface ZoneSpec {
  name: string;
  center_x: number;
  center_y: number;
  radius_m: number;
  rings: Ring[];
  ortho: OrthoLevel[];
  meteo_date: string;
}

export interface Products {
  rings: { layer: string; cell_m: number; file: string }[];
  vectors: { name: string; count: number }[];
  buildings: { total: number; residential: number; dwellings: number };
  lonlat_bounds: [number, number, number, number];
  center_lonlat: [number, number];
  ortho_zooms: number[];
  ortho_tiles: number;
  terrain_tiles: number;
  terrain_pregen_max_zoom: number;
  landcover: string;
  meteo: string | null;
  elevation_range: [number, number] | null;
  finalize_seconds: number;
}

export interface OpenZoneResponse {
  summary: ZoneSummary;
  spec: ZoneSpec;
  products: Products;
  open_ms: number;
}

export interface SpotSpec {
  radius_m: number;
  grid_m: number;
  source_dbc_1m: number;
  threshold_dbc: number;
  max_slope_pct: number;
  influence_m: number;
}

export interface Spot {
  lon: number;
  lat: number;
  exposed: number;
  nearest_m: number;
  max_dbc: number;
  slope_pct: number;
  elevation_m: number;
}

export interface SpotReport {
  spots: Spot[];
  candidates: Spot[];
  populated_cells: number;
  population: number;
  seconds: number;
}

export interface SpotProgress {
  stage: string;
  done: number;
  total: number;
  message: string;
}

export const api = {
  findSpots: (lon: number, lat: number, spec: SpotSpec, onProgress: (p: SpotProgress) => void) => {
    const ch = new Channel<SpotProgress>();
    ch.onmessage = onProgress;
    return invoke<SpotReport>('find_spots', { lon, lat, spec, onProgress: ch });
  },
  cancelJob: (kind: string) => invoke<boolean>('cancel_job', { kind }),
  appInfo: () => invoke<AppInfo>('app_info'),
  geocode: (q: string) => invoke<GeocodeResult[]>('geocode', { q }),
  estimateZone: (req: ZoneRequest) => invoke<EstimateResponse>('estimate_zone', { req }),
  downloadZone: (req: ZoneRequest, onProgress: (p: PackProgress) => void) => {
    const ch = new Channel<PackProgress>();
    ch.onmessage = onProgress;
    return invoke<ZoneSummary>('download_zone', { req, onProgress: ch });
  },
  resumeZone: (id: string, onProgress: (p: PackProgress) => void) => {
    const ch = new Channel<PackProgress>();
    ch.onmessage = onProgress;
    return invoke<ZoneSummary>('resume_zone', { id, onProgress: ch });
  },
  cancelDownload: (id: string) => invoke<boolean>('cancel_download', { id }),
  listZones: () => invoke<ZoneSummary[]>('list_zones'),
  openZone: (id: string) => invoke<OpenZoneResponse>('open_zone', { id }),
  deleteZone: (id: string) => invoke<void>('delete_zone', { id }),
  logMetric: (name: string, ms: number) => invoke<void>('log_metric', { name, ms }),
};

export function fmtBytes(b: number): string {
  if (b >= 1e9) return `${(b / 1e9).toFixed(2)} Go`;
  if (b >= 1e6) return `${(b / 1e6).toFixed(b >= 1e8 ? 0 : 1)} Mo`;
  return `${Math.max(1, Math.round(b / 1e3))} ko`;
}

export function fmtDuration(s: number): string {
  if (s < 60) return `${Math.round(s)} s`;
  const m = Math.floor(s / 60);
  const r = Math.round(s % 60);
  return r ? `${m} min ${r} s` : `${m} min`;
}
