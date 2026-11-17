//! Conditions météo : profil AROME ajusté (Monin-Obukhov), repli Pasquill,
//! scénarios manuels pour l'enveloppe d'incertitude.

use crate::scene::Scene;
use serde::{Deserialize, Serialize};
use sm_core::atmosphere::{MoProfile, Pasquill, ProfileObs, fit_monin_obukhov};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    pub id: String,
    pub label: String,
    pub profile: MoProfile,
}

const Z0: f64 = 0.05;

/// Profil observé AROME à l'heure `hour` (si la météo est dans le pack).
pub fn observation(scene: &Scene, hour: u8) -> Option<ProfileObs> {
    let f = scene.meteo.as_ref()?;
    let p = f.profile(f.index_of_hour(hour)?)?;
    Some(ProfileObs {
        temperature: p
            .temperature
            .iter()
            .map(|(z, t)| (f64::from(*z), *t))
            .collect(),
        wind: p
            .wind
            .iter()
            .map(|(z, u, d)| (f64::from(*z), *u, *d))
            .collect(),
        rh: p.relative_humidity.unwrap_or(70.0),
        cloud_cover: p.cloud_cover,
    })
}

/// Profil de la prévision : ajustement MO, sinon classe de Pasquill.
pub fn forecast_profile(scene: &Scene, hour: u8) -> Option<MoProfile> {
    let obs = observation(scene, hour)?;
    if obs.temperature.len() >= 3
        && let Some(mut p) = fit_monin_obukhov(&obs, Z0)
    {
        p.source = format!("AROME {} {hour:02}h (Monin-Obukhov)", scene.meteo_date);
        return Some(p);
    }
    let (_, u10, dir) = obs.wind.first().copied()?;
    let t2 = obs.temperature.first().map_or(15.0, |t| t.1);
    let night = !(7..=19).contains(&hour);
    let cls = Pasquill::from_conditions(u10, obs.cloud_cover.unwrap_or(50.0), night);
    Some(cls.profile(u10, dir.unwrap_or(0.0), t2, obs.rh, Z0))
}

/// Scénarios standard : prévision, neutre, inversion forte, vent portant
/// vers l'azimut critique (habitations les plus exposées).
pub fn default_scenarios(scene: &Scene, hour: u8, critical_az_deg: f64) -> Vec<Scenario> {
    let obs = observation(scene, hour);
    let t2 = obs
        .as_ref()
        .and_then(|o| o.temperature.first().map(|t| t.1))
        .unwrap_or(12.0);
    let rh = obs.as_ref().map_or(75.0, |o| o.rh);
    let mut out = Vec::new();
    if let Some(p) = forecast_profile(scene, hour) {
        out.push(Scenario {
            id: "prevision".into(),
            label: p.source.clone(),
            profile: p,
        });
    }
    out.push(Scenario {
        id: "neutre".into(),
        label: "Neutre, sans vent".into(),
        profile: MoProfile::homogeneous(t2, rh),
    });
    let mut inv = Pasquill::F.profile(1.0, critical_az_deg + 180.0, t2, rh, Z0);
    inv.source = "Inversion forte (Pasquill F, vent 1 m/s)".into();
    out.push(Scenario {
        id: "inversion".into(),
        label: inv.source.clone(),
        profile: inv,
    });
    let mut down =
        Pasquill::D.profile(5.0, (critical_az_deg + 180.0).rem_euclid(360.0), t2, rh, Z0);
    down.source = format!("Vent portant 5 m/s vers {critical_az_deg:.0}°");
    out.push(Scenario {
        id: "vent_portant".into(),
        label: down.source.clone(),
        profile: down,
    });
    // Sono en fond de vallée : lac d'air froid + vent catabatique nocturnes.
    let va = crate::valley::analyze(scene);
    if va.valley {
        let mut v = Pasquill::F.profile(1.0, va.downslope_toward_deg + 180.0, t2, rh, Z0);
        let flow = va.flow();
        v.valley = Some(flow);
        v.source = format!(
            "Vallée : lac d'air froid {:.0} m / −{:.0} K, catabatique {:.1} m/s vers {:.0}°",
            flow.pool_depth_m, flow.pool_dt_k, flow.kat_umax, flow.kat_toward_deg
        );
        out.push(Scenario {
            id: "vallee".into(),
            label: v.source.clone(),
            profile: v,
        });
    }
    out
}
