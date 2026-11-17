//! Effet de sol : impédance (Delany-Bazley, Miki), coefficient de réflexion
//! d'onde sphérique (Weyl-van der Pol) et modèle à deux rayons.
//!
//! Convention temporelle e^(−iωt) (Salomons 2001), onde sortante e^(ikr).
//! Références : Delany & Bazley (1970), Miki (1990), Attenborough et al. (1980),
//! Nordtest NT ACOU 104 (classes de résistivité σ).

use crate::faddeeva::faddeeva;
use num_complex::Complex64 as C64;
use serde::{Deserialize, Serialize};

/// Classes de sol Nordtest / Nord2000 (résistivité au passage de l'air σ).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GroundClass {
    /// Neige fraîche, mousse très meuble.
    A,
    /// Sol forestier, litière.
    B,
    /// Herbe peu dense, sol forestier compact.
    C,
    /// Prairie, pelouse, champ (sol « mou » de référence).
    D,
    /// Terre compacte, chemins de terre, cultures tassées.
    E,
    /// Gravier, sable compact, terre très tassée.
    F,
    /// Roche, asphalte poreux, béton ancien.
    G,
    /// Eau, béton, asphalte, roche lisse (très dur).
    H,
}

impl GroundClass {
    pub const ALL: [GroundClass; 8] = [
        GroundClass::A,
        GroundClass::B,
        GroundClass::C,
        GroundClass::D,
        GroundClass::E,
        GroundClass::F,
        GroundClass::G,
        GroundClass::H,
    ];

    /// Résistivité σ en kPa·s·m⁻² (valeurs représentatives Nordtest).
    pub fn sigma_kpa(self) -> f64 {
        match self {
            GroundClass::A => 12.5,
            GroundClass::B => 31.5,
            GroundClass::C => 80.0,
            GroundClass::D => 200.0,
            GroundClass::E => 500.0,
            GroundClass::F => 2_000.0,
            GroundClass::G => 20_000.0,
            GroundClass::H => 200_000.0,
        }
    }

    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(i: u8) -> GroundClass {
        Self::ALL
            .get(usize::from(i))
            .copied()
            .unwrap_or(GroundClass::D)
    }
}

/// Modèle d'impédance de sol (milieu poreux semi-infini, à réaction locale).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ImpedanceModel {
    #[default]
    DelanyBazley,
    Miki,
}

/// Impédance normalisée Z = Z_sol / (ρ₀c₀) à la fréquence `f` (Hz) pour une
/// résistivité `sigma_kpa` (kPa·s·m⁻²).
///
/// - Delany-Bazley : Z = 1 + 9,08 (f/σ)^−0,75 + i 11,9 (f/σ)^−0,73
/// - Miki          : Z = 1 + 5,50 (f/σ)^−0,632 + i 8,43 (f/σ)^−0,632
pub fn impedance(model: ImpedanceModel, f: f64, sigma_kpa: f64) -> C64 {
    let x = (f / sigma_kpa).max(1e-9);
    match model {
        ImpedanceModel::DelanyBazley => {
            let l = x.ln();
            C64::new(1.0 + 9.08 * (-0.75 * l).exp(), 11.9 * (-0.73 * l).exp())
        }
        ImpedanceModel::Miki => {
            let p = x.powf(-0.632);
            C64::new(1.0 + 5.50 * p, 8.43 * p)
        }
    }
}

/// Coefficient de réflexion d'onde plane pour un angle rasant ψ.
pub fn plane_reflection(z: C64, sin_psi: f64) -> C64 {
    (z * sin_psi - 1.0) / (z * sin_psi + 1.0)
}

/// Coefficient de réflexion d'onde sphérique Q = Rp + (1 − Rp)·F(w)
/// (formule de Weyl-van der Pol), avec
/// w = ½(1 + i)·√(k R₂)·(sin ψ + 1/Z) et F(w) = 1 + i√π·w·w_F(w).
pub fn spherical_reflection(k: f64, r2: f64, sin_psi: f64, z: C64) -> C64 {
    let rp = plane_reflection(z, sin_psi);
    let beta = 1.0 / z;
    let w = C64::new(0.5, 0.5) * (k * r2).sqrt() * (sin_psi + beta);
    let f = 1.0 + C64::new(0.0, std::f64::consts::PI.sqrt()) * w * faddeeva(w);
    rp + (1.0 - rp) * f
}

/// Géométrie à deux rayons au-dessus d'un plan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwoRay {
    /// Hauteur de la source au-dessus du plan (m).
    pub hs: f64,
    /// Hauteur du récepteur au-dessus du plan (m).
    pub hr: f64,
    /// Distance horizontale (m).
    pub d: f64,
}

impl TwoRay {
    pub fn r1(&self) -> f64 {
        self.d.hypot(self.hs - self.hr)
    }
    pub fn r2(&self) -> f64 {
        self.d.hypot(self.hs + self.hr)
    }

    /// p_total / p_direct = 1 + Q·(R₁/R₂)·e^(ik(R₂−R₁)).
    pub fn ratio(&self, f: f64, c: f64, z: C64) -> C64 {
        let k = 2.0 * std::f64::consts::PI * f / c;
        let (r1, r2) = (self.r1().max(1e-3), self.r2().max(1e-3));
        let sin_psi = (self.hs + self.hr) / r2;
        let q = spherical_reflection(k, r2, sin_psi, z);
        1.0 + q * (r1 / r2) * C64::from_polar(1.0, k * (r2 - r1))
    }

    /// Atténuation en excès (dB, positive = gain) à une fréquence.
    pub fn excess_db(&self, f: f64, c: f64, z: C64) -> f64 {
        20.0 * self.ratio(f, c, z).norm().max(1e-6).log10()
    }

    /// Atténuation en excès moyennée en énergie sur une bande [lo, hi]
    /// découpée en `n` sous-bandes géométriques : lisse les interférences
    /// comme le ferait la largeur de bande réelle du signal.
    ///
    /// Dans chaque sous-bande [fa, fb], Q est pris au centre fm et le terme
    /// croisé est moyenné analytiquement sur la phase φ = 2πf(R₂ − R₁)/c :
    /// E = 1 + |Qρ|² + 2·Re(Qρ·e^(iφm))·sinc(Δφ/2), ρ = R₁/R₂.
    /// Coût fixe (n évaluations) et pas de repliement des creux quand
    /// plusieurs lobes d'interférence tombent dans la bande (→ somme en
    /// énergie).
    pub fn excess_band_db(
        &self,
        lo: f64,
        hi: f64,
        n: usize,
        c: f64,
        model: ImpedanceModel,
        sigma_kpa: f64,
    ) -> f64 {
        let n = n.max(1);
        let (r1, r2) = (self.r1().max(1e-3), self.r2().max(1e-3));
        let (rho, sin_psi, dr) = (r1 / r2, (self.hs + self.hr) / r2, r2 - r1);
        let tau = 2.0 * std::f64::consts::PI * dr / c;
        let step = (hi / lo).powf(1.0 / n as f64);
        let half_step = step.sqrt();
        let (mut e, mut fa) = (0.0, lo);
        for _ in 0..n {
            let fb = fa * step;
            let fm = fa * half_step;
            let k = 2.0 * std::f64::consts::PI * fm / c;
            let q = spherical_reflection(k, r2, sin_psi, impedance(model, fm, sigma_kpa)) * rho;
            let half = 0.5 * tau * (fb - fa);
            let sinc = if half.abs() < 1e-9 {
                1.0
            } else {
                half.sin() / half
            };
            let phase = C64::from_polar(1.0, 0.5 * tau * (fa + fb));
            e += 1.0 + q.norm_sqr() + 2.0 * (q * phase).re * sinc;
            fa = fb;
        }
        10.0 * (e / n as f64).max(1e-12).log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: f64 = 343.0;

    #[test]
    fn delany_bazley_reference_value() {
        // f/σ = 1 → Z = 10,08 + 11,9 i.
        let z = impedance(ImpedanceModel::DelanyBazley, 200.0, 200.0);
        assert!((z.re - 10.08).abs() < 1e-9 && (z.im - 11.9).abs() < 1e-9);
        // Plus le sol est résistif, plus |Z| est grand (sol « dur »).
        let soft = impedance(ImpedanceModel::DelanyBazley, 100.0, 200.0).norm();
        let hard = impedance(ImpedanceModel::DelanyBazley, 100.0, 20_000.0).norm();
        assert!(hard > 10.0 * soft);
    }

    #[test]
    fn rigid_ground_gives_q_equal_one() {
        let z = C64::new(1e12, 0.0);
        let q = spherical_reflection(2.0 * std::f64::consts::PI * 100.0 / C, 50.0, 0.05, z);
        assert!((q - 1.0).norm() < 1e-6, "{q}");
    }

    #[test]
    fn rigid_ground_doubles_pressure_at_low_frequency() {
        // Source et récepteur proches du sol, 25 Hz : réflexion en phase → +6 dB.
        let g = TwoRay {
            hs: 0.5,
            hr: 1.5,
            d: 100.0,
        };
        let ex = g.excess_db(25.0, C, C64::new(1e12, 0.0));
        assert!((ex - 6.0).abs() < 0.2, "{ex}");
    }

    #[test]
    fn grazing_soft_ground_decays_6db_per_doubling() {
        // Incidence rasante sur sol mou, grande distance numérique : |1 + Q| ∝ 1/(kR),
        // soit 6 dB de plus par doublement de distance (onde de surface exclue).
        let z = impedance(ImpedanceModel::DelanyBazley, 500.0, 200.0);
        let a = TwoRay {
            hs: 0.0,
            hr: 0.0,
            d: 1000.0,
        }
        .excess_db(500.0, C, z);
        let b = TwoRay {
            hs: 0.0,
            hr: 0.0,
            d: 2000.0,
        }
        .excess_db(500.0, C, z);
        assert!(((a - b) - 6.0).abs() < 0.5, "{a} {b}");
    }

    #[test]
    fn plane_wave_limit_at_large_numerical_distance() {
        // Hors incidence rasante et loin : Q → Rp.
        let z = impedance(ImpedanceModel::DelanyBazley, 1000.0, 200.0);
        let k = 2.0 * std::f64::consts::PI * 1000.0 / C;
        let sin_psi = 0.5;
        let q = spherical_reflection(k, 500.0, sin_psi, z);
        let rp = plane_reflection(z, sin_psi);
        assert!((q - rp).norm() < 0.01, "{q} {rp}");
    }

    #[test]
    fn ground_dip_on_grassland() {
        // Creux d'interférence caractéristique entre ~200 et 1000 Hz pour
        // hs = 1,2 m, hr = 1,5 m, d = 50 m sur prairie (classe D).
        let g = TwoRay {
            hs: 1.2,
            hr: 1.5,
            d: 50.0,
        };
        let min = (100..2000)
            .step_by(10)
            .map(|f| {
                let f = f as f64;
                (
                    f,
                    g.excess_db(f, C, impedance(ImpedanceModel::DelanyBazley, f, 200.0)),
                )
            })
            .fold((0.0, f64::MAX), |a, b| if b.1 < a.1 { b } else { a });
        assert!(min.0 > 150.0 && min.0 < 1000.0, "creux à {} Hz", min.0);
        assert!(min.1 < -10.0, "profondeur {}", min.1);
    }

    #[test]
    fn wide_band_on_hard_ground_tends_to_energy_sum() {
        // Plusieurs lobes d'interférence par octave : la moyenne de bande doit
        // tendre vers la somme en énergie (+3 dB), sans creux replié.
        for d in [25.0, 50.0, 100.0] {
            let g = TwoRay {
                hs: 2.0,
                hr: 4.0,
                d,
            };
            let ex = g.excess_band_db(5657.0, 11314.0, 3, C, ImpedanceModel::DelanyBazley, 2e8);
            assert!((ex - 3.0).abs() < 1.0, "d = {d} : {ex}");
        }
    }

    #[test]
    fn band_average_is_smoother_than_single_frequency() {
        let g = TwoRay {
            hs: 1.2,
            hr: 1.5,
            d: 50.0,
        };
        let band = g.excess_band_db(355.0, 710.0, 9, C, ImpedanceModel::DelanyBazley, 200.0);
        assert!(band > -15.0 && band < 3.0, "{band}");
    }
}
