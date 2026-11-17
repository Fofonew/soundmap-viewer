//! Définition d'une zone de travail.

use crate::BboxL93;
use crate::wmsr::ElevationLayer;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Anneau de résolution : carré de demi-côté `half_size_m` au pas `cell_m`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ring {
    pub half_size_m: f64,
    pub cell_m: f64,
}

/// Niveau de zoom ortho téléchargé jusqu'à `radius_m` de la sono.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OrthoLevel {
    pub zoom: u8,
    pub radius_m: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneSpec {
    pub name: String,
    /// Emplacement de la sono, Lambert-93 (m).
    pub center_x: f64,
    pub center_y: f64,
    pub radius_m: f64,
    /// Anneaux du plus fin au plus grossier ; le dernier est étendu au rayon.
    pub rings: Vec<Ring>,
    pub elevation_layers: Vec<ElevationLayer>,
    /// Côté des blocs WMS-R en pixels (1000 : format validé en direct).
    pub block_px: u32,
    pub ortho: Vec<OrthoLevel>,
    pub ocsge_zoom: u8,
    /// Jour de la prévision météo (AAAA-MM-JJ).
    pub meteo_date: String,
    /// Zoom max des tuiles Terrarium pré-générées à la finalisation.
    pub terrain_pregen_max_zoom: u8,
}

impl ZoneSpec {
    /// Zone avec les réglages par défaut (RECHERCHE.md §2.4) :
    /// 0,5 m jusqu'à 500 m, 1 m jusqu'à 2 km, 5 m au-delà ; ortho z≤17 sur
    /// tout le disque, z18 sur 1 km, z19 sur 300 m.
    pub fn new(name: &str, center_x: f64, center_y: f64, radius_m: f64, meteo_date: &str) -> Self {
        let mut ortho: Vec<OrthoLevel> = (10..=17)
            .map(|zoom| OrthoLevel { zoom, radius_m })
            .collect();
        ortho.push(OrthoLevel {
            zoom: 18,
            radius_m: 1000.0,
        });
        ortho.push(OrthoLevel {
            zoom: 19,
            radius_m: 300.0,
        });
        Self {
            name: name.to_owned(),
            // Centre calé sur la grille de 5 m : les anneaux restent alignés.
            center_x: (center_x / 5.0).round() * 5.0,
            center_y: (center_y / 5.0).round() * 5.0,
            radius_m,
            rings: vec![
                Ring {
                    half_size_m: 500.0,
                    cell_m: 0.5,
                },
                Ring {
                    half_size_m: 2000.0,
                    cell_m: 1.0,
                },
                Ring {
                    half_size_m: radius_m,
                    cell_m: 5.0,
                },
            ],
            elevation_layers: vec![ElevationLayer::Mnt, ElevationLayer::Mns],
            block_px: 1000,
            ortho,
            ocsge_zoom: 16,
            meteo_date: meteo_date.to_owned(),
            terrain_pregen_max_zoom: 15,
        }
    }

    /// Anneaux effectifs : demi-côtés bornés au rayon, triés, sans doublon ;
    /// le plus grossier couvre tout le rayon.
    pub fn effective_rings(&self) -> Vec<Ring> {
        let mut rings: Vec<Ring> = self
            .rings
            .iter()
            .map(|r| Ring {
                half_size_m: r.half_size_m.min(self.radius_m),
                cell_m: r.cell_m,
            })
            .filter(|r| r.half_size_m > 0.0 && r.cell_m > 0.0)
            .collect();
        rings.sort_by(|a, b| a.cell_m.total_cmp(&b.cell_m));
        if let Some(last) = rings.last_mut() {
            last.half_size_m = self.radius_m;
        }
        // Un anneau qui ne dépasse pas l'anneau plus fin précédent est inutile.
        let mut out: Vec<Ring> = Vec::new();
        for r in rings {
            match out.last() {
                Some(prev) if r.half_size_m <= prev.half_size_m => {}
                _ => out.push(r),
            }
        }
        out
    }

    /// Emprise carrée de l'anneau : alignée sur son pas.
    pub fn ring_bbox(&self, r: &Ring) -> BboxL93 {
        let half = (r.half_size_m / r.cell_m).ceil() * r.cell_m;
        BboxL93::around(self.center_x, self.center_y, half)
    }

    /// Emprise carrée du disque d'étude.
    pub fn bbox(&self) -> BboxL93 {
        BboxL93::around(self.center_x, self.center_y, self.radius_m)
    }

    /// Identifiant stable : `{slug}-{8 hex}` (hash du centre et du rayon).
    pub fn id(&self) -> String {
        let mut h = Sha256::new();
        h.update(format!(
            "{:.0}|{:.0}|{:.0}",
            self.center_x, self.center_y, self.radius_m
        ));
        let hex: String = h.finalize()[..4]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let slug: String = self
            .name
            .to_lowercase()
            .chars()
            .map(|c| match c {
                'a'..='z' | '0'..='9' => c,
                'à' | 'â' | 'ä' => 'a',
                'é' | 'è' | 'ê' | 'ë' => 'e',
                'î' | 'ï' => 'i',
                'ô' | 'ö' => 'o',
                'ù' | 'û' | 'ü' => 'u',
                'ç' => 'c',
                _ => '-',
            })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let slug = if slug.is_empty() { "zone".into() } else { slug };
        format!("{}-{hex}", slug.chars().take(40).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rings_are_clamped_to_radius() {
        let s = ZoneSpec::new("t", 707_460.0, 6_333_667.0, 1500.0, "2026-11-17");
        let r = s.effective_rings();
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].half_size_m, r[0].cell_m), (500.0, 0.5));
        assert_eq!((r[1].half_size_m, r[1].cell_m), (1500.0, 1.0));
    }

    #[test]
    fn default_rings_for_5km() {
        let s = ZoneSpec::new("t", 707_460.0, 6_333_667.0, 5000.0, "2026-11-17");
        let r = s.effective_rings();
        assert_eq!(r.len(), 3);
        assert_eq!(r[2].half_size_m, 5000.0);
        assert_eq!(s.center_x, 707_460.0);
        assert_eq!(s.center_y, 6_333_665.0);
    }

    #[test]
    fn id_is_stable_and_slugged() {
        let a = ZoneSpec::new(
            "Causse Noir – test",
            707_460.0,
            6_333_667.0,
            5000.0,
            "2026-11-17",
        );
        let b = ZoneSpec::new(
            "Causse Noir – test",
            707_461.0,
            6_333_666.0,
            5000.0,
            "2026-11-18",
        );
        assert_eq!(a.id(), b.id());
        assert!(a.id().starts_with("causse-noir-test-"));
    }
}
