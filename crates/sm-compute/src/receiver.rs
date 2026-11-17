//! Calcul ponctuel (un récepteur) : coupe de terrain et récepteurs en façade.

use crate::pe::PeField;
use crate::polar::Air;
use crate::profile::extract_path;
use crate::project::group_heights;
use crate::scene::Scene;
use serde::Serialize;
use sm_core::atmosphere::{iso9613_1_alpha, sound_speed};
use sm_core::engineering::{BandSpec, PathConfig, PathInfo, Scratch, path_excess};
use sm_core::source::{Program, Source, band_energy};

/// Niveaux par bande en un point (coordonnées locales), méthode d'ingénierie
/// éventuellement fusionnée avec le champ GFPE.
#[allow(clippy::too_many_arguments)]
pub fn levels_at(
    scene: &Scene,
    sources: &[Source],
    program: &Program,
    bands: &[BandSpec],
    cfg: &PathConfig,
    air: Air,
    x: f64,
    y: f64,
    hr: f64,
    step: f64,
    pe: Option<&PeField>,
    scratch: &mut Scratch,
) -> (Vec<f64>, PathInfo) {
    let path = extract_path(scene, scene.origin, scene.abs(x, y), step);
    let info = levels_on_path(
        &path, sources, program, bands, cfg, air, x, y, hr, pe, scratch,
    );
    (info.0, info.1)
}

/// Comme [`levels_at`], sur un profil déjà extrait (origine → point).
#[allow(clippy::too_many_arguments)]
pub fn levels_on_path(
    path: &[sm_core::engineering::PathSample],
    sources: &[Source],
    program: &Program,
    bands: &[BandSpec],
    cfg: &PathConfig,
    air: Air,
    x: f64,
    y: f64,
    hr: f64,
    pe: Option<&PeField>,
    scratch: &mut Scratch,
) -> (Vec<f64>, PathInfo) {
    let (hs_sub, hs_top) = group_heights(sources);
    let cfg = &cfg.for_azimuth(x.atan2(y).to_degrees().rem_euclid(360.0));
    let mut ex_sub = vec![0.0; bands.len()];
    let mut ex_top = vec![0.0; bands.len()];
    let info = path_excess(path, hs_sub, hr, bands, cfg, scratch, &mut ex_sub);
    path_excess(path, hs_top, hr, bands, cfg, scratch, &mut ex_top);
    let zr = f64::from(path.last().map_or(0.0, |p| p.ground)) + hr;
    let c = sound_speed(air.t_c);
    let r = x.hypot(y);
    let mut out = Vec::with_capacity(bands.len());
    for (i, band) in bands.iter().enumerate() {
        let (e_sub, e_top) = band_energy(
            sources,
            [x, y, zr],
            band.fc,
            band.lo,
            band.hi,
            band.width_thirds,
            program,
            c,
            3,
        );
        let (mut es, mut et) = (ex_sub[i], ex_top[i]);
        if let Some(f) = pe
            && let Some((v, w)) = f.sample(i, x, y)
            && w > 0.0
        {
            let blend = |e: f64| {
                10.0 * (w * 10f64.powf(v / 10.0) + (1.0 - w) * 10f64.powf(e / 10.0)).log10()
            };
            es = blend(es);
            et = blend(et);
        }
        let e = e_sub * 10f64.powf(es / 10.0) + e_top * 10f64.powf(et / 10.0);
        out.push(
            10.0 * e.max(1e-12).log10() - iso9613_1_alpha(band.fc, air.t_c, air.rh, 101.325) * r,
        );
    }
    (out, info)
}

/// Coupe de terrain pour l'UI.
#[derive(Debug, Clone, Serialize)]
pub struct Section {
    /// (distance m, sol, sommet obstacle, sommet végétation)
    pub samples: Vec<[f32; 4]>,
    pub source_z: f64,
    pub receiver_z: f64,
    pub distance: f64,
    pub info: PathInfo,
    /// (fréquence nominale, niveau dB) par bande.
    pub bands: Vec<(f64, f64)>,
    pub dba: f64,
    pub dbc: f64,
}

#[allow(clippy::too_many_arguments)]
pub fn section(
    scene: &Scene,
    sources: &[Source],
    program: &Program,
    bands: &[BandSpec],
    nominal: &[f64],
    cfg: &PathConfig,
    air: Air,
    x: f64,
    y: f64,
    hr: f64,
    pe: Option<&PeField>,
) -> Section {
    let mut sc = Scratch::default();
    let d = x.hypot(y);
    let step = (d / 600.0).max(1.0);
    let (levels, info) = levels_at(
        scene, sources, program, bands, cfg, air, x, y, hr, step, pe, &mut sc,
    );
    let path = extract_path(scene, scene.origin, scene.abs(x, y), step);
    let samples = path
        .iter()
        .map(|p| {
            [
                p.x as f32,
                p.ground,
                p.ground + p.obstacle,
                p.ground + p.canopy,
            ]
        })
        .collect();
    let (hs_sub, _) = group_heights(sources);
    let wa = crate::render::Metric::A.weights(bands);
    let wc = crate::render::Metric::C.weights(bands);
    Section {
        source_z: f64::from(path.first().map_or(0.0, |p| p.ground)) + hs_sub,
        receiver_z: f64::from(path.last().map_or(0.0, |p| p.ground)) + hr,
        distance: d,
        info,
        dba: crate::render::weighted_level(&levels, &wa),
        dbc: crate::render::weighted_level(&levels, &wc),
        bands: nominal.iter().copied().zip(levels).collect(),
        samples,
    }
}
