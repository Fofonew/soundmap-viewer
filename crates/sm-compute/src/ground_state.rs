//! Sol : géologie (BRGM), falaises (pente du MNT) et état du sol à l'heure du
//! calcul (humidité de surface, neige ; Open-Meteo).
//!
//! Règles (heuristiques documentées dans ADR 0010) :
//! - pente ≥ 45° hors forêt et eau → roche (classe G) : parois et falaises ;
//! - roche dure (lithologie BRGM) : matériaux minéraux (F) → roche (G), et
//!   sols herbacés/nus en pente > 25° durcis d'une classe (sols minces) ;
//!   lithologie mixte : même règle au-delà de 35° ;
//! - humidité 0–1 cm : σ × 1 jusqu'à 0,25 m³/m³, puis linéaire jusqu'à × 2,5
//!   à 0,45 m³/m³ (les pores se remplissent d'eau : sol plus résistif) ;
//! - neige ≥ 5 cm : classe A (neige fraîche, σ = 12,5 kPa·s/m²) partout.

use crate::ComputeError;
use crate::scene::Grid;
use serde::Serialize;
use sm_core::ground::GroundClass;
use sm_data::brgm::{Lithology, RockKind};
use sm_data::openmeteo::SoilState;
use sm_geo::raster::MultiResDem;
use std::path::Path;

pub const LITHO_FILE: &str = "geology.json";
pub const SOIL_FILE: &str = "soil.json";

/// Lithologie et état du sol enregistrés dans le dossier de zone.
#[derive(Debug, Clone, Default)]
pub struct GroundData {
    pub lithology: Option<Vec<Lithology>>,
    pub soil: Option<SoilState>,
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let b = std::fs::read(path).ok()?;
    match serde_json::from_slice(&b) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("{} illisible : {e}", path.display());
            None
        }
    }
}

impl GroundData {
    pub fn load(dir: &Path) -> Self {
        Self {
            lithology: read_json(&dir.join(LITHO_FILE)),
            soil: read_json(&dir.join(SOIL_FILE)),
        }
    }
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<(), ComputeError> {
    let b = serde_json::to_vec(v).map_err(|e| ComputeError::Invalid(e.to_string()))?;
    let tmp = path.with_extension("json.part");
    std::fs::write(&tmp, b)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub fn save_lithology(dir: &Path, l: &[Lithology]) -> Result<(), ComputeError> {
    write_json(&dir.join(LITHO_FILE), &l)
}

pub fn save_soil(dir: &Path, s: &SoilState) -> Result<(), ComputeError> {
    write_json(&dir.join(SOIL_FILE), s)
}

/// Bilan des ajustements de la grille de sol.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GroundAdjust {
    /// Surfaces (km²) passées en roche (falaises) et durcies par la géologie.
    pub cliffs_km2: f64,
    pub hardened_km2: f64,
    /// Formations BRGM présentes (description, nature), si la géologie est chargée.
    pub formations: Option<Vec<(String, RockKind)>>,
}

fn harden(c: u8) -> u8 {
    let g = GroundClass::from_index(c);
    match g {
        GroundClass::D => GroundClass::E.index(),
        GroundClass::E => GroundClass::F.index(),
        _ => c,
    }
}

/// Applique falaises et géologie à la grille d'occupation du sol.
pub fn adjust_landcover(
    lc: &mut Grid<u8>,
    dem: &MultiResDem,
    litho: Option<&[Lithology]>,
) -> GroundAdjust {
    // Nature de la roche par cellule : 0 meuble/inconnue, 1 mixte, 2 dure.
    let mut rock = Grid {
        x0: lc.x0,
        y0: lc.y0,
        cell: lc.cell,
        w: lc.w,
        h: lc.h,
        data: vec![0u8; lc.w * lc.h],
    };
    if let Some(l) = litho {
        for p in l {
            let v = match p.kind {
                RockKind::Hard => 2,
                RockKind::Mixed => 1,
                RockKind::Soft => continue,
            };
            rock.fill_polygon(&p.rings, v);
        }
    }
    let half = lc.cell * 0.5;
    let (forest, water) = (GroundClass::C.index(), GroundClass::H.index());
    let (mut cliffs, mut hardened) = (0usize, 0usize);
    for r in 0..lc.h {
        for c in 0..lc.w {
            let i = r * lc.w + c;
            let cls = lc.data[i];
            if cls == forest || cls == water {
                continue;
            }
            let x = lc.x0 + (c as f64 + 0.5) * lc.cell;
            let y = lc.y0 - (r as f64 + 0.5) * lc.cell;
            let z = |x: f64, y: f64| dem.sample_clamped(x, y).map(f64::from);
            let (Some(e), Some(w), Some(n), Some(s)) = (
                z(x + half, y),
                z(x - half, y),
                z(x, y + half),
                z(x, y - half),
            ) else {
                continue;
            };
            let slope = ((e - w) / lc.cell)
                .hypot((n - s) / lc.cell)
                .atan()
                .to_degrees();
            if slope >= 45.0 {
                lc.data[i] = GroundClass::G.index();
                cliffs += 1;
                continue;
            }
            let kind = rock.data[i];
            if kind == 0 {
                continue;
            }
            let new = if cls == GroundClass::F.index() && kind == 2 {
                GroundClass::G.index()
            } else if slope > if kind == 2 { 25.0 } else { 35.0 } {
                harden(cls)
            } else {
                cls
            };
            if new != cls {
                lc.data[i] = new;
                hardened += 1;
            }
        }
    }
    let km2 = |n: usize| n as f64 * lc.cell * lc.cell / 1e6;
    let formations = litho.map(|l| {
        let mut f: Vec<(String, RockKind)> = Vec::new();
        for p in l {
            if !f.iter().any(|(d, _)| *d == p.descr) {
                f.push((p.descr.clone(), p.kind));
            }
        }
        f
    });
    GroundAdjust {
        cliffs_km2: km2(cliffs),
        hardened_km2: km2(hardened),
        formations,
    }
}

/// Télécharge la lithologie BRGM (emprise de la zone) et l'état du sol
/// (veille + jour J) puis les enregistre dans le dossier de zone.
pub async fn fetch(
    client: &sm_data::GeoClient,
    scene: &crate::scene::Scene,
) -> Result<(), ComputeError> {
    let token = sm_data::CancellationToken::new();
    let (ox, oy) = scene.origin;
    let r = scene.radius;
    let bbox = sm_geo::BboxL93::new(ox - r, oy - r, ox + r, oy + r);
    let litho = sm_data::brgm::fetch_lithology(client, &bbox, &token).await?;
    save_lithology(&scene.dir, &litho)?;
    let (lon, lat) = scene
        .local_to_lonlat(0.0, 0.0)
        .ok_or_else(|| ComputeError::Invalid("origine hors projection".into()))?;
    let soil = sm_data::openmeteo::fetch_soil(client, lat, lon, &scene.meteo_date, &token).await?;
    save_soil(&scene.dir, &soil)?;
    Ok(())
}

/// Résumé horaire pour l'UI.
#[derive(Debug, Clone, Serialize)]
pub struct GroundHour {
    pub hour: u8,
    pub state: GroundState,
    pub sigma_factor: f64,
    pub snow: bool,
}

/// État du sol pour chaque heure de la date de la zone.
pub fn hours(scene: &crate::scene::Scene) -> Option<Vec<GroundHour>> {
    scene.soil.as_ref()?;
    Some(
        (0..24u8)
            .filter_map(|h| {
                let s = scene.ground_state(h)?;
                Some(GroundHour {
                    hour: h,
                    sigma_factor: s.sigma_factor(),
                    snow: s.snow(),
                    state: s,
                })
            })
            .collect(),
    )
}

/// État du sol à une heure donnée.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct GroundState {
    pub moisture: Option<f64>,
    pub snow_m: Option<f64>,
    pub rain_24h_mm: f64,
}

impl GroundState {
    /// Facteur sur σ dû à l'humidité de surface.
    pub fn sigma_factor(&self) -> f64 {
        match self.moisture {
            Some(m) => 1.0 + 1.5 * ((m - 0.25) / 0.20).clamp(0.0, 1.0),
            None => 1.0,
        }
    }

    pub fn snow(&self) -> bool {
        self.snow_m.is_some_and(|s| s >= 0.05)
    }
}

/// État du sol pour la date de la zone et l'heure `hour` (0–23).
pub fn state_at(soil: &SoilState, date: &str, hour: u8) -> Option<GroundState> {
    let (moisture, snow_m, rain_24h_mm) = soil.at(&format!("{date}T{hour:02}"))?;
    Some(GroundState {
        moisture,
        snow_m,
        rain_24h_mm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moisture_and_snow() {
        let dry = GroundState {
            moisture: Some(0.15),
            snow_m: Some(0.0),
            rain_24h_mm: 0.0,
        };
        assert_eq!(dry.sigma_factor(), 1.0);
        assert!(!dry.snow());
        let wet = GroundState {
            moisture: Some(0.45),
            snow_m: Some(0.1),
            rain_24h_mm: 12.0,
        };
        assert!((wet.sigma_factor() - 2.5).abs() < 1e-9);
        assert!(wet.snow());
    }

    #[test]
    fn harden_steps_one_class() {
        assert_eq!(harden(GroundClass::D.index()), GroundClass::E.index());
        assert_eq!(harden(GroundClass::E.index()), GroundClass::F.index());
        assert_eq!(harden(GroundClass::C.index()), GroundClass::C.index());
    }
}
