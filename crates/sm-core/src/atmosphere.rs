//! Atmosphère : absorption (ISO 9613-1), célérité, profils de Monin-Obukhov
//! ajustés sur le profil AROME, classes de stabilité de Pasquill.

use serde::{Deserialize, Serialize};

/// Coefficient d'absorption atmosphérique α (dB/m), ISO 9613-1:1993 éq. (3)–(5).
pub fn iso9613_1_alpha(f: f64, t_c: f64, rh: f64, p_kpa: f64) -> f64 {
    const T0: f64 = 293.15;
    const T01: f64 = 273.16;
    const PR: f64 = 101.325;
    let t = t_c + 273.15;
    let pa = p_kpa / PR;
    let c = -6.8346 * (T01 / t).powf(1.261) + 4.6151;
    let psat = 10f64.powf(c);
    let h = rh * psat / pa;
    let fro = pa * (24.0 + 4.04e4 * h * (0.02 + h) / (0.391 + h));
    let frn = pa
        * (t / T0).powf(-0.5)
        * (9.0 + 280.0 * h * (-4.170 * ((t / T0).powf(-1.0 / 3.0) - 1.0)).exp());
    let f2 = f * f;
    8.686
        * f2
        * (1.84e-11 / pa * (t / T0).sqrt()
            + (t / T0).powf(-2.5)
                * (0.01275 * (-2239.1 / t).exp() / (fro + f2 / fro)
                    + 0.1068 * (-3352.0 / t).exp() / (frn + f2 / frn)))
}

/// Célérité du son dans l'air sec (m/s) à t (°C).
pub fn sound_speed(t_c: f64) -> f64 {
    331.3 * (1.0 + t_c / 273.15).sqrt()
}

pub const KAPPA: f64 = 0.4;
pub const GRAVITY: f64 = 9.81;
/// Gradient adiabatique sec (K/m).
pub const GAMMA_D: f64 = 0.0098;

/// Fonctions de stabilité de Businger-Dyer intégrées.
fn psi_m(zeta: f64) -> f64 {
    if zeta >= 0.0 {
        -5.0 * zeta
    } else {
        let x = (1.0 - 16.0 * zeta).powf(0.25);
        2.0 * ((1.0 + x) / 2.0).ln() + ((1.0 + x * x) / 2.0).ln() - 2.0 * x.atan()
            + std::f64::consts::FRAC_PI_2
    }
}

fn psi_h(zeta: f64) -> f64 {
    if zeta >= 0.0 {
        -5.0 * zeta
    } else {
        let x = (1.0 - 16.0 * zeta).powf(0.25);
        2.0 * ((1.0 + x * x) / 2.0).ln()
    }
}

/// Profil de couche de surface (similitude de Monin-Obukhov).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoProfile {
    /// Vitesse de frottement u* (m/s).
    pub u_star: f64,
    /// Échelle de température T* (K) : > 0 en conditions stables.
    pub t_star: f64,
    /// Inverse de la longueur d'Obukhov 1/L (m⁻¹) : > 0 stable, < 0 instable.
    pub inv_l: f64,
    /// Longueur de rugosité z₀ (m).
    pub z0: f64,
    /// Température de référence (°C) à `z_ref`.
    pub t_ref: f64,
    pub z_ref: f64,
    /// Direction d'où vient le vent (°, 0 = nord), par hauteur (interpolée).
    pub wind_dir: Vec<(f64, f64)>,
    /// Au-dessus de cette hauteur, extrapolation à gradient constant.
    pub z_top: f64,
    /// Humidité relative (%) pour l'absorption.
    pub rh: f64,
    /// Qualité de l'ajustement (écart-type des résidus : vent m/s, T K).
    pub rms_u: f64,
    pub rms_t: f64,
    /// Origine du profil (« AROME », « Pasquill F », « scénario »…).
    pub source: String,
    /// Atmosphère homogène stricte : T constante, vent nul.
    #[serde(default)]
    pub homog: bool,
    /// Écoulements de vallée nocturnes superposés au profil (lac d'air froid,
    /// vent catabatique).
    #[serde(default)]
    pub valley: Option<ValleyFlow>,
}

/// Écoulements de vallée nocturnes : lac d'air froid (inversion de surface
/// confinée par le relief) et vent catabatique (jet de pente).
/// Whiteman, *Mountain Meteorology* (2000), ch. 8–9 ; Prandtl (1942).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ValleyFlow {
    /// Épaisseur du lac d'air froid (m).
    pub pool_depth_m: f64,
    /// Déficit de température au sol (K, > 0).
    pub pool_dt_k: f64,
    /// Vent catabatique : vitesse maximale (m/s) et hauteur du maximum (m).
    pub kat_umax: f64,
    pub kat_height_m: f64,
    /// Direction vers laquelle souffle le vent catabatique (°, vers l'aval).
    pub kat_toward_deg: f64,
}

impl ValleyFlow {
    /// Écart de température (K, ≤ 0) : −ΔT·(1 − z/h)² sous h, 0 au-dessus.
    pub fn dt(&self, z: f64) -> f64 {
        let h = self.pool_depth_m.max(1.0);
        if z >= h {
            0.0
        } else {
            -self.pool_dt_k * (1.0 - z.max(0.0) / h).powi(2)
        }
    }

    /// Vitesse du vent catabatique (m/s) : profil de Prandtl
    /// u(z) = u₀·e^(−z/l)·sin(z/l), maximum u₀·e^(−π/4)·sin(π/4) en z = πl/4.
    pub fn kat(&self, z: f64) -> f64 {
        let l = 4.0 * self.kat_height_m.max(0.5) / std::f64::consts::PI;
        let u0 = self.kat_umax
            / ((-std::f64::consts::FRAC_PI_4).exp() * std::f64::consts::FRAC_PI_4.sin());
        let x = z.max(0.0) / l;
        (u0 * (-x).exp() * x.sin()).max(0.0)
    }
}

impl MoProfile {
    fn z_clamped(&self, z: f64) -> f64 {
        z.clamp(self.z0 * 1.01, self.z_top)
    }

    fn wind_raw(&self, z: f64) -> f64 {
        let z = z.max(self.z0 * 1.01);
        (self.u_star / KAPPA * ((z / self.z0).ln() - psi_m(z * self.inv_l))).max(0.0)
    }

    fn temp_raw(&self, z: f64) -> f64 {
        let z = z.max(self.z0 * 1.01);
        let zr = self.z_ref.max(self.z0 * 1.01);
        self.t_ref
            + self.t_star / KAPPA * ((z / zr).ln() - psi_h(z * self.inv_l) + psi_h(zr * self.inv_l))
            - GAMMA_D * (z - zr)
    }

    /// Vitesse du vent (m/s) à z ; gradient constant au-dessus de `z_top`.
    pub fn wind(&self, z: f64) -> f64 {
        if self.homog {
            return 0.0;
        }
        if z <= self.z_top {
            return self.wind_raw(z);
        }
        let g = (self.wind_raw(self.z_top) - self.wind_raw(self.z_top * 0.8)) / (self.z_top * 0.2);
        (self.wind_raw(self.z_top) + g * (z - self.z_top)).max(0.0)
    }

    /// Température (°C) à z ; gradient constant au-dessus de `z_top`.
    pub fn temperature(&self, z: f64) -> f64 {
        if self.homog {
            return self.t_ref;
        }
        let pool = self.valley.map_or(0.0, |v| v.dt(z));
        let zc = self.z_clamped(z);
        if z <= self.z_top {
            return self.temp_raw(zc) + pool;
        }
        let g = (self.temp_raw(self.z_top) - self.temp_raw(self.z_top * 0.8)) / (self.z_top * 0.2);
        self.temp_raw(self.z_top) + g * (z - self.z_top) + pool
    }

    /// Direction d'où vient le vent (°) à z (interpolation linéaire en z).
    pub fn wind_from_deg(&self, z: f64) -> f64 {
        interp_angle(&self.wind_dir, z)
    }

    /// Célérité effective (m/s) dans la direction de propagation `az_deg`
    /// (0 = nord) : c(T(z)) + u(z)·cos(θ_vent_vers − θ_az).
    pub fn c_eff(&self, z: f64, az_deg: f64) -> f64 {
        let to = (self.wind_from_deg(z) + 180.0).to_radians();
        let kat = match (self.valley, self.homog) {
            (Some(v), false) => {
                v.kat(z) * (v.kat_toward_deg.to_radians() - az_deg.to_radians()).cos()
            }
            _ => 0.0,
        };
        sound_speed(self.temperature(z)) + self.wind(z) * (to - az_deg.to_radians()).cos() + kat
    }

    /// Température moyenne près du sol (°C), pour l'absorption.
    pub fn ground_temperature(&self) -> f64 {
        self.temperature(2.0)
    }

    /// Profil homogène (célérité constante) : référence « neutre sans vent ».
    pub fn homogeneous(t_c: f64, rh: f64) -> Self {
        Self {
            u_star: 0.0,
            t_star: 0.0,
            inv_l: 0.0,
            z0: 0.05,
            t_ref: t_c,
            z_ref: 2.0,
            wind_dir: vec![(10.0, 0.0)],
            z_top: 150.0,
            rh,
            rms_u: 0.0,
            rms_t: 0.0,
            source: "homogène".into(),
            homog: true,
            valley: None,
        }
    }
}

/// Interpolation linéaire d'angles (déroulés) en fonction de la hauteur.
fn interp_angle(pts: &[(f64, f64)], z: f64) -> f64 {
    match pts {
        [] => 0.0,
        [(_, a)] => *a,
        _ => {
            if z <= pts[0].0 {
                return pts[0].1;
            }
            for w in pts.windows(2) {
                let ((z0, a0), (z1, a1)) = (w[0], w[1]);
                if z <= z1 {
                    let mut d = a1 - a0;
                    if d > 180.0 {
                        d -= 360.0;
                    } else if d < -180.0 {
                        d += 360.0;
                    }
                    return (a0 + d * (z - z0) / (z1 - z0)).rem_euclid(360.0);
                }
            }
            pts[pts.len() - 1].1
        }
    }
}

/// Observation d'un profil vertical (AROME via Open-Meteo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileObs {
    /// (hauteur m, température °C)
    pub temperature: Vec<(f64, f64)>,
    /// (hauteur m, vitesse m/s, direction d'où vient le vent °)
    pub wind: Vec<(f64, f64, Option<f64>)>,
    pub rh: f64,
    pub cloud_cover: Option<f64>,
}

/// Ajuste un profil de Monin-Obukhov sur les observations : recherche de 1/L
/// sur une grille, u* et (T_ref, T*) par moindres carrés à L fixé.
pub fn fit_monin_obukhov(obs: &ProfileObs, z0: f64) -> Option<MoProfile> {
    if obs.temperature.len() < 2 || obs.wind.is_empty() {
        return None;
    }
    let mut best: Option<(f64, MoProfile)> = None;
    // Grille symétrique en 1/L, plus fine près de la neutralité.
    let mut grid = vec![0.0];
    let mut v = 1e-4;
    while v <= 0.5 {
        grid.push(v);
        grid.push(-v);
        v *= 1.15;
    }
    for inv_l in grid {
        // Vent : u = (u*/κ) g(z), g = ln(z/z0) − ψm(z/L).
        let (mut sgg, mut sug) = (0.0, 0.0);
        for (z, u, _) in &obs.wind {
            let g = (z / z0).ln() - psi_m(z * inv_l);
            sgg += g * g;
            sug += u * g;
        }
        if sgg <= 0.0 {
            continue;
        }
        let u_star = (KAPPA * sug / sgg).max(0.0);
        let rms_u = (obs
            .wind
            .iter()
            .map(|(z, u, _)| {
                let m = u_star / KAPPA * ((z / z0).ln() - psi_m(z * inv_l));
                (u - m).powi(2)
            })
            .sum::<f64>()
            / obs.wind.len() as f64)
            .sqrt();
        // Température potentielle θ = T + Γz = A + (T*/κ) h(z), h = ln z − ψh(z/L).
        let n = obs.temperature.len() as f64;
        let hs: Vec<f64> = obs
            .temperature
            .iter()
            .map(|(z, _)| z.ln() - psi_h(z * inv_l))
            .collect();
        let ths: Vec<f64> = obs
            .temperature
            .iter()
            .map(|(z, t)| t + GAMMA_D * z)
            .collect();
        let mh = hs.iter().sum::<f64>() / n;
        let mt = ths.iter().sum::<f64>() / n;
        let (mut sxy, mut sxx) = (0.0, 0.0);
        for (h, t) in hs.iter().zip(&ths) {
            sxy += (h - mh) * (t - mt);
            sxx += (h - mh) * (h - mh);
        }
        let b = if sxx > 0.0 { sxy / sxx } else { 0.0 };
        let a = mt - b * mh;
        let rms_t = (hs
            .iter()
            .zip(&ths)
            .map(|(h, t)| (t - (a + b * h)).powi(2))
            .sum::<f64>()
            / n)
            .sqrt();
        let t_star = b * KAPPA;
        // Cohérence physique : 1/L = κ g T* / (u*² T).
        let t_k = mt + 273.15;
        let inv_l_phys = if u_star > 0.01 {
            KAPPA * GRAVITY * t_star / (u_star * u_star * t_k)
        } else {
            inv_l
        };
        let cons = (inv_l - inv_l_phys.clamp(-0.5, 0.5)).abs();
        let cost = (rms_u / 0.3).powi(2) + (rms_t / 0.1).powi(2) + (cons / 0.01).powi(2) * 0.25;
        let z_ref = obs.temperature[0].0;
        let t_ref = a + b * (z_ref.ln() - psi_h(z_ref * inv_l)) - GAMMA_D * z_ref;
        let mut dirs: Vec<(f64, f64)> = obs
            .wind
            .iter()
            .filter_map(|(z, _, d)| d.map(|d| (*z, d)))
            .collect();
        dirs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let prof = MoProfile {
            u_star,
            t_star,
            inv_l,
            z0,
            t_ref,
            z_ref,
            wind_dir: dirs,
            z_top: obs
                .temperature
                .iter()
                .map(|(z, _)| *z)
                .fold(0.0, f64::max)
                .max(50.0),
            rh: obs.rh,
            rms_u,
            rms_t,
            source: "AROME (Monin-Obukhov ajusté)".into(),
            homog: false,
            valley: None,
        };
        if best.as_ref().is_none_or(|(c, _)| cost < *c) {
            best = Some((cost, prof));
        }
    }
    best.map(|(_, p)| p)
}

/// Classe de stabilité de Pasquill-Gifford.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pasquill {
    A,
    B,
    C,
    D,
    E,
    F,
}

impl Pasquill {
    /// Méthode de Turner simplifiée : vent à 10 m, nébulosité (%), nuit ou jour.
    pub fn from_conditions(wind10: f64, cloud_pct: f64, night: bool) -> Pasquill {
        use Pasquill::*;
        let overcast = cloud_pct >= 50.0;
        if night {
            match wind10 {
                w if w < 2.0 => F,
                w if w < 3.0 => {
                    if overcast {
                        E
                    } else {
                        F
                    }
                }
                w if w < 5.0 => {
                    if overcast {
                        D
                    } else {
                        E
                    }
                }
                _ => D,
            }
        } else {
            // Ensoleillement « modéré » par défaut (atténué par la nébulosité).
            let strong = cloud_pct < 30.0;
            match wind10 {
                w if w < 2.0 => {
                    if strong {
                        A
                    } else {
                        B
                    }
                }
                w if w < 3.0 => B,
                w if w < 5.0 => {
                    if strong {
                        B
                    } else {
                        C
                    }
                }
                w if w < 6.0 => C,
                _ => D,
            }
        }
    }

    /// 1/L (m⁻¹) selon Golder (1972) : 1/L = a + b·log10(z0).
    pub fn inv_l(self, z0: f64) -> f64 {
        let (a, b) = match self {
            Pasquill::A => (-0.096, 0.029),
            Pasquill::B => (-0.037, 0.029),
            Pasquill::C => (-0.002, 0.018),
            Pasquill::D => (0.0, 0.0),
            Pasquill::E => (0.004, -0.018),
            Pasquill::F => (0.035, -0.036),
        };
        a + b * z0.log10()
    }

    /// Profil MO à partir de la classe, du vent à 10 m et de la température à 2 m.
    pub fn profile(self, wind10: f64, wind_from: f64, t2: f64, rh: f64, z0: f64) -> MoProfile {
        let inv_l = self.inv_l(z0);
        let u_star = KAPPA * wind10.max(0.3) / ((10.0 / z0).ln() - psi_m(10.0 * inv_l));
        let t_star = inv_l * u_star * u_star * (t2 + 273.15) / (KAPPA * GRAVITY);
        MoProfile {
            u_star,
            t_star,
            inv_l,
            z0,
            t_ref: t2,
            z_ref: 2.0,
            wind_dir: vec![(10.0, wind_from)],
            z_top: 150.0,
            rh,
            rms_u: 0.0,
            rms_t: 0.0,
            source: format!("Pasquill {self:?}"),
            homog: false,
            valley: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bands::BandSet;

    /// ISO 9613-2:1996 tableau 2 (calculé selon ISO 9613-1), dB/km,
    /// bandes d'octave 63 Hz – 8 kHz, 101,325 kPa.
    const TABLE: [(f64, f64, [f64; 8]); 6] = [
        (10.0, 70.0, [0.1, 0.4, 1.0, 1.9, 3.7, 9.7, 32.8, 117.0]),
        (20.0, 70.0, [0.1, 0.3, 1.1, 2.8, 5.0, 9.0, 22.9, 76.6]),
        (30.0, 70.0, [0.1, 0.3, 1.0, 3.1, 7.4, 12.7, 23.1, 59.3]),
        (15.0, 20.0, [0.3, 0.6, 1.2, 2.7, 8.2, 28.2, 88.8, 202.0]),
        (15.0, 50.0, [0.1, 0.5, 1.2, 2.2, 4.2, 10.8, 36.2, 129.0]),
        (15.0, 80.0, [0.1, 0.3, 1.1, 2.4, 4.1, 8.3, 23.7, 82.8]),
    ];

    #[test]
    fn valley_flow_profiles() {
        let v = ValleyFlow {
            pool_depth_m: 60.0,
            pool_dt_k: 5.0,
            kat_umax: 2.5,
            kat_height_m: 10.0,
            kat_toward_deg: 90.0,
        };
        // Jet de Prandtl : maximum u_max à la hauteur indiquée.
        assert!((v.kat(10.0) - 2.5).abs() < 1e-9);
        assert!(v.kat(5.0) < 2.5 && v.kat(30.0) < 2.5);
        assert!(v.kat(0.0).abs() < 1e-12);
        // Lac d'air froid : −ΔT au sol, 0 au sommet.
        assert!((v.dt(0.0) + 5.0).abs() < 1e-12);
        assert_eq!(v.dt(60.0), 0.0);
        // Inversion + vent vers l'aval : célérité qui croît avec z vers l'est,
        // plus fortement que vers l'ouest (vent contraire).
        let mut p = Pasquill::D.profile(0.5, 0.0, 10.0, 80.0, 0.05);
        p.valley = Some(v);
        let grad = |az: f64| p.c_eff(10.0, az) - p.c_eff(1.0, az);
        assert!(grad(90.0) > 2.0, "{}", grad(90.0));
        assert!(grad(90.0) > grad(270.0) + 1.0);
    }

    #[test]
    fn iso9613_1_matches_iso9613_2_table() {
        let bands: Vec<_> = BandSet::Octave.bands().into_iter().skip(1).collect();
        for (t, rh, vals) in TABLE {
            for (b, expected) in bands.iter().zip(vals) {
                let got = iso9613_1_alpha(b.center().0, t, rh, 101.325) * 1000.0;
                let tol = (0.05 + 0.03 * expected).max(0.06);
                assert!(
                    (got - expected).abs() <= tol,
                    "{t} °C {rh} % {} Hz : {got:.2} ≠ {expected}",
                    b.nominal().0
                );
            }
        }
    }

    #[test]
    fn sound_speed_at_20c() {
        assert!((sound_speed(20.0) - 343.2).abs() < 0.2);
    }

    fn stable_obs() -> ProfileObs {
        // Inversion nocturne typique : +2,4 K entre 2 et 150 m, vent faible.
        ProfileObs {
            temperature: vec![
                (2.0, 8.0),
                (20.0, 9.4),
                (50.0, 9.9),
                (100.0, 10.2),
                (150.0, 10.4),
            ],
            wind: vec![
                (10.0, 1.5, Some(200.0)),
                (20.0, 2.1, Some(205.0)),
                (50.0, 3.2, Some(215.0)),
                (100.0, 4.4, Some(225.0)),
                (150.0, 5.3, Some(230.0)),
            ],
            rh: 85.0,
            cloud_cover: Some(0.0),
        }
    }

    #[test]
    fn mo_fit_detects_stable_night() {
        let p = fit_monin_obukhov(&stable_obs(), 0.05).unwrap();
        assert!(p.inv_l > 0.0, "1/L = {}", p.inv_l);
        assert!(p.t_star > 0.0);
        assert!(p.rms_t < 0.3 && p.rms_u < 0.6, "{} {}", p.rms_t, p.rms_u);
        // Température croissante avec la hauteur : c_eff croissante sans vent.
        assert!(p.temperature(100.0) > p.temperature(2.0));
        // Vent de sud-sud-ouest : propagation vers le nord-nord-est favorisée.
        assert!(p.c_eff(50.0, 30.0) > p.c_eff(50.0, 210.0));
    }

    #[test]
    fn mo_fit_reproduces_observations() {
        let obs = stable_obs();
        let p = fit_monin_obukhov(&obs, 0.05).unwrap();
        for (z, t) in &obs.temperature {
            assert!((p.temperature(*z) - t).abs() < 0.5, "T({z})");
        }
        for (z, u, _) in &obs.wind {
            assert!((p.wind(*z) - u).abs() < 1.0, "u({z})");
        }
    }

    #[test]
    fn pasquill_night_classes() {
        assert_eq!(Pasquill::from_conditions(1.0, 0.0, true), Pasquill::F);
        assert_eq!(Pasquill::from_conditions(4.0, 80.0, true), Pasquill::D);
        assert!(Pasquill::F.inv_l(0.1) > 0.0);
        assert!(Pasquill::A.inv_l(0.1) < 0.0);
        let p = Pasquill::F.profile(1.5, 180.0, 10.0, 80.0, 0.1);
        assert!(
            p.temperature(100.0) > p.temperature(2.0),
            "inversion attendue en classe F"
        );
    }

    #[test]
    fn homogeneous_profile_is_constant() {
        let p = MoProfile::homogeneous(15.0, 70.0);
        assert!((p.c_eff(1.0, 0.0) - p.c_eff(200.0, 90.0)).abs() < 1e-9);
    }
}
