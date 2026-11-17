use criterion::{Criterion, criterion_group, criterion_main};
use sm_core::bands::BandSet;
use sm_core::engineering::{BandSpec, PathConfig, PathSample, Scratch, path_excess};
use sm_core::gfpe::{GfpeInput, GfpePlans, run};
use sm_core::ground::{ImpedanceModel, impedance};
use std::hint::black_box;

fn benches(c: &mut Criterion) {
    let samples: Vec<PathSample> = (0..1000)
        .map(|i| PathSample {
            x: i as f64 * 5.0,
            ground: (500.0 + 20.0 * ((i as f64) / 60.0).sin()) as f32,
            obstacle: 0.0,
            canopy: if i % 7 == 0 { 12.0 } else { 0.0 },
            class: 3,
            foliage: 1.0,
        })
        .collect();
    let bands: Vec<BandSpec> = BandSet::Octave
        .bands()
        .into_iter()
        .map(BandSpec::from_band)
        .collect();
    let cfg = PathConfig::default();
    let mut sc = Scratch::default();
    let mut out = vec![0.0; bands.len()];
    c.bench_function("engineering_path_5km_9_octaves", |b| {
        b.iter(|| {
            black_box(path_excess(
                &samples, 0.8, 1.5, &bands, &cfg, &mut sc, &mut out,
            ))
        })
    });

    for f in [50.0, 200.0] {
        let z = impedance(ImpedanceModel::DelanyBazley, f, 200.0);
        let imp = move |_: f64| z;
        let ceff = |h: f64| 340.0 + 0.05 * h;
        let ground = |r: f64| 10.0 * (r / 700.0).sin();
        let mut plans = GfpePlans::default();
        c.bench_function(&format!("gfpe_{f}Hz_5km"), |b| {
            b.iter(|| {
                black_box(run(
                    &GfpeInput {
                        f,
                        c_ref: 340.0,
                        zs: 1.0,
                        range: 5000.0,
                        dr: None,
                        height: 250.0,
                        ground: &ground,
                        impedance: &imp,
                        c_eff: &ceff,
                        receivers: &[1.5, 4.0],
                        directivity: None,
                    },
                    &mut plans,
                ))
            })
        });
    }
}

criterion_group!(core, benches);
criterion_main!(core);
