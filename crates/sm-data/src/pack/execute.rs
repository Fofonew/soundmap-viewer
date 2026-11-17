//! Exécution reprenable d'un plan : téléchargements parallèles sous quotas,
//! écriture atomique, journal par item, annulation.

use super::manifest::{Journal, Manifest, ZoneStatus};
use super::plan::{Category, Fetch, PlanItem, cosia_items};
use crate::http::GeoClient;
use crate::tile_cache::write_atomic;
use crate::{DataError, Result, wfs};
use futures::StreamExt;
use serde::Serialize;
use sm_geo::Lambert93;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

/// Requêtes lancées simultanément (les quotas par service s'appliquent en plus).
const IN_FLIGHT: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Downloading,
    Finalizing,
    Complete,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackProgress {
    pub stage: Stage,
    pub done_items: usize,
    pub total_items: usize,
    pub bytes: u64,
    pub est_total_bytes: u64,
    pub failed_items: usize,
    pub elapsed_s: f64,
    pub message: String,
}

/// Bilan d'un passage de téléchargement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DownloadReport {
    pub fetched_items: usize,
    pub skipped_items: usize,
    pub missing_items: usize,
    pub failed: Vec<(String, String)>,
    pub bytes: u64,
    pub seconds: f64,
}

enum Outcome {
    Fetched(u64),
    Missing,
}

async fn fetch_item(
    client: &GeoClient,
    dir: &Path,
    item: &PlanItem,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let dest = dir.join(&item.dest);
    match &item.fetch {
        Fetch::Bytes {
            url,
            service,
            expect_len,
            allow_missing,
        } => match client.get(*service, url, cancel).await {
            Ok(body) => {
                if let Some(n) = expect_len
                    && body.bytes.len() as u64 != *n
                {
                    return Err(DataError::Parse(format!(
                        "{} : {} octets reçus, {n} attendus",
                        item.id,
                        body.bytes.len()
                    )));
                }
                write_atomic(&dest, &body.bytes).await?;
                Ok(Outcome::Fetched(body.bytes.len() as u64))
            }
            Err(e)
                if *allow_missing
                    && (e.is_not_found() || matches!(e, DataError::Status { status: 204, .. })) =>
            {
                write_atomic(&dest, b"").await?;
                Ok(Outcome::Missing)
            }
            Err(e) => Err(e),
        },
        Fetch::WfsLayer { typename, bbox } => {
            let feats = wfs::get_all(client, typename, bbox, cancel).await?;
            let bytes = serde_json::to_vec(&wfs::feature_collection(feats))?;
            write_atomic(&dest, &bytes).await?;
            Ok(Outcome::Fetched(bytes.len() as u64))
        }
    }
}

/// Télécharge les items manquants du manifeste dans `dir`.
///
/// Reprenable : les items présents dans le journal et sur disque sont sautés.
/// Les échecs n'interrompent pas les autres items ; le statut reste `Partial`
/// tant qu'il en reste. Si l'OCS GE est absente sur toute la zone, les tuiles
/// CoSIA sont ajoutées au plan puis téléchargées.
pub async fn download(
    client: &GeoClient,
    dir: &Path,
    manifest: &mut Manifest,
    on_progress: &(dyn Fn(PackProgress) + Send + Sync),
    cancel: &CancellationToken,
) -> Result<DownloadReport> {
    tokio::fs::create_dir_all(dir).await?;
    manifest.save(dir)?;
    let mut report = run_pass(client, dir, manifest, on_progress, cancel).await?;

    // Repli CoSIA si aucune tuile OCS GE n'existe sur la zone.
    let has_cosia = manifest.items.iter().any(|i| i.id.starts_with("cosia/"));
    if !has_cosia
        && report.failed.is_empty()
        && !cancel.is_cancelled()
        && ocsge_all_missing(dir, manifest)
    {
        tracing::info!("OCS GE absente sur la zone : repli sur CoSIA");
        let proj = Lambert93::new()?;
        let extra = cosia_items(&manifest.spec, client.endpoints(), &proj)?;
        manifest.items.extend(extra);
        manifest.save(dir)?;
        let r2 = run_pass(client, dir, manifest, on_progress, cancel).await?;
        report.fetched_items += r2.fetched_items;
        report.missing_items += r2.missing_items;
        report.failed.extend(r2.failed);
        report.bytes += r2.bytes;
        report.seconds += r2.seconds;
    }

    manifest.downloaded_bytes = Journal::read_bytes(dir);
    manifest.download_seconds += report.seconds;
    let done = Journal::read_done(dir);
    if manifest.items.iter().all(|i| done.contains(&i.id)) {
        if manifest.status == ZoneStatus::Partial {
            manifest.status = ZoneStatus::Downloaded;
        }
    } else {
        manifest.status = ZoneStatus::Partial;
    }
    manifest.save(dir)?;
    if cancel.is_cancelled() {
        return Err(DataError::Cancelled);
    }
    Ok(report)
}

fn ocsge_all_missing(dir: &Path, m: &Manifest) -> bool {
    let mut any = false;
    for it in m.items.iter().filter(|i| i.id.starts_with("ocsge/")) {
        any = true;
        if std::fs::metadata(dir.join(&it.dest)).is_ok_and(|md| md.len() > 0) {
            return false;
        }
    }
    any
}

async fn run_pass(
    client: &GeoClient,
    dir: &Path,
    manifest: &Manifest,
    on_progress: &(dyn Fn(PackProgress) + Send + Sync),
    cancel: &CancellationToken,
) -> Result<DownloadReport> {
    let t0 = Instant::now();
    let done = Journal::read_done(dir);
    let todo: Vec<&PlanItem> = manifest
        .items
        .iter()
        .filter(|i| !(done.contains(&i.id) && dir.join(&i.dest).exists()))
        .collect();
    let total = manifest.items.len();
    let skipped = total - todo.len();
    let est_total: u64 = manifest.items.iter().map(|i| i.est_bytes).sum();
    let prior_bytes = Journal::read_bytes(dir);
    tracing::info!(total, skipped, todo = todo.len(), "téléchargement du pack");

    let journal = Mutex::new(Journal::open(dir)?);
    let mut done_n = skipped;
    let mut bytes = 0u64;
    let mut missing = 0usize;
    let mut fetched = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();
    let mut last_emit = Instant::now() - std::time::Duration::from_secs(1);

    // Relief et vecteurs d'abord : ce sont les données critiques.
    let mut ordered: Vec<PlanItem> = todo.into_iter().cloned().collect();
    ordered.sort_by_key(|i| match i.category {
        Category::Relief => 0,
        Category::Vecteurs => 1,
        Category::Meteo => 2,
        Category::OccupationSol => 3,
        Category::Ortho => 4,
    });

    // Futures propriétaires de leurs données : elles restent `Send` quelle que
    // soit la durée de vie des emprunts de l'appelant (requis par Tauri).
    let (c, d, k) = (client.clone(), dir.to_path_buf(), cancel.clone());
    let mut stream = futures::stream::iter(ordered.into_iter().map(move |item| {
        let (c, d, k) = (c.clone(), d.clone(), k.clone());
        async move {
            let r = fetch_item(&c, &d, &item, &k).await;
            (item, r)
        }
    }))
    .buffer_unordered(IN_FLIGHT);

    while let Some((item, res)) = stream.next().await {
        match res {
            Ok(outcome) => {
                let n = match outcome {
                    Outcome::Fetched(n) => {
                        fetched += 1;
                        n
                    }
                    Outcome::Missing => {
                        missing += 1;
                        0
                    }
                };
                bytes += n;
                done_n += 1;
                let mut j = journal
                    .lock()
                    .map_err(|_| DataError::Parse("journal verrouillé".into()))?;
                j.record(&item.id, n)?;
            }
            Err(DataError::Cancelled) => {}
            Err(e) => {
                tracing::warn!(id = %item.id, err = %e, "item en échec");
                failed.push((item.id.clone(), e.to_string()));
            }
        }
        let finished = done_n + failed.len() == total;
        if last_emit.elapsed().as_millis() >= 100 || finished {
            last_emit = Instant::now();
            on_progress(PackProgress {
                stage: Stage::Downloading,
                done_items: done_n,
                total_items: total,
                bytes: prior_bytes + bytes,
                est_total_bytes: est_total,
                failed_items: failed.len(),
                elapsed_s: t0.elapsed().as_secs_f64(),
                message: item.id.clone(),
            });
        }
    }
    Ok(DownloadReport {
        fetched_items: fetched,
        skipped_items: skipped,
        missing_items: missing,
        failed,
        bytes,
        seconds: t0.elapsed().as_secs_f64(),
    })
}
