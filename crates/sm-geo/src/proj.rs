//! Lambert-93 (EPSG:2154) ↔ WGS84 (EPSG:4326) en pur Rust via `proj4rs`.
//!
//! RGF93 et WGS84 sont confondus à mieux que le mètre (`towgs84=0,…`), ce qui
//! suffit largement pour la cartographie acoustique.

use crate::{GeoError, Result};
use proj4rs::proj::Proj;
use proj4rs::transform::transform;

const L93: &str = "+proj=lcc +lat_0=46.5 +lon_0=3 +lat_1=49 +lat_2=44 +x_0=700000 +y_0=6600000 \
                   +ellps=GRS80 +towgs84=0,0,0,0,0,0,0 +units=m +no_defs +type=crs";
const WGS84: &str = "+proj=longlat +ellps=WGS84 +datum=WGS84 +no_defs";

/// Convertisseur Lambert-93 ↔ WGS84 (à construire une fois, réutilisable).
pub struct Lambert93 {
    l93: Proj,
    wgs: Proj,
}

impl std::fmt::Debug for Lambert93 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Lambert93")
    }
}

impl Lambert93 {
    pub fn new() -> Result<Self> {
        let l93 = Proj::from_proj_string(L93).map_err(|e| GeoError::Proj(e.to_string()))?;
        let wgs = Proj::from_proj_string(WGS84).map_err(|e| GeoError::Proj(e.to_string()))?;
        Ok(Self { l93, wgs })
    }

    /// (x, y) Lambert-93 en mètres → (lon, lat) en degrés.
    pub fn to_wgs84(&self, x: f64, y: f64) -> Result<(f64, f64)> {
        let mut p = (x, y, 0.0);
        transform(&self.l93, &self.wgs, &mut p).map_err(|e| GeoError::Proj(e.to_string()))?;
        Ok((p.0.to_degrees(), p.1.to_degrees()))
    }

    /// (lon, lat) en degrés → (x, y) Lambert-93 en mètres.
    pub fn from_wgs84(&self, lon: f64, lat: f64) -> Result<(f64, f64)> {
        let mut p = (lon.to_radians(), lat.to_radians(), 0.0);
        transform(&self.wgs, &self.l93, &mut p).map_err(|e| GeoError::Proj(e.to_string()))?;
        Ok((p.0, p.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point de référence : géocodage Géoplateforme de « Millau » (17/11/2026)
    /// → lon 3.093182, lat 44.10197, x 707460.4, y 6333667.33.
    #[test]
    fn millau_reference_point() {
        let p = Lambert93::new().unwrap();
        let (x, y) = p.from_wgs84(3.093182, 44.10197).unwrap();
        assert!((x - 707_460.4).abs() < 1.0, "x = {x}");
        assert!((y - 6_333_667.33).abs() < 1.0, "y = {y}");
    }

    #[test]
    fn round_trip_below_1mm() {
        let p = Lambert93::new().unwrap();
        for &(x, y) in &[
            (700_000.0, 6_600_000.0),
            (707_460.4, 6_333_667.33),
            (1_200_000.0, 6_100_000.0),
            (150_000.0, 6_850_000.0),
        ] {
            let (lon, lat) = p.to_wgs84(x, y).unwrap();
            let (x2, y2) = p.from_wgs84(lon, lat).unwrap();
            assert!((x - x2).abs() < 1e-3 && (y - y2).abs() < 1e-3);
        }
    }

    #[test]
    fn origin_is_3e_46_5n() {
        let p = Lambert93::new().unwrap();
        let (lon, lat) = p.to_wgs84(700_000.0, 6_600_000.0).unwrap();
        assert!((lon - 3.0).abs() < 1e-9);
        assert!((lat - 46.5).abs() < 1e-9);
    }
}
