//! Le GFPE GPU (f32) doit reproduire le GFPE CPU (f64). Ignoré sans GPU.

use num_complex::Complex64 as C64;
use sm_core::gfpe::{GfpeInput, GfpePlans, run};
use sm_core::ground::{ImpedanceModel, impedance};
use sm_gpu::GpuGfpe;
use std::time::Instant;

fn ground_flat(_: f64) -> f64 {
    0.0
}

fn ground_hills(r: f64) -> f64 {
    15.0 * (r / 400.0).sin() + 0.004 * r
}

#[test]
fn gpu_matches_cpu() {
    let Some(gpu) = GpuGfpe::new() else {
        eprintln!("aucun GPU compatible : test ignoré");
        return;
    };
    eprintln!("GPU : {}", gpu.name());
    for f in [31.5, 100.0, 200.0] {
        let z_soft = impedance(ImpedanceModel::DelanyBazley, f, 200.0);
        let z_hard = C64::new(1e6, 0.0);
        let imp_soft = move |_: f64| z_soft;
        let imp_mix = move |r: f64| {
            if (r / 500.0) as i64 % 2 == 0 {
                z_soft
            } else {
                z_hard
            }
        };
        let c_hom = |_: f64| 343.0;
        let c_down = |h: f64| 343.0 + 0.08 * h;
        let cases: Vec<(&str, GfpeInput)> = vec![
            (
                "plat, sol mou",
                GfpeInput {
                    f,
                    c_ref: 343.0,
                    zs: 2.0,
                    range: 3000.0,
                    dr: None,
                    height: 250.0,
                    ground: &ground_flat,
                    impedance: &imp_soft,
                    c_eff: &c_hom,
                    receivers: &[2.0],
                    directivity: None,
                },
            ),
            (
                "réfraction vers le bas",
                GfpeInput {
                    f,
                    c_ref: 343.0,
                    zs: 2.0,
                    range: 3000.0,
                    dr: None,
                    height: 250.0,
                    ground: &ground_flat,
                    impedance: &imp_soft,
                    c_eff: &c_down,
                    receivers: &[2.0],
                    directivity: None,
                },
            ),
            (
                "relief + sol mixte",
                GfpeInput {
                    f,
                    c_ref: 343.0,
                    zs: 2.0,
                    range: 3000.0,
                    dr: None,
                    height: 250.0,
                    ground: &ground_hills,
                    impedance: &imp_mix,
                    c_eff: &c_down,
                    receivers: &[2.0],
                    directivity: None,
                },
            ),
        ];
        let inputs: Vec<GfpeInput> = cases.iter().map(|(_, i)| GfpeInput { ..*i }).collect();
        let t = Instant::now();
        let gpu_out = gpu.run_batch(&inputs, &|| false).expect("GPU");
        let t_gpu = t.elapsed();
        let t = Instant::now();
        let mut plans = GfpePlans::default();
        let cpu_out: Vec<_> = inputs.iter().map(|i| run(i, &mut plans)).collect();
        let t_cpu = t.elapsed();
        for ((name, _), (g, c)) in cases.iter().zip(gpu_out.iter().zip(&cpu_out)) {
            assert_eq!(g.r.len(), c.r.len());
            // Écart en énergie moyenné sur des fenêtres de 100 m (les creux
            // d'interférence profonds amplifient l'erreur relative en dB).
            let win = (100.0 / c.dr).ceil().max(1.0) as usize;
            let mut worst: f64 = 0.0;
            for (wg, wc) in g.dl[0].chunks(win).zip(c.dl[0].chunks(win)) {
                let e = |v: &[f32]| {
                    10.0 * (v
                        .iter()
                        .map(|l| 10f64.powf(f64::from(*l) / 10.0))
                        .sum::<f64>()
                        / v.len() as f64)
                        .log10()
                };
                worst = worst.max((e(wg) - e(wc)).abs());
            }
            eprintln!("{f} Hz, {name} : écart max {worst:.3} dB (GPU {t_gpu:?}, CPU {t_cpu:?})");
            assert!(worst < 0.5, "{f} Hz, {name} : écart {worst:.2} dB");
        }
    }
}
