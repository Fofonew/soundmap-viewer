//! Rendu des résultats : raster cartésien (L93), PNG coloré, isophones GeoJSON.

use crate::polar::LevelGrid;
use crate::scene::Scene;
use crate::{ComputeError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sm_core::engineering::BandSpec;
use sm_core::{Hz, a_weighting, c_weighting};

/// Indicateur affiché.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Metric {
    /// dB(A) global.
    A,
    /// dB(C) global.
    C,
    /// dB(Z) global.
    Z,
    /// Bande d'octave de fréquence nominale donnée (Hz, entier).
    Octave(u32),
}

impl Metric {
    pub fn key(&self) -> String {
        match self {
            Metric::A => "A".into(),
            Metric::C => "C".into(),
            Metric::Z => "Z".into(),
            Metric::Octave(f) => format!("oct{f}"),
        }
    }

    pub fn parse(s: &str) -> Option<Metric> {
        match s {
            "A" => Some(Metric::A),
            "C" => Some(Metric::C),
            "Z" => Some(Metric::Z),
            o => o
                .strip_prefix("oct")
                .and_then(|f| f.parse().ok())
                .map(Metric::Octave),
        }
    }

    /// Poids (dB) par bande ; −∞ (−1000) pour exclure une bande.
    pub fn weights(&self, bands: &[BandSpec]) -> Vec<f64> {
        bands
            .iter()
            .map(|b| match self {
                Metric::A => a_weighting(Hz(b.fc)).0,
                Metric::C => c_weighting(Hz(b.fc)).0,
                Metric::Z => 0.0,
                Metric::Octave(f) => {
                    let (lo, hi) = (f64::from(*f) / 2f64.sqrt(), f64::from(*f) * 2f64.sqrt());
                    if b.fc >= lo * 1.01 && b.fc <= hi * 0.99 {
                        0.0
                    } else {
                        -1000.0
                    }
                }
            })
            .collect()
    }
}

/// Niveau global pondéré à partir des niveaux par bande.
pub fn weighted_level(levels: &[f64], weights: &[f64]) -> f64 {
    let e: f64 = levels
        .iter()
        .zip(weights)
        .map(|(l, w)| 10f64.powf((l + w) / 10.0))
        .sum();
    10.0 * e.max(1e-12).log10()
}

/// Raster cartésien en coordonnées locales (origine = sono), NaN hors zone.
#[derive(Debug, Clone, Serialize)]
pub struct Raster {
    /// Coin haut-gauche (local, m).
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub w: usize,
    pub h: usize,
    #[serde(skip)]
    pub data: Vec<f32>,
}

impl Raster {
    pub fn get_local(&self, x: f64, y: f64) -> Option<f32> {
        let c = ((x - self.x0) / self.cell).floor();
        let r = ((self.y0 - y) / self.cell).floor();
        if c < 0.0 || r < 0.0 || c as usize >= self.w || r as usize >= self.h {
            return None;
        }
        let v = self.data[r as usize * self.w + c as usize];
        v.is_finite().then_some(v)
    }
}

/// Valeurs pondérées sur la grille polaire [az][r].
pub fn polar_weighted(g: &LevelGrid, metric: Metric) -> Vec<f32> {
    let w = metric.weights(&g.bands);
    let n_r = g.spec.n_r();
    let mut out = Vec::with_capacity(g.spec.n_az * n_r);
    for ia in 0..g.spec.n_az {
        for ir in 0..n_r {
            out.push(g.weighted(ia, ir, &w) as f32);
        }
    }
    out
}

/// Interpolation bilinéaire (azimut, distance) dans un champ polaire [az][r].
pub fn polar_sample(
    field: &[f32],
    n_az: usize,
    n_r: usize,
    dr: f64,
    x: f64,
    y: f64,
) -> Option<f32> {
    let r = x.hypot(y);
    let fr = (r / dr - 1.0).max(0.0);
    if fr > (n_r - 1) as f64 {
        return None;
    }
    let az = x.atan2(y).to_degrees().rem_euclid(360.0);
    let fa = az / (360.0 / n_az as f64);
    let a0 = fa.floor() as usize % n_az;
    let a1 = (a0 + 1) % n_az;
    let ta = (fa - fa.floor()) as f32;
    let r0 = fr.floor() as usize;
    let r1 = (r0 + 1).min(n_r - 1);
    let tr = (fr - fr.floor()) as f32;
    let v = |a: usize, rr: usize| field[a * n_r + rr];
    let top = v(a0, r0) * (1.0 - tr) + v(a0, r1) * tr;
    let bot = v(a1, r0) * (1.0 - tr) + v(a1, r1) * tr;
    Some(top * (1.0 - ta) + bot * ta)
}

/// Rasterise un champ polaire sur une grille cartésienne de ~`px` pixels de côté.
pub fn rasterize(field: &[f32], g: &LevelGrid, px: usize) -> Raster {
    let n_r = g.spec.n_r();
    let rmax = g.spec.r_of(n_r - 1);
    let cell = 2.0 * rmax / px as f64;
    let mut data = vec![f32::NAN; px * px];
    for r in 0..px {
        let y = rmax - (r as f64 + 0.5) * cell;
        for c in 0..px {
            let x = -rmax + (c as f64 + 0.5) * cell;
            if let Some(v) = polar_sample(field, g.spec.n_az, n_r, g.spec.dr, x, y) {
                data[r * px + c] = v;
            }
        }
    }
    Raster {
        x0: -rmax,
        y0: rmax,
        cell,
        w: px,
        h: px,
        data,
    }
}

/// Palette par classes de 5 dB (vert → jaune → rouge → violet), à la manière
/// des cartes de bruit. Sous `stops[0]` : transparent.
pub const PALETTE: [(f64, [u8; 3]); 14] = [
    (30.0, [0xd4, 0xf2, 0xd4]),
    (35.0, [0x9b, 0xd9, 0x9b]),
    (40.0, [0x57, 0xb4, 0x5a]),
    (45.0, [0x1f, 0x8a, 0x3b]),
    (50.0, [0xf3, 0xe4, 0x5a]),
    (55.0, [0xf5, 0xb3, 0x42]),
    (60.0, [0xef, 0x7d, 0x2d]),
    (65.0, [0xe4, 0x48, 0x2c]),
    (70.0, [0xc4, 0x1f, 0x3a]),
    (75.0, [0x9b, 0x14, 0x46]),
    (80.0, [0x6c, 0x0f, 0x4f]),
    (85.0, [0x46, 0x10, 0x4f]),
    (90.0, [0x2c, 0x0a, 0x3d]),
    (95.0, [0x1a, 0x05, 0x26]),
];

fn color(v: f32, offset: f64) -> [u8; 4] {
    if !v.is_finite() {
        return [0, 0, 0, 0];
    }
    let v = f64::from(v) - offset;
    let mut out = [0, 0, 0, 0];
    for (t, rgb) in PALETTE {
        if v >= t {
            out = [rgb[0], rgb[1], rgb[2], 255];
        }
    }
    out
}

/// PNG RGBA du raster. `offset` décale la palette (ex. +20 dB pour le dB(C)).
pub fn to_png(r: &Raster, offset: f64) -> Result<Vec<u8>> {
    let mut rgba = Vec::with_capacity(r.w * r.h * 4);
    for v in &r.data {
        rgba.extend_from_slice(&color(*v, offset));
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, r.w as u32, r.h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut w = enc
            .write_header()
            .map_err(|e| ComputeError::Invalid(e.to_string()))?;
        w.write_image_data(&rgba)
            .map_err(|e| ComputeError::Invalid(e.to_string()))?;
        w.finish()
            .map_err(|e| ComputeError::Invalid(e.to_string()))?;
    }
    Ok(out)
}

/// Coins du raster en WGS84 [haut-gauche, haut-droit, bas-droit, bas-gauche].
pub fn corners_lonlat(scene: &Scene, r: &Raster) -> Vec<[f64; 2]> {
    let (x1, y1) = (r.x0 + r.cell * r.w as f64, r.y0 - r.cell * r.h as f64);
    [(r.x0, r.y0), (x1, r.y0), (x1, y1), (r.x0, y1)]
        .iter()
        .filter_map(|(x, y)| scene.local_to_lonlat(*x, *y).map(|(a, b)| [a, b]))
        .collect()
}

/// Isophones (lignes de niveau) en GeoJSON WGS84.
pub fn isophones(scene: &Scene, r: &Raster, thresholds: &[f64]) -> Result<Value> {
    let values: Vec<f64> = r
        .data
        .iter()
        .map(|v| if v.is_finite() { f64::from(*v) } else { -999.0 })
        .collect();
    let builder = contour::ContourBuilder::new(r.w, r.h, true)
        .x_origin(r.x0 + r.cell / 2.0)
        .y_origin(r.y0 - r.cell / 2.0)
        .x_step(r.cell)
        .y_step(-r.cell);
    let lines = builder
        .lines(&values, thresholds)
        .map_err(|e| ComputeError::Invalid(format!("isophones : {e:?}")))?;
    let mut feats = Vec::new();
    for l in lines {
        let mut parts = Vec::new();
        for ls in &l.geometry().0 {
            let pts: Vec<Value> =
                ls.0.iter()
                    .filter_map(|c| scene.local_to_lonlat(c.x, c.y))
                    .map(|(a, b)| json!([(a * 1e6).round() / 1e6, (b * 1e6).round() / 1e6]))
                    .collect();
            if pts.len() >= 2 {
                parts.push(Value::Array(pts));
            }
        }
        if !parts.is_empty() {
            feats.push(json!({
                "type": "Feature",
                "properties": { "db": l.threshold() },
                "geometry": { "type": "MultiLineString", "coordinates": parts }
            }));
        }
    }
    Ok(json!({ "type": "FeatureCollection", "features": feats }))
}

/// Statistiques d'un raster : max hors champ proche et surfaces au-dessus de seuils.
#[derive(Debug, Clone, Serialize)]
pub struct RasterStats {
    pub max_db: f64,
    /// (seuil dB, surface km²)
    pub area_above: Vec<(f64, f64)>,
}

pub fn stats(r: &Raster, near_exclude_m: f64, thresholds: &[f64]) -> RasterStats {
    let mut max = f64::MIN;
    let mut counts = vec![0usize; thresholds.len()];
    for row in 0..r.h {
        let y = r.y0 - (row as f64 + 0.5) * r.cell;
        for c in 0..r.w {
            let v = r.data[row * r.w + c];
            if !v.is_finite() {
                continue;
            }
            let x = r.x0 + (c as f64 + 0.5) * r.cell;
            if x.hypot(y) > near_exclude_m {
                max = max.max(f64::from(v));
            }
            for (i, t) in thresholds.iter().enumerate() {
                if f64::from(v) >= *t {
                    counts[i] += 1;
                }
            }
        }
    }
    let a = r.cell * r.cell / 1e6;
    RasterStats {
        max_db: max,
        area_above: thresholds
            .iter()
            .zip(counts)
            .map(|(t, n)| (*t, n as f64 * a))
            .collect(),
    }
}
