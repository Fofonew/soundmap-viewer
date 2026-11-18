// Accès au backend Rust : commandes IPC et URL du protocole soundmap://.
import { Channel, invoke } from '@tauri-apps/api/core';
import { num, t } from './i18n';

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
  /** Vérifier clairière et accès carrossable (BD TOPO). */
  check_site?: boolean;
  site?: SiteSpec;
}

/** Critères du site (sm_compute::site::SiteSpec). */
export interface SiteSpec {
  clear_radius_m: number;
  max_access_m: number;
  search_m: number;
  access_route: boolean;
  access_piste: boolean;
  access_chemin: boolean;
  allow_restricted: boolean;
  block_crops: boolean;
  block_scrub: boolean;
  block_open_forest: boolean;
}

export const DEFAULT_SITE: SiteSpec = {
  clear_radius_m: 50,
  max_access_m: 300,
  search_m: 250,
  access_route: true,
  access_piste: true,
  access_chemin: true,
  allow_restricted: true,
  block_crops: true,
  block_scrub: true,
  block_open_forest: true,
};

export type AccessKind = 'route' | 'piste' | 'chemin';

export interface Site {
  x: number;
  y: number;
  shift_m: number;
  access: AccessKind;
  access_restricted: boolean;
  access_m: number;
  wooded_pct: number;
}

export interface Spot {
  lon: number;
  lat: number;
  exposed: number;
  nearest_m: number;
  max_dbc: number;
  slope_pct: number;
  elevation_m: number;
  site?: Site;
}

export interface SpotReport {
  spots: Spot[];
  candidates: Spot[];
  populated_cells: number;
  population: number;
  seconds: number;
  site_checked: number;
  rejected_wooded: number;
  rejected_access: number;
}

export interface SpotProgress {
  stage: string;
  done: number;
  total: number;
  message: string;
}

export interface PoliceNear {
  name: string;
  nature: string;
  lon: number;
  lat: number;
  distance_m: number;
}

export type OwnerCategory = 'commune' | 'section' | 'etat' | 'public' | 'agricole' | 'forestier' | 'societe';

/** Attributs d'une parcelle de tuile PCI envoyés pour la jointure DGFiP. */
export interface ParcelRef {
  idu: string;
  code_dep: string;
  code_com: string;
  com_abs: string;
  section: string;
  numero: string;
}

export interface ParcelOwner {
  idu: string;
  category: OwnerCategory;
  owner_id: string;
  name: string;
  forme: string;
  droit: string;
}

export interface OwnerRight {
  droit: string;
  denomination: string;
  forme: string;
  siren: string;
  culture: string;
}

export interface ParcelInfo {
  parcel: {
    idu: string;
    commune: string;
    code_insee: string;
    section: string;
    numero: string;
    contenance_m2: number;
    ring: [number, number][];
  } | null;
  owners: OwnerRight[];
  owners_error: string | null;
  owners_year: number;
  public_forest: string | null;
  cadastre_url: string | null;
}

export const api = {
  parcelInfo: (lon: number, lat: number) => invoke<ParcelInfo>('parcel_info', { lon, lat }),
  parcelOwners: (parcels: ParcelRef[]) => invoke<ParcelOwner[]>('parcel_owners', { parcels }),
  nearbyPolice: (lon: number, lat: number, radiusM: number) => invoke<PoliceNear[]>('nearby_police', { lon, lat, radiusM }),
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
  if (b >= 1e9) return `${num(b / 1e9, 2)} ${t('units.gb')}`;
  if (b >= 1e6) return `${num(b / 1e6, b >= 1e8 ? 0 : 1)} ${t('units.mb')}`;
  return `${Math.max(1, Math.round(b / 1e3))} ${t('units.kb')}`;
}

export function fmtDuration(s: number): string {
  if (s < 60) return `${Math.round(s)} s`;
  const m = Math.floor(s / 60);
  const r = Math.round(s % 60);
  return r ? `${m} min ${r} s` : `${m} min`;
}
