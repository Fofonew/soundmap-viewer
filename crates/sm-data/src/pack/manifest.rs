//! Manifeste versionné d'un pack et journal de progression (reprise).

use super::plan::PlanItem;
use super::spec::ZoneSpec;
use crate::tile_cache::write_atomic_sync;
use crate::wfs::BuildingStats;
use crate::{BboxL93, DataError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;
pub const MANIFEST: &str = "manifest.json";
pub const JOURNAL: &str = "progress.log";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneStatus {
    /// Plan créé, téléchargement non terminé (reprenable).
    Partial,
    /// Tous les items sont là, finalisation à faire.
    Downloaded,
    /// Pack complet, utilisable hors ligne.
    Complete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RingProduct {
    pub layer: String,
    pub cell_m: f64,
    pub bbox: BboxL93,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorProduct {
    pub name: String,
    pub file: String,
    pub count: usize,
}

/// Produits de la finalisation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Products {
    pub rings: Vec<RingProduct>,
    pub vectors: Vec<VectorProduct>,
    pub buildings_ui: Option<String>,
    pub buildings: BuildingStats,
    /// [ouest, sud, est, nord] en degrés.
    pub lonlat_bounds: [f64; 4],
    pub center_lonlat: [f64; 2],
    pub ortho_zooms: Vec<u8>,
    pub ortho_tiles: usize,
    pub terrain_pregen_max_zoom: u8,
    pub terrain_tiles: usize,
    /// "ocsge", "cosia" ou "absente".
    pub landcover: String,
    pub meteo: Option<String>,
    pub elevation_range: Option<(f32, f32)>,
    pub finalize_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub spec: ZoneSpec,
    pub created_at: u64,
    pub updated_at: u64,
    pub status: ZoneStatus,
    /// Hash des URL du plan : change si les sources changent.
    pub sources_hash: String,
    pub items: Vec<PlanItem>,
    #[serde(default)]
    pub downloaded_bytes: u64,
    #[serde(default)]
    pub download_seconds: f64,
    #[serde(default)]
    pub products: Option<Products>,
}

impl Manifest {
    pub fn new(spec: ZoneSpec, items: Vec<PlanItem>) -> Self {
        let now = super::now_unix();
        Self {
            schema_version: SCHEMA_VERSION,
            id: spec.id(),
            sources_hash: sources_hash(&items),
            spec,
            created_at: now,
            updated_at: now,
            status: ZoneStatus::Partial,
            items,
            downloaded_bytes: 0,
            download_seconds: 0.0,
            products: None,
        }
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let bytes = std::fs::read(dir.join(MANIFEST))?;
        let m: Manifest = serde_json::from_slice(&bytes)?;
        if m.schema_version != SCHEMA_VERSION {
            return Err(DataError::Parse(format!(
                "manifeste : schéma {} non géré (attendu {SCHEMA_VERSION})",
                m.schema_version
            )));
        }
        Ok(m)
    }

    pub fn save(&mut self, dir: &Path) -> Result<()> {
        self.updated_at = super::now_unix();
        let bytes = serde_json::to_vec_pretty(self)?;
        write_atomic_sync(&dir.join(MANIFEST), &bytes)
    }

    pub fn summary(&self, dir: &Path) -> ZoneSummary {
        let done = Journal::read_done(dir).len().min(self.items.len());
        ZoneSummary {
            id: self.id.clone(),
            name: self.spec.name.clone(),
            status: self.status,
            radius_m: self.spec.radius_m,
            center_l93: [self.spec.center_x, self.spec.center_y],
            center_lonlat: self.products.as_ref().map(|p| p.center_lonlat),
            done_items: if self.status == ZoneStatus::Complete {
                self.items.len()
            } else {
                done
            },
            total_items: self.items.len(),
            downloaded_bytes: self.downloaded_bytes,
            updated_at: self.updated_at,
            dir: dir.to_path_buf(),
        }
    }
}

pub fn sources_hash(items: &[PlanItem]) -> String {
    let mut h = Sha256::new();
    for it in items {
        h.update(it.id.as_bytes());
        if let super::plan::Fetch::Bytes { url, .. } = &it.fetch {
            h.update(url.as_bytes());
        }
    }
    h.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Résumé pour la liste des zones.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ZoneSummary {
    pub id: String,
    pub name: String,
    pub status: ZoneStatus,
    pub radius_m: f64,
    pub center_l93: [f64; 2],
    pub center_lonlat: Option<[f64; 2]>,
    pub done_items: usize,
    pub total_items: usize,
    pub downloaded_bytes: u64,
    pub updated_at: u64,
    pub dir: PathBuf,
}

/// Zones présentes dans `root` (les plus récentes d'abord).
pub fn list_zones(root: &Path) -> Vec<ZoneSummary> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<ZoneSummary> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join(MANIFEST).exists())
        .filter_map(|e| {
            let dir = e.path();
            Manifest::load(&dir).ok().map(|m| m.summary(&dir))
        })
        .collect();
    out.sort_by_key(|z| std::cmp::Reverse(z.updated_at));
    out
}

/// Journal append-only : une ligne `{id}\t{octets}` par item terminé.
/// Bien moins coûteux que de réécrire le manifeste après chaque tuile.
pub struct Journal {
    file: std::fs::File,
}

impl Journal {
    pub fn open(dir: &Path) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(JOURNAL))?;
        Ok(Self { file })
    }

    pub fn record(&mut self, id: &str, bytes: u64) -> Result<()> {
        writeln!(self.file, "{id}\t{bytes}")?;
        Ok(())
    }

    /// Identifiants terminés (une ligne tronquée par une coupure est ignorée).
    pub fn read_done(dir: &Path) -> HashSet<String> {
        std::fs::read_to_string(dir.join(JOURNAL))
            .map(|s| {
                s.lines()
                    .filter_map(|l| l.split_once('\t').map(|(id, _)| id.to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Octets déjà téléchargés selon le journal.
    pub fn read_bytes(dir: &Path) -> u64 {
        std::fs::read_to_string(dir.join(JOURNAL))
            .map(|s| {
                s.lines()
                    .filter_map(|l| l.split_once('\t').and_then(|(_, b)| b.parse::<u64>().ok()))
                    .sum()
            })
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_ignores_truncated_line() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut j = Journal::open(dir.path()).unwrap();
            j.record("a", 10).unwrap();
            j.record("b", 5).unwrap();
        }
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join(JOURNAL))
            .unwrap()
            .write_all(b"c")
            .unwrap();
        let done = Journal::read_done(dir.path());
        assert_eq!(done.len(), 2);
        assert_eq!(Journal::read_bytes(dir.path()), 15);
    }

    #[test]
    fn manifest_round_trip_and_listing() {
        let dir = tempfile::tempdir().unwrap();
        let spec = ZoneSpec::new("Test", 707_460.0, 6_333_667.0, 1000.0, "2026-11-17");
        let zdir = dir.path().join(spec.id());
        std::fs::create_dir_all(&zdir).unwrap();
        let mut m = Manifest::new(spec, vec![]);
        m.save(&zdir).unwrap();
        let m2 = Manifest::load(&zdir).unwrap();
        assert_eq!(m2.id, m.id);
        let zones = list_zones(dir.path());
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].status, ZoneStatus::Partial);
    }
}
