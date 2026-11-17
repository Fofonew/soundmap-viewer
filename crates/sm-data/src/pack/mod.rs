//! Packs de zone : tout ce qu'il faut pour travailler hors ligne sur une zone.
//!
//! Cycle de vie : [`ZoneSpec`] → [`plan::build_plan`] → [`plan::estimate`]
//! (validation par l'utilisateur) → [`execute::download`] (reprenable) →
//! [`finalize::finalize`] → [`zone::OpenZone`] (lecture seule, memory-mappée).
//!
//! Arborescence d'un pack `zones/{id}/` :
//! ```text
//! manifest.json        spec, plan, état, produits (schéma versionné)
//! progress.log         journal append-only des items terminés (reprise)
//! raw/                 blocs BIL bruts (supprimés après assemblage), GeoJSON WFS, météo brute
//! rasters/             {mnt,mns,mnh}_r{i}.smr — float32 tuilés, memory-mappés
//! vectors/             *.fgb (Lambert-93)
//! ui/buildings.geojson bâtiments en WGS84 pour l'extrusion MapLibre
//! ortho/{z}/{x}/{y}.jpg, ocsge/{z}/{x}/{y}.pbf, terrain/{z}/{x}/{y}.png
//! meteo.json           prévision AROME normalisée
//! ```

pub mod execute;
pub mod finalize;
pub mod manifest;
pub mod plan;
pub mod spec;
pub mod zone;

pub use execute::{PackProgress, Stage, download};
pub use finalize::finalize;
pub use manifest::{Manifest, ZoneStatus, ZoneSummary, list_zones};
pub use plan::{Category, Estimate, PlanItem, build_plan, estimate};
pub use spec::{OrthoLevel, Ring, ZoneSpec};
pub use zone::OpenZone;

/// Date du jour (UTC) au format AAAA-MM-JJ, sans dépendance calendrier.
pub fn today_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    civil_from_days((secs / 86_400) as i64)
}

/// Algorithme de H. Hinnant : jours depuis 1970-01-01 → date civile.
pub fn civil_from_days(z: i64) -> String {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Horodatage Unix courant (s).
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), "1970-01-01");
        assert_eq!(civil_from_days(20_774), "2026-11-17");
        assert_eq!(civil_from_days(11_016), "2000-02-29");
    }
}
