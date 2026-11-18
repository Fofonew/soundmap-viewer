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

/// Gendarmerie ou commissariat, avec sa distance à vol d'oiseau.
#[derive(Debug, Serialize)]
pub struct PoliceNear {
    name: String,
    nature: String,
    lon: f64,
    lat: f64,
    distance_m: f64,
}

/// Gendarmeries et commissariats dans `radius_m` autour d'un point (BD TOPO),
/// du plus proche au plus éloigné.
#[tauri::command]
pub async fn nearby_police(
    state: State<'_, AppState>,
    lon: f64,
    lat: f64,
    radius_m: f64,
) -> CmdResult<Vec<PoliceNear>> {
    if state.offline {
        return Err("hors ligne".into());
    }
    let proj = Lambert93::new().map_err(err)?;
    let (x, y) = proj.from_wgs84(lon, lat).map_err(err)?;
    let r = radius_m.clamp(1_000.0, 60_000.0);
    let bbox = sm_data::BboxL93::new(x - r, y - r, x + r, y + r);
    let found = sm_data::police::fetch(&state.client, &bbox, &CancellationToken::new())
        .await
        .map_err(err)?;
    let mut out: Vec<PoliceNear> = found
        .into_iter()
        .filter_map(|p| {
            let d = (p.x - x).hypot(p.y - y);
            let (lon, lat) = proj.to_wgs84(p.x, p.y).ok()?;
            (d <= r).then_some(PoliceNear {
                name: p.name,
                nature: p.nature,
                lon,
                lat,
                distance_m: d,
            })
        })
        .collect();
    out.sort_by(|a, b| a.distance_m.total_cmp(&b.distance_m));
    Ok(out)
}

/// Parcelle cadastrale en un point, avec ses propriétaires s'ils sont des
/// personnes morales (open data DGFiP) et la forêt publique éventuelle.
#[derive(Debug, Serialize)]
pub struct ParcelInfo {
    parcel: Option<ParcelOut>,
    /// Droits de personnes morales (vide : personne physique, non publié).
    owners: Vec<sm_data::cadastre::OwnerRight>,
    /// Erreur de lecture des fichiers DGFiP (la parcelle reste affichée).
    owners_error: Option<String>,
    owners_year: u32,
    public_forest: Option<String>,
    cadastre_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ParcelOut {
    idu: String,
    commune: String,
    code_insee: String,
    section: String,
    numero: String,
    contenance_m2: f64,
    /// Contour (WGS84).
    ring: Vec<[f64; 2]>,
}

async fn pm_index(
    state: &AppState,
    dep: &str,
) -> anyhow::Result<std::sync::Arc<sm_data::cadastre::PmIndex>> {
    use sm_data::cadastre::{PM_YEAR, fetch_pm_department, parse_pm_csv};
    anyhow::ensure!(
        !dep.is_empty() && dep.chars().all(|c| c.is_ascii_alphanumeric()),
        "département invalide"
    );
    if let Some(i) = state.pm.lock().map_err(lock_err)?.get(dep) {
        return Ok(i.clone());
    }
    let dir = state.cache.root().join("cadastre");
    let file = dir.join(format!("pm_{PM_YEAR}_{dep}.csv"));
    let text = match tokio::fs::read_to_string(&file).await {
        Ok(t) => t,
        Err(_) => {
            let t = fetch_pm_department(&state.client, dep, &CancellationToken::new()).await?;
            tokio::fs::create_dir_all(&dir).await?;
            let tmp = file.with_extension("part");
            tokio::fs::write(&tmp, &t).await?;
            tokio::fs::rename(&tmp, &file).await?;
            t
        }
    };
    let idx = std::sync::Arc::new(
        tauri::async_runtime::spawn_blocking(move || parse_pm_csv(&text)).await?,
    );
    state
        .pm
        .lock()
        .map_err(lock_err)?
        .insert(dep.to_owned(), idx.clone());
    Ok(idx)
}

#[tauri::command]
pub async fn parcel_info(state: State<'_, AppState>, lon: f64, lat: f64) -> CmdResult<ParcelInfo> {
    use sm_data::cadastre::{PM_YEAR, cadastre_gouv_url, parcel_at, public_forest_at};
    if state.offline {
        return Err("hors ligne".into());
    }
    let proj = Lambert93::new().map_err(err)?;
    let (x, y) = proj.from_wgs84(lon, lat).map_err(err)?;
    let cancel = CancellationToken::new();
    let (parcel, forest) = tokio::join!(
        parcel_at(&state.client, x, y, &cancel),
        public_forest_at(&state.client, x, y, &cancel)
    );
    let parcel = parcel.map_err(err)?;
    let (mut owners, mut owners_error) = (Vec::new(), None);
    if let Some(p) = &parcel {
        match pm_index(&state, &p.code_dep).await {
            Ok(idx) => owners = idx.get(&p.pm_key()).cloned().unwrap_or_default(),
            Err(e) => owners_error = Some(e.to_string()),
        }
    }
    Ok(ParcelInfo {
        cadastre_url: parcel.as_ref().map(|p| cadastre_gouv_url(&p.code_insee)),
        parcel: parcel.map(|p| ParcelOut {
            ring: p
                .ring
                .iter()
                .filter_map(|&(x, y)| proj.to_wgs84(x, y).ok().map(|(a, b)| [a, b]))
                .collect(),
            idu: p.idu,
            commune: p.nom_com,
            code_insee: p.code_insee,
            section: p.section.trim_start_matches('0').to_owned(),
            numero: p.numero.trim_start_matches('0').to_owned(),
            contenance_m2: p.contenance_m2,
        }),
        owners,
        owners_error,
        owners_year: PM_YEAR,
        public_forest: forest.ok().flatten(),
    })
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

/// Parcelle d'une tuile PCI (attributs du parcellaire), pour la carte des propriétés.
#[derive(Debug, Deserialize)]
pub struct ParcelRef {
    idu: String,
    code_dep: String,
    code_com: String,
    #[serde(default)]
    com_abs: String,
    section: String,
    numero: String,
}

/// Propriétaire personne morale d'une parcelle.
#[derive(Debug, Serialize)]
pub struct ParcelOwner {
    idu: String,
    category: sm_data::cadastre::OwnerCategory,
    /// SIREN (sinon dénomination) : regroupe les parcelles d'un même propriétaire.
    owner_id: String,
    name: String,
    forme: String,
    droit: String,
}

/// Propriétaires personnes morales d'une liste de parcelles (fichiers DGFiP,
/// un téléchargement par département puis lecture en mémoire). Les parcelles
/// absentes du résultat appartiennent à des particuliers.
#[tauri::command]
pub async fn parcel_owners(
    state: State<'_, AppState>,
    parcels: Vec<ParcelRef>,
) -> CmdResult<Vec<ParcelOwner>> {
    use sm_data::cadastre::{main_right, pm_key};
    if state.offline {
        return Err("hors ligne".into());
    }
    if parcels.len() > 20_000 {
        return Err("trop de parcelles demandées".into());
    }
    let mut by_dep: std::collections::BTreeMap<&str, Vec<&ParcelRef>> = Default::default();
    for p in &parcels {
        by_dep.entry(p.code_dep.as_str()).or_default().push(p);
    }
    let mut out = Vec::new();
    for (dep, ps) in by_dep {
        let idx = pm_index(&state, dep).await.map_err(err)?;
        out.extend(ps.into_iter().filter_map(|p| {
            let rights = idx.get(&pm_key(&p.code_com, &p.com_abs, &p.section, &p.numero))?;
            let r = main_right(rights)?;
            Some(ParcelOwner {
                idu: p.idu.clone(),
                category: r.category(),
                owner_id: r.owner_id().to_owned(),
                name: r.denomination.clone(),
                forme: r.forme.clone(),
                droit: r.droit.clone(),
            })
        }));
    }
    Ok(out)
}
