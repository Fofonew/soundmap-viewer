//! Basses fréquences par GFPE en N×2D : un calcul par azimut et par fréquence,
//! célérité effective selon l'azimut (vent), impédance par segment (classe de
//! sol), relief, starter pondéré par la directivité complexe du réseau de subs.
//! Fusion PE (< 200 Hz) / ingénierie (> 200 Hz) avec transition progressive.

use crate::polar::{LevelGrid, Profiles};
use crate::project::group_heights;
use crate::{CancelFlag, ProgressCounter};
use rayon::prelude::*;
use sm_core::C64;
use sm_core::atmosphere::MoProfile;
use sm_core::engineering::BandSpec;
use sm_core::gfpe::{GfpeInput, GfpeOutput, GfpePlans, interp, run};
use sm_core::ground::{GroundClass, ImpedanceModel, impedance};
use sm_core::source::{Source, SpeakerKind};
use std::f64::consts::PI;

/// Poids de la solution PE dans la fusion, selon la fréquence centrale.
pub fn fusion_weight(fc: f64) -> f64 {
    if fc <= 130.0 {
        1.0
    } else if fc <= 170.0 {
        0.67
    } else if fc <= 215.0 {
        0.33
    } else {
        0.0
    }
}

/// Champ d'atténuation en excès GFPE sur la grille polaire [az][r][bande]
/// (bandes de la grille complète ; 0 et poids nul hors basses fréquences).
#[derive(Debug, Clone)]
pub struct PeField {
    pub n_az: usize,
    pub n_r: usize,
    pub dr: f64,
    pub nb: usize,
    pub data: Vec<f32>,
    pub weights: Vec<f64>,
    /// Moteur utilisé : « CPU » ou « GPU (adaptateur) ».
    pub backend: String,
}

impl PeField {
    /// (excès dB, poids de fusion) en un point local.
    pub fn sample(&self, b: usize, x: f64, y: f64) -> Option<(f64, f64)> {
        let w = *self.weights.get(b)?;
        if w <= 0.0 {
            return None;
        }
        let r = x.hypot(y);
        let fr = (r / self.dr - 1.0).max(0.0);
        if fr > (self.n_r - 1) as f64 {
            return None;
        }
        let az = x.atan2(y).to_degrees().rem_euclid(360.0);
        let fa = az / (360.0 / self.n_az as f64);
        let a0 = fa.floor() as usize % self.n_az;
        let a1 = (a0 + 1) % self.n_az;
        let ta = fa - fa.floor();
        let r0 = fr.floor() as usize;
        let r1 = (r0 + 1).min(self.n_r - 1);
        let tr = fr - fr.floor();
        let v = |a: usize, rr: usize| f64::from(self.data[(a * self.n_r + rr) * self.nb + b]);
        let top = v(a0, r0) * (1.0 - tr) + v(a0, r1) * tr;
        let bot = v(a1, r0) * (1.0 - tr) + v(a1, r1) * tr;
        Some((top * (1.0 - ta) + bot * ta, w))
    }
}

/// Facteur de réseau complexe des subs dans la direction (azimut, site).
fn array_factor(subs: &[&Source], f: f64, c: f64, az: f64, elev: f64, z_ref: f64) -> C64 {
    let k = 2.0 * PI * f / c;
    let (sa, ca) = az.sin_cos();
    let (se, ce) = elev.sin_cos();
    let u = [sa * ce, ca * ce, se];
    let mut p = C64::new(0.0, 0.0);
    for s in subs {
        let a = 10f64.powf((s.gain_db + s.model.response_db(f)) / 20.0) * s.polarity;
        let proj = u[0] * s.x + u[1] * s.y + u[2] * (s.z - z_ref);
        p += C64::from_polar(a, -k * proj + 2.0 * PI * f * s.delay_s);
    }
    p
}

type RFn<'a> = Box<dyn Fn(f64) -> f64 + Send + Sync + 'a>;
type CFn<'a> = Box<dyn Fn(f64) -> C64 + Send + Sync + 'a>;

/// Fonctions d'entrée du GFPE pour un azimut et une fréquence.
struct AzInput<'a> {
    ground: RFn<'a>,
    imp: CFn<'a>,
    ceff: RFn<'a>,
    dir: Option<CFn<'a>>,
}

/// Contexte commun aux calculs d'un scénario.
struct PeCtx<'a> {
    profiles: &'a Profiles,
    atm: &'a MoProfile,
    subs: Vec<&'a Source>,
    z_ref: f64,
    c_ref: f64,
    hs_sub: f64,
    model: ImpedanceModel,
    sigma_scale: f64,
    snow: bool,
}

impl<'c> PeCtx<'c> {
    fn az_input(&self, ia: usize, f: f64) -> AzInput<'c> {
        let spec = self.profiles.spec;
        let ray: &'c [_] = &self.profiles.rays[ia];
        let step = spec.step;
        let az = spec.az_of(ia);
        let ray_g = ray;
        let ground = move |r: f64| {
            let x = (r / step).clamp(0.0, (ray_g.len() - 1) as f64);
            let i = (x.floor() as usize).min(ray_g.len() - 2);
            let t = x - i as f64;
            f64::from(ray_g[i].ground) * (1.0 - t) + f64::from(ray_g[i + 1].ground) * t
        };
        let (model, scale, snow) = (self.model, self.sigma_scale, self.snow);
        let imp = move |r: f64| {
            let i = ((r / step).round() as usize).min(ray.len() - 1);
            let sigma = if snow {
                GroundClass::A.sigma_kpa()
            } else {
                GroundClass::from_index(ray[i].class).sigma_kpa() * scale
            };
            impedance(model, f, sigma)
        };
        let atm: &'c MoProfile = self.atm;
        let ceff = move |z: f64| atm.c_eff(z.max(0.1), az);
        // Starter pondéré par la directivité complexe du réseau selon le site,
        // normalisée par la valeur à l'horizontale (réintroduite exactement par
        // la somme cohérente au récepteur). Dans les creux de directivité
        // (|AF(0)| < 25 % de Σ|a|), le rapport est mal conditionné : starter omni.
        let (c_ref, z_ref) = (self.c_ref, self.z_ref);
        let af0 = array_factor(&self.subs, f, c_ref, az.to_radians(), 0.0, z_ref);
        let sum_a: f64 = self
            .subs
            .iter()
            .map(|s| 10f64.powf((s.gain_db + s.model.response_db(f)) / 20.0))
            .sum();
        let dir: Option<CFn<'c>> = if !self.subs.is_empty() && af0.norm() >= 0.25 * sum_a {
            let subs: Vec<&'c Source> = self.subs.clone();
            Some(Box::new(move |elev: f64| {
                let d = array_factor(&subs, f, c_ref, az.to_radians(), elev, z_ref) / af0;
                if d.norm() > 4.0 {
                    d * (4.0 / d.norm())
                } else {
                    d
                }
            }))
        } else {
            None
        };
        AzInput {
            ground: Box::new(ground),
            imp: Box::new(imp),
            ceff: Box::new(ceff),
            dir,
        }
    }

    fn input<'a>(&self, a: &'a AzInput<'c>, f: f64, hr: &'a [f64]) -> GfpeInput<'a> {
        let spec = self.profiles.spec;
        GfpeInput {
            f,
            c_ref: self.c_ref,
            zs: self.hs_sub,
            range: spec.radius + spec.dr,
            dr: None,
            height: 250.0,
            ground: &*a.ground,
            impedance: &*a.imp,
            c_eff: &*a.ceff,
            receivers: hr,
            directivity: a.dir.as_deref().map(|d| d as &(dyn Fn(f64) -> C64 + Sync)),
        }
    }

    /// ΔL relatif au champ libre d'une source unité, échantillonné sur la grille polaire.
    fn row(&self, out: &GfpeOutput) -> Vec<f32> {
        let spec = self.profiles.spec;
        (0..spec.n_r())
            .map(|ir| interp(out, 0, spec.r_of(ir)).unwrap_or(0.0))
            .collect()
    }
}

/// Moteur GFPE GPU partagé (initialisé au premier appel). `SOUNDMAP_GPU=0`
/// force le CPU. Sans la fonctionnalité `gpu`, toujours `None`.
#[cfg(feature = "gpu")]
pub fn gpu() -> Option<&'static sm_gpu::GpuGfpe> {
    static GPU: std::sync::OnceLock<Option<sm_gpu::GpuGfpe>> = std::sync::OnceLock::new();
    GPU.get_or_init(|| {
        if std::env::var("SOUNDMAP_GPU").is_ok_and(|v| v == "0") {
            return None;
        }
        let g = sm_gpu::GpuGfpe::new();
        match &g {
            Some(g) => tracing::info!("GFPE sur GPU : {}", g.name()),
            None => tracing::info!("aucun GPU compatible : GFPE sur CPU"),
        }
        g
    })
    .as_ref()
}

/// Nom du moteur GFPE qui sera utilisé.
pub fn backend_name() -> String {
    #[cfg(feature = "gpu")]
    if let Some(g) = gpu() {
        return format!("GPU ({})", g.name());
    }
    "CPU".into()
}

/// Lance les calculs GFPE pour un scénario météo (GPU si disponible, sinon CPU).
#[allow(clippy::too_many_arguments)]
pub fn run_pe(
    profiles: &Profiles,
    sources: &[Source],
    bands: &[BandSpec],
    atm: &MoProfile,
    hr: f64,
    cfg: &sm_core::engineering::PathConfig,
    freqs_per_band: usize,
    cancel: &CancelFlag,
    progress: &ProgressCounter,
) -> Option<PeField> {
    let spec = profiles.spec;
    let n_r = spec.n_r();
    let nb = bands.len();
    let weights: Vec<f64> = bands.iter().map(|b| fusion_weight(b.fc)).collect();
    let (hs_sub, _) = group_heights(sources);
    let subs: Vec<&Source> = sources
        .iter()
        .filter(|s| s.model.kind == SpeakerKind::Sub)
        .collect();
    let z_ref = subs.iter().map(|s| s.z).sum::<f64>() / subs.len().max(1) as f64;
    let ctx = PeCtx {
        profiles,
        atm,
        subs,
        z_ref,
        c_ref: sm_core::atmosphere::sound_speed(atm.ground_temperature()),
        hs_sub,
        model: cfg.model,
        sigma_scale: cfg.sigma_scale,
        snow: cfg.snow,
    };
    let n_f = freqs_per_band.max(1);
    // (bande, fréquence) à calculer.
    let freqs: Vec<(usize, f64)> = bands
        .iter()
        .enumerate()
        .filter(|(b, _)| weights[*b] > 0.0)
        .flat_map(|(b, band)| {
            (0..n_f).map(move |i| {
                let t = (i as f64 + 0.5) / n_f as f64;
                (b, band.lo * (band.hi / band.lo).powf(t))
            })
        })
        .collect();
    let hr_arr = [hr];

    let mut backend = "CPU".to_string();
    let mut results: Option<Vec<(usize, usize, Vec<f32>)>> = None;
    #[cfg(feature = "gpu")]
    if let Some(g) = gpu() {
        match run_gpu(g, &ctx, &freqs, &hr_arr, cancel, progress) {
            Ok(r) => {
                results = Some(r);
                backend = format!("GPU ({})", g.name());
            }
            Err(sm_gpu::GpuError::Cancelled) => return None,
            Err(e) => tracing::warn!("GFPE GPU en échec ({e}) : repli sur le CPU"),
        }
    }
    let results = match results {
        Some(r) => r,
        None => {
            let tasks: Vec<(usize, usize, f64)> = (0..spec.n_az)
                .flat_map(|ia| freqs.iter().map(move |&(b, f)| (ia, b, f)))
                .collect();
            let r: Vec<Option<(usize, usize, Vec<f32>)>> = tasks
                .par_iter()
                .map_init(GfpePlans::default, |plans, &(ia, b, f)| {
                    if cancel.is_cancelled() {
                        return None;
                    }
                    let a = ctx.az_input(ia, f);
                    let out = run(&ctx.input(&a, f, &hr_arr), plans);
                    progress.inc(1);
                    Some((ia, b, ctx.row(&out)))
                })
                .collect();
            r.into_iter().flatten().collect()
        }
    };
    if cancel.is_cancelled() {
        return None;
    }
    // Moyenne énergétique des fréquences d'une même bande.
    let mut acc = vec![0.0f64; spec.n_az * n_r * nb];
    let mut cnt = vec![0u32; spec.n_az * nb];
    for (ia, b, row) in results {
        cnt[ia * nb + b] += 1;
        for (ir, v) in row.iter().enumerate() {
            acc[(ia * n_r + ir) * nb + b] += 10f64.powf(f64::from(*v) / 10.0);
        }
    }
    let mut data = vec![0.0f32; acc.len()];
    for ia in 0..spec.n_az {
        for b in 0..nb {
            let n = f64::from(cnt[ia * nb + b].max(1));
            for ir in 0..n_r {
                let i = (ia * n_r + ir) * nb + b;
                data[i] = if weights[b] > 0.0 {
                    (10.0 * (acc[i] / n).max(1e-12).log10()) as f32
                } else {
                    0.0
                };
            }
        }
    }
    Some(PeField {
        n_az: spec.n_az,
        n_r,
        dr: spec.dr,
        nb,
        data,
        weights,
        backend,
    })
}

/// Calcul GPU : un lot par fréquence (tous les azimuts), découpé selon la mémoire.
#[cfg(feature = "gpu")]
fn run_gpu(
    gpu: &sm_gpu::GpuGfpe,
    ctx: &PeCtx,
    freqs: &[(usize, f64)],
    hr: &[f64],
    cancel: &CancelFlag,
    progress: &ProgressCounter,
) -> Result<Vec<(usize, usize, Vec<f32>)>, sm_gpu::GpuError> {
    let n_az = ctx.profiles.spec.n_az;
    let mut out = Vec::with_capacity(n_az * freqs.len());
    for &(b, f) in freqs {
        // Préparation CPU (closures, starters) en parallèle.
        let az: Vec<AzInput> = (0..n_az)
            .into_par_iter()
            .map(|ia| ctx.az_input(ia, f))
            .collect();
        let inputs: Vec<GfpeInput> = az.iter().map(|a| ctx.input(a, f, hr)).collect();
        let grid = sm_core::gfpe::GfpeGrid::new(&inputs[0]);
        let chunk = gpu.max_batch(&grid).min(n_az).max(1);
        let mut ia0 = 0;
        for part in inputs.chunks(chunk) {
            let res = gpu.run_batch(part, &|| cancel.is_cancelled())?;
            for (k, o) in res.iter().enumerate() {
                out.push((ia0 + k, b, ctx.row(o)));
            }
            ia0 += part.len();
            progress.inc(part.len() as u64);
        }
    }
    Ok(out)
}

/// Nombre de calculs GFPE élémentaires (pour la barre de progression).
pub fn task_count(n_az: usize, bands: &[BandSpec], freqs_per_band: usize) -> u64 {
    (n_az * bands.iter().filter(|b| fusion_weight(b.fc) > 0.0).count() * freqs_per_band.max(1))
        as u64
}

/// Enveloppe sur plusieurs scénarios : médiane et P90 cellule par cellule.
pub fn envelope(grids: &[LevelGrid]) -> Option<(LevelGrid, LevelGrid)> {
    let first = grids.first()?;
    let n = first.levels.len();
    let mut med = first.clone();
    let mut p90 = first.clone();
    let mut vals = vec![0.0f32; grids.len()];
    for i in 0..n {
        for (k, g) in grids.iter().enumerate() {
            vals[k] = g.levels[i];
        }
        vals.sort_by(|a, b| a.total_cmp(b));
        let q = |p: f32| {
            let x = p * (vals.len() - 1) as f32;
            let lo = x.floor() as usize;
            let hi = (lo + 1).min(vals.len() - 1);
            vals[lo] + (vals[hi] - vals[lo]) * (x - lo as f32)
        };
        med.levels[i] = q(0.5);
        p90.levels[i] = q(0.9);
    }
    Some((med, p90))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fusion_weights_transition() {
        assert_eq!(fusion_weight(63.0), 1.0);
        assert!(fusion_weight(160.0) > fusion_weight(200.0));
        assert_eq!(fusion_weight(250.0), 0.0);
    }
}
