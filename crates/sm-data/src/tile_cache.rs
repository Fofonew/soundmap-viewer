//! Cache disque de tuiles `{root}/{layer}/{z}/{x}/{y}.{ext}`, écriture atomique.
//!
//! Pas de SQLite (dépendance C) : un fichier par tuile, ce qui suffit
//! largement pour le mode Exploration et les packs de zone.

use crate::Result;
use bytes::Bytes;
use sm_geo::TileId;
use std::path::{Path, PathBuf};

/// Marqueur stocké pour une tuile absente côté serveur (évite de redemander).
const EMPTY_MARKER: &[u8] = b"";

#[derive(Debug, Clone)]
pub struct TileCache {
    root: PathBuf,
}

impl TileCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, layer: &str, t: TileId, ext: &str) -> PathBuf {
        self.root
            .join(layer)
            .join(t.z.to_string())
            .join(t.x.to_string())
            .join(format!("{}.{ext}", t.y))
    }

    /// `Some(Some(bytes))` : tuile en cache ; `Some(None)` : absente côté serveur ;
    /// `None` : pas en cache.
    pub async fn get(&self, layer: &str, t: TileId, ext: &str) -> Option<Option<Bytes>> {
        match tokio::fs::read(self.path(layer, t, ext)).await {
            Ok(b) if b.is_empty() => Some(None),
            Ok(b) => Some(Some(Bytes::from(b))),
            Err(_) => None,
        }
    }

    pub async fn put(&self, layer: &str, t: TileId, ext: &str, data: Option<&[u8]>) -> Result<()> {
        write_atomic(&self.path(layer, t, ext), data.unwrap_or(EMPTY_MARKER)).await
    }
}

/// Écrit `data` dans `path` via un fichier `.part` puis un renommage.
pub async fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.part", fastrand::u32(..)));
    let tmp = PathBuf::from(tmp);
    tokio::fs::write(&tmp, data).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

/// Version synchrone de [`write_atomic`].
pub fn write_atomic_sync(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.part", fastrand::u32(..)));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_get_and_empty_marker() {
        let dir = tempfile::tempdir().unwrap();
        let c = TileCache::new(dir.path());
        let t = TileId::new(14, 8332, 5951);
        assert!(c.get("ortho", t, "jpg").await.is_none());
        c.put("ortho", t, "jpg", Some(b"abc")).await.unwrap();
        assert_eq!(
            c.get("ortho", t, "jpg").await,
            Some(Some(Bytes::from_static(b"abc")))
        );
        let t2 = TileId::new(14, 1, 1);
        c.put("ortho", t2, "jpg", None).await.unwrap();
        assert_eq!(c.get("ortho", t2, "jpg").await, Some(None));
        assert!(
            c.path("ortho", t, "jpg")
                .ends_with("ortho/14/8332/5951.jpg")
        );
    }
}
