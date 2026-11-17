//! Dimensionnement inverse : dancefloor + niveau cible → nombre de boîtes,
//! puissance, marge (headroom), homogénéité de couverture.

use crate::polar::Air;
use crate::project::Project;
use crate::receiver::levels_at;
use crate::render::{Metric, weighted_level};
use crate::scene::{Scene, point_in_rings};
use serde::Serialize;
use sm_core::engineering::{BandSpec, PathConfig, Scratch};
use sm_core::source::{Program, Source, SpeakerKind, preset};

#[derive(Debug, Clone, Serialize)]
pub struct SizingReport {
    pub target_dba: f64,
    pub target_dbc: f64,
    /// Niveaux au centre du dancefloor avec les réglages actuels.
    pub center_dba: f64,
    pub center_dbc: f64,
    /// Niveaux max atteignables (gains à 0 dB) avec le système actuel.
    pub max_dba: f64,
    pub max_dbc: f64,
    pub subs_now: usize,
    pub tops_now: usize,
    pub subs_needed: usize,
    pub tops_needed: usize,
    pub power_w: f64,
    pub headroom_dba: f64,
    pub headroom_dbc: f64,
    /// Homogénéité dB(A) sur le dancefloor : écart-type et écart max − min.
    pub uniformity_std_db: f64,
    pub uniformity_range_db: f64,
    pub samples: usize,
    pub notes: Vec<String>,
}

fn level_center(
    scene: &Scene,
    sources: &[Source],
    program: &Program,
    bands: &[BandSpec],
    cfg: &PathConfig,
    air: Air,
    c: (f64, f64),
) -> (f64, f64) {
    let mut sc = Scratch::default();
    let (lv, _) = levels_at(
        scene, sources, program, bands, cfg, air, c.0, c.1, 1.5, 1.0, None, &mut sc,
    );
    (
        weighted_level(&lv, &Metric::A.weights(bands)),
        weighted_level(&lv, &Metric::C.weights(bands)),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn evaluate(
    scene: &Scene,
    project: &Project,
    sources: &[Source],
    program: &Program,
    bands: &[BandSpec],
    cfg: &PathConfig,
    air: Air,
) -> Option<SizingReport> {
    let poly = project.dancefloor_local(scene)?;
    let n = poly.len() as f64;
    let center = (
        poly.iter().map(|p| p.0).sum::<f64>() / n,
        poly.iter().map(|p| p.1).sum::<f64>() / n,
    );
    let target = &project.settings.target;
    let mut notes = Vec::new();

    let (center_dba, center_dbc) = level_center(scene, sources, program, bands, cfg, air, center);
    let at_max: Vec<Source> = sources
        .iter()
        .map(|s| Source {
            gain_db: 0.0,
            ..s.clone()
        })
        .collect();
    let (max_dba, max_dbc) = level_center(scene, &at_max, program, bands, cfg, air, center);

    let group = |sub: bool| -> Vec<Source> {
        at_max
            .iter()
            .filter(|s| (s.model.kind == SpeakerKind::Sub) == sub)
            .cloned()
            .collect()
    };
    let mut subs = group(true);
    let mut tops = group(false);
    let (subs_now, tops_now) = (subs.len(), tops.len());
    let reference = |id: &str, h: f64| -> Option<Source> {
        Some(Source {
            x: 0.0,
            y: 0.0,
            z: scene.ground_local(0.0, 0.0) + h,
            height: h,
            azimuth_deg: center.0.atan2(center.1).to_degrees(),
            tilt_deg: 0.0,
            delay_s: 0.0,
            polarity: 1.0,
            gain_db: 0.0,
            model: preset(id)?,
        })
    };
    if subs.is_empty() {
        notes.push("Aucun sub : calcul avec un sub 18\" de référence au point sono.".into());
        subs.extend(reference("sub18", 0.5));
    }
    if tops.is_empty() {
        notes.push("Aucune tête : calcul avec un top 15\" de référence au point sono.".into());
        tops.extend(reference("top15", 2.5));
    }
    let (_, sub_c) = level_center(scene, &subs, program, bands, cfg, air, center);
    let (top_a, _) = level_center(scene, &tops, program, bands, cfg, air, center);
    let n_sub = subs.len().max(1) as f64;
    let n_top = tops.len().max(1) as f64;
    // Subs serrés : somme cohérente en basses fréquences (+6 dB par doublement).
    let subs_needed = (n_sub * 10f64.powf((target.dbc - sub_c) / 20.0))
        .ceil()
        .max(1.0);
    // Têtes : somme énergétique (+3 dB par doublement).
    let tops_needed = (n_top * 10f64.powf((target.dba - top_a) / 10.0))
        .ceil()
        .max(1.0);
    let power = subs_needed * subs.first().map_or(0.0, |s| s.model.power_w)
        + tops_needed * tops.first().map_or(0.0, |s| s.model.power_w);
    let headroom_dbc = sub_c + 20.0 * (subs_needed / n_sub).log10() - target.dbc;
    let headroom_dba = top_a + 10.0 * (tops_needed / n_top).log10() - target.dba;

    // Homogénéité : grille de 4 m dans le polygone.
    let (mut xmin, mut xmax, mut ymin, mut ymax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in &poly {
        xmin = xmin.min(p.0);
        xmax = xmax.max(p.0);
        ymin = ymin.min(p.1);
        ymax = ymax.max(p.1);
    }
    let step = ((xmax - xmin) * (ymax - ymin) / 300.0).sqrt().max(2.0);
    let mut ring = poly.clone();
    if let Some(&f) = poly.first() {
        ring.push(f);
    }
    let rings = vec![ring];
    let mut vals = Vec::new();
    let mut sc = Scratch::default();
    let mut y = ymin + step / 2.0;
    while y < ymax {
        let mut x = xmin + step / 2.0;
        while x < xmax {
            if point_in_rings(&rings, x, y) {
                let (lv, _) = levels_at(
                    scene, sources, program, bands, cfg, air, x, y, 1.5, 1.0, None, &mut sc,
                );
                vals.push(weighted_level(&lv, &Metric::A.weights(bands)));
            }
            x += step;
        }
        y += step;
    }
    let (std, range) = if vals.len() > 1 {
        let m = vals.iter().sum::<f64>() / vals.len() as f64;
        let s = (vals.iter().map(|v| (v - m).powi(2)).sum::<f64>() / vals.len() as f64).sqrt();
        let (lo, hi) = vals
            .iter()
            .fold((f64::MAX, f64::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
        (s, hi - lo)
    } else {
        (0.0, 0.0)
    };
    if range > 10.0 {
        notes.push("Couverture peu homogène (> 10 dB) : rapprocher ou répartir les têtes.".into());
    }
    Some(SizingReport {
        target_dba: target.dba,
        target_dbc: target.dbc,
        center_dba,
        center_dbc,
        max_dba,
        max_dbc,
        subs_now,
        tops_now,
        subs_needed: subs_needed as usize,
        tops_needed: tops_needed as usize,
        power_w: power,
        headroom_dba,
        headroom_dbc,
        uniformity_std_db: std,
        uniformity_range_db: range,
        samples: vals.len(),
        notes,
    })
}
