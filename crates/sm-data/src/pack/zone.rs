//! Zone ouverte en lecture : manifeste + MNT memory-mappé. Aucun accès réseau.

use super::finalize::{open_dem, terrain_path};
use super::manifest::{Manifest, ZoneStatus};
use crate::tile_cache::write_atomic_sync;
use crate::{DataError, Result};
use sm_geo::raster::MultiResDem;
use sm_geo::{Lambert93, TileId, terrarium};
use std::path::{Path, PathBuf};

/// Zoom max des tuiles terrain générées à la demande (≈ 0,6 m/px à 44° N).
pub const TERRAIN_MAX_ZOOM: u8 = 17;

#[derive(Debug)]
pub struct OpenZone {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub dem: MultiResDem,
    proj: Lambert93,
    fallback: f32,
}

impl OpenZone {
    pub fn open(dir: &Path) -> Result<Self> {
        let manifest = Manifest::load(dir)?;
        if manifest.status != ZoneStatus::Complete {
            return Err(DataError::Invalid(format!(
                "zone {} non finalisée ({:?})",
                manifest.id, manifest.status
            )));
        }
        let products = manifest
            .products
            .as_ref()
            .ok_or_else(|| DataError::Invalid("produits absents du manifeste".into()))?;
        let dem = open_dem(dir, products)?;
        let fallback = products.elevation_range.map_or(0.0, |(a, b)| (a + b) / 2.0);
        Ok(Self {
            dir: dir.to_path_buf(),
            manifest,
            dem,
            proj: Lambert93::new()?,
            fallback,
        })
    }

    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    /// Tuile Terrarium : pré-générée, sinon calculée puis mise en cache disque.
    pub fn terrain_tile(&self, t: TileId) -> Result<Option<Vec<u8>>> {
        if t.z > TERRAIN_MAX_ZOOM {
            return Ok(None);
        }
        let path = terrain_path(&self.dir, t);
        if let Ok(b) = std::fs::read(&path) {
            return Ok(Some(b));
        }
        let png = terrarium::render_tile(&self.dem, &self.proj, t, 256, self.fallback)?;
        if let Err(e) = write_atomic_sync(&path, &png) {
            tracing::warn!(err = %e, "cache terrain non écrit");
        }
        Ok(Some(png))
    }

    /// Fichier du pack (chemin relatif, sans remontée hors du dossier).
    pub fn file(&self, rel: &str) -> Option<PathBuf> {
        if rel.split(['/', '\\']).any(|c| c == ".." || c.contains(':')) {
            return None;
        }
        Some(self.dir.join(rel))
    }

    /// Tuile stockée (ortho, ocsge…) ; `None` si absente ou marquée vide.
    pub fn stored_tile(&self, kind: &str, t: TileId, ext: &str) -> Option<Vec<u8>> {
        let p = self.file(&format!("{kind}/{}/{}/{}.{ext}", t.z, t.x, t.y))?;
        std::fs::read(p).ok().filter(|b| !b.is_empty())
    }

    /// Altitude du MNT en (lon, lat).
    pub fn elevation_at(&self, lon: f64, lat: f64) -> Option<f32> {
        let (x, y) = self.proj.from_wgs84(lon, lat).ok()?;
        self.dem.sample(x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_path_traversal() {
        // Construit une OpenZone minimale sans passer par open().
        let z = OpenZone {
            dir: PathBuf::from("zones/x"),
            manifest: Manifest::new(
                super::super::ZoneSpec::new("x", 7e5, 6.6e6, 1000.0, "2026-11-17"),
                vec![],
            ),
            dem: MultiResDem::default(),
            proj: Lambert93::new().unwrap(),
            fallback: 0.0,
        };
        assert!(z.file("../secret").is_none());
        assert!(z.file("C:/Windows").is_none());
        assert!(z.file("ui/buildings.geojson").is_some());
    }
}
