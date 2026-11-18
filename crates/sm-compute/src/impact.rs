//! Impact : récepteurs en façade des habitations (1,5 m et 4 m), bruit
//! résiduel, émergence globale et par octave, seuils libres, habitants
//! exposés, lieux sensibles, faune (ZNIEFF), marge disponible.

use crate::pe::PeField;
use crate::polar::Air;
use crate::project::Project;
use crate::receiver::levels_at;
use crate::render::{Metric, Raster, weighted_level};
use crate::scene::Scene;
#[cfg(test)]
use crate::scene::point_in_rings;
use rayon::prelude::*;
use serde::Serialize;
use sm_core::engineering::{BandSpec, PathConfig, Scratch};
use sm_core::source::{Program, Source};
use sm_core::{Hz, a_weighting, c_weighting};

pub const OCTAVES: [f64; 9] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0,
];

/// Spectre type du bruit résiduel rural de nuit (dB relatifs par octave).
const RESIDUAL_SHAPE: [f64; 9] = [14.0, 10.0, 5.0, 1.0, -2.0, -5.0, -9.0, -14.0, -20.0];

/// Spectre résiduel par octave dont le niveau A vaut `l_a`.
pub fn residual_spectrum(l_a: f64) -> [f64; 9] {
    let raw_a = 10.0
        * RESIDUAL_SHAPE
            .iter()
            .zip(OCTAVES)
            .map(|(s, f)| 10f64.powf((s + a_weighting(Hz(f)).0) / 10.0))
            .sum::<f64>()
            .log10();
    let mut out = [0.0; 9];
    for (o, s) in out.iter_mut().zip(RESIDUAL_SHAPE) {
        *o = s + l_a - raw_a;
    }
    out
}

/// Bruit routier de nuit (modèle simple de source linéique) : niveau dB(A) à
/// 10 m par importance BD TOPO, décroissance 10·log10(d/10).
fn road_level_10m(importance: u8) -> f64 {
    match importance {
        1 => 58.0,
        2 => 52.0,
        3 => 46.0,
        4 => 40.0,
        _ => 34.0,
    }
}

fn dist_to_polyline(pts: &[(f64, f64)], x: f64, y: f64) -> f64 {
    let mut best = f64::MAX;
    for s in pts.windows(2) {
        let ((x1, y1), (x2, y2)) = (s[0], s[1]);
        let (dx, dy) = (x2 - x1, y2 - y1);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 0.0 {
            (((x - x1) * dx + (y - y1) * dy) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        best = best.min((x - (x1 + t * dx)).hypot(y - (y1 + t * dy)));
    }
    best
}

/// Bruit résiduel dB(A) en un point absolu L93.
pub fn residual_dba(scene: &Scene, p: &Project, x: f64, y: f64) -> f64 {
    let s = &p.settings.residual;
    if let Some(m) = s.measured_dba {
        return m;
    }
    let mut e = 10f64.powf(s.rural_night_dba / 10.0);
    if s.include_roads {
        for road in &scene.roads {
            let d = dist_to_polyline(&road.pts, x, y).max(10.0);
            if d < 1500.0 {
                e += 10f64
                    .powf((road_level_10m(road.importance) - 10.0 * (d / 10.0).log10()) / 10.0);
            }
        }
    }
    10.0 * e.log10()
}

/// Émergence (dB) : niveau total avec la sono − résiduel.
pub fn emergence(l_source: f64, l_res: f64) -> f64 {
    10.0 * (10f64.powf(l_source / 10.0) + 10f64.powf(l_res / 10.0)).log10() - l_res
}

/// Niveau source qui produirait exactement l'émergence `e` (dB).
pub fn source_level_for_emergence(l_res: f64, e: f64) -> f64 {
    l_res + 10.0 * (10f64.powf(e / 10.0) - 1.0).max(1e-12).log10()
}

/// Récepteurs en façade : milieu des façades ≥ 3 m, à 1 m à l'extérieur.
pub fn facade_points(rings: &[Vec<(f64, f64)>]) -> Vec<(f64, f64)> {
    let Some(outer) = rings.first() else {
        return Vec::new();
    };
    let area2: f64 = outer
        .windows(2)
        .map(|e| e[0].0 * e[1].1 - e[1].0 * e[0].1)
        .sum();
    let orient = if area2 >= 0.0 { 1.0 } else { -1.0 }; // + : sens trigonométrique
    let mut pts = Vec::new();
    for e in outer.windows(2) {
        let ((x1, y1), (x2, y2)) = (e[0], e[1]);
        let len = (x2 - x1).hypot(y2 - y1);
        if len < 3.0 {
            continue;
        }
        // Normale extérieure (à droite si sens trigonométrique).
        let (nx, ny) = (orient * (y2 - y1) / len, -orient * (x2 - x1) / len);
        pts.push(((x1 + x2) / 2.0 + nx, (y1 + y2) / 2.0 + ny));
    }
    if pts.is_empty()
        && let Some(&(x, y)) = outer.first()
    {
        pts.push((x, y));
    }
    pts.truncate(12);
    pts
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct BuildingImpact {
    pub id: String,
    pub lon: f64,
    pub lat: f64,
    pub usage: String,
    pub inhabitants: f64,
    /// Habitants estimés (hors carreau Filosofi peuplé).
    #[serde(default)]
    pub inhabitants_estimated: bool,
    pub dwellings: u32,
    pub distance_m: f64,
    /// Façade la plus exposée.
    pub level_dba: f64,
    pub level_dbc: f64,
    pub height_m: f64,
    pub residual_dba: f64,
    pub residual_dbc: f64,
    pub emergence_dba: f64,
    pub emergence_dbc: f64,
    /// Émergence par octave 31,5 Hz … 8 kHz.
    pub emergence_oct: Vec<f64>,
    /// Indice de classe (nombre de seuils dépassés).
    pub class: usize,
    /// Marge avant le premier seuil (dB, négative si dépassé).
    pub margin_db: f64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ClassCount {
    pub label: String,
    pub buildings: usize,
    pub inhabitants: f64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct SensitiveImpact {
    pub name: String,
    pub kind: String,
    pub lon: f64,
    pub lat: f64,
    pub level_dba: f64,
    pub emergence_dba: f64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct FaunaImpact {
    pub name: String,
    pub level: u8,
    /// Surface (km²) de la ZNIEFF dans la zone où le niveau dépasse 45 dB(A).
    pub area_above_45_km2: f64,
    pub max_dba: f64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ImpactReport {
    pub thresholds_db: Vec<f64>,
    pub buildings: Vec<BuildingImpact>,
    pub classes: Vec<ClassCount>,
    pub most_exposed: Option<BuildingImpact>,
    /// Marge globale : de combien de dB on peut monter avant que le bâtiment le
    /// plus exposé atteigne le premier seuil.
    pub margin_db: f64,
    pub inhabitants_total: f64,
    pub sensitive: Vec<SensitiveImpact>,
    pub fauna: Vec<FaunaImpact>,
    pub method: String,
    pub compute_ms: f64,
}

fn octave_levels(levels: &[f64], bands: &[BandSpec]) -> [f64; 9] {
    let mut e = [0.0f64; 9];
    for (l, b) in levels.iter().zip(bands) {
        let i = ((b.fc / 31.5).log2().round().max(0.0) as usize).min(8);
        e[i] += 10f64.powf(l / 10.0);
    }
    e.map(|v| 10.0 * v.max(1e-12).log10())
}

/// Évalue l'impact. `raster_a` (dB(A), local) sert à la couche faune.
#[allow(clippy::too_many_arguments)]
pub fn evaluate(
    scene: &Scene,
    project: &Project,
    sources: &[Source],
    program: &Program,
    bands: &[BandSpec],
    cfg: &PathConfig,
    air: Air,
    pe: Option<&PeField>,
    raster_a: Option<&Raster>,
    method: &str,
) -> ImpactReport {
    let t0 = std::time::Instant::now();
    let th = {
        let mut t = project.settings.thresholds_db.clone();
        t.sort_by(|a, b| a.total_cmp(b));
        if t.is_empty() {
            t.push(5.0);
        }
        t
    };
    let wa = Metric::A.weights(bands);
    let wc = Metric::C.weights(bands);
    let c_oct: Vec<f64> = OCTAVES.iter().map(|f| c_weighting(Hz(*f)).0).collect();
    let residents: Vec<usize> = scene
        .buildings
        .iter()
        .enumerate()
        .filter(|(_, b)| b.residential && b.centroid.0.hypot(b.centroid.1) > 0.0)
        .filter(|(_, b)| {
            let (x, y) = (b.centroid.0 - scene.origin.0, b.centroid.1 - scene.origin.1);
            x.hypot(y) < scene.radius * 0.98
        })
        .map(|(i, _)| i)
        .collect();
    let mut buildings: Vec<BuildingImpact> = residents
        .par_iter()
        .map_init(Scratch::default, |sc, &i| {
            let b = &scene.buildings[i];
            let mut best: Option<(f64, f64, f64, [f64; 9])> = None; // (A, C, h, octaves)
            for (fx, fy) in facade_points(&b.rings) {
                let (lx, ly) = (fx - scene.origin.0, fy - scene.origin.1);
                let d = lx.hypot(ly);
                let step = (d / 400.0).clamp(2.0, 10.0);
                for hr in [1.5, 4.0] {
                    let (lv, _) = levels_at(
                        scene, sources, program, bands, cfg, air, lx, ly, hr, step, pe, sc,
                    );
                    let la = weighted_level(&lv, &wa);
                    if best.is_none_or(|bb| la > bb.0) {
                        best = Some((la, weighted_level(&lv, &wc), hr, octave_levels(&lv, bands)));
                    }
                }
            }
            let (la, lc, h, oct) = best.unwrap_or((0.0, 0.0, 1.5, [0.0; 9]));
            let res_a = residual_dba(scene, project, b.centroid.0, b.centroid.1);
            let res_oct = residual_spectrum(res_a);
            let res_c = 10.0
                * res_oct
                    .iter()
                    .zip(&c_oct)
                    .map(|(l, w)| 10f64.powf((l + w) / 10.0))
                    .sum::<f64>()
                    .log10();
            let e_a = emergence(la, res_a);
            let class = th.iter().filter(|t| e_a >= **t).count();
            let margin = source_level_for_emergence(res_a, th[0]) - la;
            let (lon, lat) = scene
                .proj
                .to_wgs84(b.centroid.0, b.centroid.1)
                .unwrap_or((0.0, 0.0));
            BuildingImpact {
                id: b.id.clone(),
                lon,
                lat,
                usage: b.usage.clone(),
                inhabitants: b.inhabitants,
                inhabitants_estimated: b.inhabitants_estimated,
                dwellings: b.dwellings,
                distance_m: (b.centroid.0 - scene.origin.0).hypot(b.centroid.1 - scene.origin.1),
                level_dba: la,
                level_dbc: lc,
                height_m: h,
                residual_dba: res_a,
                residual_dbc: res_c,
                emergence_dba: e_a,
                emergence_dbc: emergence(lc, res_c),
                emergence_oct: oct
                    .iter()
                    .zip(res_oct)
                    .map(|(l, r)| emergence(*l, r))
                    .collect(),
                class,
                margin_db: margin,
            }
        })
        .collect();
    buildings.sort_by(|a, b| b.emergence_dba.total_cmp(&a.emergence_dba));

    let mut classes = Vec::new();
    let mut labels = vec![format!("< {} dB", th[0])];
    for w in th.windows(2) {
        labels.push(format!("{} – {} dB", w[0], w[1]));
    }
    labels.push(format!("≥ {} dB", th[th.len() - 1]));
    for (k, label) in labels.into_iter().enumerate() {
        let sel: Vec<&BuildingImpact> = buildings.iter().filter(|b| b.class == k).collect();
        classes.push(ClassCount {
            label,
            buildings: sel.len(),
            inhabitants: sel.iter().map(|b| b.inhabitants).sum(),
        });
    }
    let most = buildings.first().cloned();
    let margin = buildings
        .iter()
        .map(|b| b.margin_db)
        .fold(f64::INFINITY, f64::min);

    let mut sc = Scratch::default();
    let sensitive = scene
        .sensitive
        .iter()
        .map(|s| {
            let (lx, ly) = (s.pos.0 - scene.origin.0, s.pos.1 - scene.origin.1);
            let (lv, _) = levels_at(
                scene, sources, program, bands, cfg, air, lx, ly, 1.5, 5.0, pe, &mut sc,
            );
            let la = weighted_level(&lv, &wa);
            let res = residual_dba(scene, project, s.pos.0, s.pos.1);
            let (lon, lat) = scene.proj.to_wgs84(s.pos.0, s.pos.1).unwrap_or((0.0, 0.0));
            SensitiveImpact {
                name: s.name.clone(),
                kind: s.kind.clone(),
                lon,
                lat,
                level_dba: la,
                emergence_dba: emergence(la, res),
            }
        })
        .collect();

    let fauna = raster_a
        .map(|r| {
            scene
                .znieff
                .iter()
                .map(|z| {
                    let (mut n, mut max) = (0usize, f64::MIN);
                    // Masque rasterisé de la ZNIEFF sur la grille du résultat :
                    // O(pixels) au lieu d'un test point-dans-polygone par pixel.
                    let (ax0, ay0) = scene.abs(r.x0, r.y0);
                    let bbox = sm_geo::BboxL93::new(
                        ax0,
                        ay0 - r.cell * r.h as f64,
                        ax0 + r.cell * r.w as f64,
                        ay0,
                    );
                    let mut mask = crate::scene::Grid::new(&bbox, r.cell, 0u8);
                    mask.fill_polygon(&z.rings, 1);
                    for (i, v) in r.data.iter().enumerate() {
                        if mask.data.get(i) == Some(&1) && v.is_finite() {
                            max = max.max(f64::from(*v));
                            if *v >= 45.0 {
                                n += 1;
                            }
                        }
                    }
                    FaunaImpact {
                        name: z.name.clone(),
                        level: z.level,
                        area_above_45_km2: n as f64 * r.cell * r.cell / 1e6,
                        max_dba: if max > f64::MIN { max } else { f64::NAN },
                    }
                })
                .filter(|f| f.max_dba.is_finite())
                .collect()
        })
        .unwrap_or_default();

    ImpactReport {
        thresholds_db: th,
        inhabitants_total: buildings.iter().map(|b| b.inhabitants).sum(),
        buildings,
        classes,
        most_exposed: most,
        margin_db: if margin.is_finite() { margin } else { 0.0 },
        sensitive,
        fauna,
        method: method.into(),
        compute_ms: t0.elapsed().as_secs_f64() * 1e3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emergence_math() {
        // Source = résiduel → +3 dB.
        assert!((emergence(30.0, 30.0) - 3.01).abs() < 0.01);
        // Source très faible → émergence ≈ 0.
        assert!(emergence(0.0, 30.0) < 0.01);
        // Inverse exact.
        let l = source_level_for_emergence(28.0, 5.0);
        assert!((emergence(l, 28.0) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn residual_spectrum_has_requested_a_level() {
        let s = residual_spectrum(27.0);
        let la = 10.0
            * s.iter()
                .zip(OCTAVES)
                .map(|(l, f)| 10f64.powf((l + a_weighting(Hz(f)).0) / 10.0))
                .sum::<f64>()
                .log10();
        assert!((la - 27.0).abs() < 1e-9);
    }

    #[test]
    fn facade_points_are_outside() {
        let sq = vec![vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (0.0, 0.0),
        ]];
        let pts = facade_points(&sq);
        assert_eq!(pts.len(), 4);
        for (x, y) in &pts {
            assert!(!point_in_rings(&sq, *x, *y), "({x}, {y}) à l'intérieur");
        }
        // Sens horaire : mêmes points extérieurs.
        let cw = vec![sq[0].iter().rev().copied().collect::<Vec<_>>()];
        for (x, y) in facade_points(&cw) {
            assert!(!point_in_rings(&cw, x, y));
        }
    }
}
