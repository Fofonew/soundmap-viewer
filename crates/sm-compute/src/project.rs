//! Projet `.smproj` : enceintes, dancefloor et réglages, en WGS84 côté UI.

use crate::scene::Scene;
use serde::{Deserialize, Serialize};
use sm_core::engineering::Condition;
use sm_core::ground::ImpedanceModel;
use sm_core::source::{Source, SpeakerKind, SpeakerModel};

pub const PROJECT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerSpec {
    pub id: String,
    pub name: String,
    /// Modèle complet (copie du preset, modifiable en saisie manuelle).
    pub model: SpeakerModel,
    pub lon: f64,
    pub lat: f64,
    pub height_m: f64,
    pub azimuth_deg: f64,
    #[serde(default)]
    pub tilt_deg: f64,
    #[serde(default)]
    pub delay_ms: f64,
    /// +1 ou −1.
    #[serde(default = "one")]
    pub polarity: i8,
    #[serde(default)]
    pub gain_db: f64,
    #[serde(default)]
    pub muted: bool,
}

fn one() -> i8 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResidualSettings {
    /// Bruit résiduel rural de nuit par défaut, dB(A).
    pub rural_night_dba: f64,
    /// Majoration à proximité des routes (modèle simple).
    pub include_roads: bool,
    /// Mesure saisie par l'utilisateur, remplace le modèle si présente.
    pub measured_dba: Option<f64>,
}

impl Default for ResidualSettings {
    fn default() -> Self {
        Self {
            rural_night_dba: 25.0,
            include_roads: true,
            measured_dba: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetSettings {
    /// Niveau visé au centre du dancefloor.
    pub dba: f64,
    pub dbc: f64,
}

impl Default for TargetSettings {
    fn default() -> Self {
        Self {
            dba: 100.0,
            dbc: 115.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub program: String,
    pub master_gain_db: f64,
    /// Hauteur des récepteurs de la carte (m).
    pub receiver_height_m: f64,
    pub condition: Condition,
    pub impedance_model: ImpedanceModel,
    /// Heure de la prévision AROME utilisée (0–23).
    pub meteo_hour: u8,
    /// Seuils d'émergence choisis par l'utilisateur (dB), croissants.
    pub thresholds_db: Vec<f64>,
    pub residual: ResidualSettings,
    pub target: TargetSettings,
    /// Paramètres calés sur des mesures (sonomètre), appliqués à la méthode
    /// d'ingénierie : facteur σ, réfraction, décalage de niveau.
    #[serde(default)]
    pub calibration: Option<crate::calibrate::Calibration>,
    /// Tenir compte de l'état du sol (humidité, neige) à l'heure du calcul.
    #[serde(default = "yes")]
    pub use_ground_state: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            program: "tekno".into(),
            master_gain_db: 0.0,
            receiver_height_m: 1.5,
            condition: Condition::Favorable,
            impedance_model: ImpedanceModel::DelanyBazley,
            meteo_hour: 23,
            thresholds_db: vec![3.0, 5.0, 10.0],
            residual: ResidualSettings::default(),
            target: TargetSettings::default(),
            calibration: None,
            use_ground_state: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub zone_id: String,
    pub speakers: Vec<SpeakerSpec>,
    /// Polygone du dancefloor [lon, lat].
    #[serde(default)]
    pub dancefloor: Option<Vec<[f64; 2]>>,
    #[serde(default)]
    pub settings: Settings,
}

impl Project {
    pub fn new(name: &str, zone_id: &str) -> Self {
        Self {
            version: PROJECT_VERSION,
            name: name.into(),
            zone_id: zone_id.into(),
            speakers: Vec::new(),
            dancefloor: None,
            settings: Settings::default(),
        }
    }

    /// Sources en coordonnées locales de la scène (origine = centre de la zone).
    pub fn sources(&self, scene: &Scene) -> Vec<Source> {
        self.speakers
            .iter()
            .filter(|s| !s.muted)
            .filter_map(|s| {
                let (x, y) = scene.lonlat_to_local(s.lon, s.lat)?;
                let ground = scene.ground_local(x, y);
                Some(Source {
                    x,
                    y,
                    z: ground + s.height_m,
                    height: s.height_m,
                    azimuth_deg: s.azimuth_deg,
                    tilt_deg: s.tilt_deg,
                    delay_s: s.delay_ms / 1000.0,
                    polarity: if s.polarity < 0 { -1.0 } else { 1.0 },
                    gain_db: s.gain_db
                        + self.settings.master_gain_db
                        + self.settings.calibration.map_or(0.0, |c| c.offset_db),
                    model: s.model.clone(),
                })
            })
            .collect()
    }

    /// Configuration de la méthode d'ingénierie : condition, modèle d'impédance,
    /// calage éventuel et état du sol à l'heure du calcul.
    pub fn path_config(&self, scene: &Scene) -> sm_core::engineering::PathConfig {
        let st = &self.settings;
        let cal = st.calibration;
        let ground = if st.use_ground_state {
            scene.ground_state(st.meteo_hour)
        } else {
            None
        };
        sm_core::engineering::PathConfig {
            condition: st.condition,
            model: st.impedance_model,
            sigma_scale: cal.map_or(1.0, |c| c.sigma_scale)
                * ground.map_or(1.0, |g| g.sigma_factor()),
            refraction: cal.map(|c| c.refraction),
            snow: ground.is_some_and(|g| g.snow()),
            ..sm_core::engineering::PathConfig::default()
        }
    }

    /// Dancefloor en coordonnées locales.
    pub fn dancefloor_local(&self, scene: &Scene) -> Option<Vec<(f64, f64)>> {
        let poly: Vec<(f64, f64)> = self
            .dancefloor
            .as_ref()?
            .iter()
            .filter_map(|p| scene.lonlat_to_local(p[0], p[1]))
            .collect();
        (poly.len() >= 3).then_some(poly)
    }
}

/// Hauteurs moyennes des groupes (subs, autres) pour l'effet de sol.
pub fn group_heights(sources: &[Source]) -> (f64, f64) {
    let mean = |kind: fn(&SpeakerKind) -> bool, default: f64| {
        let v: Vec<f64> = sources
            .iter()
            .filter(|s| kind(&s.model.kind))
            .map(|s| s.height)
            .collect();
        if v.is_empty() {
            default
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };
    (
        mean(|k| *k == SpeakerKind::Sub, 0.5),
        mean(|k| *k != SpeakerKind::Sub, 2.5),
    )
}

/// Barycentre (x, y) des sources actives.
pub fn centroid(sources: &[Source]) -> Option<(f64, f64)> {
    if sources.is_empty() {
        return None;
    }
    let n = sources.len() as f64;
    Some((
        sources.iter().map(|s| s.x).sum::<f64>() / n,
        sources.iter().map(|s| s.y).sum::<f64>() / n,
    ))
}
