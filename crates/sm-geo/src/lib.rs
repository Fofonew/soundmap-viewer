//! `sm-geo` : géométrie et rasters pour SoundMap.
//!
//! - [`proj`] : Lambert-93 (EPSG:2154) ↔ WGS84, en pur Rust (`proj4rs`) ;
//! - [`tiles`] : maths des tuiles Web Mercator (z/x/y) ;
//! - [`raster`] : rasters float32 tuilés `.smr`, lus par `memmap2` ;
//! - [`terrarium`] : encodage Terrarium pour le terrain 3D de MapLibre.

pub mod proj;
pub mod raster;
pub mod terrarium;
pub mod tiles;

pub use proj::Lambert93;
pub use raster::{MmapRaster, MultiResDem, RasterHeader, RasterWriter};
pub use tiles::TileId;

/// Emprise rectangulaire en Lambert-93 (mètres).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BboxL93 {
    pub xmin: f64,
    pub ymin: f64,
    pub xmax: f64,
    pub ymax: f64,
}

impl BboxL93 {
    pub fn new(xmin: f64, ymin: f64, xmax: f64, ymax: f64) -> Self {
        Self {
            xmin,
            ymin,
            xmax,
            ymax,
        }
    }
    /// Carré centré sur (cx, cy) de demi-côté `half`.
    pub fn around(cx: f64, cy: f64, half: f64) -> Self {
        Self::new(cx - half, cy - half, cx + half, cy + half)
    }
    pub fn width(&self) -> f64 {
        self.xmax - self.xmin
    }
    pub fn height(&self) -> f64 {
        self.ymax - self.ymin
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.xmin && x <= self.xmax && y >= self.ymin && y <= self.ymax
    }
    /// Distance du point (x, y) au rectangle (0 si dedans).
    pub fn distance_to(&self, x: f64, y: f64) -> f64 {
        let dx = (self.xmin - x).max(0.0).max(x - self.xmax);
        let dy = (self.ymin - y).max(0.0).max(y - self.ymax);
        dx.hypot(dy)
    }
    pub fn area_km2(&self) -> f64 {
        self.width() * self.height() / 1e6
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GeoError {
    #[error("E/S : {0}")]
    Io(#[from] std::io::Error),
    #[error("projection : {0}")]
    Proj(String),
    #[error("format raster invalide : {0}")]
    Format(String),
    #[error("encodage PNG : {0}")]
    Png(#[from] png::EncodingError),
}

pub type Result<T> = std::result::Result<T, GeoError>;
