//! Assistant « choix de spot » sur un grand rayon (jusqu'à 30 km), avant tout
//! téléchargement de zone : MNT grossier (RGE ALTI, 100 m) et population
//! Filosofi (carreaux 200 m), modèle de criblage à 63 Hz.
//!
//! Pour chaque candidat d'une grille (500 m par défaut) :
//! L_C(d) = L₁ − 20 lg d + 3 dB (sol) − A_écran, A_écran = 10 lg(3 + 20N),
//! N = 2δ/λ (Maekawa, λ = 5,4 m), δ calculé sur le profil du MNT avec des
//! rayons courbes de conditions favorables (abaissement x(d−x)/2Rc,
//! Rc = max(1000, 8d), comme la méthode d'ingénierie), plafonné à 20 dB.
//! Score : habitants exposés au-delà du seuil, puis distance au premier
//! habitant ; pente maximale imposée. C'est un criblage : la zone retenue doit
//! ensuite être téléchargée et calculée finement.

use crate::{CancelFlag, ComputeError, ProgressCounter};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sm_data::wmsr::{ElevationGrid, ElevationLayer, get_elevation};
use sm_data::{BboxL93, CancellationToken, GeoClient};
use sm_geo::proj::Lambert93;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpotSpec {
    /// Rayon de recherche (m), 30 km au plus.
    pub radius_m: f64,
    /// Pas de la grille de candidats (m).
    pub grid_m: f64,
    /// Niveau du système ramené à 1 m (dB(C)) : 4 subs 18" ≈ 125.
    pub source_dbc_1m: f64,
    /// Seuil d'exposition chez les habitants (dB(C)).
    pub threshold_dbc: f64,
    /// Pente maximale du terrain au spot (%).
    pub max_slope_pct: f64,
    /// Distance d'influence considérée (m).
    pub influence_m: f64,
}

impl Default for SpotSpec {
    fn default() -> Self {
        Self {
            radius_m: 30_000.0,
            grid_m: 500.0,
            source_dbc_1m: 125.0,
            threshold_dbc: 50.0,
            max_slope_pct: 8.0,
            influence_m: 6_000.0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Spot {
    pub lon: f64,
    pub lat: f64,
    /// Habitants au-delà du seuil (estimation de criblage).
    pub exposed: f64,
    /// Distance au carreau habité le plus proche (m).
    pub nearest_m: f64,
    /// Niveau maximal chez un habitant (dB(C)).
    pub max_dbc: f64,
    pub slope_pct: f64,
    pub elevation_m: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpotReport {
    /// Meilleurs spots (séparés d'au moins 2 km).
    pub spots: Vec<Spot>,
    /// Tous les candidats évalués (pour la carte).
    pub candidates: Vec<Spot>,
    pub populated_cells: usize,
    pub population: f64,
    pub seconds: f64,
}

struct Dem {
    g: ElevationGrid,
    cell: f64,
}

impl Dem {
    fn z(&self, x: f64, y: f64) -> Option<f64> {
        let b = &self.g.bbox;
        let fx = (x - b.xmin) / self.cell - 0.5;
        let fy = (b.ymax - y) / self.cell - 0.5;
        if fx < 0.0 || fy < 0.0 {
            return None;
        }
        let (i, j) = (fx.floor() as usize, fy.floor() as usize);
        if i + 1 >= self.g.width || j + 1 >= self.g.height {
            return None;
        }
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        let v = |i: usize, j: usize| {
            let v = f64::from(self.g.data[j * self.g.width + i]);
            (v > -500.0 && v < 9000.0).then_some(v)
        };
        let (a, b2, c, d) = (v(i, j)?, v(i + 1, j)?, v(i, j + 1)?, v(i + 1, j + 1)?);
        Some((a * (1.0 - tx) + b2 * tx) * (1.0 - ty) + (c * (1.0 - tx) + d * tx) * ty)
    }
}

/// Atténuation d'écran (dB) par le relief entre deux points (hauteurs 2 m).
fn barrier_db(dem: &Dem, a: (f64, f64), b: (f64, f64), step: f64) -> Option<f64> {
    let d = (b.0 - a.0).hypot(b.1 - a.1);
    let n = (d / step).ceil().max(2.0) as usize;
    let za = dem.z(a.0, a.1)? + 2.0;
    let zb = dem.z(b.0, b.1)? + 2.0;
    let rc = (8.0 * d).max(1000.0);
    // Point le plus haut au-dessus de la droite (relief abaissé).
    let mut best: Option<(f64, f64)> = None;
    let mut best_h = 0.0;
    for k in 1..n {
        let t = k as f64 / n as f64;
        let x = t * d;
        let Some(z) = dem.z(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t) else {
            continue;
        };
        let zc = z - x * (d - x) / (2.0 * rc);
        let line = za + (zb - za) * t;
        if zc - line > best_h {
            best_h = zc - line;
            best = Some((x, zc));
        }
    }
    let Some((x, z)) = best else { return Some(0.0) };
    let delta = x.hypot(z - za) + (d - x).hypot(zb - z) - d.hypot(zb - za);
    let fresnel = 2.0 * delta / 5.4;
    Some((10.0 * (3.0 + 20.0 * fresnel).log10()).min(20.0))
}

/// Cherche les meilleurs spots autour de (lon, lat).
pub async fn find_spots(
    client: &GeoClient,
    lon: f64,
    lat: f64,
    spec: SpotSpec,
    progress: &ProgressCounter,
    cancel: &CancelFlag,
) -> Result<SpotReport, ComputeError> {
    let t0 = std::time::Instant::now();
    let token = CancellationToken::new();
    let proj = Lambert93::new()?;
    let (cx, cy) = proj.from_wgs84(lon, lat)?;
    let r = spec.radius_m.clamp(2_000.0, 30_000.0);
    let margin = spec.influence_m;
    let bbox = BboxL93::new(
        cx - r - margin,
        cy - r - margin,
        cx + r + margin,
        cy + r + margin,
    );
    progress.set_total(3);
    // MNT 100 m (RGE ALTI : couverture nationale).
    let px = ((bbox.width() / 100.0).ceil() as usize).min(1000);
    let g = get_elevation(client, ElevationLayer::RgeAlti, &bbox, px, px, &token).await?;
    let dem = Dem {
        cell: bbox.width() / px as f64,
        g,
    };
    progress.inc(1);
    if cancel.is_cancelled() {
        return Err(ComputeError::Cancelled);
    }
    // Population (centres des carreaux habités).
    let feats =
        sm_data::wfs::get_all(client, sm_data::wfs::layers::FILOSOFI, &bbox, &token).await?;
    let mut cells: Vec<(f64, f64, f64)> = Vec::new();
    for f in &feats {
        let ind = f
            .pointer("/properties/ind")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        // Premier anneau, quelle que soit l'imbrication (Polygon, MultiPolygon).
        let mut v = f.pointer("/geometry/coordinates");
        while let Some(a) = v.and_then(serde_json::Value::as_array)
            && a.first()
                .and_then(|x| x.get(0))
                .is_some_and(serde_json::Value::is_array)
        {
            v = a.first();
        }
        let Some(ring) = v.and_then(serde_json::Value::as_array) else {
            continue;
        };
        let pts: Vec<(f64, f64)> = ring
            .iter()
            .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
            .collect();
        if ind <= 0.0 || pts.is_empty() {
            continue;
        }
        let n = pts.len() as f64;
        let (sx, sy) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
        cells.push((sx / n, sy / n, ind));
    }
    progress.inc(1);
    tracing::info!(carreaux = feats.len(), habités = cells.len(), "Filosofi");
    // Index spatial des carreaux (seaux de 2 km).
    const B: f64 = 2_000.0;
    let mut buckets: std::collections::HashMap<(i64, i64), Vec<usize>> =
        std::collections::HashMap::new();
    for (i, c) in cells.iter().enumerate() {
        buckets
            .entry(((c.0 / B).floor() as i64, (c.1 / B).floor() as i64))
            .or_default()
            .push(i);
    }
    let reach = (spec.influence_m / B).ceil() as i64;
    // Candidats.
    let k = (r / spec.grid_m) as i64;
    let mut cand = Vec::new();
    for i in -k..=k {
        for j in -k..=k {
            let (dx, dy) = (i as f64 * spec.grid_m, j as f64 * spec.grid_m);
            if dx.hypot(dy) <= r {
                cand.push((cx + dx, cy + dy));
            }
        }
    }
    let evals: Vec<Option<Spot>> = cand
        .par_iter()
        .map(|&(x, y)| {
            if cancel.is_cancelled() {
                return None;
            }
            let z = dem.z(x, y)?;
            let h = 50.0;
            let sx = (dem.z(x + h, y)? - dem.z(x - h, y)?) / (2.0 * h);
            let sy = (dem.z(x, y + h)? - dem.z(x, y - h)?) / (2.0 * h);
            let slope = 100.0 * sx.hypot(sy);
            if slope > spec.max_slope_pct {
                return None;
            }
            let (bx, by) = ((x / B).floor() as i64, (y / B).floor() as i64);
            let (mut exposed, mut nearest, mut max_l) = (0.0, f64::MAX, f64::NEG_INFINITY);
            for di in -reach..=reach {
                for dj in -reach..=reach {
                    let Some(list) = buckets.get(&(bx + di, by + dj)) else {
                        continue;
                    };
                    for &ci in list {
                        let (px, py, ind) = cells[ci];
                        let d = (px - x).hypot(py - y);
                        nearest = nearest.min(d);
                        if d > spec.influence_m {
                            continue;
                        }
                        let a = barrier_db(&dem, (x, y), (px, py), 100.0).unwrap_or(0.0);
                        let l = spec.source_dbc_1m - 20.0 * d.max(1.0).log10() + 3.0 - a;
                        max_l = f64::max(max_l, l);
                        if l > spec.threshold_dbc {
                            exposed += ind;
                        }
                    }
                }
            }
            let (lon, lat) = proj.to_wgs84(x, y).ok()?;
            Some(Spot {
                lon,
                lat,
                exposed,
                nearest_m: if nearest == f64::MAX {
                    spec.influence_m * 2.0
                } else {
                    nearest
                },
                max_dbc: if max_l.is_finite() { max_l } else { 0.0 },
                slope_pct: slope,
                elevation_m: z,
            })
        })
        .collect();
    if cancel.is_cancelled() {
        return Err(ComputeError::Cancelled);
    }
    progress.inc(1);
    let mut candidates: Vec<Spot> = evals.into_iter().flatten().collect();
    candidates.sort_by(|a, b| {
        a.exposed
            .total_cmp(&b.exposed)
            .then(b.nearest_m.total_cmp(&a.nearest_m))
            .then(a.slope_pct.total_cmp(&b.slope_pct))
    });
    // Meilleurs spots, séparés d'au moins 2 km.
    let mut spots: Vec<Spot> = Vec::new();
    for c in &candidates {
        let far = spots.iter().all(|s| {
            let (dx, dy) = (
                (s.lon - c.lon) * 111_320.0 * c.lat.to_radians().cos(),
                (s.lat - c.lat) * 110_540.0,
            );
            dx.hypot(dy) >= 2_000.0
        });
        if far {
            spots.push(c.clone());
            if spots.len() == 10 {
                break;
            }
        }
    }
    Ok(SpotReport {
        spots,
        populated_cells: cells.len(),
        population: cells.iter().map(|c| c.2).sum(),
        candidates,
        seconds: t0.elapsed().as_secs_f64(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dem_with_ridge() -> Dem {
        // 2 km × 2 km à 10 m ; crête de 30 m à x = 1000 m.
        let n = 200;
        let mut data = vec![100f32; n * n];
        for j in 0..n {
            for i in 98..102 {
                data[j * n + i] = 130.0;
            }
        }
        Dem {
            g: ElevationGrid {
                bbox: BboxL93::new(0.0, 0.0, 2000.0, 2000.0),
                width: n,
                height: n,
                data,
            },
            cell: 10.0,
        }
    }

    #[test]
    fn ridge_screens_flat_does_not() {
        let dem = dem_with_ridge();
        let screened = barrier_db(&dem, (500.0, 1000.0), (1500.0, 1000.0), 10.0).unwrap();
        let open = barrier_db(&dem, (200.0, 1000.0), (800.0, 1000.0), 10.0).unwrap();
        // Crête de 28 m au-dessus de la ligne, abaissée de 15,6 m par la courbure
        // favorable (Rc = 8 km) : δ ≈ 0,31 m, N ≈ 0,11, A ≈ 7,2 dB.
        assert!((screened - 7.2).abs() < 0.5, "{screened}");
        assert_eq!(open, 0.0);
    }
}
