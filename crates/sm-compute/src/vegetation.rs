//! Végétation et rugosité du sol tirées du LiDAR HD (COPC à la demande).
//!
//! Voxels de 5 m × 5 m × 2 m au-dessus du sol (MNT). Pour chaque voxel, la
//! fraction d'interception des impulsions qui l'atteignent :
//! d_l = n_l / Σ_{k ≤ l} n_k (n_k : points de la couche k, comptés du sol vers
//! le haut ; le faisceau vient d'en haut). Pour l'acoustique, le taux de
//! passage entre 11 m et 1 m, P = n(<1 m) / n(<11 m), donne un facteur de
//! feuillage par la loi de Beer-Lambert, rapporté au « feuillage dense » de
//! l'ISO 9613-2 (P = 0,2) : f = ln P / ln 0,2, borné à [0 ; 1,5].
//! Rugosité : écart quadratique des points sol (classe 2) au MNT, moyenne retirée.

use crate::scene::Scene;
use crate::{CancelFlag, ComputeError, ProgressCounter};
use serde::{Deserialize, Serialize};
use sm_data::copc::{CopcFile, LidarPoint};
use sm_data::{CancellationToken, GeoClient};
use std::path::Path;

/// Taille horizontale d'un voxel (m).
pub const CELL_M: f64 = 5.0;
/// Couches : [0, 1 m[ puis 15 couches de 2 m (1 → 31 m).
pub const LAYERS: usize = 16;
const P_DENSE: f64 = 0.2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VegetationMeta {
    /// Coin sud-ouest (L93) et dimensions de la grille.
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub w: usize,
    pub h: usize,
    pub layers: usize,
    pub radius_m: f64,
    pub points: u64,
    pub tiles: usize,
    /// Espacement visé des points lus (m) : niveau de détail COPC.
    pub spacing_m: f64,
    pub depth: i32,
    pub downloaded_bytes: u64,
}

/// Grille de végétation (voxels) et de rugosité.
#[derive(Debug, Clone)]
pub struct VegetationGrid {
    pub meta: VegetationMeta,
    /// Comptes par cellule et couche [cellule][couche].
    counts: Vec<u32>,
    veg: Vec<u32>,
    /// Statistiques des points sol par rapport au MNT : n, Σdz, Σdz².
    ground: Vec<(u32, f32, f32)>,
}

impl VegetationGrid {
    pub fn new(origin: (f64, f64), radius_m: f64) -> Self {
        let n = ((2.0 * radius_m) / CELL_M).ceil() as usize;
        Self {
            meta: VegetationMeta {
                x0: origin.0 - radius_m,
                y0: origin.1 - radius_m,
                cell: CELL_M,
                w: n,
                h: n,
                layers: LAYERS,
                radius_m,
                points: 0,
                tiles: 0,
                spacing_m: 0.0,
                depth: 0,
                downloaded_bytes: 0,
            },
            counts: vec![0; n * n * LAYERS],
            veg: vec![0; n * n * LAYERS],
            ground: vec![(0, 0.0, 0.0); n * n],
        }
    }

    fn cell_of(&self, x: f64, y: f64) -> Option<usize> {
        let m = &self.meta;
        let i = ((x - m.x0) / m.cell).floor();
        let j = ((y - m.y0) / m.cell).floor();
        (i >= 0.0 && j >= 0.0 && (i as usize) < m.w && (j as usize) < m.h)
            .then(|| j as usize * m.w + i as usize)
    }

    /// Ajoute des points (hauteur au-dessus du sol par `ground(x, y)`).
    pub fn add_points(&mut self, pts: &[LidarPoint], ground: impl Fn(f64, f64) -> Option<f32>) {
        for p in pts {
            // Bâti (6), bruit (7, 18), eau (9), ponts (17) : ignorés.
            if matches!(p.class, 6 | 7 | 9 | 17 | 18) {
                continue;
            }
            let Some(c) = self.cell_of(p.x, p.y) else {
                continue;
            };
            let Some(g) = ground(p.x, p.y) else { continue };
            let dz = p.z - f64::from(g);
            if p.class == 2 {
                let s = &mut self.ground[c];
                s.0 += 1;
                s.1 += dz as f32;
                s.2 += (dz * dz) as f32;
            }
            let layer = if dz < 1.0 {
                0
            } else {
                (1 + ((dz - 1.0) / 2.0) as usize).min(LAYERS - 1)
            };
            self.counts[c * LAYERS + layer] += 1;
            if matches!(p.class, 3..=5) && layer > 0 {
                self.veg[c * LAYERS + layer] += 1;
            }
            self.meta.points += 1;
        }
    }

    /// Fraction d'interception du voxel (cellule, couche ≥ 1), 0–1.
    pub fn density(&self, cell: usize, layer: usize) -> Option<f32> {
        let c = &self.counts[cell * LAYERS..(cell + 1) * LAYERS];
        let entering: u32 = c[..=layer].iter().sum();
        (entering >= 4).then(|| self.veg[cell * LAYERS + layer] as f32 / entering as f32)
    }

    /// Facteur de feuillage (1 = feuillage dense ISO 9613-2) au point L93.
    pub fn foliage_at(&self, x: f64, y: f64) -> Option<f32> {
        self.foliage_cell(self.cell_of(x, y)?)
    }

    fn foliage_cell(&self, cell: usize) -> Option<f32> {
        let c = &self.counts[cell * LAYERS..(cell + 1) * LAYERS];
        let below: u32 = c[0];
        let entering: u32 = c[..=5].iter().sum();
        if entering < 8 {
            return None;
        }
        let p = (f64::from(below.max(1)) / f64::from(entering)).min(1.0);
        Some((p.ln() / P_DENSE.ln()).clamp(0.0, 1.5) as f32)
    }

    /// Rugosité du sol (m, écart-type au MNT) au point L93.
    pub fn roughness_at(&self, x: f64, y: f64) -> Option<f32> {
        self.roughness_cell(self.cell_of(x, y)?)
    }

    fn roughness_cell(&self, cell: usize) -> Option<f32> {
        let (n, s, s2) = self.ground[cell];
        (n >= 3).then(|| {
            let n = n as f32;
            (s2 / n - (s / n).powi(2)).max(0.0).sqrt()
        })
    }

    /// Statistiques de synthèse (pour l'UI).
    pub fn summary(&self) -> VegetationSummary {
        let n = self.meta.w * self.meta.h;
        let (mut cells, mut dense, mut rough_n, mut rough_s) = (0usize, 0usize, 0usize, 0.0f64);
        let mut fsum = 0.0f64;
        for c in 0..n {
            if let Some(f) = self.foliage_cell(c) {
                cells += 1;
                fsum += f64::from(f);
                if f >= 0.5 {
                    dense += 1;
                }
            }
            if let Some(r) = self.roughness_cell(c) {
                rough_n += 1;
                rough_s += f64::from(r);
            }
        }
        let area = |k: usize| k as f64 * self.meta.cell * self.meta.cell / 1e6;
        VegetationSummary {
            points: self.meta.points,
            tiles: self.meta.tiles,
            radius_m: self.meta.radius_m,
            covered_km2: area(cells),
            dense_foliage_km2: area(dense),
            mean_foliage: if cells > 0 { fsum / cells as f64 } else { 0.0 },
            mean_roughness_m: if rough_n > 0 {
                rough_s / rough_n as f64
            } else {
                0.0
            },
            downloaded_mb: self.meta.downloaded_bytes as f64 / 1e6,
        }
    }

    /// Enregistre `vegetation.json` + `vegetation.bin` dans le dossier de zone.
    pub fn save(&self, dir: &Path) -> Result<(), ComputeError> {
        let mut b = Vec::with_capacity((self.counts.len() * 2 + self.ground.len() * 3) * 4);
        for v in self.counts.iter().chain(&self.veg) {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for (n, s, s2) in &self.ground {
            b.extend_from_slice(&n.to_le_bytes());
            b.extend_from_slice(&s.to_le_bytes());
            b.extend_from_slice(&s2.to_le_bytes());
        }
        let tmp = dir.join("vegetation.bin.part");
        std::fs::write(&tmp, &b)?;
        std::fs::rename(&tmp, dir.join("vegetation.bin"))?;
        let json = serde_json::to_vec_pretty(&self.meta)
            .map_err(|e| ComputeError::Invalid(e.to_string()))?;
        std::fs::write(dir.join("vegetation.json"), json)?;
        Ok(())
    }

    /// Charge la grille si elle existe.
    pub fn load(dir: &Path) -> Result<Option<Self>, ComputeError> {
        let (mj, mb) = (dir.join("vegetation.json"), dir.join("vegetation.bin"));
        if !mj.exists() || !mb.exists() {
            return Ok(None);
        }
        let meta: VegetationMeta = serde_json::from_slice(&std::fs::read(mj)?)
            .map_err(|e| ComputeError::Invalid(format!("vegetation.json : {e}")))?;
        let b = std::fs::read(mb)?;
        let n = meta.w * meta.h;
        let nl = n * meta.layers;
        if meta.layers != LAYERS || b.len() != (2 * nl + 3 * n) * 4 {
            return Err(ComputeError::Invalid(
                "vegetation.bin : taille inattendue".into(),
            ));
        }
        let word = |i: usize| [b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]];
        let counts = (0..nl).map(|i| u32::from_le_bytes(word(i))).collect();
        let veg = (0..nl).map(|i| u32::from_le_bytes(word(nl + i))).collect();
        let ground = (0..n)
            .map(|c| {
                let o = 2 * nl + 3 * c;
                (
                    u32::from_le_bytes(word(o)),
                    f32::from_le_bytes(word(o + 1)),
                    f32::from_le_bytes(word(o + 2)),
                )
            })
            .collect();
        Ok(Some(Self {
            meta,
            counts,
            veg,
            ground,
        }))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VegetationSummary {
    pub points: u64,
    pub tiles: usize,
    pub radius_m: f64,
    pub covered_km2: f64,
    pub dense_foliage_km2: f64,
    pub mean_foliage: f64,
    pub mean_roughness_m: f64,
    pub downloaded_mb: f64,
}

/// Télécharge les points LiDAR HD (COPC) dans un rayon autour de la sono, au
/// niveau de détail donnant un espacement d'environ `spacing_m`, et construit
/// la grille. Progression : nœuds traités / nœuds à lire.
pub async fn fetch(
    client: &GeoClient,
    scene: &Scene,
    radius_m: f64,
    spacing_m: f64,
    progress: &ProgressCounter,
    cancel: &CancelFlag,
) -> Result<VegetationGrid, ComputeError> {
    let token = CancellationToken::new();
    let check = || {
        if cancel.is_cancelled() {
            token.cancel();
            Err(ComputeError::Cancelled)
        } else {
            Ok(())
        }
    };
    let (ox, oy) = scene.origin;
    let r = radius_m.min(scene.radius);
    let bbox = sm_geo::BboxL93::new(ox - r, oy - r, ox + r, oy + r);
    let tiles = sm_data::lidar_index::tiles_in_bbox(client, &bbox, &token).await?;
    let urls: Vec<String> = tiles.into_iter().filter_map(|t| t.url_copc).collect();
    if urls.is_empty() {
        return Err(ComputeError::Invalid(
            "aucune dalle LiDAR HD (COPC) publiée sur cette zone".into(),
        ));
    }
    let mut grid = VegetationGrid::new(scene.origin, r);
    grid.meta.tiles = urls.len();
    grid.meta.spacing_m = spacing_m;
    let area = [ox - r, oy - r, ox + r, oy + r];
    // Inventaire des nœuds de toutes les dalles.
    let mut work = Vec::new();
    for url in &urls {
        check()?;
        let f = CopcFile::open(client, url, &token).await?;
        let depth = f.depth_for_spacing(spacing_m);
        grid.meta.depth = grid.meta.depth.max(depth);
        let nodes = f.nodes(client, area, depth, &token).await?;
        work.push((f, nodes));
    }
    let total: usize = work.iter().map(|(_, n)| n.len()).sum();
    progress.set_total(total as u64);
    let r2 = r * r;
    let ground = |x: f64, y: f64| scene.dem.sample_clamped(x, y);
    for (f, nodes) in &work {
        // 4 nœuds en parallèle (le client limite le débit par service).
        let mut it = nodes.chunks(4);
        for chunk in &mut it {
            check()?;
            let res =
                futures::future::join_all(chunk.iter().map(|e| f.points(client, e, &token))).await;
            for (e, pts) in chunk.iter().zip(res) {
                let pts = pts?;
                grid.meta.downloaded_bytes += u64::try_from(e.byte_size).unwrap_or(0);
                let inside: Vec<LidarPoint> = pts
                    .into_iter()
                    .filter(|p| (p.x - ox).powi(2) + (p.y - oy).powi(2) <= r2)
                    .collect();
                grid.add_points(&inside, ground);
                progress.inc(1);
            }
        }
    }
    Ok(grid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64, z: f64, class: u8) -> LidarPoint {
        LidarPoint { x, y, z, class }
    }

    #[test]
    fn dense_canopy_gives_foliage_factor_near_one() {
        let mut g = VegetationGrid::new((0.0, 0.0), 50.0);
        // Cellule (2,2) : 100 impulsions, 80 interceptées entre 1 et 11 m.
        let mut pts = Vec::new();
        for i in 0..80 {
            pts.push(pt(12.0 - 50.0, 12.0 - 50.0, 2.0 + f64::from(i % 9), 4));
        }
        for _ in 0..20 {
            pts.push(pt(12.0 - 50.0, 12.0 - 50.0, 0.1, 2));
        }
        g.add_points(&pts, |_, _| Some(0.0));
        let f = g.foliage_at(-38.0, -38.0).unwrap();
        assert!((f - 1.0).abs() < 0.01, "{f}");
        // Cellule vide : pas de donnée.
        assert!(g.foliage_at(20.0, 20.0).is_none());
    }

    #[test]
    fn open_field_gives_zero_and_roughness() {
        let mut g = VegetationGrid::new((0.0, 0.0), 50.0);
        let pts: Vec<LidarPoint> = (0..40)
            .map(|i| pt(1.0, 1.0, if i % 2 == 0 { 0.1 } else { -0.1 }, 2))
            .collect();
        g.add_points(&pts, |_, _| Some(0.0));
        assert_eq!(g.foliage_at(1.0, 1.0), Some(0.0));
        let r = g.roughness_at(1.0, 1.0).unwrap();
        assert!((r - 0.1).abs() < 1e-3, "{r}");
    }

    #[test]
    fn voxel_density_is_interception_fraction() {
        let mut g = VegetationGrid::new((0.0, 0.0), 10.0);
        // 10 impulsions : 5 arrêtées dans la couche 2 (3–5 m), 5 au sol.
        let mut pts: Vec<LidarPoint> = (0..5).map(|_| pt(0.0, 0.0, 4.0, 5)).collect();
        pts.extend((0..5).map(|_| pt(0.0, 0.0, 0.0, 2)));
        g.add_points(&pts, |_, _| Some(0.0));
        let c = g.cell_of(0.0, 0.0).unwrap();
        assert_eq!(g.density(c, 2), Some(0.5));
        assert_eq!(g.density(c, 1), Some(0.0));
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut g = VegetationGrid::new((100.0, 200.0), 20.0);
        g.add_points(
            &[pt(101.0, 201.0, 5.0, 4), pt(101.0, 201.0, 0.0, 2)],
            |_, _| Some(0.0),
        );
        g.save(dir.path()).unwrap();
        let h = VegetationGrid::load(dir.path()).unwrap().unwrap();
        assert_eq!(h.meta.points, 2);
        assert_eq!(h.counts, g.counts);
        assert_eq!(h.ground, g.ground);
    }
}
