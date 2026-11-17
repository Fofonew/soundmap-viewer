//! Rasters float32 tuilés `.smr`, lus par `memmap2`.
//!
//! Format (little-endian) :
//! ```text
//! 0   [u8;4]  magic "SMR1"
//! 4   u32     version (1)
//! 8   u32     largeur (px)
//! 12  u32     hauteur (px)
//! 16  u32     côté de tuile (px)
//! 20  u32     réservé
//! 24  f64     x0 : bord gauche (L93, m)
//! 32  f64     y0 : bord haut (L93, m)
//! 40  f64     pas (m)
//! 48  f32     nodata
//! 52  [u8;12] réservé
//! 64  données : tuiles T×T en ordre ligne, chaque tuile en ordre ligne ;
//!     les tuiles de bord sont complétées par nodata.
//! ```
//! Le centre du pixel (col, row) est en `(x0 + (col+½)·pas, y0 − (row+½)·pas)`.

use crate::{BboxL93, GeoError, Result};
use memmap2::{Mmap, MmapMut};
use std::fs::{File, OpenOptions};
use std::path::Path;

const MAGIC: &[u8; 4] = b"SMR1";
const HEADER_LEN: usize = 64;
/// Valeur nodata des services IGN (vérifiée hors couverture).
pub const NODATA: f32 = -9999.0;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RasterHeader {
    pub width: u32,
    pub height: u32,
    pub tile: u32,
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub nodata: f32,
}

impl RasterHeader {
    /// Raster couvrant `bbox` au pas `cell` (tuiles de 256 px).
    pub fn for_bbox(bbox: &BboxL93, cell: f64) -> Self {
        Self {
            width: (bbox.width() / cell).round() as u32,
            height: (bbox.height() / cell).round() as u32,
            tile: 256,
            x0: bbox.xmin,
            y0: bbox.ymax,
            cell,
            nodata: NODATA,
        }
    }

    pub fn bbox(&self) -> BboxL93 {
        BboxL93::new(
            self.x0,
            self.y0 - f64::from(self.height) * self.cell,
            self.x0 + f64::from(self.width) * self.cell,
            self.y0,
        )
    }

    fn tiles_x(&self) -> usize {
        self.width.div_ceil(self.tile) as usize
    }
    fn tiles_y(&self) -> usize {
        self.height.div_ceil(self.tile) as usize
    }

    pub fn data_len(&self) -> usize {
        let t = self.tile as usize;
        self.tiles_x() * self.tiles_y() * t * t * 4
    }

    pub fn file_len(&self) -> usize {
        HEADER_LEN + self.data_len()
    }

    #[inline]
    fn offset(&self, col: u32, row: u32) -> usize {
        let t = self.tile as usize;
        let (tx, ix) = ((col / self.tile) as usize, (col % self.tile) as usize);
        let (ty, iy) = ((row / self.tile) as usize, (row % self.tile) as usize);
        HEADER_LEN + ((ty * self.tiles_x() + tx) * t * t + iy * t + ix) * 4
    }

    fn encode(&self) -> [u8; HEADER_LEN] {
        let mut h = [0u8; HEADER_LEN];
        h[0..4].copy_from_slice(MAGIC);
        h[4..8].copy_from_slice(&1u32.to_le_bytes());
        h[8..12].copy_from_slice(&self.width.to_le_bytes());
        h[12..16].copy_from_slice(&self.height.to_le_bytes());
        h[16..20].copy_from_slice(&self.tile.to_le_bytes());
        h[24..32].copy_from_slice(&self.x0.to_le_bytes());
        h[32..40].copy_from_slice(&self.y0.to_le_bytes());
        h[40..48].copy_from_slice(&self.cell.to_le_bytes());
        h[48..52].copy_from_slice(&self.nodata.to_le_bytes());
        h
    }

    fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < HEADER_LEN || &b[0..4] != MAGIC {
            return Err(GeoError::Format("magic SMR1 absent".into()));
        }
        let u = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let f = |o: usize| {
            let mut a = [0u8; 8];
            a.copy_from_slice(&b[o..o + 8]);
            f64::from_le_bytes(a)
        };
        if u(4) != 1 {
            return Err(GeoError::Format(format!("version {} non gérée", u(4))));
        }
        let h = Self {
            width: u(8),
            height: u(12),
            tile: u(16),
            x0: f(24),
            y0: f(32),
            cell: f(40),
            nodata: f32::from_le_bytes([b[48], b[49], b[50], b[51]]),
        };
        if h.tile == 0 || h.cell <= 0.0 {
            return Err(GeoError::Format("tuile ou pas nul".into()));
        }
        Ok(h)
    }
}

/// Écriture d'un raster `.smr` par blocs (le fichier est pré-rempli de nodata).
pub struct RasterWriter {
    header: RasterHeader,
    map: MmapMut,
}

impl RasterWriter {
    pub fn create(path: &Path, header: RasterHeader) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(header.file_len() as u64)?;
        // SAFETY : le fichier vient d'être créé par nous et n'est pas partagé.
        let mut map = unsafe { MmapMut::map_mut(&file)? };
        map[..HEADER_LEN].copy_from_slice(&header.encode());
        let nd = header.nodata.to_le_bytes();
        for px in map[HEADER_LEN..].as_chunks_mut::<4>().0 {
            px.copy_from_slice(&nd);
        }
        Ok(Self { header, map })
    }

    pub fn header(&self) -> &RasterHeader {
        &self.header
    }

    #[inline]
    pub fn set(&mut self, col: u32, row: u32, v: f32) {
        if col < self.header.width && row < self.header.height {
            let o = self.header.offset(col, row);
            self.map[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
    }

    /// Écrit un bloc `w×h` (ordre ligne) dont le coin haut-gauche est (col0, row0).
    /// Les pixels hors raster sont ignorés.
    pub fn write_block(&mut self, col0: i64, row0: i64, w: usize, h: usize, data: &[f32]) {
        for r in 0..h {
            let row = row0 + r as i64;
            if row < 0 || row >= i64::from(self.header.height) {
                continue;
            }
            for c in 0..w {
                let col = col0 + c as i64;
                if col < 0 || col >= i64::from(self.header.width) {
                    continue;
                }
                if let Some(&v) = data.get(r * w + c) {
                    self.set(col as u32, row as u32, v);
                }
            }
        }
    }

    pub fn finish(self) -> Result<()> {
        self.map.flush()?;
        Ok(())
    }
}

/// Raster `.smr` en lecture, memory-mappé.
pub struct MmapRaster {
    header: RasterHeader,
    map: Mmap,
}

impl std::fmt::Debug for MmapRaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MmapRaster")
            .field("header", &self.header)
            .finish()
    }
}

impl MmapRaster {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        // SAFETY : les fichiers du pack ne sont jamais modifiés une fois finalisés.
        let map = unsafe { Mmap::map(&file)? };
        let header = RasterHeader::decode(&map)?;
        if map.len() < header.file_len() {
            return Err(GeoError::Format(format!(
                "fichier tronqué : {} < {} octets",
                map.len(),
                header.file_len()
            )));
        }
        Ok(Self { header, map })
    }

    pub fn header(&self) -> &RasterHeader {
        &self.header
    }

    /// Valeur brute d'un pixel (`None` si hors raster ou nodata).
    #[inline]
    pub fn get(&self, col: u32, row: u32) -> Option<f32> {
        if col >= self.header.width || row >= self.header.height {
            return None;
        }
        let o = self.header.offset(col, row);
        let v = f32::from_le_bytes([
            self.map[o],
            self.map[o + 1],
            self.map[o + 2],
            self.map[o + 3],
        ]);
        (v != self.header.nodata && v.is_finite()).then_some(v)
    }

    /// Le point est-il dans l'emprise du raster ?
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.header.bbox().contains(x, y)
    }

    /// Interpolation bilinéaire en (x, y) Lambert-93. Si un voisin est nodata,
    /// renvoie le plus proche voisin valide ; `None` si hors emprise.
    pub fn sample_bilinear(&self, x: f64, y: f64) -> Option<f32> {
        let h = &self.header;
        if !self.contains(x, y) {
            return None;
        }
        let fx = ((x - h.x0) / h.cell - 0.5).clamp(0.0, f64::from(h.width - 1));
        let fy = ((h.y0 - y) / h.cell - 0.5).clamp(0.0, f64::from(h.height - 1));
        let c0 = fx.floor() as u32;
        let r0 = fy.floor() as u32;
        let c1 = (c0 + 1).min(h.width - 1);
        let r1 = (r0 + 1).min(h.height - 1);
        let tx = (fx - f64::from(c0)) as f32;
        let ty = (fy - f64::from(r0)) as f32;
        match (
            self.get(c0, r0),
            self.get(c1, r0),
            self.get(c0, r1),
            self.get(c1, r1),
        ) {
            (Some(a), Some(b), Some(c), Some(d)) => {
                let top = a + (b - a) * tx;
                let bot = c + (d - c) * tx;
                Some(top + (bot - top) * ty)
            }
            _ => {
                let cn = if tx < 0.5 { c0 } else { c1 };
                let rn = if ty < 0.5 { r0 } else { r1 };
                self.get(cn, rn)
            }
        }
    }
}

/// MNT multi-résolution : anneaux concentriques, le plus fin d'abord.
#[derive(Debug, Default)]
pub struct MultiResDem {
    rings: Vec<MmapRaster>,
}

impl MultiResDem {
    pub fn new(mut rings: Vec<MmapRaster>) -> Self {
        rings.sort_by(|a, b| a.header.cell.total_cmp(&b.header.cell));
        Self { rings }
    }

    pub fn rings(&self) -> &[MmapRaster] {
        &self.rings
    }

    /// Emprise de l'anneau le plus grossier.
    pub fn outer_bbox(&self) -> Option<BboxL93> {
        self.rings.last().map(|r| r.header.bbox())
    }

    /// Altitude au point, depuis l'anneau le plus fin qui le couvre.
    pub fn sample(&self, x: f64, y: f64) -> Option<f32> {
        self.rings.iter().find_map(|r| r.sample_bilinear(x, y))
    }

    /// Comme [`Self::sample`], mais hors emprise on prend la valeur du bord le
    /// plus proche (pour éviter une falaise artificielle dans la vue 3D).
    pub fn sample_clamped(&self, x: f64, y: f64) -> Option<f32> {
        if let Some(v) = self.sample(x, y) {
            return Some(v);
        }
        let b = self.outer_bbox()?;
        let eps = self.rings.last().map_or(0.0, |r| r.header.cell * 0.5);
        let cx = x.clamp(b.xmin + eps, b.xmax - eps);
        let cy = y.clamp(b.ymin + eps, b.ymax - eps);
        self.sample(cx, cy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(dir: &Path, name: &str, bbox: BboxL93, cell: f64) -> MmapRaster {
        let h = RasterHeader::for_bbox(&bbox, cell);
        let path = dir.join(name);
        let mut w = RasterWriter::create(&path, h).unwrap();
        let data: Vec<f32> = (0..h.height)
            .flat_map(|r| (0..h.width).map(move |c| (c + 1000 * r) as f32))
            .collect();
        w.write_block(0, 0, h.width as usize, h.height as usize, &data);
        w.finish().unwrap();
        MmapRaster::open(&path).unwrap()
    }

    #[test]
    fn header_round_trip_and_tiling() {
        let dir = tempfile::tempdir().unwrap();
        let r = ramp(
            dir.path(),
            "a.smr",
            BboxL93::new(0.0, 0.0, 600.0, 300.0),
            1.0,
        );
        assert_eq!(r.header().width, 600);
        assert_eq!(r.header().height, 300);
        assert_eq!(r.get(0, 0), Some(0.0));
        assert_eq!(r.get(599, 299), Some(599.0 + 299_000.0));
        assert_eq!(r.get(257, 3), Some(257.0 + 3000.0));
        assert_eq!(r.get(600, 0), None);
    }

    #[test]
    fn bilinear_on_ramp_is_exact() {
        let dir = tempfile::tempdir().unwrap();
        let r = ramp(
            dir.path(),
            "a.smr",
            BboxL93::new(0.0, 0.0, 100.0, 100.0),
            1.0,
        );
        // centre du pixel (10, 20) : x = 10.5, y = 100 − 20.5 = 79.5
        assert_eq!(r.sample_bilinear(10.5, 79.5), Some(20_010.0));
        // milieu entre col 10 et 11
        assert_eq!(r.sample_bilinear(11.0, 79.5), Some(20_010.5));
        assert_eq!(r.sample_bilinear(-1.0, 50.0), None);
    }

    #[test]
    fn nodata_is_none_and_partial_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let h = RasterHeader::for_bbox(&BboxL93::new(0.0, 0.0, 10.0, 10.0), 1.0);
        let p = dir.path().join("b.smr");
        let mut w = RasterWriter::create(&p, h).unwrap();
        w.write_block(8, 8, 4, 4, &[1.0; 16]); // déborde : seuls 2×2 écrits
        w.finish().unwrap();
        let r = MmapRaster::open(&p).unwrap();
        assert_eq!(r.get(9, 9), Some(1.0));
        assert_eq!(r.get(0, 0), None);
    }

    #[test]
    fn multires_prefers_finest_ring_and_clamps() {
        let dir = tempfile::tempdir().unwrap();
        let fine = ramp(
            dir.path(),
            "f.smr",
            BboxL93::new(40.0, 40.0, 60.0, 60.0),
            0.5,
        );
        let coarse = ramp(
            dir.path(),
            "c.smr",
            BboxL93::new(0.0, 0.0, 100.0, 100.0),
            5.0,
        );
        let dem = MultiResDem::new(vec![coarse, fine]);
        assert_eq!(dem.rings()[0].header().cell, 0.5);
        let v = dem.sample(50.0, 50.0).unwrap();
        let fine_v = dem.rings()[0].sample_bilinear(50.0, 50.0).unwrap();
        assert_eq!(v, fine_v);
        assert!(dem.sample(200.0, 50.0).is_none());
        assert!(dem.sample_clamped(200.0, 50.0).is_some());
    }

    #[test]
    fn rejects_bad_magic() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.smr");
        std::fs::write(&p, [0u8; 100]).unwrap();
        assert!(MmapRaster::open(&p).is_err());
    }
}
