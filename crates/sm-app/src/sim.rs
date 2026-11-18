//! Commandes de simulation : aperçu, basses fréquences (GFPE), coupe,
//! impact, dimensionnement, optimiseur, projets et exports.

use crate::state::{AppState, lock_err};
use serde::{Deserialize, Serialize};
use sm_compute::impact::{self, ImpactReport};
use sm_compute::meteo::{Scenario, default_scenarios, forecast_profile};
use sm_compute::optimize::{Candidate, OptimizeSpec, optimize, position_count};
use sm_compute::pe::{PeField, envelope, run_pe, task_count};
use sm_compute::polar::{Air, Engine, ExcessKey, LevelGrid, PolarSpec, combine, compute_excess};
use sm_compute::project::{Project, group_heights};
use sm_compute::receiver::{Section, levels_at, section};
use sm_compute::render::{
    Metric, Raster, RasterStats, corners_lonlat, isophones, polar_weighted, rasterize, stats,
    to_png, weighted_level,
};
use sm_compute::scene::Scene;
use sm_compute::sizing::{self, SizingReport};
use sm_compute::{CancelFlag, ProgressCounter, export};
use sm_core::bands::BandSet;
use sm_core::engineering::{BandSpec, Condition, PathConfig};
use sm_core::source::{Program, SpeakerModel, presets, program, programs};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::State;
use tauri::ipc::Channel;

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Simulation attachée à une zone ouverte.
pub struct Sim {
    pub scene: Arc<Scene>,
    pub engine: Mutex<Engine>,
}

/// Jeu de résultats affichables (aperçu ou calcul basses fréquences).
pub struct ResultSet {
    pub zone_id: String,
    /// Variantes : "main" (aperçu ou médiane), "p90", et un id par scénario.
    pub grids: HashMap<String, LevelGrid>,
    pub rasters: Mutex<HashMap<(String, String), Arc<Raster>>>,
    /// Champ GFPE du scénario « prévision » (pour l'impact et la coupe).
    pub pe: Option<PeField>,
    pub method: String,
}

#[derive(Default)]
pub struct SimStore {
    pub sims: Mutex<HashMap<String, Arc<Sim>>>,
    pub results: Mutex<HashMap<String, Arc<ResultSet>>>,
    pub jobs: Mutex<HashMap<String, CancelFlag>>,
    pub counter: std::sync::atomic::AtomicU64,
}

impl SimStore {
    fn next_id(&self) -> String {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("r{n}")
    }
}

fn sim(state: &AppState, zone_id: &str) -> anyhow::Result<Arc<Sim>> {
    if let Some(s) = state.sim.sims.lock().map_err(lock_err)?.get(zone_id) {
        return Ok(s.clone());
    }
    let scene = Scene::load(&state.zone_dir(zone_id)?)?;
    let s = Arc::new(Sim {
        scene: Arc::new(scene),
        engine: Mutex::new(Engine::default()),
    });
    state
        .sim
        .sims
        .lock()
        .map_err(lock_err)?
        .insert(zone_id.to_owned(), s.clone());
    Ok(s)
}

fn octaves() -> Vec<BandSpec> {
    BandSet::Octave
        .bands()
        .into_iter()
        .map(BandSpec::from_band)
        .collect()
}

fn thirds() -> Vec<BandSpec> {
    BandSet::ThirdOctave
        .bands()
        .into_iter()
        .map(BandSpec::from_band)
        .collect()
}

/// Air pour l'absorption : prévision à l'heure choisie, sinon 10 °C / 80 %.
fn air_for(scene: &Scene, p: &Project) -> Air {
    forecast_profile(scene, p.settings.meteo_hour)
        .map(|m| Air {
            t_c: m.ground_temperature(),
            rh: m.rh,
        })
        .unwrap_or(Air {
            t_c: 10.0,
            rh: 80.0,
        })
}

fn path_cfg(p: &Project, scene: &Scene) -> PathConfig {
    p.path_config(scene)
}

#[derive(Serialize)]
pub struct Catalog {
    speakers: Vec<SpeakerModel>,
    programs: Vec<Program>,
}

/// Bibliothèque « Mes enceintes » (JSON dans le dossier de données).
fn custom_speakers_path(state: &AppState) -> std::path::PathBuf {
    state.data_dir.join("speakers.json")
}

fn load_custom_speakers(state: &AppState) -> Vec<SpeakerModel> {
    std::fs::read(custom_speakers_path(state))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn store_custom_speakers(state: &AppState, list: &[SpeakerModel]) -> anyhow::Result<()> {
    let p = custom_speakers_path(state);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("json.part");
    std::fs::write(&tmp, serde_json::to_vec_pretty(list)?)?;
    std::fs::rename(tmp, p)?;
    Ok(())
}

#[tauri::command]
pub fn catalog(state: State<'_, AppState>) -> Catalog {
    let mut speakers = presets();
    speakers.extend(load_custom_speakers(&state));
    Catalog {
        speakers,
        programs: programs(),
    }
}

/// Ajoute ou remplace (même `id`) un modèle de la bibliothèque.
#[tauri::command]
pub fn save_speaker_model(state: State<'_, AppState>, mut model: SpeakerModel) -> CmdResult<()> {
    if presets().iter().any(|p| p.id == model.id) {
        return Err("identifiant réservé à un preset".into());
    }
    if !(model.max_spl_1m.is_finite() && model.f_lo > 0.0 && model.f_hi > model.f_lo) {
        return Err("SPL max, f basse et f haute invalides".into());
    }
    model.custom = true;
    model
        .response
        .retain(|p| p[0] > 0.0 && p[0].is_finite() && p[1].is_finite());
    model.response.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut list = load_custom_speakers(&state);
    match list.iter_mut().find(|m| m.id == model.id) {
        Some(m) => *m = model,
        None => list.push(model),
    }
    store_custom_speakers(&state, &list).map_err(err)
}

#[tauri::command]
pub fn delete_speaker_model(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let mut list = load_custom_speakers(&state);
    list.retain(|m| m.id != id);
    store_custom_speakers(&state, &list).map_err(err)
}

#[derive(Serialize)]
pub struct SceneInfo {
    buildings: usize,
    residential: usize,
    inhabitants: f64,
    sensitive: usize,
    znieff: usize,
    landcover: String,
    meteo_hours: Vec<String>,
    load_ms: f64,
    vegetation: Option<sm_compute::vegetation::VegetationSummary>,
    ground: sm_compute::ground_state::GroundAdjust,
    ground_hours: Option<Vec<sm_compute::ground_state::GroundHour>>,
    valley: sm_compute::valley::ValleyAnalysis,
}

#[tauri::command]
pub async fn load_scene(app: tauri::AppHandle, zone_id: String) -> CmdResult<SceneInfo> {
    use tauri::Manager;
    let t0 = Instant::now();
    let s = tauri::async_runtime::spawn_blocking(move || sim(&app.state::<AppState>(), &zone_id))
        .await
        .map_err(err)?
        .map_err(err)?;
    let sc = &s.scene;
    Ok(SceneInfo {
        buildings: sc.buildings.len(),
        residential: sc.buildings.iter().filter(|b| b.residential).count(),
        inhabitants: sc.buildings.iter().map(|b| b.inhabitants).sum(),
        sensitive: sc.sensitive.len(),
        znieff: sc.znieff.len(),
        landcover: sc.landcover_source.clone(),
        meteo_hours: sc
            .meteo
            .as_ref()
            .map(|m| m.times.clone())
            .unwrap_or_default(),
        load_ms: t0.elapsed().as_secs_f64() * 1e3,
        vegetation: sc.vegetation.as_ref().map(|v| v.summary()),
        ground: sc.ground_adjust.clone(),
        ground_hours: sm_compute::ground_state::hours(sc),
        valley: sm_compute::valley::analyze(sc),
    })
}

/// Auralisation : extrait sonore au dancefloor et au point cliqué.
#[tauri::command]
pub async fn auralize_point(
    state: State<'_, AppState>,
    project: Project,
    result_id: Option<String>,
    lon: f64,
    lat: f64,
) -> CmdResult<sm_compute::auralize::Auralization> {
    let s = sim(&state, &project.zone_id).map_err(err)?;
    let rs = result_id.as_ref().and_then(|id| result(&state, id).ok());
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
        let sc = &s.scene;
        let pe = rs.as_ref().and_then(|r| r.pe.clone());
        let bands = if pe.is_some() { thirds() } else { octaves() };
        let sources = project.sources(sc);
        anyhow::ensure!(!sources.is_empty(), "aucune enceinte active");
        let prog = program(&project.settings.program);
        let cfg = path_cfg(&project, &s.scene);
        let air = air_for(sc, &project);
        let (x, y) = sc
            .lonlat_to_local(lon, lat)
            .ok_or_else(|| anyhow::anyhow!("point hors projection"))?;
        // Référence : centre du dancefloor, sinon 10 m devant les enceintes.
        let (dx, dy) = match project.dancefloor_local(sc) {
            Some(poly) if !poly.is_empty() => {
                let n = poly.len() as f64;
                poly.iter()
                    .fold((0.0, 0.0), |a, p| (a.0 + p.0 / n, a.1 + p.1 / n))
            }
            _ => {
                let (cx, cy) = sm_compute::project::centroid(&sources).unwrap_or((0.0, 0.0));
                let az = sources[0].azimuth_deg.to_radians();
                (cx + 10.0 * az.sin(), cy + 10.0 * az.cos())
            }
        };
        let mut scratch = sm_core::engineering::Scratch::default();
        let mut at = |x: f64, y: f64, hr: f64| {
            let d = x.hypot(y);
            levels_at(
                sc,
                &sources,
                &prog,
                &bands,
                &cfg,
                air,
                x,
                y,
                hr,
                (d / 500.0).max(2.0),
                pe.as_ref(),
                &mut scratch,
            )
            .0
        };
        let l_df = at(dx, dy, 1.5);
        let l_pt = at(x, y, project.settings.receiver_height_m);
        let (wa, wc) = (Metric::A.weights(&bands), Metric::C.weights(&bands));
        let residual = sm_compute::impact::residual_dba(sc, &project, x, y);
        let centers: Vec<f64> = bands.iter().map(|b| b.fc).collect();
        Ok(sm_compute::auralize::render(
            &centers,
            &l_df,
            &l_pt,
            (
                weighted_level(&l_df, &wa),
                weighted_level(&l_pt, &wa),
                weighted_level(&l_pt, &wc),
                residual,
            ),
            (x - dx).hypot(y - dy),
        ))
    })
    .await
    .map_err(err)?
    .map_err(err)
}

/// Assistant « choix de spot » (mode Exploration, sans zone téléchargée).
#[tauri::command]
pub async fn find_spots(
    state: State<'_, AppState>,
    lon: f64,
    lat: f64,
    spec: sm_compute::spots::SpotSpec,
    on_progress: Channel<JobProgress>,
) -> CmdResult<sm_compute::spots::SpotReport> {
    let cancel = register_job(&state, "spots").map_err(err)?;
    let progress = ProgressCounter::new(3);
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let relay = {
        let (p, d, ch) = (progress.clone(), done.clone(), on_progress.clone());
        tauri::async_runtime::spawn(async move {
            while !d.load(std::sync::atomic::Ordering::Relaxed) {
                let snap = p.snapshot();
                let msg = [
                    "relief (RGE ALTI 100 m)",
                    "population (Filosofi)",
                    "criblage des spots",
                    "clairière et accès (BD TOPO)",
                ];
                let _ = ch.send(JobProgress {
                    stage: "spots".into(),
                    done: snap.done,
                    total: snap.total,
                    message: msg[(snap.done as usize).min(3)].into(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        })
    };
    let res =
        sm_compute::spots::find_spots(&state.client, lon, lat, spec, &progress, &cancel).await;
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = relay.await;
    end_job(&state, "spots");
    res.map_err(err)
}

/// Télécharge la géologie BRGM et l'état du sol (humidité, neige, pluie) de
/// la zone, puis recharge la scène.
#[tauri::command]
pub async fn fetch_ground_data(state: State<'_, AppState>, zone_id: String) -> CmdResult<()> {
    let s = sim(&state, &zone_id).map_err(err)?;
    sm_compute::ground_state::fetch(&state.client, &s.scene)
        .await
        .map_err(err)?;
    state
        .sim
        .sims
        .lock()
        .map_err(lock_err)
        .map_err(err)?
        .remove(&zone_id);
    Ok(())
}

/// Télécharge à la demande les points LiDAR HD (COPC) dans un rayon autour de
/// la sono et construit les voxels de végétation et la rugosité du sol. La
/// scène est rechargée ensuite (le feuillage mesuré remplace le forfait).
#[tauri::command]
pub async fn fetch_vegetation(
    state: State<'_, AppState>,
    zone_id: String,
    radius_m: f64,
    on_progress: Channel<JobProgress>,
) -> CmdResult<sm_compute::vegetation::VegetationSummary> {
    let s = sim(&state, &zone_id).map_err(err)?;
    let cancel = register_job(&state, "vegetation").map_err(err)?;
    let progress = ProgressCounter::new(0);
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let relay = {
        let (p, d, ch) = (progress.clone(), done.clone(), on_progress.clone());
        tauri::async_runtime::spawn(async move {
            while !d.load(std::sync::atomic::Ordering::Relaxed) {
                let snap = p.snapshot();
                let _ = ch.send(JobProgress {
                    stage: "vegetation".into(),
                    done: snap.done,
                    total: snap.total,
                    message: "LiDAR HD (COPC)".into(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
        })
    };
    let res = sm_compute::vegetation::fetch(
        &state.client,
        &s.scene,
        radius_m.clamp(200.0, 3000.0),
        2.0,
        &progress,
        &cancel,
    )
    .await;
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = relay.await;
    end_job(&state, "vegetation");
    let grid = res.map_err(err)?;
    let dir = state.zone_dir(&zone_id).map_err(err)?;
    grid.save(&dir).map_err(err)?;
    // Recharge la scène au prochain accès (profils et caches recalculés).
    state
        .sim
        .sims
        .lock()
        .map_err(lock_err)
        .map_err(err)?
        .remove(&zone_id);
    Ok(grid.summary())
}

#[derive(Serialize)]
pub struct PreviewResult {
    result_id: String,
    timing: sm_compute::polar::Timing,
    total_ms: f64,
    method: String,
    air: Air,
}

#[tauri::command]
pub async fn compute_preview(
    state: State<'_, AppState>,
    project: Project,
) -> CmdResult<PreviewResult> {
    let s = sim(&state, &project.zone_id).map_err(err)?;
    let id = state.sim.next_id();
    let zone_id = project.zone_id.clone();
    let (grid, timing, total_ms, air) = tauri::async_runtime::spawn_blocking(move || {
        let t0 = Instant::now();
        let sc = &s.scene;
        let sources = project.sources(sc);
        let air = air_for(sc, &project);
        let mut eng = s.engine.lock().map_err(lock_err)?;
        let (g, t) = eng.compute(
            sc,
            PolarSpec::preview(sc.radius),
            &sources,
            &program(&project.settings.program),
            &octaves(),
            path_cfg(&project, &s.scene),
            project.settings.receiver_height_m,
            air,
        );
        Ok::<_, anyhow::Error>((g, t, t0.elapsed().as_secs_f64() * 1e3, air))
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    let method =
        "Méthode d'ingénierie (ISO 9613-2 / CNOSSOS simplifiée), bandes d'octave".to_string();
    let mut grids = HashMap::new();
    grids.insert("main".to_owned(), grid);
    state.sim.results.lock().map_err(err)?.insert(
        id.clone(),
        Arc::new(ResultSet {
            zone_id,
            grids,
            rasters: Mutex::new(HashMap::new()),
            pe: None,
            method: method.clone(),
        }),
    );
    // On ne garde que les 8 derniers résultats.
    if let Ok(mut r) = state.sim.results.lock()
        && r.len() > 8
    {
        let mut keys: Vec<String> = r.keys().cloned().collect();
        keys.sort_by_key(|k| k[1..].parse::<u64>().unwrap_or(0));
        for k in keys.iter().take(r.len() - 8) {
            r.remove(k);
        }
    }
    Ok(PreviewResult {
        result_id: id,
        timing,
        total_ms,
        method,
        air,
    })
}

fn result(state: &AppState, id: &str) -> anyhow::Result<Arc<ResultSet>> {
    state
        .sim
        .results
        .lock()
        .map_err(lock_err)?
        .get(id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("résultat {id} expiré"))
}

/// Raster (mis en cache) d'une variante et d'un indicateur.
pub fn raster_of(
    state: &AppState,
    id: &str,
    variant: &str,
    metric: &str,
) -> anyhow::Result<Arc<Raster>> {
    let rs = result(state, id)?;
    let key = (variant.to_owned(), metric.to_owned());
    if let Some(r) = rs.rasters.lock().map_err(lock_err)?.get(&key) {
        return Ok(r.clone());
    }
    let g = rs
        .grids
        .get(variant)
        .ok_or_else(|| anyhow::anyhow!("variante {variant} absente"))?;
    let m = Metric::parse(metric).ok_or_else(|| anyhow::anyhow!("indicateur {metric} inconnu"))?;
    let r = Arc::new(rasterize(&polar_weighted(g, m), g, 720));
    rs.rasters.lock().map_err(lock_err)?.insert(key, r.clone());
    Ok(r)
}

/// Décalage de palette : le dB(C) et les bandes basses sont plus élevés.
pub fn palette_offset(metric: &str) -> f64 {
    match metric {
        "C" | "Z" => 20.0,
        "oct31" | "oct63" | "oct125" => 15.0,
        _ => 0.0,
    }
}

pub fn result_png(
    state: &AppState,
    id: &str,
    variant: &str,
    metric: &str,
) -> anyhow::Result<Vec<u8>> {
    let r = raster_of(state, id, variant, metric)?;
    Ok(to_png(&r, palette_offset(metric))?)
}

#[derive(Serialize)]
pub struct Layer {
    corners: Vec<[f64; 2]>,
    isophones: serde_json::Value,
    stats: RasterStats,
    palette_offset: f64,
    variants: Vec<String>,
    method: String,
}

#[tauri::command]
pub async fn result_layer(
    state: State<'_, AppState>,
    result_id: String,
    variant: String,
    metric: String,
) -> CmdResult<Layer> {
    let rs = result(&state, &result_id).map_err(err)?;
    let s = sim(&state, &rs.zone_id).map_err(err)?;
    let r = raster_of(&state, &result_id, &variant, &metric).map_err(err)?;
    let off = palette_offset(&metric);
    let th: Vec<f64> = (6..=19).map(|i| f64::from(i) * 5.0 + off).collect();
    let iso = isophones(&s.scene, &r, &th).map_err(err)?;
    let mut variants: Vec<String> = rs.grids.keys().cloned().collect();
    variants.sort();
    Ok(Layer {
        corners: corners_lonlat(&s.scene, &r),
        isophones: iso,
        stats: stats(&r, 50.0, &[35.0 + off, 45.0 + off, 55.0 + off]),
        palette_offset: off,
        variants,
        method: rs.method.clone(),
    })
}

/// Niveau affiché (dB) en un point de la carte, pour le survol.
#[tauri::command]
pub async fn result_value_at(
    state: State<'_, AppState>,
    result_id: String,
    variant: String,
    metric: String,
    lon: f64,
    lat: f64,
) -> CmdResult<Option<f64>> {
    let rs = result(&state, &result_id).map_err(err)?;
    let s = sim(&state, &rs.zone_id).map_err(err)?;
    let r = raster_of(&state, &result_id, &variant, &metric).map_err(err)?;
    Ok(s.scene
        .lonlat_to_local(lon, lat)
        .and_then(|(x, y)| r.get_local(x, y))
        .map(f64::from))
}

#[tauri::command]
pub async fn section_at(
    state: State<'_, AppState>,
    project: Project,
    result_id: Option<String>,
    lon: f64,
    lat: f64,
) -> CmdResult<Section> {
    let s = sim(&state, &project.zone_id).map_err(err)?;
    let pe = result_id
        .and_then(|id| result(&state, &id).ok())
        .and_then(|r| r.pe.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let sc = &s.scene;
        let (x, y) = sc.lonlat_to_local(lon, lat).ok_or("hors projection")?;
        let bands = if pe.is_some() { thirds() } else { octaves() };
        let nominal: Vec<f64> = if pe.is_some() {
            BandSet::ThirdOctave
                .bands()
                .iter()
                .map(|b| b.nominal().0)
                .collect()
        } else {
            BandSet::Octave
                .bands()
                .iter()
                .map(|b| b.nominal().0)
                .collect()
        };
        Ok::<_, String>(section(
            sc,
            &project.sources(sc),
            &program(&project.settings.program),
            &bands,
            &nominal,
            &path_cfg(&project, &s.scene),
            air_for(sc, &project),
            x,
            y,
            project.settings.receiver_height_m,
            pe.as_ref(),
        ))
    })
    .await
    .map_err(err)?
}

#[derive(Clone, Serialize)]
pub struct JobProgress {
    stage: String,
    done: u64,
    total: u64,
    message: String,
}

#[derive(Serialize)]
pub struct ScenarioInfo {
    id: String,
    label: String,
    max_dbc: f64,
    area_55_dbc_km2: f64,
    u_star: f64,
    inv_l: f64,
    seconds: f64,
}

#[derive(Serialize)]
pub struct LowFreqResult {
    result_id: String,
    scenarios: Vec<ScenarioInfo>,
    total_s: f64,
    method: String,
}

fn register_job(state: &AppState, kind: &str) -> anyhow::Result<CancelFlag> {
    let mut jobs = state.sim.jobs.lock().map_err(lock_err)?;
    anyhow::ensure!(!jobs.contains_key(kind), "calcul « {kind} » déjà en cours");
    let c = CancelFlag::new();
    jobs.insert(kind.to_owned(), c.clone());
    Ok(c)
}

fn end_job(state: &AppState, kind: &str) {
    if let Ok(mut j) = state.sim.jobs.lock() {
        j.remove(kind);
    }
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, kind: String) -> bool {
    state
        .sim
        .jobs
        .lock()
        .ok()
        .and_then(|j| j.get(&kind).map(CancelFlag::cancel))
        .is_some()
}

/// Calcul basses fréquences GFPE sur tous les scénarios + enveloppe.
#[tauri::command]
pub async fn run_lowfreq(
    state: State<'_, AppState>,
    project: Project,
    on_progress: Channel<JobProgress>,
) -> CmdResult<LowFreqResult> {
    let s = sim(&state, &project.zone_id).map_err(err)?;
    let cancel = register_job(&state, "lowfreq").map_err(err)?;
    let zone_id = project.zone_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
        let t0 = Instant::now();
        let sc = &s.scene;
        let sources = project.sources(sc);
        let bands = thirds();
        let spec = PolarSpec::preview(sc.radius);
        let prog = program(&project.settings.program);
        let (profiles, _) = s.engine.lock().map_err(lock_err)?.profiles(sc, spec);
        // Azimut critique : habitation la plus proche dans l'axe de la sono.
        let crit = sc
            .buildings
            .iter()
            .filter(|b| b.residential)
            .min_by(|a, b| {
                let d = |c: (f64, f64)| (c.0 - sc.origin.0).hypot(c.1 - sc.origin.1);
                d(a.centroid).total_cmp(&d(b.centroid))
            })
            .map_or(0.0, |b| {
                (b.centroid.0 - sc.origin.0)
                    .atan2(b.centroid.1 - sc.origin.1)
                    .to_degrees()
                    .rem_euclid(360.0)
            });
        let scenarios: Vec<Scenario> = default_scenarios(sc, project.settings.meteo_hour, crit);
        let per = task_count(spec.n_az, &bands, 2);
        let counter = ProgressCounter::new(per * scenarios.len() as u64);
        let (hs_sub, hs_top) = group_heights(&sources);
        let mut grids = HashMap::new();
        let mut infos = Vec::new();
        let mut list = Vec::new();
        let mut forecast_pe = None;
        // Relais de progression (tous les 200 ms).
        let done_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let relay = {
            let (c, ch, d) = (counter.clone(), on_progress.clone(), done_flag.clone());
            std::thread::spawn(move || {
                while !d.load(std::sync::atomic::Ordering::Relaxed) {
                    let p = c.snapshot();
                    let _ = ch.send(JobProgress {
                        stage: "gfpe".into(),
                        done: p.done,
                        total: p.total,
                        message: "GFPE".into(),
                    });
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            })
        };
        for sc_m in &scenarios {
            let t = Instant::now();
            let _ = on_progress.send(JobProgress {
                stage: "gfpe".into(),
                done: counter.snapshot().done,
                total: counter.snapshot().total,
                message: sc_m.label.clone(),
            });
            let pe = run_pe(
                &profiles,
                &sources,
                &bands,
                &sc_m.profile,
                project.settings.receiver_height_m,
                &path_cfg(&project, &s.scene),
                2,
                &cancel,
                &counter,
            )
            .ok_or_else(|| anyhow::anyhow!("calcul annulé"))?;
            let cfg = PathConfig {
                condition: if sc_m.id == "neutre" {
                    Condition::Homogeneous
                } else {
                    Condition::Favorable
                },
                model: project.settings.impedance_model,
                ..PathConfig::default()
            };
            let key = ExcessKey {
                hs_sub,
                hs_top,
                hr: project.settings.receiver_height_m,
                cfg,
                n_bands: bands.len(),
                band0: 25.0,
            };
            let ex = compute_excess(&profiles, key, &bands);
            let air = Air {
                t_c: sc_m.profile.ground_temperature(),
                rh: sc_m.profile.rh,
            };
            let g = combine(
                sc,
                &profiles,
                &ex,
                &sources,
                &prog,
                &bands,
                air,
                Some((&pe.data, &pe.weights)),
            );
            let r = rasterize(&polar_weighted(&g, Metric::C), &g, 400);
            let st = stats(&r, 50.0, &[55.0]);
            infos.push(ScenarioInfo {
                id: sc_m.id.clone(),
                label: sc_m.label.clone(),
                max_dbc: st.max_db,
                area_55_dbc_km2: st.area_above.first().map_or(0.0, |a| a.1),
                u_star: sc_m.profile.u_star,
                inv_l: sc_m.profile.inv_l,
                seconds: t.elapsed().as_secs_f64(),
            });
            if forecast_pe.is_none() || sc_m.id == "prevision" {
                forecast_pe = Some(pe);
            }
            list.push(g.clone());
            grids.insert(sc_m.id.clone(), g);
        }
        done_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = relay.join();
        if let Some((med, p90)) = envelope(&list) {
            grids.insert("main".into(), med);
            grids.insert("p90".into(), p90);
        }
        Ok((grids, infos, forecast_pe, t0.elapsed().as_secs_f64()))
    })
    .await;
    end_job(&state, "lowfreq");
    let (grids, infos, pe, total_s) = res.map_err(err)?.map_err(err)?;
    let id = state.sim.next_id();
    let backend = pe
        .as_ref()
        .map_or_else(sm_compute::pe::backend_name, |p| p.backend.clone());
    let method = format!(
        "GFPE N×2D sur {backend} (25–200 Hz, 2 fréquences par tiers d'octave, relief, sol par \
         segment, météo Monin-Obukhov) fusionné avec la méthode d'ingénierie au-delà de 200 Hz ; \
         « main » = médiane des scénarios, « p90 » = 90e centile"
    );
    state.sim.results.lock().map_err(err)?.insert(
        id.clone(),
        Arc::new(ResultSet {
            zone_id,
            grids,
            rasters: Mutex::new(HashMap::new()),
            pe,
            method: method.clone(),
        }),
    );
    Ok(LowFreqResult {
        result_id: id,
        scenarios: infos,
        total_s,
        method,
    })
}

#[tauri::command]
pub async fn compute_impact(
    state: State<'_, AppState>,
    project: Project,
    result_id: Option<String>,
) -> CmdResult<ImpactReport> {
    let s = sim(&state, &project.zone_id).map_err(err)?;
    let rs = result_id.as_ref().and_then(|id| result(&state, id).ok());
    let raster = result_id
        .as_ref()
        .and_then(|id| raster_of(&state, id, "main", "A").ok());
    tauri::async_runtime::spawn_blocking(move || {
        let sc = &s.scene;
        let pe = rs.as_ref().and_then(|r| r.pe.clone());
        let bands = if pe.is_some() { thirds() } else { octaves() };
        let method = if pe.is_some() {
            "GFPE (scénario prévision) + ingénierie, façades à 1,5 et 4 m"
        } else {
            "Méthode d'ingénierie, façades à 1,5 et 4 m"
        };
        impact::evaluate(
            sc,
            &project,
            &project.sources(sc),
            &program(&project.settings.program),
            &bands,
            &path_cfg(&project, &s.scene),
            air_for(sc, &project),
            pe.as_ref(),
            raster.as_deref(),
            method,
        )
    })
    .await
    .map_err(err)
}

/// Calage sur mesures (CSV ou GPX de sonomètre).
#[tauri::command]
pub async fn calibrate_measurements(
    state: State<'_, AppState>,
    project: Project,
    path: String,
) -> CmdResult<sm_compute::calibrate::CalibrationReport> {
    let s = sim(&state, &project.zone_id).map_err(err)?;
    tauri::async_runtime::spawn_blocking(move || {
        let text = std::fs::read_to_string(&path)?;
        let meas = sm_compute::calibrate::parse_measurements(&path, &text)?;
        sm_compute::calibrate::calibrate(
            &s.scene,
            &project,
            &meas,
            sm_compute::calibrate::CalibrationSpec::default(),
            &sm_compute::CancelFlag::new(),
        )
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub async fn compute_sizing(
    state: State<'_, AppState>,
    project: Project,
) -> CmdResult<Option<SizingReport>> {
    let s = sim(&state, &project.zone_id).map_err(err)?;
    tauri::async_runtime::spawn_blocking(move || {
        let sc = &s.scene;
        sizing::evaluate(
            sc,
            &project,
            &project.sources(sc),
            &program(&project.settings.program),
            &octaves(),
            &path_cfg(&project, &s.scene),
            air_for(sc, &project),
        )
    })
    .await
    .map_err(err)
}

#[derive(Serialize)]
pub struct OptimizeResult {
    candidates: Vec<Candidate>,
    seconds: f64,
}

#[tauri::command]
pub async fn run_optimizer(
    state: State<'_, AppState>,
    project: Project,
    spec: Option<OptimizeSpec>,
    on_progress: Channel<JobProgress>,
) -> CmdResult<OptimizeResult> {
    let s = sim(&state, &project.zone_id).map_err(err)?;
    let cancel = register_job(&state, "optimize").map_err(err)?;
    let res = tauri::async_runtime::spawn_blocking(move || {
        let t0 = Instant::now();
        let sc = &s.scene;
        let spec = spec.unwrap_or_default();
        let counter = ProgressCounter::new(position_count(&spec));
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let relay = {
            let (c, ch, d) = (counter.clone(), on_progress.clone(), done.clone());
            std::thread::spawn(move || {
                while !d.load(std::sync::atomic::Ordering::Relaxed) {
                    let p = c.snapshot();
                    let _ = ch.send(JobProgress {
                        stage: "optimize".into(),
                        done: p.done,
                        total: p.total,
                        message: "positions".into(),
                    });
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            })
        };
        let mut c = optimize(
            sc,
            &project,
            &project.sources(sc),
            &program(&project.settings.program),
            &octaves(),
            &path_cfg(&project, &s.scene),
            air_for(sc, &project),
            &spec,
            &cancel,
            &counter,
        );
        // Vérification GFPE des 3 meilleurs candidats (scénario prévision,
        // 72 azimuts, 1 fréquence par tiers d'octave, profils depuis la sono).
        let atm = forecast_profile(sc, project.settings.meteo_hour)
            .unwrap_or_else(|| sm_core::atmosphere::MoProfile::homogeneous(10.0, 80.0));
        let vspec = PolarSpec {
            n_az: 72,
            ..PolarSpec::preview(sc.radius)
        };
        let profiles = sm_compute::polar::build_profiles(sc, vspec);
        let bands = thirds();
        for cand in c.iter_mut().take(3) {
            if cancel.is_cancelled() {
                break;
            }
            let _ = on_progress.send(JobProgress {
                stage: "verify".into(),
                done: 0,
                total: 0,
                message: "vérification GFPE des meilleurs candidats".into(),
            });
            let p2 = Project {
                speakers: cand.speakers.clone(),
                ..project.clone()
            };
            let srcs = p2.sources(sc);
            let pc = ProgressCounter::new(task_count(vspec.n_az, &bands, 1));
            if let Some(pe) = run_pe(
                &profiles,
                &srcs,
                &bands,
                &atm,
                1.5,
                &path_cfg(&project, &s.scene),
                1,
                &cancel,
                &pc,
            ) {
                let rep = impact::evaluate(
                    sc,
                    &p2,
                    &srcs,
                    &program(&project.settings.program),
                    &bands,
                    &path_cfg(&project, &s.scene),
                    air_for(sc, &project),
                    Some(&pe),
                    None,
                    "vérification",
                );
                cand.gfpe_worst_emergence_dba = rep.most_exposed.map(|b| b.emergence_dba);
                cand.gfpe_margin_db = Some(rep.margin_db);
            }
        }
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = relay.join();
        if cancel.is_cancelled() {
            return Err("optimisation annulée".to_string());
        }
        Ok(OptimizeResult {
            candidates: c,
            seconds: t0.elapsed().as_secs_f64(),
        })
    })
    .await;
    end_job(&state, "optimize");
    res.map_err(err)?
}

#[tauri::command]
pub fn save_project(path: String, project: Project) -> CmdResult<()> {
    let bytes = serde_json::to_vec_pretty(&project).map_err(err)?;
    sm_data::tile_cache::write_atomic_sync(std::path::Path::new(&path), &bytes).map_err(err)
}

#[tauri::command]
pub fn load_project(path: String) -> CmdResult<Project> {
    let bytes = std::fs::read(&path).map_err(err)?;
    serde_json::from_slice(&bytes).map_err(err)
}

/// Sauvegarde automatique dans le dossier de la zone.
#[tauri::command]
pub fn autosave_project(state: State<'_, AppState>, project: Project) -> CmdResult<()> {
    let dir = state.zone_dir(&project.zone_id).map_err(err)?;
    let bytes = serde_json::to_vec_pretty(&project).map_err(err)?;
    sm_data::tile_cache::write_atomic_sync(&dir.join("projet.smproj"), &bytes).map_err(err)
}

#[tauri::command]
pub fn autoload_project(state: State<'_, AppState>, zone_id: String) -> Option<Project> {
    let dir = state.zone_dir(&zone_id).ok()?;
    serde_json::from_slice(&std::fs::read(dir.join("projet.smproj")).ok()?).ok()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportKind {
    Geotiff,
    Geojson,
    Html,
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn export_result(
    state: State<'_, AppState>,
    kind: ExportKind,
    path: String,
    project: Project,
    result_id: String,
    variant: String,
    metric: String,
    impact: Option<serde_json::Value>,
    sizing: Option<serde_json::Value>,
) -> CmdResult<()> {
    let rs = result(&state, &result_id).map_err(err)?;
    let s = sim(&state, &rs.zone_id).map_err(err)?;
    let r = raster_of(&state, &result_id, &variant, &metric).map_err(err)?;
    let sc = &s.scene;
    let impact: Option<ImpactReport> = impact.and_then(|v| serde_json::from_value(v).ok());
    let _ = sizing;
    match kind {
        ExportKind::Geotiff => {
            let f = std::fs::File::create(&path).map_err(err)?;
            export::geotiff(sc, &r, std::io::BufWriter::new(f)).map_err(err)
        }
        ExportKind::Geojson => {
            let iso = isophones(
                sc,
                &r,
                &(6..=19)
                    .map(|i| f64::from(i) * 5.0 + palette_offset(&metric))
                    .collect::<Vec<_>>(),
            )
            .map_err(err)?;
            std::fs::write(
                &path,
                serde_json::to_vec(&export::geojson(&iso, impact.as_ref())).map_err(err)?,
            )
            .map_err(err)
        }
        ExportKind::Html => {
            let png = to_png(&r, palette_offset(&metric)).map_err(err)?;
            let html = export::html_report(
                &project,
                sc,
                impact.as_ref(),
                None,
                Some(&png),
                &rs.method,
                "±3 dB à moins de 500 m, ±5 à 10 dB à plusieurs km de nuit ; voir l'enveloppe des scénarios (médiane / P90)",
            );
            std::fs::write(&path, html).map_err(err)
        }
    }
}
