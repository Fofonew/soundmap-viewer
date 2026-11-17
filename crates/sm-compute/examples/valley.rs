//! Analyse de relief autour de la sono (vallée, pente, direction aval).
//!
//! `cargo run --release -p sm-compute --example valley -- zones/<id>`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .ok_or("usage : valley <dossier de zone>")?;
    let scene = sm_compute::scene::Scene::load(std::path::Path::new(&dir))?;
    let a = sm_compute::valley::analyze(&scene);
    println!(
        "{} : TPI {:.1} m, pente {:.1}°, aval vers {:.0}° → vallée : {}",
        scene.name, a.tpi_m, a.slope_deg, a.downslope_toward_deg, a.valley
    );
    if a.valley {
        println!("  écoulement : {:?}", a.flow());
    }
    Ok(())
}
