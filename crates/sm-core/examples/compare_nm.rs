//! Comparaison de la méthode d'ingénierie SoundMap avec NoiseModelling
//! (CNOSSOS-EU) sur les scènes de `tests/noisemodelling/`.
//!
//! 1. `tests/noisemodelling/run.sh` produit `out/<scene>_<H|F>.csv` ;
//! 2. `cargo run --release -p sm-core --example compare_nm [dossier_out]`.
//!
//! Géométrie : source omnidirectionnelle à 2 m, Lw = 100 dB par octave,
//! récepteurs à 4 m, sol plat, 15 °C, 70 % HR, 101,325 kPa.

use sm_core::atmosphere::iso9613_1_alpha;
use sm_core::bands::BandSet;
use sm_core::engineering::{BandSpec, Condition, PathConfig, PathSample, Scratch, path_excess};
use sm_core::ground::GroundClass;
use sm_core::units::Hz;
use sm_core::weighting::a_weighting;
use std::error::Error;
use std::path::PathBuf;

const LW: f64 = 100.0;
const HS: f64 = 2.0;
const HR: f64 = 4.0;
const X0: f64 = 700_000.0;
const NM_BANDS: [f64; 8] = [63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0];

struct NmRow {
    d: f64,
    bands: [f64; 8],
    laeq: f64,
}

fn read_nm(path: &PathBuf) -> Result<Vec<NmRow>, Box<dyn Error>> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header: Vec<String> = lines
        .next()
        .ok_or("CSV vide")?
        .split(',')
        .map(|s| s.trim_matches('"').to_string())
        .collect();
    let col = |name: &str| {
        header
            .iter()
            .position(|h| h == name)
            .ok_or_else(|| format!("colonne {name} absente"))
    };
    let (i_period, i_geom, i_laeq) = (col("PERIOD")?, col("THE_GEOM")?, col("LAEQ")?);
    let i_bands: Vec<usize> = NM_BANDS
        .iter()
        .map(|f| col(&format!("HZ{f}")))
        .collect::<Result<_, _>>()?;
    let mut rows = Vec::new();
    for line in lines {
        let f: Vec<&str> = line.split(',').map(|s| s.trim_matches('"')).collect();
        if f.get(i_period) != Some(&"D") {
            continue;
        }
        let geom = f.get(i_geom).ok_or("géométrie absente")?;
        let x: f64 = geom
            .split('(')
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .ok_or("géométrie illisible")?
            .parse()?;
        let mut bands = [0.0; 8];
        for (b, &i) in bands.iter_mut().zip(&i_bands) {
            *b = f.get(i).ok_or("bande absente")?.parse()?;
        }
        rows.push(NmRow {
            d: x - X0,
            bands,
            laeq: f.get(i_laeq).ok_or("LAEQ absent")?.parse()?,
        });
    }
    rows.sort_by(|a, b| a.d.total_cmp(&b.d));
    Ok(rows)
}

fn profile(scene: &str, d: f64) -> Vec<PathSample> {
    let class = if scene == "flat_hard" {
        GroundClass::H
    } else {
        GroundClass::D
    };
    let step = 1.0;
    (0..=(d / step).round() as usize)
        .map(|i| {
            let x = i as f64 * step;
            let wall = scene == "barrier" && (80.0..=100.0).contains(&x);
            PathSample {
                x,
                ground: 0.0,
                obstacle: if wall { 10.0 } else { 0.0 },
                canopy: 0.0,
                class: class.index(),
                foliage: 1.0,
            }
        })
        .collect()
}

fn soundmap_levels(scene: &str, cond: Condition, d: f64, bands: &[BandSpec]) -> [f64; 8] {
    let cfg = PathConfig {
        condition: cond,
        ..PathConfig::default()
    };
    let mut ex = vec![0.0; bands.len()];
    path_excess(
        &profile(scene, d),
        HS,
        HR,
        bands,
        &cfg,
        &mut Scratch::default(),
        &mut ex,
    );
    let r = d.hypot(HR - HS);
    let mut out = [0.0; 8];
    for (o, fc) in out.iter_mut().zip(NM_BANDS) {
        let (i, b) = bands
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1.fc - fc).abs().total_cmp(&(b.1.fc - fc).abs()))
            .unwrap_or((0, &bands[0]));
        let alpha = iso9613_1_alpha(b.fc, 15.0, 70.0, 101.325);
        *o = LW - 20.0 * r.log10() - 11.0 - alpha * r + ex[i];
    }
    out
}

fn laeq(levels: &[f64; 8]) -> f64 {
    let e: f64 = levels
        .iter()
        .zip(NM_BANDS)
        .map(|(l, f)| 10f64.powf((l + a_weighting(Hz(f)).0) / 10.0))
        .sum();
    10.0 * e.log10()
}

/// Écart de LAeq (SoundMap − NoiseModelling) par scène, condition et distance.
pub struct Gap {
    pub scene: &'static str,
    pub favorable: bool,
    pub d: f64,
    pub dlaeq: f64,
}

/// Compare toutes les scènes ; imprime les tableaux Markdown si `print`.
pub fn compare(dir: &std::path::Path, print: bool) -> Result<Vec<Gap>, Box<dyn Error>> {
    let bands: Vec<BandSpec> = BandSet::Octave
        .bands()
        .into_iter()
        .map(BandSpec::from_band)
        .collect();
    let mut all = Vec::new();
    for scene in ["flat_soft", "flat_hard", "barrier"] {
        for (tag, cond) in [("H", Condition::Homogeneous), ("F", Condition::Favorable)] {
            let rows = read_nm(&dir.join(format!("{scene}_{tag}.csv")))?;
            if print {
                println!(
                    "
### {scene} — {tag} (écart SoundMap − NoiseModelling, dB)
"
                );
                println!(
                    "| d (m) | 63 | 125 | 250 | 500 | 1k | 2k | 4k | 8k | LAeq NM | LAeq SM | Δ LAeq |"
                );
                println!("|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
            }
            for row in &rows {
                let sm = soundmap_levels(scene, cond, row.d, &bands);
                let (la_nm, la_sm) = (row.laeq, laeq(&sm));
                all.push(Gap {
                    scene,
                    favorable: tag == "F",
                    d: row.d,
                    dlaeq: la_sm - la_nm,
                });
                if print {
                    let cells: Vec<String> = sm
                        .iter()
                        .zip(&row.bands)
                        .map(|(s, n)| format!("{:+.1}", s - n))
                        .collect();
                    println!(
                        "| {:.0} | {} | {la_nm:.1} | {la_sm:.1} | {:+.1} |",
                        row.d,
                        cells.join(" | "),
                        la_sm - la_nm
                    );
                }
            }
        }
    }
    Ok(all)
}

#[allow(dead_code)]
fn main() -> Result<(), Box<dyn Error>> {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tests/noisemodelling/out"));
    let gaps = compare(&dir, true)?;
    for fav in [false, true] {
        let worst = gaps
            .iter()
            .filter(|g| g.favorable == fav)
            .fold(0.0f64, |m, g| m.max(g.dlaeq.abs()));
        let label = if fav { "favorables" } else { "homogènes" };
        println!(
            "
Pire |Δ LAeq|, conditions {label} : {worst:.1} dB"
        );
    }
    Ok(())
}
