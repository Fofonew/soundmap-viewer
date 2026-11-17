//! Maths des tuiles Web Mercator (schéma XYZ, origine en haut à gauche).

use crate::proj::Lambert93;
use crate::{BboxL93, Result};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// Latitude maximale de Web Mercator.
pub const MAX_LAT: f64 = 85.051_128_779_806_59;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TileId {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

/// Emprise géographique en degrés.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LonLatBounds {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

/// (lon, lat) → coordonnées de tuile fractionnaires au zoom `z`.
pub fn lonlat_to_tile_frac(lon: f64, lat: f64, z: u8) -> (f64, f64) {
    let n = f64::from(1u32 << z);
    let lat = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
    let x = (lon + 180.0) / 360.0 * n;
    let y = (1.0 - lat.tan().asinh() / PI) / 2.0 * n;
    (x, y)
}

/// Coordonnées de tuile fractionnaires → (lon, lat).
pub fn tile_frac_to_lonlat(x: f64, y: f64, z: u8) -> (f64, f64) {
    let n = f64::from(1u32 << z);
    let lon = x / n * 360.0 - 180.0;
    let lat = (PI * (1.0 - 2.0 * y / n)).sinh().atan().to_degrees();
    (lon, lat)
}

impl TileId {
    pub fn new(z: u8, x: u32, y: u32) -> Self {
        Self { z, x, y }
    }

    pub fn from_lonlat(lon: f64, lat: f64, z: u8) -> Self {
        let (x, y) = lonlat_to_tile_frac(lon, lat, z);
        let max = (1u32 << z) - 1;
        Self::new(
            z,
            (x.floor().max(0.0) as u32).min(max),
            (y.floor().max(0.0) as u32).min(max),
        )
    }

    pub fn bounds(&self) -> LonLatBounds {
        let (west, north) = tile_frac_to_lonlat(f64::from(self.x), f64::from(self.y), self.z);
        let (east, south) =
            tile_frac_to_lonlat(f64::from(self.x + 1), f64::from(self.y + 1), self.z);
        LonLatBounds {
            west,
            south,
            east,
            north,
        }
    }

    /// Emprise de la tuile en Lambert-93 (enveloppe de 4 coins + 4 milieux).
    pub fn bbox_l93(&self, proj: &Lambert93) -> Result<BboxL93> {
        let mut b = BboxL93::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (fx, fy) in [
            (0.0, 0.0),
            (0.5, 0.0),
            (1.0, 0.0),
            (0.0, 0.5),
            (1.0, 0.5),
            (0.0, 1.0),
            (0.5, 1.0),
            (1.0, 1.0),
        ] {
            let (lon, lat) =
                tile_frac_to_lonlat(f64::from(self.x) + fx, f64::from(self.y) + fy, self.z);
            let (x, y) = proj.from_wgs84(lon, lat)?;
            b.xmin = b.xmin.min(x);
            b.ymin = b.ymin.min(y);
            b.xmax = b.xmax.max(x);
            b.ymax = b.ymax.max(y);
        }
        Ok(b)
    }
}

/// Tuiles d'un zoom couvrant une emprise géographique.
pub fn tiles_in_bounds(b: &LonLatBounds, z: u8) -> Vec<TileId> {
    let tl = TileId::from_lonlat(b.west, b.north, z);
    let br = TileId::from_lonlat(b.east, b.south, z);
    let mut out = Vec::with_capacity(((br.x - tl.x + 1) * (br.y - tl.y + 1)) as usize);
    for y in tl.y..=br.y {
        for x in tl.x..=br.x {
            out.push(TileId::new(z, x, y));
        }
    }
    out
}

/// Emprise géographique d'une emprise Lambert-93 (enveloppe échantillonnée).
pub fn l93_bbox_to_lonlat(proj: &Lambert93, b: &BboxL93) -> Result<LonLatBounds> {
    let mut out = LonLatBounds {
        west: f64::MAX,
        south: f64::MAX,
        east: f64::MIN,
        north: f64::MIN,
    };
    const N: usize = 8;
    for i in 0..=N {
        let t = i as f64 / N as f64;
        for (x, y) in [
            (b.xmin + t * b.width(), b.ymin),
            (b.xmin + t * b.width(), b.ymax),
            (b.xmin, b.ymin + t * b.height()),
            (b.xmax, b.ymin + t * b.height()),
        ] {
            let (lon, lat) = proj.to_wgs84(x, y)?;
            out.west = out.west.min(lon);
            out.east = out.east.max(lon);
            out.south = out.south.min(lat);
            out.north = out.north.max(lat);
        }
    }
    Ok(out)
}

/// Tuiles du zoom `z` qui intersectent le disque Lambert-93 (cx, cy, r).
pub fn tiles_covering_disk(
    proj: &Lambert93,
    cx: f64,
    cy: f64,
    r: f64,
    z: u8,
) -> Result<Vec<TileId>> {
    let ll = l93_bbox_to_lonlat(proj, &BboxL93::around(cx, cy, r))?;
    let mut out = Vec::new();
    for t in tiles_in_bounds(&ll, z) {
        if t.bbox_l93(proj)?.distance_to(cx, cy) <= r {
            out.push(t);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_tile_for_millau() {
        // Vérifié à la main : Millau (3.093182, 44.10197) à z14 → x 8332, y 5951.
        let t = TileId::from_lonlat(3.093182, 44.10197, 14);
        assert_eq!((t.x, t.y), (8332, 5951));
    }

    #[test]
    fn frac_round_trip() {
        let (x, y) = lonlat_to_tile_frac(3.1, 44.1, 17);
        let (lon, lat) = tile_frac_to_lonlat(x, y, 17);
        assert!((lon - 3.1).abs() < 1e-9 && (lat - 44.1).abs() < 1e-9);
    }

    #[test]
    fn bounds_contain_point() {
        let t = TileId::from_lonlat(3.1, 44.1, 15);
        let b = t.bounds();
        assert!(b.west <= 3.1 && 3.1 <= b.east && b.south <= 44.1 && 44.1 <= b.north);
    }

    #[test]
    fn disk_cover_is_smaller_than_square_cover() {
        let p = Lambert93::new().unwrap();
        let (cx, cy) = (707_460.0, 6_333_667.0);
        let disk = tiles_covering_disk(&p, cx, cy, 5_000.0, 15).unwrap();
        let ll = l93_bbox_to_lonlat(&p, &BboxL93::around(cx, cy, 5_000.0)).unwrap();
        let square = tiles_in_bounds(&ll, 15);
        assert!(!disk.is_empty());
        assert!(disk.len() < square.len());
        // Tuile z15 ≈ 880 m à 44° N : ≥ π·(5/0.88)² ≈ 100 tuiles.
        assert!(disk.len() > 100, "{}", disk.len());
    }
}
