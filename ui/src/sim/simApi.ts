// Types et appels de la simulation (miroir des structures Rust de sm-compute).
import { t } from '../i18n';
import { Channel, invoke } from '@tauri-apps/api/core';

export type SpeakerKind = 'sub' | 'top' | 'line_array';

export type Directivity =
  | { type: 'omni' }
  | { type: 'horn'; h_cov_deg: number; v_cov_deg: number; f_control: number; radius_m: number }
  | { type: 'line'; n: number; spacing_m: number; h_cov_deg: number }
  | { type: 'cardioid'; rear_rejection_db: number };

export interface Dims {
  width_m: number;
  height_m: number;
  depth_m: number;
}

export interface SpeakerModel {
  id: string;
  name: string;
  kind: SpeakerKind;
  max_spl_1m: number;
  f_lo: number;
  f_hi: number;
  rolloff_db_oct: number;
  directivity: Directivity;
  power_w: number;
  /** Dimensions pour le dessin 3D (null : valeurs types). */
  dims?: Dims | null;
  /** Réponse mesurée [Hz, dB relatifs], remplace f_lo/f_hi si ≥ 2 points. */
  response?: [number, number][];
  notes?: string;
  custom?: boolean;
}

/** Dimensions de dessin (valeurs types si absentes, comme sm-core). */
export function dimsOf(m: SpeakerModel): Dims {
  if (m.dims) return m.dims;
  if (m.directivity.type === 'line') return { width_m: 1, height_m: m.directivity.n * m.directivity.spacing_m, depth_m: 0.5 };
  if (m.kind === 'sub') return { width_m: 0.6, height_m: 0.65, depth_m: 0.8 };
  return { width_m: 0.45, height_m: 0.75, depth_m: 0.45 };
}

export interface Program {
  id: string;
  name: string;
  octave_db: number[];
}

export interface SpeakerSpec {
  id: string;
  name: string;
  model: SpeakerModel;
  lon: number;
  lat: number;
  height_m: number;
  azimuth_deg: number;
  tilt_deg: number;
  delay_ms: number;
  polarity: number;
  gain_db: number;
  muted: boolean;
}

export interface Settings {
  program: string;
  master_gain_db: number;
  receiver_height_m: number;
  condition: 'homogeneous' | 'favorable';
  impedance_model: 'DelanyBazley' | 'Miki';
  meteo_hour: number;
  thresholds_db: number[];
  residual: { rural_night_dba: number; include_roads: boolean; measured_dba: number | null };
  target: { dba: number; dbc: number };
  calibration?: Calibration | null;
  use_ground_state?: boolean;
}

export interface Refraction {
  grad_t: number;
  grad_w: number;
  wind_toward_deg: number;
}

export interface Calibration {
  sigma_scale: number;
  refraction: Refraction;
  offset_db: number;
  rms_db: number;
  n_points: number;
}

export interface ParamPosterior {
  mean: number;
  sd: number;
  map: number;
}

export interface PointFit {
  lon: number;
  lat: number;
  label: string | null;
  distance_m: number;
  la: number | null;
  lc: number | null;
  model_la_before: number;
  model_lc_before: number;
  model_la_after: number;
  model_lc_after: number;
}

export interface CalibrationReport {
  n_points: number;
  n_ignored: number;
  rms_before_db: number;
  rms_after_db: number;
  bias_before_db: number;
  sigma_scale: ParamPosterior;
  grad_t: ParamPosterior;
  grad_w: ParamPosterior;
  offset_db: ParamPosterior;
  wind_toward_deg: number;
  calibration: Calibration;
  points: PointFit[];
  notes: string[];
}

export interface Project {
  version: number;
  name: string;
  zone_id: string;
  speakers: SpeakerSpec[];
  dancefloor: [number, number][] | null;
  settings: Settings;
}

export interface Catalog {
  speakers: SpeakerModel[];
  programs: Program[];
}

export interface SceneInfo {
  buildings: number;
  residential: number;
  inhabitants: number;
  sensitive: number;
  znieff: number;
  landcover: string;
  meteo_hours: string[];
  load_ms: number;
  vegetation: VegetationSummary | null;
  ground: GroundAdjust;
  ground_hours: GroundHour[] | null;
  valley: { tpi_m: number; slope_deg: number; downslope_toward_deg: number; valley: boolean };
}

export interface GroundAdjust {
  cliffs_km2: number;
  hardened_km2: number;
  formations: [string, 'hard' | 'soft' | 'mixed'][] | null;
}

export interface GroundHour {
  hour: number;
  state: { moisture: number | null; snow_m: number | null; rain_24h_mm: number };
  sigma_factor: number;
  snow: boolean;
}

export interface Auralization {
  dancefloor_wav: string;
  point_wav: string;
  dancefloor_dba: number;
  point_dba: number;
  point_dbc: number;
  residual_dba: number;
  level_drop_dba: number;
  distance_m: number;
}

export interface VegetationSummary {
  points: number;
  tiles: number;
  radius_m: number;
  covered_km2: number;
  dense_foliage_km2: number;
  mean_foliage: number;
  mean_roughness_m: number;
  downloaded_mb: number;
}

export interface Timing {
  profiles_ms: number;
  excess_ms: number;
  combine_ms: number;
  cached_profiles: boolean;
  cached_excess: boolean;
}

export interface PreviewResult {
  result_id: string;
  timing: Timing;
  total_ms: number;
  method: string;
  air: { t_c: number; rh: number };
}

export interface Layer {
  corners: [number, number][];
  isophones: GeoJSON.FeatureCollection;
  stats: { max_db: number; area_above: [number, number][] };
  palette_offset: number;
  variants: string[];
  method: string;
}

export interface PathInfo {
  distance: number;
  delta: number;
  n_edges: number;
  foliage_m: number;
  sigma_kpa: number;
}

export interface Section {
  samples: [number, number, number, number][];
  source_z: number;
  receiver_z: number;
  distance: number;
  info: PathInfo;
  bands: [number, number][];
  dba: number;
  dbc: number;
}

export interface JobProgress {
  stage: string;
  done: number;
  total: number;
  message: string;
}

export interface ScenarioInfo {
  id: string;
  label: string;
  max_dbc: number;
  area_55_dbc_km2: number;
  u_star: number;
  inv_l: number;
  seconds: number;
}

export interface LowFreqResult {
  result_id: string;
  scenarios: ScenarioInfo[];
  total_s: number;
  method: string;
}

export interface BuildingImpact {
  id: string;
  lon: number;
  lat: number;
  usage: string;
  inhabitants: number;
  /** Hors carreau Filosofi peuplé : 2,2 habitants par logement. */
  inhabitants_estimated?: boolean;
  dwellings: number;
  distance_m: number;
  level_dba: number;
  level_dbc: number;
  height_m: number;
  residual_dba: number;
  residual_dbc: number;
  emergence_dba: number;
  emergence_dbc: number;
  emergence_oct: number[];
  class: number;
  margin_db: number;
}

export interface ImpactReport {
  thresholds_db: number[];
  buildings: BuildingImpact[];
  classes: { label: string; buildings: number; inhabitants: number }[];
  most_exposed: BuildingImpact | null;
  margin_db: number;
  inhabitants_total: number;
  sensitive: { name: string; kind: string; lon: number; lat: number; level_dba: number; emergence_dba: number }[];
  fauna: { name: string; level: number; area_above_45_km2: number; max_dba: number }[];
  method: string;
  compute_ms: number;
}

export interface SizingReport {
  target_dba: number;
  target_dbc: number;
  center_dba: number;
  center_dbc: number;
  max_dba: number;
  max_dbc: number;
  subs_now: number;
  tops_now: number;
  subs_needed: number;
  tops_needed: number;
  power_w: number;
  headroom_dba: number;
  headroom_dbc: number;
  uniformity_std_db: number;
  uniformity_range_db: number;
  samples: number;
  notes: string[];
}

export type SubConfig = 'as_is' | 'cardioid' | 'end_fire';

export interface Candidate {
  dx: number;
  dy: number;
  rotation_deg: number;
  azimuth_deg: number;
  config: SubConfig;
  dancefloor_dba: number;
  dancefloor_dbc: number;
  worst_emergence_dba: number;
  margin_db: number;
  score: number;
  speakers: SpeakerSpec[];
  gfpe_worst_emergence_dba: number | null;
  gfpe_margin_db: number | null;
}

const withProgress = <T,>(cmd: string, args: Record<string, unknown>, onProgress: (p: JobProgress) => void) => {
  const ch = new Channel<JobProgress>();
  ch.onmessage = onProgress;
  return invoke<T>(cmd, { ...args, onProgress: ch });
};

export const simApi = {
  catalog: () => invoke<Catalog>('catalog'),
  saveModel: (model: SpeakerModel) => invoke<void>('save_speaker_model', { model }),
  deleteModel: (id: string) => invoke<void>('delete_speaker_model', { id }),
  loadScene: (zoneId: string) => invoke<SceneInfo>('load_scene', { zoneId }),
  preview: (project: Project) => invoke<PreviewResult>('compute_preview', { project }),
  valueAt: (resultId: string, variant: string, metric: string, lon: number, lat: number) =>
    invoke<number | null>('result_value_at', { resultId, variant, metric, lon, lat }),
  layer: (resultId: string, variant: string, metric: string) =>
    invoke<Layer>('result_layer', { resultId, variant, metric }),
  section: (project: Project, resultId: string | null, lon: number, lat: number) =>
    invoke<Section>('section_at', { project, resultId, lon, lat }),
  lowfreq: (project: Project, onProgress: (p: JobProgress) => void) =>
    withProgress<LowFreqResult>('run_lowfreq', { project }, onProgress),
  cancel: (kind: string) => invoke<boolean>('cancel_job', { kind }),
  impact: (project: Project, resultId: string | null) => invoke<ImpactReport>('compute_impact', { project, resultId }),
  sizing: (project: Project) => invoke<SizingReport | null>('compute_sizing', { project }),
  calibrate: (project: Project, path: string) => invoke<CalibrationReport>('calibrate_measurements', { project, path }),
  fetchGround: (zoneId: string) => invoke<void>('fetch_ground_data', { zoneId }),
  auralize: (project: Project, resultId: string | null, lon: number, lat: number) =>
    invoke<Auralization>('auralize_point', { project, resultId, lon, lat }),
  fetchVegetation: (zoneId: string, radiusM: number, cb: (p: JobProgress) => void) =>
    withProgress<VegetationSummary>('fetch_vegetation', { zoneId, radiusM }, cb),
  optimize: (project: Project, onProgress: (p: JobProgress) => void) =>
    withProgress<{ candidates: Candidate[]; seconds: number }>('run_optimizer', { project, spec: null }, onProgress),
  saveProject: (path: string, project: Project) => invoke<void>('save_project', { path, project }),
  loadProject: (path: string) => invoke<Project>('load_project', { path }),
  autosave: (project: Project) => invoke<void>('autosave_project', { project }),
  autoload: (zoneId: string) => invoke<Project | null>('autoload_project', { zoneId }),
  exportResult: (args: {
    kind: 'geotiff' | 'geojson' | 'html';
    path: string;
    project: Project;
    resultId: string;
    variant: string;
    metric: string;
    impact: ImpactReport | null;
    sizing: SizingReport | null;
  }) => invoke<void>('export_result', args),
};

export function defaultSettings(): Settings {
  return {
    program: 'tekno',
    master_gain_db: 0,
    receiver_height_m: 1.5,
    condition: 'favorable',
    impedance_model: 'DelanyBazley',
    meteo_hour: 23,
    thresholds_db: [3, 5, 10],
    residual: { rural_night_dba: 25, include_roads: true, measured_dba: null },
    target: { dba: 100, dbc: 115 },
  };
}

/** Déplacement local (m) → (lon, lat). */
export function offsetLonLat(lon: number, lat: number, dx: number, dy: number): [number, number] {
  const k = 111_320;
  return [lon + dx / (k * Math.cos((lat * Math.PI) / 180)), lat + dy / k];
}

let uid = 0;
export function newId(prefix: string): string {
  uid += 1;
  return `${prefix}-${Date.now().toString(36)}-${uid}`;
}

/** Système type free party : 4 subs 18" en ligne et 2 têtes, orienté selon `az`. */
export function freePartySystem(catalog: Catalog, lon: number, lat: number, az: number): SpeakerSpec[] {
  const sub = catalog.speakers.find((s) => s.id === 'sub18');
  const top = catalog.speakers.find((s) => s.id === 'top15');
  if (!sub || !top) return [];
  const a = (az * Math.PI) / 180;
  const ax = Math.sin(a);
  const ay = Math.cos(a);
  const out: SpeakerSpec[] = [];
  for (let i = 0; i < 4; i++) {
    const off = (i - 1.5) * 0.8;
    const [lo, la] = offsetLonLat(lon, lat, ay * off, -ax * off);
    out.push({ id: newId('sub'), name: `${t('sim.kind.sub')} ${i + 1}`, model: sub, lon: lo, lat: la, height_m: 0.5, azimuth_deg: az, tilt_deg: 0, delay_ms: 0, polarity: 1, gain_db: 0, muted: false });
  }
  [-3, 3].forEach((side, i) => {
    const [lo, la] = offsetLonLat(lon, lat, ay * side + ax * 0.5, -ax * side + ay * 0.5);
    out.push({ id: newId('top'), name: `${t('sim.kind.top')} ${i + 1}`, model: top, lon: lo, lat: la, height_m: 2.5, azimuth_deg: az, tilt_deg: 0, delay_ms: 0, polarity: 1, gain_db: 0, muted: false });
  });
  return out;
}

export const METRICS: { key: string; label: string }[] = [
  { key: 'A', label: 'dB(A)' },
  { key: 'C', label: 'dB(C)' },
  { key: 'Z', label: 'dB(Z)' },
  { key: 'oct31', label: '31.5 Hz' },
  { key: 'oct63', label: '63 Hz' },
  { key: 'oct125', label: '125 Hz' },
  { key: 'oct250', label: '250 Hz' },
  { key: 'oct500', label: '500 Hz' },
  { key: 'oct1000', label: '1 kHz' },
  { key: 'oct2000', label: '2 kHz' },
  { key: 'oct4000', label: '4 kHz' },
  { key: 'oct8000', label: '8 kHz' },
];

/** Palette de 5 dB (identique à sm-compute::render::PALETTE). */
export const PALETTE: [number, string][] = [
  [30, '#d4f2d4'],
  [35, '#9bd99b'],
  [40, '#57b45a'],
  [45, '#1f8a3b'],
  [50, '#f3e45a'],
  [55, '#f5b342'],
  [60, '#ef7d2d'],
  [65, '#e4482c'],
  [70, '#c41f3a'],
  [75, '#9b1446'],
  [80, '#6c0f4f'],
  [85, '#46104f'],
  [90, '#2c0a3d'],
  [95, '#1a0526'],
];
