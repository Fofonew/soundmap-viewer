//! Commandes IPC. Les erreurs sont renvoyées en texte à l'UI.

use crate::state::{AppState, lock_err};
use serde::{Deserialize, Serialize};
use sm_data::pack::{
    self, Estimate, Manifest, OrthoLevel, PackProgress, Ring, Stage, ZoneSpec, ZoneStatus,
    ZoneSummary, manifest::Products,
};
use sm_data::{CancellationToken, geocode};
use sm_geo::Lambert93;
use std::path::PathBuf;
use std::time::Instant;
use tauri::State;
use tauri::ipc::Channel;

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Debug, Serialize)]
pub struct AppInfo {
    version: &'static str,
    zones_dir: PathBuf,
    cache_dir: PathBuf,
    offline: bool,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        zones_dir: state.zones_dir.clone(),
        cache_dir: state.cache.root().to_path_buf(),
        offline: state.offline,
    }
}

#[tauri::command]
pub async fn geocode(
    state: State<'_, AppState>,
    q: String,
) -> CmdResult<Vec<geocode::GeocodeResult>> {
    if state.offline {
        return Err("hors ligne".into());
    }
    geocode::search(&state.client, &q, 6, &CancellationToken::new())
        .await
        .map_err(err)
}

/// Zone demandée par l'UI (coordonnées WGS84).
#[derive(Debug, Clone, Deserialize)]
pub struct ZoneRequest {
    name: String,
    lon: f64,
    lat: f64,
    radius_m: f64,
    meteo_date: Option<String>,
}

impl ZoneRequest {
    fn spec(&self) -> anyhow::Result<ZoneSpec> {
        anyhow::ensure!(
            (200.0..=20_000.0).contains(&self.radius_m),
            "rayon hors limites (200 m – 20 km)"
        );
        let (x, y) = Lambert93::new()?.from_wgs84(self.lon, self.lat)?;
        let date = self.meteo_date.clone().unwrap_or_else(pack::today_utc);
        let name = if self.name.trim().is_empty() {
            "zone"
        } else {
            self.name.trim()
        };
        Ok(ZoneSpec::new(name, x, y, self.radius_m, &date))
    }
}

#[derive(Debug, Serialize)]
pub struct EstimateResponse {
    id: String,
    existing: Option<ZoneStatus>,
    estimate: Estimate,
    rings: Vec<Ring>,
    ortho: Vec<OrthoLevel>,
}

#[tauri::command]
pub async fn estimate_zone(
    state: State<'_, AppState>,
    req: ZoneRequest,
) -> CmdResult<EstimateResponse> {
    let spec = req.spec().map_err(err)?;
    let ep = state.client.endpoints().clone();
    let existing = Manifest::load(&state.zone_dir(&spec.id()).map_err(err)?)
        .ok()
        .map(|m| m.status);
    tauri::async_runtime::spawn_blocking(move || {
        let proj = Lambert93::new()?;
        let items = pack::build_plan(&spec, &ep, &proj)?;
        Ok::<_, anyhow::Error>(EstimateResponse {
            id: spec.id(),
            existing,
            estimate: pack::estimate(&items),
            rings: spec.effective_rings(),
            ortho: spec.ortho.clone(),
        })
    })
    .await
    .map_err(err)?
    .map_err(err)
}

/// Télécharge (ou reprend) puis finalise un pack. Progression par `channel`.
async fn run_job(
    state: &AppState,
    mut manifest: Manifest,
    channel: Channel<PackProgress>,
) -> anyhow::Result<ZoneSummary> {
    let id = manifest.id.clone();
    let dir = state.zone_dir(&id)?;
    let cancel = CancellationToken::new();
    {
        let mut d = state.downloads.lock().map_err(lock_err)?;
        anyhow::ensure!(
            !d.contains_key(&id),
            "téléchargement déjà en cours pour {id}"
        );
        d.insert(id.clone(), cancel.clone());
    }
    state.forget_zone(&id);
    let result = async {
        let ch = channel.clone();
        if manifest.status == ZoneStatus::Partial {
            let report = pack::download(
                &state.client,
                &dir,
                &mut manifest,
                &move |p| {
                    let _ = ch.send(p);
                },
                &cancel,
            )
            .await?;
            tracing::info!(
                id,
                fetched = report.fetched_items,
                skipped = report.skipped_items,
                missing = report.missing_items,
                failed = report.failed.len(),
                mo = report.bytes as f64 / 1e6,
                s = report.seconds,
                "passage de téléchargement terminé"
            );
            if !report.failed.is_empty() {
                anyhow::bail!(
                    "{} éléments en échec (ex. {} : {}). Relancer « Reprendre ».",
                    report.failed.len(),
                    report.failed[0].0,
                    report.failed[0].1
                );
            }
        }
        if manifest.status == ZoneStatus::Downloaded {
            let ch = channel.clone();
            let total = manifest.items.len();
            let bytes = manifest.downloaded_bytes;
            let dir2 = dir.clone();
            let t0 = Instant::now();
            manifest = tauri::async_runtime::spawn_blocking(move || {
                let step = move |s: &str| {
                    let _ = ch.send(PackProgress {
                        stage: Stage::Finalizing,
                        done_items: total,
                        total_items: total,
                        bytes,
                        est_total_bytes: bytes,
                        failed_items: 0,
                        elapsed_s: t0.elapsed().as_secs_f64(),
                        message: s.to_owned(),
                    });
                };
                pack::finalize(&dir2, &mut manifest, &step).map(|()| manifest)
            })
            .await??;
        }
        Ok::<_, anyhow::Error>(manifest.summary(&dir))
    }
    .await;
    if let Ok(mut d) = state.downloads.lock() {
        d.remove(&id);
    }
    let final_stage = match &result {
        Ok(_) => Stage::Complete,
        Err(e)
            if e.downcast_ref::<sm_data::DataError>()
                .is_some_and(|e| matches!(e, sm_data::DataError::Cancelled)) =>
        {
            Stage::Cancelled
        }
        Err(_) => Stage::Failed,
    };
    let _ = channel.send(PackProgress {
        stage: final_stage,
        done_items: 0,
        total_items: 0,
        bytes: 0,
        est_total_bytes: 0,
        failed_items: 0,
        elapsed_s: 0.0,
        message: result
            .as_ref()
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default(),
    });
    result
}

#[tauri::command]
pub async fn download_zone(
    state: State<'_, AppState>,
    req: ZoneRequest,
    on_progress: Channel<PackProgress>,
) -> CmdResult<ZoneSummary> {
    let spec = req.spec().map_err(err)?;
    let dir = state.zone_dir(&spec.id()).map_err(err)?;
    // Une zone déjà connue est reprise (jamais re-téléchargée de zéro).
    let manifest = match Manifest::load(&dir) {
        Ok(m) => m,
        Err(_) => {
            let ep = state.client.endpoints().clone();
            let spec2 = spec.clone();
            let items = tauri::async_runtime::spawn_blocking(move || {
                pack::build_plan(&spec2, &ep, &Lambert93::new()?)
            })
            .await
            .map_err(err)?
            .map_err(err)?;
            Manifest::new(spec, items)
        }
    };
    run_job(&state, manifest, on_progress).await.map_err(err)
}

#[tauri::command]
pub async fn resume_zone(
    state: State<'_, AppState>,
    id: String,
    on_progress: Channel<PackProgress>,
) -> CmdResult<ZoneSummary> {
    let manifest = Manifest::load(&state.zone_dir(&id).map_err(err)?).map_err(err)?;
    run_job(&state, manifest, on_progress).await.map_err(err)
}

#[tauri::command]
pub fn cancel_download(state: State<'_, AppState>, id: String) -> CmdResult<bool> {
    let d = state.downloads.lock().map_err(err)?;
    Ok(d.get(&id).map(|c| c.cancel()).is_some())
}

#[tauri::command]
pub fn list_zones(state: State<'_, AppState>) -> Vec<ZoneSummary> {
    pack::list_zones(&state.zones_dir)
}

#[derive(Debug, Serialize)]
pub struct OpenZoneResponse {
    summary: ZoneSummary,
    spec: ZoneSpec,
    products: Products,
    open_ms: f64,
}

#[tauri::command]
pub async fn open_zone(state: State<'_, AppState>, id: String) -> CmdResult<OpenZoneResponse> {
    let t0 = Instant::now();
    state.forget_zone(&id);
    let zone = state.zone(&id).map_err(err)?;
    let products = zone.manifest.products.clone().ok_or("produits absents")?;
    let open_ms = t0.elapsed().as_secs_f64() * 1e3;
    tracing::info!(id, open_ms, "zone ouverte (manifeste + MNT memory-mappé)");
    Ok(OpenZoneResponse {
        summary: zone.manifest.summary(&zone.dir),
        spec: zone.manifest.spec.clone(),
        products,
        open_ms,
    })
}

#[tauri::command]
pub fn delete_zone(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let dir = state.zone_dir(&id).map_err(err)?;
    if state.downloads.lock().map_err(err)?.contains_key(&id) {
        return Err("téléchargement en cours : annuler d'abord".into());
    }
    state.forget_zone(&id);
    std::fs::remove_dir_all(dir).map_err(err)
}

/// Mesures de performance côté UI (journalisées côté Rust).
#[tauri::command]
pub fn log_metric(name: String, ms: f64) {
    tracing::info!(metric = name, ms, "mesure UI");
}
