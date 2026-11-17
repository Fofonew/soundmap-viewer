//! Équation parabolique à fonction de Green (GFPE) en 2D (r, z), pour les
//! basses fréquences. Gilbert & Di (1993) ; Salomons, *Computational
//! Atmospheric Acoustics* (2001), annexe H. Convention e^(−iωt).
//!
//! Pas de r à r + Δr :
//!
//! ψ(r+Δr, z) = e^(iΔr δk(z)) · { (1/2π)∫ [Ψ(kz) + Rp(kz) Ψ(−kz)] P(kz) e^(ikz z) dkz
//!                               + 2iβ Ψ(β) P(β) e^(−iβz) }
//!
//! (Gilbert & Di 1993, forme de l'éq. (3.40) de la thèse de V. Cardoso, IST
//! Lisbonne, vérifiée numériquement contre Weyl-van der Pol), avec
//! Ψ(kz) = ∫₀^∞ ψ e^(−ikz z) dz, P(kz) = e^(iΔr(√(ka²−kz²)−ka)),
//! Rp(kz) = (kz Z − ka)/(kz Z + ka), β = ka/Z et δk(z) = k(z) − ka (partie
//! imaginaire = couche absorbante en haut du domaine).
//!
//! Stabilité : avec des pas Δr ≪ 10 λ sur sol mou, le terme d'onde de surface
//! dérive (problème connu du GFPE original, cf. Salomons 2015) ; on impose
//! donc Δr ≥ 8 λ.
//!
//! Relief : coordonnées qui suivent le terrain, linéaire par morceaux. À
//! chaque changement de pente Δθ, le champ est tourné par le facteur de phase
//! e^(−i ka z sin Δθ) (approximation valable pour des pentes modérées,
//! < ~15–20° ; au-delà, voir la méthode d'ingénierie). Voir ADR 0007.

use num_complex::Complex64 as C64;
use rustfft::{Fft, FftPlanner};
use std::f64::consts::PI;
use std::sync::Arc;

/// Paramètres d'un calcul GFPE (une fréquence, un azimut).
pub struct GfpeInput<'a> {
    pub f: f64,
    /// Célérité de référence (m/s).
    pub c_ref: f64,
    /// Hauteur de la source au-dessus du sol (m).
    pub zs: f64,
    /// Portée maximale (m).
    pub range: f64,
    /// Pas en distance (m). `None` : 10 λ borné à [5, 50] m.
    pub dr: Option<f64>,
    /// Hauteur physique utile du domaine (m), hors couche absorbante.
    pub height: f64,
    /// Altitude du sol à la distance r (m).
    pub ground: &'a (dyn Fn(f64) -> f64 + Sync),
    /// Impédance normalisée du sol à la distance r.
    pub impedance: &'a (dyn Fn(f64) -> C64 + Sync),
    /// Célérité effective à la hauteur z au-dessus du sol (m/s).
    pub c_eff: &'a (dyn Fn(f64) -> f64 + Sync),
    /// Hauteurs des récepteurs au-dessus du sol (m).
    pub receivers: &'a [f64],
    /// Pondération complexe de la source selon l'angle de site (rad) : directivité
    /// du réseau dans l'azimut calculé. `None` : source omnidirectionnelle.
    pub directivity: Option<&'a (dyn Fn(f64) -> C64 + Sync)>,
}

/// Résultat : niveaux relatifs au champ libre ΔL(r) (dB) par hauteur de récepteur.
#[derive(Debug, Clone)]
pub struct GfpeOutput {
    pub r: Vec<f64>,
    /// `dl[i][j]` : récepteur i, distance `r[j]`.
    pub dl: Vec<Vec<f32>>,
    pub dz: f64,
    pub dr: f64,
    pub n: usize,
}

/// Plans FFT réutilisables entre calculs (une taille par fréquence).
pub struct GfpePlans {
    planner: FftPlanner<f64>,
}

impl Default for GfpePlans {
    fn default() -> Self {
        Self {
            planner: FftPlanner::new(),
        }
    }
}

impl GfpePlans {
    fn get(&mut self, m: usize) -> (Arc<dyn Fft<f64>>, Arc<dyn Fft<f64>>) {
        (
            self.planner.plan_fft_forward(m),
            self.planner.plan_fft_inverse(m),
        )
    }
}

fn sqrt_branch(x: C64) -> C64 {
    // Racine de partie imaginaire ≥ 0 (ondes évanescentes décroissantes).
    let s = x.sqrt();
    if s.im < 0.0 { -s } else { s }
}

/// Discrétisation d'un calcul GFPE, commune aux moteurs CPU et GPU.
///
/// Règle du point milieu (thèse IST, éq. 4.10–4.11) : z_j = (j + ½)Δz et
/// kz_n = (s_n + ½)Δk (s_n signé). Ni z = 0 ni kz = 0 ne sont échantillonnés,
/// ce qui supprime les erreurs du point au sol (compté deux fois par le terme
/// image) et de l'échantillon rasant kz = 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GfpeGrid {
    pub ka: f64,
    pub lambda: f64,
    /// Pas en distance (m).
    pub dr: f64,
    /// Épaisseur de la couche absorbante (m).
    pub h_abs: f64,
    /// Hauteur physique utile (m).
    pub height: f64,
    /// Points en z (puissance de 2) ; la FFT porte sur m = 2n points.
    pub n: usize,
    pub m: usize,
    pub dz: f64,
    pub dk: f64,
    /// Distance du champ de départ (m).
    pub r0: f64,
    pub steps: usize,
}

impl GfpeGrid {
    pub fn new(input: &GfpeInput) -> Self {
        let ka = 2.0 * PI * input.f / input.c_ref;
        let lambda = input.c_ref / input.f;
        let dr = input
            .dr
            .unwrap_or(10.0 * lambda)
            .max(8.0 * lambda)
            .min(60.0f64.max(8.0 * lambda));
        // Couche absorbante : épaisseur max(30 λ, 100 m), atténuation quadratique.
        let h_abs = (30.0 * lambda).max(100.0);
        let h_tot = input.height + h_abs;
        let n = ((h_tot / (lambda / 10.0)).ceil().max(64.0) as usize).next_power_of_two();
        let dz = h_tot / n as f64;
        let m = 2 * n;
        let r0 = (3.0 * lambda).max(10.0);
        Self {
            ka,
            lambda,
            dr,
            h_abs,
            height: input.height,
            n,
            m,
            dz,
            dk: 2.0 * PI / (m as f64 * dz),
            r0,
            steps: ((input.range - r0) / dr).ceil().max(1.0) as usize,
        }
    }

    /// Indice fréquentiel signé de l'échantillon i de la FFT.
    pub fn signed(&self, i: usize) -> f64 {
        if i < self.n {
            i as f64
        } else {
            i as f64 - self.m as f64
        }
    }

    pub fn kz(&self, i: usize) -> f64 {
        (self.signed(i) + 0.5) * self.dk
    }

    pub fn zj(&self, j: usize) -> f64 {
        (j as f64 + 0.5) * self.dz
    }

    /// √(ka² − kz²) − ka (branche évanescente décroissante).
    pub fn kz_term(&self, i: usize) -> C64 {
        let k = self.kz(i);
        sqrt_branch(C64::new(self.ka * self.ka - k * k, 0.0)) - self.ka
    }

    /// Facteurs de rotation des transformées décalées d'un demi-pas :
    /// (tw_k[i], tw_z[i]).
    pub fn twiddles(&self, i: usize) -> (C64, C64) {
        let mf = self.m as f64;
        (
            C64::from_polar(1.0, -PI * self.signed(i) / mf),
            C64::from_polar(1.0, -PI * i as f64 / mf),
        )
    }

    pub fn tw_c(&self) -> C64 {
        C64::from_polar(1.0, -PI / (2.0 * self.m as f64))
    }

    /// δk(z_j) = k(z_j) − ka + i·absorption (couche absorbante en haut).
    pub fn delta_k(&self, input: &GfpeInput, j: usize) -> C64 {
        let z = self.zj(j);
        let a_t = 1.0 / self.dr.max(1.0) * 4.0;
        let absorb = if z > self.height {
            a_t * ((z - self.height) / self.h_abs).powi(2)
        } else {
            0.0
        };
        let k = 2.0 * PI * input.f / (input.c_eff)(z);
        C64::new(k - self.ka, absorb)
    }

    /// Distance après `step` pas.
    pub fn r_of(&self, step: usize) -> f64 {
        self.r0 + step as f64 * self.dr
    }
}

/// Paramètres d'un pas en distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepParams {
    /// Rotation au changement de pente : ka·sin Δθ (0 si pas de changement).
    pub rot: f64,
    /// Longueur du pas le long du sol (m).
    pub ds: f64,
    /// β = ka/Z au pas courant.
    pub beta: C64,
    /// P(β) = e^(i ds (√(ka² − β²) − ka)).
    pub p_beta: C64,
}

/// Paramètres de tous les pas (relief, impédance).
pub fn step_params(input: &GfpeInput, g: &GfpeGrid) -> Vec<StepParams> {
    let mut prev_slope = ((input.ground)(g.r0 + g.dr) - (input.ground)(g.r0)).atan2(g.dr);
    (1..=g.steps)
        .map(|step| {
            let ra = g.r_of(step - 1);
            let r1 = g.r_of(step);
            let slope = ((input.ground)(r1) - (input.ground)(ra)).atan2(g.dr);
            let dtheta = slope - prev_slope;
            prev_slope = slope;
            let rot = if dtheta.abs() > 1e-6 {
                g.ka * dtheta.sin()
            } else {
                0.0
            };
            let ds = g.dr / slope.cos();
            let beta = g.ka / (input.impedance)(r1);
            let p_beta =
                (C64::new(0.0, ds) * (sqrt_branch(g.ka * g.ka - beta * beta) - g.ka)).exp();
            StepParams {
                rot,
                ds,
                beta,
                p_beta,
            }
        })
        .collect()
}

/// Champ de départ analytique à r₀ (onde directe + réflexion de
/// Weyl-van der Pol) : exact en atmosphère homogène au-dessus d'un sol plan,
/// pondéré par la directivité du réseau selon le site.
pub fn starter(input: &GfpeInput, g: &GfpeGrid) -> Vec<C64> {
    let (ka, r0) = (g.ka, g.r0);
    let z0 = (input.impedance)(0.0);
    let one = C64::new(1.0, 0.0);
    (0..g.n)
        .map(|j| {
            let z = g.zj(j);
            let r1 = r0.hypot(z - input.zs);
            let r2 = r0.hypot(z + input.zs);
            let (d1, d2) = match input.directivity {
                Some(d) => (d((z - input.zs).atan2(r0)), d((-(z + input.zs)).atan2(r0))),
                None => (one, one),
            };
            let q = crate::ground::spherical_reflection(ka, r2, (z + input.zs) / r2, z0);
            let p = d1 * C64::from_polar(1.0 / r1, ka * r1)
                + d2 * q * C64::from_polar(1.0 / r2, ka * r2);
            p * r0.sqrt() * C64::from_polar(1.0, -ka * r0)
        })
        .collect()
}

/// ΔL = 20 log|ψ| − 10 log r + 20 log R au récepteur de hauteur `h`.
pub fn receiver_level(psi: &[C64], g: &GfpeGrid, h: f64, zs: f64, r: f64) -> f32 {
    let x = (h / g.dz - 0.5).max(0.0);
    let j = (x.floor() as usize).min(g.n - 2);
    let t = x - j as f64;
    let v = psi[j] * (1.0 - t) + psi[j + 1] * t;
    let rr = r.hypot(h - zs);
    (20.0 * v.norm().max(1e-12).log10() - 10.0 * r.log10() + 20.0 * rr.log10()) as f32
}

/// Lance le calcul GFPE (CPU).
pub fn run(input: &GfpeInput, plans: &mut GfpePlans) -> GfpeOutput {
    let g = GfpeGrid::new(input);
    let (n, m) = (g.n, g.m);
    let (fft, ifft) = plans.get(m);
    let kz: Vec<f64> = (0..m).map(|i| g.kz(i)).collect();
    let kzt: Vec<C64> = (0..m).map(|i| g.kz_term(i)).collect();
    let tw: Vec<(C64, C64)> = (0..m).map(|i| g.twiddles(i)).collect();
    let tw_c = g.tw_c();
    let dk: Vec<C64> = (0..n).map(|j| g.delta_k(input, j)).collect();
    let params = step_params(input, &g);
    let mut psi = starter(input, &g);

    let mut out_r = Vec::with_capacity(g.steps);
    let mut dl: Vec<Vec<f32>> = vec![Vec::with_capacity(g.steps); input.receivers.len()];
    let mut buf = vec![C64::new(0.0, 0.0); m];
    let mut buf2 = vec![C64::new(0.0, 0.0); m];
    let norm = 1.0 / m as f64;
    let mut prop_ds = f64::NAN;
    let mut prop: Vec<C64> = vec![C64::new(0.0, 0.0); m];

    for (step, sp) in params.iter().enumerate() {
        let r1 = g.r_of(step + 1);
        // Rotation au changement de pente.
        if sp.rot != 0.0 {
            for (j, v) in psi.iter_mut().enumerate() {
                *v *= C64::from_polar(1.0, -sp.rot * g.zj(j));
            }
        }
        if sp.ds != prop_ds {
            for (p, k) in prop.iter_mut().zip(&kzt) {
                *p = (C64::new(0.0, sp.ds) * k).exp();
            }
            prop_ds = sp.ds;
        }
        let beta = sp.beta;

        // Ψ(kz_n) = Σ ψ_j e^(−i kz_n z_j)   (Δz simplifié avec Δk/2π).
        buf.iter_mut().for_each(|v| *v = C64::new(0.0, 0.0));
        for j in 0..n {
            buf[j] = psi[j] * tw[j].1;
        }
        fft.process(&mut buf);
        // Ψ(β) avec la même quadrature (point milieu) et P(β).
        let mut psi_beta = C64::new(0.0, 0.0);
        for (j, v) in psi.iter().enumerate() {
            psi_beta += v * (C64::new(0.0, -1.0) * beta * g.zj(j)).exp();
        }
        let h_beta = psi_beta * sp.p_beta;
        // Formulation à pôle soustrait : spectre réfléchi régulier
        // Ψ·P + 2β·[Ψ·P(kz) − Ψ·P(β)]/(kz − β) ; l'onde de surface est
        // exactement compensée par le résidu du terme soustrait.
        for i in 0..m {
            let p = buf[i] * tw[i].0 * tw_c * prop[i];
            let refl = p + 2.0 * beta * (p - h_beta) / (kz[i] - beta);
            // Direct : Σ G e^(+i kz z_j) → transformée inverse (facteurs conjugués).
            buf[i] = p * tw[i].0.conj();
            // Réfléchi évalué en −z_j : Σ G e^(−i kz z_j) → transformée directe.
            buf2[i] = refl * tw[i].0;
        }
        ifft.process(&mut buf);
        fft.process(&mut buf2);
        for j in 0..n {
            let direct = buf[j] * tw[j].1.conj() * tw_c.conj();
            let reflected = buf2[j] * tw[j].1 * tw_c;
            psi[j] = (direct + reflected) * norm * (C64::new(0.0, sp.ds) * dk[j]).exp();
        }

        out_r.push(r1);
        for (i, &h) in input.receivers.iter().enumerate() {
            dl[i].push(receiver_level(&psi, &g, h, input.zs, r1));
        }
    }
    GfpeOutput {
        r: out_r,
        dl,
        dz: g.dz,
        dr: g.dr,
        n,
    }
}

/// Interpole ΔL à la distance `r` (linéaire en dB).
pub fn interp(out: &GfpeOutput, rec: usize, r: f64) -> Option<f32> {
    let v = out.dl.get(rec)?;
    if v.is_empty() || r < out.r[0] - out.dr {
        return None;
    }
    let x = (r - out.r[0]) / out.dr;
    if x <= 0.0 {
        return v.first().copied();
    }
    let i = x.floor() as usize;
    if i + 1 >= v.len() {
        return v.last().copied();
    }
    let t = (x - i as f64) as f32;
    Some(v[i] * (1.0 - t) + v[i + 1] * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ground::{ImpedanceModel, TwoRay, impedance};

    fn flat(_: f64) -> f64 {
        0.0
    }

    fn compare(f: f64, z: C64, tol: f64) {
        let c = 343.0;
        let imp = move |_: f64| z;
        let ceff = |_: f64| 343.0;
        let input = GfpeInput {
            f,
            c_ref: c,
            zs: 2.0,
            range: 1000.0,
            dr: None,
            height: 150.0,
            ground: &flat,
            impedance: &imp,
            c_eff: &ceff,
            receivers: &[1.5],
            directivity: None,
        };
        let out = run(&input, &mut GfpePlans::default());
        let mut worst: f64 = 0.0;
        let mut log = String::new();
        for r in [200.0, 300.0, 500.0, 700.0, 1000.0] {
            let pe = f64::from(interp(&out, 0, r).unwrap_or(f32::NAN));
            let an = TwoRay {
                hs: 2.0,
                hr: 1.5,
                d: r,
            }
            .excess_db(f, c, z);
            log += &format!(" {r} m : PE {pe:.2} / analytique {an:.2} ;");
            worst = worst.max((pe - an).abs());
        }
        assert!(
            worst < tol,
            "{f} Hz, Z = {z} : écart max {worst:.2} dB —{log}"
        );
    }

    #[test]
    fn matches_analytic_on_rigid_ground() {
        compare(63.0, C64::new(1e9, 0.0), 1.0);
        compare(125.0, C64::new(1e9, 0.0), 1.0);
    }

    #[test]
    fn matches_weyl_van_der_pol_on_grassland() {
        for f in [50.0, 100.0, 160.0] {
            compare(f, impedance(ImpedanceModel::DelanyBazley, f, 200.0), 1.0);
        }
    }

    fn level_at(grad: f64, r: f64) -> f64 {
        let f = 100.0;
        let z = impedance(ImpedanceModel::DelanyBazley, f, 200.0);
        let imp = move |_: f64| z;
        let ceff = move |h: f64| 343.0 + grad * h;
        let input = GfpeInput {
            f,
            c_ref: 343.0,
            zs: 2.0,
            range: r,
            dr: None,
            height: 300.0,
            ground: &flat,
            impedance: &imp,
            c_eff: &ceff,
            receivers: &[1.5],
            directivity: None,
        };
        let out = run(&input, &mut GfpePlans::default());
        // Moyenne sur les derniers 200 m (lisse les interférences).
        let v: Vec<f64> = out
            .r
            .iter()
            .zip(&out.dl[0])
            .filter(|(x, _)| **x > r - 200.0)
            .map(|(_, l)| f64::from(*l))
            .collect();
        10.0 * (v.iter().map(|l| 10f64.powf(l / 10.0)).sum::<f64>() / v.len() as f64).log10()
    }

    #[test]
    fn refraction_benchmarks_salomons_qualitative() {
        // Salomons (2001) ch. 2 : réfraction vers le bas → niveaux élevés à
        // grande distance ; vers le haut → zone d'ombre.
        let hom = level_at(0.0, 2000.0);
        let down = level_at(0.1, 2000.0);
        let up = level_at(-0.1, 2000.0);
        assert!(
            down > hom + 5.0,
            "vers le bas {down:.1} vs homogène {hom:.1}"
        );
        assert!(up < hom - 10.0, "vers le haut {up:.1} vs homogène {hom:.1}");
    }

    #[test]
    fn interp_is_linear_between_steps() {
        let out = GfpeOutput {
            r: vec![10.0, 20.0, 30.0],
            dl: vec![vec![0.0, 10.0, 20.0]],
            dz: 0.1,
            dr: 10.0,
            n: 64,
        };
        assert_eq!(interp(&out, 0, 15.0), Some(5.0));
        assert_eq!(interp(&out, 0, 40.0), Some(20.0));
    }
}
