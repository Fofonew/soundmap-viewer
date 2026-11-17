//! Téléchargement d'un pack de zone en ligne de commande (même pipeline que l'app).
//!
//! `cargo run --release -p sm-data --example zone -- [lon lat rayon_m nom] [--dir zones] [--yes]`
//!
//! Affiche l'estimation, télécharge (reprenable : relancer la commande),
//! finalise, puis mesure la réouverture hors ligne.

use anyhow::Context;
use sm_data::pack::{self, Manifest, OpenZone, ZoneSpec, ZoneStatus};
use sm_data::{CancellationToken, GeoClient};
use sm_geo::{Lambert93, TileId};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir_arg = args
        .iter()
        .position(|a| a == "--dir")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "zones".into());
    let pos: Vec<&String> = args
        .iter()
        .filter(|a| !a.starts_with("--") && **a != dir_arg)
        .collect();
    let num = |i: usize, d: f64| pos.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    let (lon, lat, radius) = (num(0, 3.0130), num(1, 44.0040), num(2, 5000.0));
    let name = pos.get(3).map_or("Causse du Larzac nord", |s| s.as_str());

    let proj = Lambert93::new()?;
    let (x, y) = proj.from_wgs84(lon, lat)?;
    let spec = ZoneSpec::new(name, x, y, radius, &pack::today_utc());
    let client = GeoClient::public()?;
    let root = PathBuf::from(dir_arg);
    let dir = root.join(spec.id());

    let mut manifest = match Manifest::load(&dir) {
        Ok(m) => {
            println!("Zone existante {} ({:?}) : reprise.", m.id, m.status);
            m
        }
        Err(_) => {
            let t = Instant::now();
            let items = pack::build_plan(&spec, client.endpoints(), &proj)?;
            println!(
                "Plan : {} éléments en {:.0} ms",
                items.len(),
                t.elapsed().as_secs_f64() * 1e3
            );
            Manifest::new(spec.clone(), items)
        }
    };

    let e = pack::estimate(&manifest.items);
    println!(
        "\nEstimation pour « {} » ({:.1} km) :",
        manifest.spec.name,
        radius / 1000.0
    );
    for c in &e.categories {
        println!(
            "  {:<15} {:>6} éléments  {:>8.1} Mo",
            format!("{:?}", c.category),
            c.items,
            c.bytes as f64 / 1e6
        );
    }
    println!(
        "  total : {:.1} Mo à télécharger, {:.1} Mo sur disque, ≈ {:.0} s",
        e.total_bytes as f64 / 1e6,
        e.disk_bytes as f64 / 1e6,
        e.est_seconds
    );

    let t_dl = Instant::now();
    if manifest.status == ZoneStatus::Partial {
        let last = Mutex::new(0usize);
        let report = pack::download(
            &client,
            &dir,
            &mut manifest,
            &|p| {
                if let Ok(mut l) = last.lock() {
                    let pct = p.done_items * 100 / p.total_items.max(1);
                    if pct >= *l + 10 || p.done_items == p.total_items {
                        *l = pct;
                        println!(
                            "  {:>3} %  {:>5}/{:<5} {:>7.1} Mo  {:>5.1} s",
                            pct,
                            p.done_items,
                            p.total_items,
                            p.bytes as f64 / 1e6,
                            p.elapsed_s
                        );
                    }
                }
            },
            &CancellationToken::new(),
        )
        .await?;
        println!(
            "\nTéléchargement : {} récupérés, {} déjà présents, {} absents côté serveur, {} échecs, {:.1} Mo en {:.1} s ({:.1} Mo/s)",
            report.fetched_items,
            report.skipped_items,
            report.missing_items,
            report.failed.len(),
            report.bytes as f64 / 1e6,
            report.seconds,
            report.bytes as f64 / 1e6 / report.seconds.max(1e-3)
        );
        for (id, err) in report.failed.iter().take(5) {
            println!("  échec {id} : {err}");
        }
        anyhow::ensure!(
            report.failed.is_empty(),
            "échecs : relancer la commande pour reprendre"
        );
    }
    if manifest.status == ZoneStatus::Downloaded {
        let t = Instant::now();
        pack::finalize(&dir, &mut manifest, &|s| println!("  finalisation : {s}"))?;
        println!("Finalisation : {:.1} s", t.elapsed().as_secs_f64());
    }
    let wall = t_dl.elapsed().as_secs_f64();

    // Réouverture hors ligne (aucun client réseau utilisé ici).
    let t = Instant::now();
    let z = OpenZone::open(&dir).context("ouverture du pack")?;
    let open_ms = t.elapsed().as_secs_f64() * 1e3;
    let p = z.manifest.products.clone().context("produits")?;
    let (clon, clat) = (p.center_lonlat[0], p.center_lonlat[1]);
    let t = Instant::now();
    let tile = TileId::from_lonlat(clon, clat, 17);
    z.terrain_tile(tile)?;
    let tile_ms = t.elapsed().as_secs_f64() * 1e3;
    let du = dir_size(&dir);
    println!("\nPack {} :", dir.display());
    println!("  durée totale (téléchargement + finalisation) : {wall:.1} s");
    println!("  taille sur disque : {:.1} Mo", du as f64 / 1e6);
    println!(
        "  bâtiments {} dont {} habitations · occupation du sol : {} · altitudes {:?}",
        p.buildings.total, p.buildings.residential, p.landcover, p.elevation_range
    );
    println!(
        "  ortho {} tuiles · terrain pré-généré {} tuiles (z ≤ {}) · finalisation {:.1} s",
        p.ortho_tiles, p.terrain_tiles, p.terrain_pregen_max_zoom, p.finalize_seconds
    );
    println!(
        "  réouverture hors ligne : {open_ms:.1} ms ; tuile terrain z17 à la demande : {tile_ms:.1} ms"
    );
    if let Some(h) = z.elevation_at(clon, clat) {
        println!("  altitude au point sono : {h:.2} m");
    }
    println!("  id : {}", z.id());
    Ok(())
}

fn dir_size(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}
