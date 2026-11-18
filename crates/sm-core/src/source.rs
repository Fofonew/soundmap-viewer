//! Sources : modèles d'enceintes (presets génériques et saisie manuelle),
//! directivité, spectres de programme, sommes cohérente (subs) et
//! incohérente (têtes, line arrays).
//!
//! Les niveaux `max_spl_1m` sont des niveaux **en bande d'octave**, en champ
//! libre (sans sol), sur l'axe, à 1 m, en régime continu. L'effet du sol
//! (+6 dB en basses fréquences pour un caisson posé au sol) est calculé
//! séparément par le modèle de propagation : ne pas l'inclure ici.

use num_complex::Complex64 as C64;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerKind {
    /// Caisson de basses : sommé de façon **cohérente** avec les autres subs.
    Sub,
    /// Tête (médium-aigu) : sommée en énergie.
    Top,
    /// Line array : sommé en énergie, directivité verticale de ligne.
    LineArray,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Directivity {
    /// Monopôle (subs en basses fréquences).
    Omni,
    /// Pavillon : couverture nominale (−6 dB) au-dessus de `f_control`, piston
    /// de rayon `radius_m` en dessous.
    Horn {
        h_cov_deg: f64,
        v_cov_deg: f64,
        f_control: f64,
        radius_m: f64,
    },
    /// Ligne verticale de `n` éléments espacés de `spacing_m`, couverture
    /// horizontale `h_cov_deg`.
    Line {
        n: u32,
        spacing_m: f64,
        h_cov_deg: f64,
    },
    /// Cardioïde du premier ordre (sub cardioïde intégré, à haut-parleur
    /// arrière) : p(θ) = A + B·cos θ, A + B = 1, A − B = 10^(−R/20), avec R
    /// l'atténuation à l'arrière, indépendante de la fréquence dans la bande.
    Cardioid { rear_rejection_db: f64 },
}

/// Dimensions extérieures d'une enceinte (m), pour le dessin 3D.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Dims {
    pub width_m: f64,
    pub height_m: f64,
    pub depth_m: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerModel {
    pub id: String,
    pub name: String,
    pub kind: SpeakerKind,
    /// SPL max continu sur l'axe à 1 m, champ libre, par bande d'octave (dB).
    pub max_spl_1m: f64,
    /// Bande passante à −3 dB (Hz).
    pub f_lo: f64,
    pub f_hi: f64,
    /// Pente hors bande (dB/octave).
    pub rolloff_db_oct: f64,
    pub directivity: Directivity,
    /// Puissance d'amplification nominale (W) pour le dimensionnement.
    pub power_w: f64,
    /// Dimensions (dessin 3D) ; `None` = valeurs types du genre d'enceinte.
    #[serde(default)]
    pub dims: Option<Dims>,
    /// Réponse mesurée (Hz, dB relatifs au niveau max), points croissants en
    /// fréquence. Remplace f_lo/f_hi/pente quand elle a au moins 2 points ;
    /// prolongée hors des points par la pente `rolloff_db_oct`.
    #[serde(default)]
    pub response: Vec<[f64; 2]>,
    /// Description libre (haut-parleurs, charge, marque…).
    #[serde(default)]
    pub notes: String,
    /// Modèle saisi par l'utilisateur (bibliothèque « Mes enceintes »).
    #[serde(default)]
    pub custom: bool,
}

/// Presets génériques (ordres de grandeur de modèles courants, à ajuster).
pub fn presets() -> Vec<SpeakerModel> {
    vec![
        SpeakerModel {
            id: "sub18".into(),
            name: "Sub 18\" (bass-reflex)".into(),
            kind: SpeakerKind::Sub,
            max_spl_1m: 119.0,
            f_lo: 35.0,
            f_hi: 110.0,
            rolloff_db_oct: 24.0,
            directivity: Directivity::Omni,
            power_w: 1200.0,
            dims: Some(Dims {
                width_m: 0.6,
                height_m: 0.65,
                depth_m: 0.8,
            }),
            response: Vec::new(),
            notes: "1 × 18\", bass-reflex".into(),
            custom: false,
        },
        SpeakerModel {
            id: "sub2x18".into(),
            name: "Sub 2×18\"".into(),
            kind: SpeakerKind::Sub,
            max_spl_1m: 125.0,
            f_lo: 32.0,
            f_hi: 110.0,
            rolloff_db_oct: 24.0,
            directivity: Directivity::Omni,
            power_w: 2400.0,
            dims: Some(Dims {
                width_m: 1.2,
                height_m: 0.65,
                depth_m: 0.8,
            }),
            response: Vec::new(),
            notes: "2 × 18\", bass-reflex".into(),
            custom: false,
        },
        SpeakerModel {
            id: "top15".into(),
            name: "Top 15\" + 2\" (90×50)".into(),
            kind: SpeakerKind::Top,
            max_spl_1m: 123.0,
            f_lo: 90.0,
            f_hi: 16_000.0,
            rolloff_db_oct: 24.0,
            directivity: Directivity::Horn {
                h_cov_deg: 90.0,
                v_cov_deg: 50.0,
                f_control: 1200.0,
                radius_m: 0.19,
            },
            power_w: 800.0,
            dims: Some(Dims {
                width_m: 0.45,
                height_m: 0.75,
                depth_m: 0.45,
            }),
            response: Vec::new(),
            notes: "15\" + moteur 2\"".into(),
            custom: false,
        },
        SpeakerModel {
            id: "linearray8".into(),
            name: "Line array générique (8 éléments)".into(),
            kind: SpeakerKind::LineArray,
            max_spl_1m: 129.0,
            f_lo: 70.0,
            f_hi: 18_000.0,
            rolloff_db_oct: 24.0,
            directivity: Directivity::Line {
                n: 8,
                spacing_m: 0.35,
                h_cov_deg: 110.0,
            },
            power_w: 8.0 * 1000.0,
            dims: Some(Dims {
                width_m: 1.0,
                height_m: 2.8,
                depth_m: 0.5,
            }),
            response: Vec::new(),
            notes: "8 éléments 2 × 10\" + moteur".into(),
            custom: false,
        },
    ]
}

pub fn preset(id: &str) -> Option<SpeakerModel> {
    presets().into_iter().find(|p| p.id == id)
}

/// Bessel J1 (approximation polynomiale, Numerical Recipes §6.5, erreur < 1e-7).
pub fn bessel_j1(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let a = x
            * (72_362_614_232.0
                + y * (-7_895_059_235.0
                    + y * (242_396_853.1
                        + y * (-2_972_611.439 + y * (15_704.482_60 + y * (-30.160_366_06))))));
        let b = 144_725_228_442.0
            + y * (2_300_535_178.0
                + y * (18_583_304.74 + y * (99_447.433_94 + y * (376.999_139_7 + y))));
        a / b
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 2.356_194_491;
        let a = 1.0
            + y * (0.183_105e-2
                + y * (-0.351_639_649_6e-4 + y * (0.245_752_017_3e-5 + y * (-0.240_337_019e-6))));
        let b = 0.046_874_999_95
            + y * (-0.200_269_087_3e-3
                + y * (0.844_919_909_6e-5 + y * (-0.882_289_87e-6 + y * 0.105_787_412e-6)));
        let ans = (std::f64::consts::FRAC_2_PI / ax).sqrt() * (xx.cos() * a - z * xx.sin() * b);
        if x < 0.0 { -ans } else { ans }
    }
}

/// Directivité d'un piston plan bafflé : 20·log10|2J1(ka sinθ)/(ka sinθ)|,
/// avec une décroissance supplémentaire vers l'arrière (θ > 90°).
pub fn piston_db(ka: f64, theta: f64) -> f64 {
    let th = theta.abs().min(PI);
    let front = th.min(PI / 2.0);
    let v = ka * front.sin();
    let d = if v < 1e-6 {
        1.0
    } else {
        (2.0 * bessel_j1(v) / v).abs()
    };
    let back = if th > PI / 2.0 {
        -6.0 * (th - PI / 2.0) / (PI / 2.0)
    } else {
        0.0
    };
    (20.0 * d.max(1e-3).log10()).max(-30.0) + back
}

impl SpeakerModel {
    /// Dimensions pour le dessin (valeurs types si non renseignées).
    pub fn dims_or_default(&self) -> Dims {
        self.dims.unwrap_or(match (&self.kind, &self.directivity) {
            (_, Directivity::Line { n, spacing_m, .. }) => Dims {
                width_m: 1.0,
                height_m: f64::from(*n) * spacing_m,
                depth_m: 0.5,
            },
            (SpeakerKind::Sub, _) => Dims {
                width_m: 0.6,
                height_m: 0.65,
                depth_m: 0.8,
            },
            _ => Dims {
                width_m: 0.45,
                height_m: 0.75,
                depth_m: 0.45,
            },
        })
    }

    /// Réponse en fréquence relative au niveau max (dB, ≤ 0 en général).
    pub fn response_db(&self, f: f64) -> f64 {
        if let [first, .., last] = self.response.as_slice() {
            let lf = f.max(1e-3).log2();
            if f <= first[0] {
                return first[1] - self.rolloff_db_oct * (first[0] / f).log2();
            }
            if f >= last[0] {
                return last[1] - self.rolloff_db_oct * (f / last[0]).log2();
            }
            for w in self.response.windows(2) {
                let (a, b) = (w[0], w[1]);
                if f <= b[0] {
                    let (la, lb) = (a[0].log2(), b[0].log2());
                    let t = if lb > la { (lf - la) / (lb - la) } else { 0.0 };
                    return a[1] + (b[1] - a[1]) * t;
                }
            }
            return last[1];
        }
        if f < self.f_lo {
            -self.rolloff_db_oct * (self.f_lo / f).log2()
        } else if f > self.f_hi {
            -self.rolloff_db_oct * (f / self.f_hi).log2()
        } else {
            0.0
        }
    }

    /// Directivité (dB, ≤ 0) pour un écart horizontal `h` et vertical `v`
    /// (radians) par rapport à l'axe de l'enceinte.
    pub fn directivity_db(&self, f: f64, c: f64, h: f64, v: f64) -> f64 {
        let h = wrap_pi(h);
        let theta = (h.cos() * v.cos()).clamp(-1.0, 1.0).acos();
        match &self.directivity {
            Directivity::Omni => 0.0,
            Directivity::Horn {
                h_cov_deg,
                v_cov_deg,
                f_control,
                radius_m,
            } => {
                let k = 2.0 * PI * f / c;
                let piston = piston_db(k * radius_m, theta);
                let hh = h.abs() / (h_cov_deg.to_radians() / 2.0);
                let vv = v.abs() / (v_cov_deg.to_radians() / 2.0);
                let horn = (-6.0 * (hh * hh + vv * vv)).max(-25.0)
                    + if theta > PI / 2.0 { -6.0 } else { 0.0 };
                let g = (f / f_control).powi(2).min(1.0);
                g * horn + (1.0 - g) * piston
            }
            Directivity::Cardioid { rear_rejection_db } => {
                let r = 10f64.powf(-rear_rejection_db.max(0.0) / 20.0);
                let p = (1.0 + r) / 2.0 + (1.0 - r) / 2.0 * theta.cos();
                (20.0 * p.abs().max(1e-3).log10()).max(-40.0)
            }
            Directivity::Line {
                n,
                spacing_m,
                h_cov_deg,
            } => {
                let n = f64::from((*n).max(1));
                let x = PI * spacing_m * f / c * v.sin();
                let af = if x.sin().abs() < 1e-9 {
                    1.0
                } else {
                    ((n * x).sin() / (n * x.sin())).abs()
                };
                let hh = h.abs() / (h_cov_deg.to_radians() / 2.0);
                let horiz = (-6.0 * hh * hh).max(-20.0);
                let back = if h.abs() > PI / 2.0 { -10.0 } else { 0.0 };
                (20.0 * af.max(1e-3).log10()).max(-30.0) + horiz + back
            }
        }
    }
}

fn wrap_pi(a: f64) -> f64 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

/// Spectre de programme : niveaux relatifs par octave (31,5 Hz … 8 kHz).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    pub id: String,
    pub name: String,
    pub octave_db: [f64; 9],
}

pub fn programs() -> Vec<Program> {
    let p = |id: &str, name: &str, v: [f64; 9]| Program {
        id: id.into(),
        name: name.into(),
        octave_db: v,
    };
    vec![
        p(
            "tekno",
            "Tekno / hardtek",
            [-3.0, 0.0, -1.0, -6.0, -9.0, -11.0, -13.0, -15.0, -19.0],
        ),
        p(
            "techno",
            "Techno",
            [-6.0, 0.0, -3.0, -6.0, -8.0, -10.0, -12.0, -14.0, -18.0],
        ),
        p(
            "dnb",
            "Drum'n'bass",
            [-2.0, 0.0, -2.0, -6.0, -9.0, -11.0, -12.0, -14.0, -18.0],
        ),
        p(
            "rock",
            "Rock / live",
            [-12.0, -5.0, -2.0, -2.0, -3.0, -4.0, -6.0, -9.0, -14.0],
        ),
        p("rose", "Bruit rose (plat par octave)", [0.0; 9]),
    ]
}

pub fn program(id: &str) -> Program {
    programs()
        .into_iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| programs().remove(0))
}

impl Program {
    /// Niveau relatif (dB) à la fréquence f (interpolation en log f).
    pub fn level_db(&self, f: f64) -> f64 {
        let x = (f / 31.5).log2(); // 0 à 31,5 Hz, 8 à 8 kHz
        if x <= 0.0 {
            return self.octave_db[0] + 6.0 * x; // −6 dB/oct sous 31,5 Hz
        }
        if x >= 8.0 {
            return self.octave_db[8] - 6.0 * (x - 8.0);
        }
        let i = x.floor() as usize;
        let t = x - i as f64;
        self.octave_db[i] * (1.0 - t) + self.octave_db[(i + 1).min(8)] * t
    }
}

/// Enceinte placée, en coordonnées locales (m, x vers l'est, y vers le nord).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub x: f64,
    pub y: f64,
    /// Altitude absolue du centre acoustique (m) = sol + hauteur.
    pub z: f64,
    /// Hauteur au-dessus du sol (m).
    pub height: f64,
    /// Azimut de l'axe (°, 0 = nord, sens horaire).
    pub azimuth_deg: f64,
    /// Site (°, positif vers le haut).
    pub tilt_deg: f64,
    pub delay_s: f64,
    /// +1 ou −1.
    pub polarity: f64,
    pub gain_db: f64,
    pub model: SpeakerModel,
}

impl Source {
    /// Niveau émis sur l'axe à 1 m dans la bande [lo, hi] centrée en `fc`
    /// (dB), pour le programme donné.
    pub fn band_level_1m(&self, fc: f64, width_thirds: u8, program: &Program) -> f64 {
        self.model.max_spl_1m
            + self.model.response_db(fc)
            + program.level_db(fc)
            + self.gain_db
            + 10.0 * (f64::from(width_thirds) / 3.0).log10()
    }

    /// Angles (h, v) du récepteur par rapport à l'axe, et distance 3D.
    pub fn geometry_to(&self, rx: [f64; 3]) -> (f64, f64, f64) {
        let (dx, dy, dz) = (rx[0] - self.x, rx[1] - self.y, rx[2] - self.z);
        let dh = dx.hypot(dy);
        let r = dh.hypot(dz).max(0.1);
        let bearing = dx.atan2(dy); // 0 = nord, sens horaire
        let elev = dz.atan2(dh);
        (
            bearing - self.azimuth_deg.to_radians(),
            elev - self.tilt_deg.to_radians(),
            r,
        )
    }
}

/// Énergies au récepteur dans une bande, en p² relatif (10·log10 → dB SPL),
/// divergence sphérique comprise : (subs sommés en cohérence, autres en énergie).
///
/// La cohérence est évaluée sur `n_sub` fréquences réparties dans la bande et
/// moyennée en énergie, ce qui reproduit la largeur de bande du signal réel.
#[allow(clippy::too_many_arguments)]
pub fn band_energy(
    sources: &[Source],
    rx: [f64; 3],
    fc: f64,
    lo: f64,
    hi: f64,
    width_thirds: u8,
    program: &Program,
    c: f64,
    n_sub: usize,
) -> (f64, f64) {
    let mut incoherent = 0.0;
    let mut coh_energy = 0.0;
    let n = n_sub.max(1);
    let mut has_sub = false;
    for s in sources {
        if s.model.kind != SpeakerKind::Sub {
            let (h, v, r) = s.geometry_to(rx);
            let l =
                s.band_level_1m(fc, width_thirds, program) + s.model.directivity_db(fc, c, h, v);
            incoherent += 10f64.powf(l / 10.0) / (r * r);
        } else {
            has_sub = true;
        }
    }
    if has_sub {
        for i in 0..n {
            let t = (i as f64 + 0.5) / n as f64;
            let f = lo * (hi / lo).powf(t);
            let k = 2.0 * PI * f / c;
            let w = 2.0 * PI * f;
            let mut p = C64::new(0.0, 0.0);
            for s in sources.iter().filter(|s| s.model.kind == SpeakerKind::Sub) {
                let (h, v, r) = s.geometry_to(rx);
                let l =
                    s.band_level_1m(fc, width_thirds, program) + s.model.directivity_db(f, c, h, v);
                let a = 10f64.powf(l / 20.0) * s.polarity / r;
                p += C64::from_polar(a, k * r + w * s.delay_s);
            }
            coh_energy += p.norm_sqr();
        }
        coh_energy /= n as f64;
    }
    (coh_energy, incoherent)
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: f64 = 343.0;

    fn sub_at(x: f64, y: f64, delay: f64, pol: f64) -> Source {
        Source {
            x,
            y,
            z: 0.5,
            height: 0.5,
            azimuth_deg: 0.0,
            tilt_deg: 0.0,
            delay_s: delay,
            polarity: pol,
            gain_db: 0.0,
            model: preset("sub18").unwrap(),
        }
    }

    #[test]
    fn bessel_j1_reference() {
        assert!((bessel_j1(1.0) - 0.440_050_6).abs() < 1e-6);
        assert!((bessel_j1(3.831_705_97)).abs() < 1e-6); // premier zéro
        assert!((bessel_j1(10.0) - 0.043_472_75).abs() < 1e-6);
    }

    #[test]
    fn free_field_inverse_square_law() {
        let p = program("rose");
        let s = [sub_at(0.0, 0.0, 0.0, 1.0)];
        let (e1, _) = band_energy(&s, [0.0, 10.0, 0.5], 63.0, 44.7, 89.1, 3, &p, C, 5);
        let (e2, _) = band_energy(&s, [0.0, 20.0, 0.5], 63.0, 44.7, 89.1, 3, &p, C, 5);
        let d = 10.0 * (e1 / e2).log10();
        assert!((d - 6.02).abs() < 0.05, "{d}");
    }

    #[test]
    fn two_coherent_subs_add_6db_on_axis() {
        let p = program("rose");
        let one = band_energy(
            &[sub_at(0.0, 0.0, 0.0, 1.0)],
            [100.0, 0.0, 0.5],
            63.0,
            56.0,
            70.7,
            1,
            &p,
            C,
            3,
        )
        .0;
        let two = band_energy(
            &[sub_at(0.0, -0.3, 0.0, 1.0), sub_at(0.0, 0.3, 0.0, 1.0)],
            [100.0, 0.0, 0.5],
            63.0,
            56.0,
            70.7,
            1,
            &p,
            C,
            3,
        )
        .0;
        assert!((10.0 * (two / one).log10() - 6.0).abs() < 0.1);
    }

    #[test]
    fn cardioid_pair_rejects_rear() {
        // Montage cardioïde : sub arrière à d = λ/4 derrière, inversé et retardé de d/c.
        let p = program("rose");
        // Vers l'avant : le retard compense le trajet, l'inversion donne +6 dB à λ/4.
        // Vers l'arrière : les deux arrivent en phase, l'inversion les annule.
        let d = C / 63.0 / 4.0;
        let subs = [sub_at(0.0, 0.0, 0.0, 1.0), sub_at(0.0, -d, d / C, -1.0)];
        let front = band_energy(&subs, [0.0, 200.0, 0.5], 63.0, 56.0, 70.7, 1, &p, C, 3).0;
        let rear = band_energy(&subs, [0.0, -200.0, 0.5], 63.0, 56.0, 70.7, 1, &p, C, 3).0;
        let ratio = 10.0 * (front / rear).log10();
        assert!(ratio > 15.0, "réjection arrière {ratio:.1} dB");
    }

    #[test]
    fn horn_coverage_minus_6db_at_edge() {
        let m = preset("top15").unwrap();
        let edge = m.directivity_db(4000.0, C, 45f64.to_radians(), 0.0);
        assert!((edge + 6.0).abs() < 0.5, "{edge}");
        assert!(m.directivity_db(4000.0, C, PI, 0.0) < -20.0);
        // En basses fréquences le pavillon devient quasi omnidirectionnel.
        assert!(m.directivity_db(100.0, C, 45f64.to_radians(), 0.0) > -1.5);
    }

    #[test]
    fn line_array_narrows_vertically_at_high_frequency() {
        let m = preset("linearray8").unwrap();
        let lf = m.directivity_db(200.0, C, 0.0, 20f64.to_radians());
        let hf = m.directivity_db(2000.0, C, 0.0, 20f64.to_radians());
        assert!(hf < lf - 6.0, "{lf} {hf}");
    }

    #[test]
    fn program_interpolation() {
        let p = program("tekno");
        assert_eq!(p.level_db(63.0), 0.0);
        assert!(p.level_db(25.0) < p.level_db(31.5));
        assert!((p.level_db(1000.0) - (-11.0)).abs() < 0.2);
    }

    #[test]
    fn cardioid_pattern() {
        let mut m = preset("sub18").unwrap();
        m.directivity = Directivity::Cardioid {
            rear_rejection_db: 20.0,
        };
        assert!(m.directivity_db(63.0, C, 0.0, 0.0).abs() < 1e-9);
        assert!((m.directivity_db(63.0, C, PI, 0.0) + 20.0).abs() < 1e-6);
        // Côté : A = (1 + 0,1)/2 = 0,55 → −5,2 dB.
        assert!((m.directivity_db(63.0, C, PI / 2.0, 0.0) + 5.19).abs() < 0.02);
    }

    #[test]
    fn measured_response_overrides_band_limits() {
        let mut m = preset("sub18").unwrap();
        m.response = vec![[30.0, -6.0], [40.0, 0.0], [100.0, 0.0], [150.0, -10.0]];
        assert!((m.response_db(40.0)).abs() < 1e-9);
        assert!(
            (m.response_db(35.0) + 2.79).abs() < 0.05,
            "{}",
            m.response_db(35.0)
        );
        // Sous le premier point : pente de 24 dB/oct.
        assert!((m.response_db(15.0) + 30.0).abs() < 1e-9);
        assert!((m.response_db(300.0) + 34.0).abs() < 1e-9);
        assert_eq!(m.dims_or_default().width_m, 0.6);
    }
}
