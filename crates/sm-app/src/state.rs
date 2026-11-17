//! État partagé de l'application.

use anyhow::Context;
use sm_data::pack::OpenZone;
use sm_data::tile_cache::TileCache;
use sm_data::{CancellationToken, GeoClient};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub client: GeoClient,
    pub cache: TileCache,
    pub zones_dir: PathBuf,
    /// `SOUNDMAP_OFFLINE=1` : le proxy ne sert que le cache (test hors ligne).
    pub offline: bool,
    pub open_zones: Mutex<HashMap<String, Arc<OpenZone>>>,
    pub downloads: Mutex<HashMap<String, CancellationToken>>,
    pub sim: crate::sim::SimStore,
}

impl AppState {
    /// Dossiers : `SOUNDMAP_ZONES_DIR` / `SOUNDMAP_CACHE_DIR`, sinon le dossier
    /// de données de l'application.
    pub fn new(app_data: &Path) -> anyhow::Result<Self> {
        let env_dir = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let zones_dir = env_dir("SOUNDMAP_ZONES_DIR").unwrap_or_else(|| app_data.join("zones"));
        let cache_dir = env_dir("SOUNDMAP_CACHE_DIR").unwrap_or_else(|| app_data.join("cache"));
        std::fs::create_dir_all(&zones_dir).context("création du dossier des zones")?;
        std::fs::create_dir_all(&cache_dir).context("création du cache")?;
        Ok(Self {
            client: GeoClient::public()?,
            cache: TileCache::new(cache_dir),
            zones_dir: std::path::absolute(zones_dir)?,
            offline: std::env::var("SOUNDMAP_OFFLINE").is_ok_and(|v| v == "1"),
            open_zones: Mutex::new(HashMap::new()),
            downloads: Mutex::new(HashMap::new()),
            sim: crate::sim::SimStore::default(),
        })
    }

    pub fn zone_dir(&self, id: &str) -> anyhow::Result<PathBuf> {
        anyhow::ensure!(
            !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "identifiant de zone invalide"
        );
        Ok(self.zones_dir.join(id))
    }

    /// Zone ouverte (mise en cache mémoire ; le MNT est memory-mappé).
    pub fn zone(&self, id: &str) -> anyhow::Result<Arc<OpenZone>> {
        if let Some(z) = self.open_zones.lock().map_err(lock_err)?.get(id) {
            return Ok(z.clone());
        }
        let z = Arc::new(OpenZone::open(&self.zone_dir(id)?)?);
        self.open_zones
            .lock()
            .map_err(lock_err)?
            .insert(id.to_owned(), z.clone());
        Ok(z)
    }

    pub fn forget_zone(&self, id: &str) {
        if let Ok(mut m) = self.open_zones.lock() {
            m.remove(id);
        }
    }
}

pub fn lock_err<T>(_: std::sync::PoisonError<T>) -> anyhow::Error {
    anyhow::anyhow!("verrou empoisonné")
}
