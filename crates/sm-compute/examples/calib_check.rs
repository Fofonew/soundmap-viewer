//! Vérification du calage sur mesures synthétiques : on génère des « mesures »
//! avec des paramètres connus (σ × 2, gradient 0,05 s⁻¹, +2 dB) et un bruit
//! de 1 dB aux habitations, puis on vérifie que le calage les retrouve.
//!
//! `cargo run --release -p sm-compute --example calib_check -- zones/<id>`

use sm_compute::CancelFlag;
use sm_compute::calibrate::{CalibrationSpec, Measurement, calibrate};
use sm_compute::meteo::forecast_profile;
use sm_compute::polar::Air;
use sm_compute::project::{Project, SpeakerSpec};
use sm_compute::receiver::levels_at;
use sm_compute::render::{Metric, weighted_level};
use sm_compute::scene::Scene;
use sm_core::bands::BandSet;
use sm_core::engineering::{BandSpec, PathConfig, Refraction, Scratch};
use sm_core::source::{preset, program};
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage : calib_check <dossier de zone>")?,
    );
    let scene = Scene::load(&dir)?;
    let mut project = Project::new("Calage", &scene.zone_id);
    for (i, (kind, h)) in [("sub2x18", 0.6), ("sub2x18", 0.6), ("top15", 2.5)]
        .iter()
        .enumerate()
    {
        let (lon, lat) = scene
            .local_to_lonlat(i as f64 * 1.5, 0.0)
            .ok_or("position")?;
        project.speakers.push(SpeakerSpec {
            id: format!("s{i}"),
            name: format!("S{i}"),
            model: preset(kind).ok_or("preset")?,
            lon,
            lat,
            height_m: *h,
            azimuth_deg: 90.0,
            tilt_deg: 0.0,
            delay_ms: 0.0,
            polarity: 1,
            gain_db: 0.0,
            muted: false,
        });
    }
    let truth = Refraction {
        grad_t: 0.05,
        grad_w: 0.0,
        wind_toward_deg: 0.0,
    };
    let (true_scale, true_offset) = (2.0, 2.0);
    let forecast = forecast_profile(&scene, project.settings.meteo_hour);
    let air = forecast.as_ref().map_or(
        Air {
            t_c: 10.0,
            rh: 80.0,
        },
        |p| Air {
            t_c: p.ground_temperature(),
            rh: p.rh,
        },
    );
    let mut truth_project = project.clone();
    truth_project.settings.master_gain_db = true_offset;
    let sources = truth_project.sources(&scene);
    let prog = program(&project.settings.program);
    let bands: Vec<BandSpec> = BandSet::Octave
        .bands()
        .into_iter()
        .map(BandSpec::from_band)
        .collect();
    let cfg = PathConfig {
        sigma_scale: true_scale,
        refraction: Some(truth),
        c: sm_core::atmosphere::sound_speed(air.t_c),
        ..PathConfig::default()
    };
    let (wa, wc) = (Metric::A.weights(&bands), Metric::C.weights(&bands));
    // Bruit pseudo-aléatoire reproductible (LCG + Box-Muller), σ = 1 dB.
    let mut state: u64 = 0x5eed;
    let mut uniform = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    };
    let mut noise = || {
        let (u1, u2) = (uniform(), uniform());
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    };
    let mut sc = Scratch::default();
    let mut meas = Vec::new();
    for b in scene.buildings.iter().filter(|b| b.residential) {
        let (x, y) = (b.centroid.0 - scene.origin.0, b.centroid.1 - scene.origin.1);
        let d = x.hypot(y);
        if !(100.0..=2500.0).contains(&d) || meas.len() >= 30 {
            continue;
        }
        let (lv, _) = levels_at(
            &scene,
            &sources,
            &prog,
            &bands,
            &cfg,
            air,
            x,
            y,
            1.5,
            (d / 500.0).max(2.0),
            None,
            &mut sc,
        );
        let (lon, lat) = scene.local_to_lonlat(x, y).ok_or("lonlat")?;
        meas.push(Measurement {
            lon,
            lat,
            la: Some(weighted_level(&lv, &wa) + noise()),
            lc: Some(weighted_level(&lv, &wc) + noise()),
            label: Some(b.id.clone()),
        });
    }
    println!("{} mesures synthétiques", meas.len());
    let t = Instant::now();
    let r = calibrate(
        &scene,
        &project,
        &meas,
        CalibrationSpec::default(),
        &CancelFlag::new(),
    )?;
    println!("Calage en {:.2} s", t.elapsed().as_secs_f64());
    println!(
        "RMS avant {:.2} dB (biais {:+.2}) → après {:.2} dB",
        r.rms_before_db, r.bias_before_db, r.rms_after_db
    );
    println!(
        "facteur σ : MAP {:.2} (moyenne {:.2}, ±{:.2} oct.) — vrai {true_scale}",
        r.sigma_scale.map, r.sigma_scale.mean, r.sigma_scale.sd
    );
    println!(
        "gradient thermique : MAP {:.3} (moyenne {:.3} ± {:.3}) s⁻¹ — vrai {}",
        r.grad_t.map, r.grad_t.mean, r.grad_t.sd, truth.grad_t
    );
    println!(
        "gradient vent : MAP {:.3} (moyenne {:.3} ± {:.3}) s⁻¹ — vrai {}",
        r.grad_w.map, r.grad_w.mean, r.grad_w.sd, truth.grad_w
    );
    println!(
        "décalage : MAP {:+.2} (moyenne {:+.2} ± {:.2}) dB — vrai {true_offset:+}",
        r.offset_db.map, r.offset_db.mean, r.offset_db.sd
    );
    let mut pts: Vec<_> = r.points.iter().collect();
    pts.sort_by(|a, b| {
        (b.lc.unwrap_or(0.0) - b.model_lc_after)
            .abs()
            .total_cmp(&(a.lc.unwrap_or(0.0) - a.model_lc_after).abs())
    });
    for p in pts.iter().take(6) {
        println!(
            "  {:>6.0} m  LA mes {:5.1} avant {:5.1} après {:5.1} | LC mes {:5.1} avant {:5.1} après {:5.1}",
            p.distance_m,
            p.la.unwrap_or(f64::NAN),
            p.model_la_before,
            p.model_la_after,
            p.lc.unwrap_or(f64::NAN),
            p.model_lc_before,
            p.model_lc_after
        );
    }
    for n in &r.notes {
        println!("note : {n}");
    }
    Ok(())
}
