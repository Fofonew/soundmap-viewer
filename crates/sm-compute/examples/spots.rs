//! Assistant « choix de spot » sur 30 km autour d'un point.
//!
//! `cargo run --release -p sm-compute --example spots -- <lon> <lat> [rayon_m]`

use sm_compute::spots::{SpotSpec, find_spots};
use sm_compute::{CancelFlag, ProgressCounter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<f64> = std::env::args()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    let (lon, lat) = (*a.first().ok_or("lon")?, *a.get(1).ok_or("lat")?);
    let spec = SpotSpec {
        radius_m: a.get(2).copied().unwrap_or(30_000.0),
        ..SpotSpec::default()
    };
    let client = sm_data::GeoClient::public()?;
    let r = find_spots(
        &client,
        lon,
        lat,
        spec,
        &ProgressCounter::new(0),
        &CancelFlag::new(),
    )
    .await?;
    println!(
        "{} candidats, {} carreaux habités ({:.0} habitants) en {:.1} s",
        r.candidates.len(),
        r.populated_cells,
        r.population,
        r.seconds
    );
    println!(
        "praticabilité : {} vérifiés, {} boisés, {} sans accès",
        r.site_checked, r.rejected_wooded, r.rejected_access
    );
    for s in &r.spots {
        println!(
            "  {:.5} {:.5} : {:.0} exposés, 1er habitant à {:.0} m, max {:.1} dB(C), pente {:.1} %, alt. {:.0} m",
            s.lat, s.lon, s.exposed, s.nearest_m, s.max_dbc, s.slope_pct, s.elevation_m
        );
        if let Some(site) = &s.site {
            println!(
                "      accès {:?}{} à {:.0} m, déplacé de {:.0} m, boisé {:.0} % dans 150 m",
                site.access,
                if site.access_restricted {
                    " (restreint)"
                } else {
                    ""
                },
                site.access_m,
                site.shift_m,
                site.wooded_pct
            );
        }
    }
    Ok(())
}
