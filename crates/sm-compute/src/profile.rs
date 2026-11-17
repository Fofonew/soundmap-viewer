//! Extraction de profils de terrain (sol, bâti, végétation, classe de sol).

use crate::scene::Scene;
use sm_core::engineering::PathSample;
use sm_core::ground::GroundClass;

/// Échantillon en un point absolu L93 (x du profil fixé par l'appelant).
#[inline]
pub fn sample_at(scene: &Scene, s: f64, x: f64, y: f64) -> PathSample {
    let ground = scene.dem.sample_clamped(x, y).unwrap_or(scene.fallback_z);
    let obstacle = scene.bati.get(x, y).map_or(0.0, |v| f32::from(v) * 0.25);
    // Le MNH contient aussi les bâtiments : on ne garde que la végétation.
    let canopy = if obstacle > 0.0 {
        0.0
    } else {
        scene.canopy.sample(x, y).unwrap_or(0.0).max(0.0)
    };
    PathSample {
        x: s,
        ground,
        obstacle,
        canopy,
        class: scene.landcover.get(x, y).unwrap_or(GroundClass::D.index()),
        foliage: scene
            .vegetation
            .as_ref()
            .and_then(|v| v.foliage_at(x, y))
            .unwrap_or(1.0),
    }
}

/// Profil de `a` à `b` (L93 absolus), pas `step` ; inclut les deux extrémités.
pub fn extract_path(scene: &Scene, a: (f64, f64), b: (f64, f64), step: f64) -> Vec<PathSample> {
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    let n = (len / step).ceil().max(1.0) as usize;
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            sample_at(scene, t * len, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        })
        .collect()
}

/// Rayon depuis `o` (L93 absolu) dans l'azimut `az_deg` (0 = nord, horaire).
pub fn extract_ray(
    scene: &Scene,
    o: (f64, f64),
    az_deg: f64,
    step: f64,
    length: f64,
) -> Vec<PathSample> {
    let (s, c) = az_deg.to_radians().sin_cos();
    let n = (length / step).ceil() as usize;
    (0..=n)
        .map(|i| {
            let d = i as f64 * step;
            sample_at(scene, d, o.0 + s * d, o.1 + c * d)
        })
        .collect()
}
