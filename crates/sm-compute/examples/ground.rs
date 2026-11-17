//! Télécharge la géologie BRGM et l'état du sol d'une zone, puis affiche les
//! ajustements de sol et l'état heure par heure.
//!
//! `cargo run --release -p sm-compute --example ground -- zones/<id>`

use sm_compute::ground_state::{fetch, hours};
use sm_compute::scene::Scene;
use sm_data::GeoClient;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage : ground <dossier de zone>")?,
    );
    let scene = Scene::load(&dir)?;
    fetch(&GeoClient::public()?, &scene).await?;
    let scene = Scene::load(&dir)?;
    let a = &scene.ground_adjust;
    println!(
        "falaises {:.3} km², sols durcis {:.3} km²",
        a.cliffs_km2, a.hardened_km2
    );
    for (d, k) in a.formations.iter().flatten() {
        println!("  formation : {d} ({k:?})");
    }
    for h in hours(&scene).unwrap_or_default().iter().step_by(3) {
        println!(
            "  {:02} h : humidité {:?}, neige {:?} m, pluie 24 h {:.1} mm → σ × {:.2}{}",
            h.hour,
            h.state.moisture,
            h.state.snow_m,
            h.state.rain_24h_mm,
            h.sigma_factor,
            if h.snow { ", neige" } else { "" }
        );
    }
    Ok(())
}
