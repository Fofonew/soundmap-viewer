//! Simulation de bout en bout sur une zone téléchargée (mesures de performance).
//!
//! `cargo run --release -p sm-compute --example simulate -- zones/<id> [--pe] [--opt] [--out dossier]`

use sm_compute::impact;
use sm_compute::meteo::default_scenarios;
use sm_compute::optimize::{OptimizeSpec, optimize, position_count};
use sm_compute::pe::{envelope, run_pe, task_count};
use sm_compute::polar::{Air, Engine, ExcessKey, PolarSpec, combine, compute_excess};
use sm_compute::project::{Project, SpeakerSpec, group_heights};
use sm_compute::render::{Metric, isophones, polar_weighted, rasterize, stats, to_png};
use sm_compute::scene::Scene;
use sm_compute::{CancelFlag, ProgressCounter, export, sizing};
use sm_core::bands::BandSet;
use sm_core::engineering::{BandSpec, Condition, PathConfig};
use sm_core::source::{preset, program};
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(args.first().ok_or("usage : simulate <dossier de zone>")?);
    let with_pe = args.iter().any(|a| a == "--pe");
    let with_opt = args.iter().any(|a| a == "--opt");
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);

    let t = Instant::now();
    let scene = Scene::load(&dir)?;
    println!(
        "Scène « {} » : {} bâtiments ({} habitations, {:.0} habitants), {} lieux sensibles, {} ZNIEFF, sol : {} — {:.0} ms",
        scene.name,
        scene.buildings.len(),
        scene.buildings.iter().filter(|b| b.residential).count(),
        scene.buildings.iter().map(|b| b.inhabitants).sum::<f64>(),
        scene.sensitive.len(),
        scene.znieff.len(),
        scene.landcover_source,
        t.elapsed().as_secs_f64() * 1e3
    );

    // Système type free party : 4 subs 18" en ligne + 2 têtes, orienté au sud-est.
    let mut project = Project::new("Test", &scene.zone_id);
    let (lon0, lat0) = scene.local_to_lonlat(0.0, 0.0).ok_or("origine")?;
    let az = 135.0f64;
    let (ax, ay) = (az.to_radians().sin(), az.to_radians().cos());
    for i in 0..4 {
        let off = (i as f64 - 1.5) * 0.8;
        let (lon, lat) = scene.local_to_lonlat(ay * off, -ax * off).ok_or("pos")?;
        project.speakers.push(SpeakerSpec {
            id: format!("sub{i}"),
            name: format!("Sub {}", i + 1),
            model: preset("sub18").ok_or("preset")?,
            lon,
            lat,
            height_m: 0.5,
            azimuth_deg: az,
            tilt_deg: 0.0,
            delay_ms: 0.0,
            polarity: 1,
            gain_db: 0.0,
            muted: false,
        });
    }
    for (i, side) in [-3.0, 3.0].iter().enumerate() {
        let (lon, lat) = scene
            .local_to_lonlat(ay * side + ax * 0.5, -ax * side + ay * 0.5)
            .ok_or("pos")?;
        project.speakers.push(SpeakerSpec {
            id: format!("top{i}"),
            name: format!("Top {}", i + 1),
            model: preset("top15").ok_or("preset")?,
            lon,
            lat,
            height_m: 2.5,
            azimuth_deg: az,
            tilt_deg: 0.0,
            delay_ms: 0.0,
            polarity: 1,
            gain_db: 0.0,
            muted: false,
        });
    }
    let (dlon, dlat) = scene.local_to_lonlat(ax * 15.0, ay * 15.0).ok_or("df")?;
    let d = 0.0001;
    project.dancefloor = Some(vec![
        [dlon - d, dlat - d],
        [dlon + d, dlat - d],
        [dlon + d, dlat + d],
        [dlon - d, dlat + d],
    ]);
    let _ = (lon0, lat0);

    let sources = project.sources(&scene);
    let prog = program(&project.settings.program);
    let octaves: Vec<BandSpec> = BandSet::Octave
        .bands()
        .into_iter()
        .map(BandSpec::from_band)
        .collect();
    let cfg = project.path_config(&scene);
    let air = Air {
        t_c: 10.0,
        rh: 80.0,
    };
    let spec = PolarSpec::preview(scene.radius);

    // Aperçu : 1er calcul (profils + excès), puis déplacement d'une enceinte (incrémental).
    let mut engine = Engine::default();
    let (grid, t1) = engine.compute(&scene, spec, &sources, &prog, &octaves, cfg, 1.5, air);
    println!(
        "\nAperçu ingénierie {}×{} récepteurs, 9 octaves :",
        spec.n_az,
        spec.n_r()
    );
    println!(
        "  1er calcul : profils {:.0} ms + excès {:.0} ms + sources {:.0} ms = {:.0} ms",
        t1.profiles_ms,
        t1.excess_ms,
        t1.combine_ms,
        t1.profiles_ms + t1.excess_ms + t1.combine_ms
    );
    let mut moved = project.clone();
    moved.speakers[4].azimuth_deg += 20.0;
    let s2 = moved.sources(&scene);
    let (_, t2) = engine.compute(&scene, spec, &s2, &prog, &octaves, cfg, 1.5, air);
    println!(
        "  après déplacement d'une enceinte : {:.0} ms (profils en cache : {}, excès en cache : {})",
        t2.profiles_ms + t2.excess_ms + t2.combine_ms,
        t2.cached_profiles,
        t2.cached_excess
    );
    let t = Instant::now();
    let fa = polar_weighted(&grid, Metric::A);
    let ra = rasterize(&fa, &grid, 700);
    let png = to_png(&ra, 0.0)?;
    let iso = isophones(&scene, &ra, &[35.0, 40.0, 45.0, 50.0, 60.0, 70.0])?;
    let st = stats(&ra, 50.0, &[35.0, 45.0, 55.0]);
    println!(
        "  rendu raster 700² + PNG ({:.0} ko) + isophones : {:.0} ms ; max hors 50 m {:.1} dB(A), surfaces {:?}",
        png.len() as f64 / 1e3,
        t.elapsed().as_secs_f64() * 1e3,
        st.max_db,
        st.area_above
    );

    // Impact (ingénierie seule).
    let thirds: Vec<BandSpec> = BandSet::ThirdOctave
        .bands()
        .into_iter()
        .map(BandSpec::from_band)
        .collect();
    let rep = impact::evaluate(
        &scene,
        &project,
        &sources,
        &prog,
        &octaves,
        &cfg,
        air,
        None,
        Some(&ra),
        "ingénierie",
    );
    println!(
        "\nImpact (ingénierie, favorable) : {:.0} ms",
        rep.compute_ms
    );
    for c in &rep.classes {
        println!(
            "  {:<12} {:>4} bâtiments, {:>6.0} habitants",
            c.label, c.buildings, c.inhabitants
        );
    }
    if let Some(b) = &rep.most_exposed {
        println!(
            "  plus exposé : {:.0} m, {:.1} dB(A) / {:.1} dB(C), résiduel {:.1}, émergence {:+.1} dB(A) {:+.1} dB(C), marge {:+.1} dB",
            b.distance_m,
            b.level_dba,
            b.level_dbc,
            b.residual_dba,
            b.emergence_dba,
            b.emergence_dbc,
            b.margin_db
        );
    }
    println!(
        "  marge globale {:+.1} dB ; lieux sensibles {} ; ZNIEFF exposées {}",
        rep.margin_db,
        rep.sensitive.len(),
        rep.fauna.len()
    );

    if let Some(z) = sizing::evaluate(&scene, &project, &sources, &prog, &octaves, &cfg, air) {
        println!(
            "\nDancefloor : {:.1} dB(A) / {:.1} dB(C) ; cible {:.0}/{:.0} → {} subs, {} têtes, {:.0} W ; homogénéité σ {:.1} dB",
            z.center_dba,
            z.center_dbc,
            z.target_dba,
            z.target_dbc,
            z.subs_needed,
            z.tops_needed,
            z.power_w,
            z.uniformity_std_db
        );
    }

    // GFPE : 4 scénarios, tiers d'octave 25–200 Hz, 2 fréquences par bande.
    if with_pe {
        let crit = rep.most_exposed.as_ref().map_or(0.0, |b| {
            let (x, y) = scene.lonlat_to_local(b.lon, b.lat).unwrap_or((0.0, 1.0));
            x.atan2(y).to_degrees().rem_euclid(360.0)
        });
        let scen = default_scenarios(&scene, project.settings.meteo_hour, crit);
        let (p, _) = engine.profiles(&scene, spec);
        let (hs_sub, hs_top) = group_heights(&sources);
        let mut grids = Vec::new();
        for sc in &scen {
            let t = Instant::now();
            let prog_c = ProgressCounter::new(task_count(spec.n_az, &thirds, 2));
            let pe = run_pe(
                &p,
                &sources,
                &thirds,
                &sc.profile,
                1.5,
                &cfg,
                2,
                &CancelFlag::new(),
                &prog_c,
            )
            .ok_or("annulé")?;
            let t_pe = t.elapsed().as_secs_f64();
            let cfg_s = PathConfig {
                condition: if sc.id == "neutre" {
                    Condition::Homogeneous
                } else {
                    Condition::Favorable
                },
                ..cfg
            };
            let key = ExcessKey {
                hs_sub,
                hs_top,
                hr: 1.5,
                cfg: cfg_s,
                n_bands: thirds.len(),
                band0: 25.0,
            };
            let ex = compute_excess(&p, key, &thirds);
            let air_s = Air {
                t_c: sc.profile.ground_temperature(),
                rh: sc.profile.rh,
            };
            let g = combine(
                &scene,
                &p,
                &ex,
                &sources,
                &prog,
                &thirds,
                air_s,
                Some((&pe.data, &pe.weights)),
            );
            let fa = polar_weighted(&g, Metric::C);
            let rc = rasterize(&fa, &g, 500);
            let st = stats(&rc, 50.0, &[55.0, 65.0]);
            println!(
                "\nGFPE « {} » [{}] : {} calculs en {:.1} s (+ fusion {:.1} s) ; max dB(C) hors 50 m {:.1} ; surfaces ≥55/65 dB(C) {:?}",
                sc.label,
                pe.backend,
                prog_c.snapshot().total,
                t_pe,
                t.elapsed().as_secs_f64() - t_pe,
                st.max_db,
                st.area_above
            );
            println!(
                "  profil : u* {:.2} m/s, T* {:.3} K, 1/L {:.4} m⁻¹, écarts {:.2} m/s / {:.2} K",
                sc.profile.u_star,
                sc.profile.t_star,
                sc.profile.inv_l,
                sc.profile.rms_u,
                sc.profile.rms_t
            );
            grids.push(g);
        }
        if let Some((med, p90)) = envelope(&grids) {
            let m = rasterize(&polar_weighted(&med, Metric::C), &med, 500);
            let q = rasterize(&polar_weighted(&p90, Metric::C), &p90, 500);
            println!(
                "\nEnveloppe : max dB(C) médiane {:.1}, P90 {:.1}",
                stats(&m, 50.0, &[]).max_db,
                stats(&q, 50.0, &[]).max_db
            );
        }
    }

    if with_opt {
        let os = OptimizeSpec::default();
        let pc = ProgressCounter::new(position_count(&os));
        let t = Instant::now();
        let cands = optimize(
            &scene,
            &project,
            &sources,
            &prog,
            &octaves,
            &cfg,
            air,
            &os,
            &CancelFlag::new(),
            &pc,
        );
        println!(
            "\nOptimiseur : {} positions × {} azimuts × {} configurations en {:.1} s",
            position_count(&os),
            (360.0 / os.azimuth_step_deg) as usize,
            os.configs.len(),
            t.elapsed().as_secs_f64()
        );
        for c in cands.iter().take(3) {
            println!(
                "  Δ({:+.0}, {:+.0}) m, azimut {:.0}°, {:?} : émergence max {:+.1} dB(A), marge {:+.1}, dancefloor {:.1} dB(A) / {:.1} dB(C)",
                c.dx,
                c.dy,
                c.azimuth_deg,
                c.config,
                c.worst_emergence_dba,
                c.margin_db,
                c.dancefloor_dba,
                c.dancefloor_dbc
            );
        }
    }

    if let Some(out) = out {
        std::fs::create_dir_all(&out)?;
        let f = std::fs::File::create(out.join("dba.tif"))?;
        export::geotiff(&scene, &ra, std::io::BufWriter::new(f))?;
        std::fs::write(
            out.join("isophones.geojson"),
            serde_json::to_vec(&export::geojson(&iso, Some(&rep)))?,
        )?;
        let html = export::html_report(
            &project,
            &scene,
            Some(&rep),
            None,
            Some(&png),
            "ingénierie",
            "±3 dB < 500 m, ±5–10 dB au-delà de nuit",
        );
        std::fs::write(out.join("rapport.html"), html)?;
        std::fs::write(out.join("dba.png"), &png)?;
        println!("\nExports écrits dans {}", out.display());
    }
    Ok(())
}
