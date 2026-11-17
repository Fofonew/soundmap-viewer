//! Téléchargement à la demande des points LiDAR HD (COPC) autour de la sono et
//! construction des voxels de végétation et de la rugosité.
//!
//! `cargo run --release -p sm-compute --example vegetation -- zones/<id> [rayon_m] [espacement_m]`

use sm_compute::scene::Scene;
use sm_compute::vegetation::fetch;
use sm_compute::{CancelFlag, ProgressCounter};
use sm_data::GeoClient;
use std::path::PathBuf;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(args.first().ok_or("usage : vegetation <dossier de zone>")?);
    let radius: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1000.0);
    let spacing: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let scene = Scene::load(&dir)?;
    let client = GeoClient::public()?;
    let progress = ProgressCounter::new(0);
    let t = Instant::now();
    let p2 = progress.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let s = p2.snapshot();
            eprintln!("  {} / {} nœuds", s.done, s.total);
        }
    });
    let grid = fetch(
        &client,
        &scene,
        radius,
        spacing,
        &progress,
        &CancelFlag::new(),
    )
    .await?;
    ticker.abort();
    let s = grid.summary();
    println!(
        "{} dalles, profondeur {}, {} points, {:.1} Mo en {:.1} s",
        s.tiles,
        grid.meta.depth,
        s.points,
        s.downloaded_mb,
        t.elapsed().as_secs_f64()
    );
    println!(
        "couvert {:.2} km² ; feuillage dense (f ≥ 0,5) {:.2} km² ; facteur moyen {:.2} ; rugosité moyenne {:.2} m",
        s.covered_km2, s.dense_foliage_km2, s.mean_foliage, s.mean_roughness_m
    );
    grid.save(&dir)?;
    println!("enregistré dans {}", dir.display());
    Ok(())
}
