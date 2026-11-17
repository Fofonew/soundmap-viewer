//! Grille polaire et méthode d'ingénierie.
//!
//! 1. Profils extraits **une fois par azimut** depuis l'origine (la sono) ;
//! 2. atténuations en excès par récepteur et par bande, mises en cache pour
//!    un jeu (hauteurs des sources, condition, bandes) ;
//! 3. combinaison avec les sources (sommes cohérente/incohérente, directivité,
//!    divergence, absorption) : seule étape refaite quand une enceinte bouge.

use crate::project::group_heights;
use crate::scene::Scene;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sm_core::atmosphere::iso9613_1_alpha;
use sm_core::engineering::{BandSpec, PathConfig, PathSample, Scratch, path_excess};
use sm_core::source::{Program, Source, band_energy};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PolarSpec {
    pub n_az: usize,
    /// Pas radial des récepteurs (m).
    pub dr: f64,
    pub radius: f64,
    /// Pas des profils (m).
    pub step: f64,
}

impl PolarSpec {
    pub fn preview(radius: f64) -> Self {
        Self {
            n_az: 360,
            dr: (radius / 400.0).max(10.0),
            radius,
            step: (radius / 1000.0).max(4.0),
        }
    }
    pub fn n_r(&self) -> usize {
        (self.radius / self.dr).floor() as usize
    }
    pub fn r_of(&self, ir: usize) -> f64 {
        (ir + 1) as f64 * self.dr
    }
    pub fn az_of(&self, ia: usize) -> f64 {
        ia as f64 * 360.0 / self.n_az as f64
    }
}

/// Profils de la grille (un par azimut).
pub struct Profiles {
    pub spec: PolarSpec,
    pub rays: Vec<Vec<PathSample>>,
}

pub fn build_profiles(scene: &Scene, spec: PolarSpec) -> Profiles {
    let rays = (0..spec.n_az)
        .into_par_iter()
        .map(|ia| {
            crate::profile::extract_ray(
                scene,
                scene.origin,
                spec.az_of(ia),
                spec.step,
                spec.radius + spec.step,
            )
        })
        .collect();
    Profiles { spec, rays }
}

/// Clé du cache d'atténuations en excès.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExcessKey {
    pub hs_sub: f64,
    pub hs_top: f64,
    pub hr: f64,
    pub cfg: PathConfig,
    pub n_bands: usize,
    pub band0: f64,
}

/// Atténuations en excès [az][r][groupe][bande] (dB).
pub struct Excess {
    pub key: ExcessKey,
    pub data: Vec<f32>,
    pub n_bands: usize,
}

impl Excess {
    #[inline]
    pub fn get(&self, n_r: usize, ia: usize, ir: usize, group: usize, b: usize) -> f32 {
        self.data[((ia * n_r + ir) * 2 + group) * self.n_bands + b]
    }
}

pub fn compute_excess(p: &Profiles, key: ExcessKey, bands: &[BandSpec]) -> Excess {
    let spec = p.spec;
    let n_r = spec.n_r();
    let nb = bands.len();
    let data: Vec<f32> = p
        .rays
        .par_iter()
        .enumerate()
        .flat_map_iter(|(ia, ray)| {
            let mut sc = Scratch::default();
            let cfg = key.cfg.for_azimuth(spec.az_of(ia));
            let mut out = vec![0.0f64; nb];
            let mut row = Vec::with_capacity(n_r * 2 * nb);
            for ir in 0..n_r {
                let k = ((spec.r_of(ir) / spec.step).round() as usize).clamp(1, ray.len() - 1);
                for hs in [key.hs_sub, key.hs_top] {
                    path_excess(&ray[..=k], hs, key.hr, bands, &cfg, &mut sc, &mut out);
                    row.extend(out.iter().map(|v| *v as f32));
                }
            }
            row
        })
        .collect();
    Excess {
        key,
        data,
        n_bands: nb,
    }
}

/// Atmosphère pour l'absorption.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Air {
    pub t_c: f64,
    pub rh: f64,
}

/// Niveaux par bande sur la grille polaire.
#[derive(Debug, Clone)]
pub struct LevelGrid {
    pub spec: PolarSpec,
    pub bands: Vec<BandSpec>,
    /// [az][r][bande] dB SPL.
    pub levels: Vec<f32>,
    pub hr: f64,
}

impl LevelGrid {
    #[inline]
    pub fn get(&self, ia: usize, ir: usize, b: usize) -> f32 {
        self.levels[(ia * self.spec.n_r() + ir) * self.bands.len() + b]
    }

    /// Niveau global pondéré (poids dB par bande) en une cellule.
    pub fn weighted(&self, ia: usize, ir: usize, weights: &[f64]) -> f64 {
        let mut e = 0.0;
        for (b, w) in weights.iter().enumerate() {
            e += 10f64.powf((f64::from(self.get(ia, ir, b)) + w) / 10.0);
        }
        10.0 * e.max(1e-12).log10()
    }
}

/// Combine les excès avec les sources (étape rapide, refaite à chaque
/// déplacement d'enceinte). `pe_excess` remplace (fusionne) les excès des
/// basses fréquences si fourni : [az][r][bande] avec poids de fusion par bande.
#[allow(clippy::too_many_arguments)]
pub fn combine(
    scene: &Scene,
    p: &Profiles,
    ex: &Excess,
    sources: &[Source],
    program: &Program,
    bands: &[BandSpec],
    air: Air,
    fused: Option<(&[f32], &[f64])>,
) -> LevelGrid {
    let spec = p.spec;
    let n_r = spec.n_r();
    let nb = bands.len();
    let c = sm_core::atmosphere::sound_speed(air.t_c);
    let alpha: Vec<f64> = bands
        .iter()
        .map(|b| iso9613_1_alpha(b.fc, air.t_c, air.rh, 101.325))
        .collect();
    let n_sub = 3;
    let hr = ex.key.hr;
    let levels: Vec<f32> = (0..spec.n_az)
        .into_par_iter()
        .flat_map_iter(|ia| {
            let (s, co) = spec.az_of(ia).to_radians().sin_cos();
            let ray = &p.rays[ia];
            let mut row = Vec::with_capacity(n_r * nb);
            for ir in 0..n_r {
                let r = spec.r_of(ir);
                let k = ((r / spec.step).round() as usize).min(ray.len() - 1);
                let zr = f64::from(ray[k].ground) + hr;
                let rx = [r * s, r * co, zr];
                for (b, band) in bands.iter().enumerate() {
                    let (e_sub, e_top) = band_energy(
                        sources,
                        rx,
                        band.fc,
                        band.lo,
                        band.hi,
                        band.width_thirds,
                        program,
                        c,
                        n_sub,
                    );
                    let mut ex_sub = f64::from(ex.get(n_r, ia, ir, 0, b));
                    let mut ex_top = f64::from(ex.get(n_r, ia, ir, 1, b));
                    if let Some((pe, w)) = fused
                        && w[b] > 0.0
                    {
                        let v = f64::from(pe[(ia * n_r + ir) * nb + b]);
                        let blend = |e: f64| {
                            10.0 * (w[b] * 10f64.powf(v / 10.0)
                                + (1.0 - w[b]) * 10f64.powf(e / 10.0))
                            .log10()
                        };
                        ex_sub = blend(ex_sub);
                        ex_top = blend(ex_top);
                    }
                    let e = e_sub * 10f64.powf(ex_sub / 10.0) + e_top * 10f64.powf(ex_top / 10.0);
                    let l = 10.0 * e.max(1e-12).log10() - alpha[b] * r;
                    row.push(l as f32);
                }
            }
            row
        })
        .collect();
    let _ = scene;
    LevelGrid {
        spec,
        bands: bands.to_vec(),
        levels,
        hr,
    }
}

/// Moteur de prévisualisation avec caches (profils et excès).
#[derive(Default)]
pub struct Engine {
    profiles: Option<Arc<Profiles>>,
    excess: Option<Arc<Excess>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Timing {
    pub profiles_ms: f64,
    pub excess_ms: f64,
    pub combine_ms: f64,
    pub cached_profiles: bool,
    pub cached_excess: bool,
}

impl Engine {
    pub fn profiles(&mut self, scene: &Scene, spec: PolarSpec) -> (Arc<Profiles>, bool) {
        if let Some(p) = &self.profiles
            && p.spec == spec
        {
            return (p.clone(), true);
        }
        let p = Arc::new(build_profiles(scene, spec));
        self.profiles = Some(p.clone());
        self.excess = None;
        (p, false)
    }

    /// Calcul complet avec réutilisation des caches.
    #[allow(clippy::too_many_arguments)]
    pub fn compute(
        &mut self,
        scene: &Scene,
        spec: PolarSpec,
        sources: &[Source],
        program: &Program,
        bands: &[BandSpec],
        cfg: PathConfig,
        hr: f64,
        air: Air,
    ) -> (LevelGrid, Timing) {
        let t0 = std::time::Instant::now();
        let (p, cached_profiles) = self.profiles(scene, spec);
        let t1 = std::time::Instant::now();
        let (hs_sub, hs_top) = group_heights(sources);
        let key = ExcessKey {
            hs_sub: (hs_sub * 10.0).round() / 10.0,
            hs_top: (hs_top * 10.0).round() / 10.0,
            hr,
            cfg,
            n_bands: bands.len(),
            band0: bands.first().map_or(0.0, |b| b.fc),
        };
        let cached_excess = self.excess.as_ref().is_some_and(|e| e.key == key);
        if !cached_excess {
            self.excess = Some(Arc::new(compute_excess(&p, key, bands)));
        }
        let ex = self
            .excess
            .clone()
            .unwrap_or_else(|| Arc::new(compute_excess(&p, key, bands)));
        let t2 = std::time::Instant::now();
        let grid = combine(scene, &p, &ex, sources, program, bands, air, None);
        let t3 = std::time::Instant::now();
        let ms = |a: std::time::Instant, b: std::time::Instant| (b - a).as_secs_f64() * 1e3;
        (
            grid,
            Timing {
                profiles_ms: ms(t0, t1),
                excess_ms: ms(t1, t2),
                combine_ms: ms(t2, t3),
                cached_profiles,
                cached_excess,
            },
        )
    }

    pub fn cached_profiles(&self) -> Option<Arc<Profiles>> {
        self.profiles.clone()
    }
    pub fn cached_excess(&self) -> Option<Arc<Excess>> {
        self.excess.clone()
    }
}
