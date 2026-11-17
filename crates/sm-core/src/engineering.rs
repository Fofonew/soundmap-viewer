//! Méthode d'ingénierie (type ISO 9613-2 / CNOSSOS-EU, simplifiée) sur un
//! profil de terrain : effet de sol par plan moyen et Weyl-van der Pol,
//! diffraction multi-arêtes par chemin élastique, feuillage.
//!
//! Conditions homogènes : rayons droits. Conditions favorables (vent portant
//! ou inversion) : rayons courbes de rayon Rc = max(1000 m, 8d), traités
//! comme un abaissement apparent du relief z' = z − x(d−x)/(2Rc), approche
//! équivalente à celle de CNOSSOS-EU pour les différences de marche.
//!
//! Vue obstruée : sol et diffraction combinés comme dans CNOSSOS-EU
//! (Δground(S,O) et Δground(O,R) pondérés par la diffraction des chemins
//! image), avec un plan moyen unique pour tout le trajet.

use crate::diffraction::{DiffractionPath, delta_dif_db, rubber_band};
use crate::foliage::foliage_db;
use crate::ground::{GroundClass, ImpedanceModel, TwoRay};
use serde::{Deserialize, Serialize};

/// Échantillon de profil le long du trajet source → récepteur.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PathSample {
    /// Abscisse le long du trajet (m), 0 à la source.
    pub x: f64,
    /// Altitude du sol (m).
    pub ground: f32,
    /// Hauteur des obstacles opaques au-dessus du sol (bâtiments), m.
    pub obstacle: f32,
    /// Hauteur de la végétation (MNH) au-dessus du sol, m.
    pub canopy: f32,
    /// Classe de sol (index de [`GroundClass`]).
    pub class: u8,
    /// Densité relative du feuillage (1 = « feuillage dense » de l'ISO 9613-2,
    /// 0 = transparent), issue du LiDAR quand il est disponible.
    #[serde(default = "one_f32")]
    pub foliage: f32,
}

fn one_f32() -> f32 {
    1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    Homogeneous,
    #[default]
    Favorable,
}

/// Bande de calcul.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BandSpec {
    pub fc: f64,
    pub lo: f64,
    pub hi: f64,
    pub width_thirds: u8,
}

impl BandSpec {
    pub fn from_band(b: crate::bands::Band) -> Self {
        let (lo, hi) = b.edges();
        Self {
            fc: b.center().0,
            lo: lo.0,
            hi: hi.0,
            width_thirds: b.width_thirds,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PathConfig {
    pub condition: Condition,
    pub model: ImpedanceModel,
    /// Célérité de référence (m/s).
    pub c: f64,
    /// Fréquences par bande pour moyenner les interférences de sol.
    pub n_sub: usize,
    /// Hauteur minimale de végétation comptée comme feuillage dense (m).
    pub canopy_min: f32,
    /// Facteur multiplicatif sur la résistivité σ du sol (calage), 1 par défaut.
    #[serde(default = "one")]
    pub sigma_scale: f64,
    /// Rayon de courbure des rayons imposé (m), prioritaire sur `condition` ;
    /// `f64::INFINITY` = rayons droits. Voir [`Refraction`].
    #[serde(default)]
    pub rc: Option<f64>,
    /// Réfraction calée : l'appelant qui connaît l'azimut du trajet en déduit
    /// `rc` (voir [`PathConfig::for_azimuth`]).
    #[serde(default)]
    pub refraction: Option<Refraction>,
    /// Sol enneigé : classe A (neige fraîche) partout.
    #[serde(default)]
    pub snow: bool,
}

impl PathConfig {
    /// Configuration pour un trajet d'azimut `az_deg` : `rc` déduit de la
    /// réfraction calée si elle existe.
    pub fn for_azimuth(&self, az_deg: f64) -> PathConfig {
        match self.refraction {
            Some(r) => PathConfig {
                rc: Some(r.radius(az_deg, self.c)),
                ..*self
            },
            None => *self,
        }
    }
}

fn one() -> f64 {
    1.0
}

/// Réfraction explicite, issue d'un calage sur mesures : gradient vertical de
/// célérité effective g(az) = g_T + g_W·cos(az − az_vent) (s⁻¹), avec az_vent
/// la direction vers laquelle souffle le vent. g > 0 : rayons courbés vers le
/// bas, de rayon Rc = c/g. g ≤ 0 : rayons droits (la méthode d'ingénierie ne
/// modélise pas les zones d'ombre).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Refraction {
    /// Part thermique (stabilité) du gradient (s⁻¹).
    pub grad_t: f64,
    /// Part due au vent, maximale sous le vent (s⁻¹).
    pub grad_w: f64,
    /// Direction vers laquelle souffle le vent (°, depuis le nord, sens horaire).
    pub wind_toward_deg: f64,
}

impl Refraction {
    pub fn gradient(&self, az_deg: f64) -> f64 {
        self.grad_t + self.grad_w * (az_deg - self.wind_toward_deg).to_radians().cos()
    }

    /// Rayon de courbure (m) dans l'azimut `az_deg` ; ∞ si g ≤ 0. Borné à
    /// 200 m (au-delà, l'approximation par abaissement du relief ne tient plus).
    pub fn radius(&self, az_deg: f64, c: f64) -> f64 {
        let g = self.gradient(az_deg);
        if g > 1e-6 {
            (c / g).max(200.0)
        } else {
            f64::INFINITY
        }
    }
}

impl Default for PathConfig {
    fn default() -> Self {
        Self {
            condition: Condition::Favorable,
            model: ImpedanceModel::DelanyBazley,
            c: 343.0,
            n_sub: 3,
            canopy_min: 3.0,
            sigma_scale: 1.0,
            rc: None,
            refraction: None,
            snow: false,
        }
    }
}

/// Informations sur le trajet (pour la coupe et le diagnostic).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PathInfo {
    pub distance: f64,
    pub delta: f64,
    pub n_edges: usize,
    pub foliage_m: f64,
    pub sigma_kpa: f64,
    /// Hauteurs de la source et du récepteur au-dessus du plan moyen.
    pub hs_plane: f64,
    pub hr_plane: f64,
}

/// Tampons réutilisables (aucune allocation dans la boucle chaude).
#[derive(Debug, Default)]
pub struct Scratch {
    hull: Vec<(f64, f64)>,
}

/// Calcule l'atténuation en excès par bande (dB, positive = gain, négative =
/// atténuation) : sol + diffraction + feuillage. La divergence et
/// l'absorption atmosphérique sont traitées à part.
pub fn path_excess(
    samples: &[PathSample],
    hs: f64,
    hr: f64,
    bands: &[BandSpec],
    cfg: &PathConfig,
    scratch: &mut Scratch,
    out: &mut [f64],
) -> PathInfo {
    let n = samples.len();
    if n < 2 {
        out.iter_mut().for_each(|v| *v = 0.0);
        return PathInfo::default();
    }
    let x0 = samples[0].x;
    let d = (samples[n - 1].x - x0).max(0.1);
    let rc = match (cfg.rc, cfg.condition) {
        (Some(rc), _) => rc,
        (None, Condition::Homogeneous) => f64::INFINITY,
        (None, Condition::Favorable) => (8.0 * d).max(1000.0),
    };
    let corr = |x: f64| {
        let u = x - x0;
        if rc.is_finite() {
            u * (d - u) / (2.0 * rc)
        } else {
            0.0
        }
    };
    let zs = f64::from(samples[0].ground) + hs;
    let zr = f64::from(samples[n - 1].ground) + hr;
    let s = (x0, zs);
    let r = (x0 + d, zr);

    // 1. Diffraction : obstacles = sol + bâtiments (la végétation ne fait pas écran).
    let path: DiffractionPath = rubber_band(
        s,
        r,
        samples[1..n - 1]
            .iter()
            .map(|p| (p.x, f64::from(p.ground) + f64::from(p.obstacle) - corr(p.x))),
        &mut scratch.hull,
    );

    // 2. Plan moyen du sol (moindres carrés) et σ effectif.
    let (mut sx, mut sz, mut sxx, mut sxz) = (0.0, 0.0, 0.0, 0.0);
    let (lo_i, hi_i) = (n / 10, (n * 9 / 10).max(n / 10 + 1).min(n));
    let mut log_sigma = 0.0;
    for (i, p) in samples.iter().enumerate() {
        let z = f64::from(p.ground) - corr(p.x);
        sx += p.x;
        sz += z;
        sxx += p.x * p.x;
        sxz += p.x * z;
        if i >= lo_i && i < hi_i {
            log_sigma += GroundClass::from_index(p.class).sigma_kpa().ln();
        }
    }
    let nf = n as f64;
    let var = sxx - sx * sx / nf;
    let b = if var > 1e-9 {
        (sxz - sx * sz / nf) / var
    } else {
        0.0
    };
    let a = (sz - b * sx) / nf;
    let norm = (1.0 + b * b).sqrt();
    let height_above = |x: f64, z: f64| ((z - (a + b * x)) / norm).max(0.01);
    let along = |x: f64, z: f64| (x + b * (z - a)) / norm;
    let hs_p = height_above(s.0, s.1);
    let hr_p = height_above(r.0, r.1);
    let dp = (along(r.0, r.1) - along(s.0, s.1)).abs().max(0.1);
    let sigma = if cfg.snow {
        GroundClass::A.sigma_kpa()
    } else {
        (log_sigma / (hi_i - lo_i).max(1) as f64).exp() * cfg.sigma_scale
    };

    // 3. Feuillage traversé par le rayon (droit en coordonnées corrigées).
    let mut foliage_m = 0.0;
    for w in samples.windows(2) {
        let p = w[1];
        if p.canopy < cfg.canopy_min {
            continue;
        }
        let line = zs + (zr - zs) * (p.x - x0) / d;
        let top = f64::from(p.ground) + f64::from(p.canopy) - corr(p.x);
        if line < top {
            foliage_m += (p.x - w[0].x) * f64::from(p.foliage);
        }
    }

    // 4. Vue obstruée : différences de marche des chemins issus de la source
    //    image et du récepteur image (miroirs par rapport au plan moyen), pour
    //    combiner sol et diffraction selon CNOSSOS-EU (Δground(S,O), Δground(O,R)).
    let dist = |p: (f64, f64), q: (f64, f64)| (q.0 - p.0).hypot(q.1 - p.1);
    let mirror = |p: (f64, f64)| {
        let h = (p.1 - (a + b * p.0)) / norm;
        (p.0 + 2.0 * h * b / norm, p.1 - 2.0 * h / norm)
    };
    let (e1, ek) = (path.first_edge.unwrap_or(s), path.last_edge.unwrap_or(r));
    let (delta_simg, delta_rimg) = if path.n_edges > 0 {
        let inner = path.delta + dist(s, r) - dist(s, e1) - dist(ek, r);
        let (si, ri) = (mirror(s), mirror(r));
        (
            dist(si, e1) + inner + dist(ek, r) - dist(si, r),
            dist(s, e1) + inner + dist(ek, ri) - dist(s, ri),
        )
    } else {
        (0.0, 0.0)
    };

    // 5. Bandes.
    for (o, band) in out.iter_mut().zip(bands) {
        let lambda = cfg.c / band.fc;
        let ground = |h1: f64, h2: f64, dist: f64| {
            TwoRay {
                hs: h1,
                hr: h2,
                d: dist.max(0.1),
            }
            .excess_band_db(band.lo, band.hi, cfg.n_sub, cfg.c, cfg.model, sigma)
        };
        let mut ex = if path.n_edges == 0 {
            ground(hs_p, hr_p, dp)
        } else {
            let dif = delta_dif_db(path.delta, lambda, path.e);
            let g1 = ground(
                hs_p,
                height_above(e1.0, e1.1),
                along(e1.0, e1.1) - along(s.0, s.1),
            )
            .clamp(-15.0, 6.0);
            let g2 = ground(
                height_above(ek.0, ek.1),
                hr_p,
                along(r.0, r.1) - along(ek.0, ek.1),
            )
            .clamp(-15.0, 6.0);
            // Δground = 20·log10(1 + (10^(g/20) − 1)·10^(−(Δdif(image) − Δdif)/20)),
            // g = gain de sol (dB, > 0 = renforcement) du côté considéré.
            let weighted = |g: f64, dif_img: f64| {
                let w = 10f64.powf(-(dif_img - dif) / 20.0);
                20.0 * (1.0 + (10f64.powf(g / 20.0) - 1.0) * w).max(1e-3).log10()
            };
            -dif + weighted(g1, delta_dif_db(delta_simg, lambda, path.e))
                + weighted(g2, delta_dif_db(delta_rimg, lambda, path.e))
        };
        ex -= foliage_db(band.fc, foliage_m);
        *o = ex;
    }
    PathInfo {
        distance: d,
        delta: path.delta,
        n_edges: path.n_edges,
        foliage_m,
        sigma_kpa: sigma,
        hs_plane: hs_p,
        hr_plane: hr_p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bands::BandSet;

    fn flat(d: f64, step: f64, class: GroundClass) -> Vec<PathSample> {
        (0..=(d / step) as usize)
            .map(|i| PathSample {
                x: i as f64 * step,
                ground: 100.0,
                obstacle: 0.0,
                canopy: 0.0,
                class: class.index(),
                foliage: 1.0,
            })
            .collect()
    }

    fn octaves() -> Vec<BandSpec> {
        BandSet::Octave
            .bands()
            .into_iter()
            .map(BandSpec::from_band)
            .collect()
    }

    #[test]
    fn imposed_radius_matches_favorable_condition() {
        let p = flat(800.0, 5.0, GroundClass::D);
        let bands = octaves();
        let run = |cfg: PathConfig| {
            let mut out = vec![0.0; bands.len()];
            path_excess(
                &p,
                2.0,
                4.0,
                &bands,
                &cfg,
                &mut Scratch::default(),
                &mut out,
            );
            out
        };
        let fav = run(PathConfig::default());
        let imposed = run(PathConfig {
            condition: Condition::Homogeneous,
            rc: Some(8.0 * 800.0),
            ..PathConfig::default()
        });
        assert_eq!(fav, imposed);
        let straight = run(PathConfig {
            rc: Some(f64::INFINITY),
            ..PathConfig::default()
        });
        let hom = run(PathConfig {
            condition: Condition::Homogeneous,
            ..PathConfig::default()
        });
        assert_eq!(straight, hom);
    }

    #[test]
    fn refraction_radius_follows_wind() {
        let r = Refraction {
            grad_t: 0.0,
            grad_w: 0.1,
            wind_toward_deg: 90.0,
        };
        assert!((r.radius(90.0, 340.0) - 3400.0).abs() < 1e-6);
        assert!(r.radius(270.0, 340.0).is_infinite());
        assert!(r.radius(0.0, 340.0).is_infinite());
    }

    #[test]
    fn sigma_scale_hardens_ground() {
        let p = flat(300.0, 5.0, GroundClass::D);
        let bands = octaves();
        let run = |scale: f64| {
            let mut out = vec![0.0; bands.len()];
            let cfg = PathConfig {
                condition: Condition::Homogeneous,
                sigma_scale: scale,
                ..PathConfig::default()
            };
            path_excess(
                &p,
                2.0,
                1.5,
                &bands,
                &cfg,
                &mut Scratch::default(),
                &mut out,
            );
            out
        };
        // Sol plus dur → creux de sol moins profond à 500 Hz (bande 5).
        assert!(run(100.0)[5] > run(1.0)[5] + 3.0);
    }

    #[test]
    fn flat_ground_matches_two_ray() {
        let p = flat(200.0, 5.0, GroundClass::D);
        let cfg = PathConfig {
            condition: Condition::Homogeneous,
            n_sub: 1,
            ..PathConfig::default()
        };
        let bands = octaves();
        let mut out = vec![0.0; bands.len()];
        let info = path_excess(
            &p,
            1.0,
            1.5,
            &bands,
            &cfg,
            &mut Scratch::default(),
            &mut out,
        );
        assert_eq!(info.n_edges, 0);
        for (b, got) in bands.iter().zip(&out) {
            let exp = TwoRay {
                hs: 1.0,
                hr: 1.5,
                d: 200.0,
            }
            .excess_band_db(b.lo, b.hi, 1, 343.0, ImpedanceModel::DelanyBazley, 200.0);
            assert!((got - exp).abs() < 0.05, "{} Hz : {got} vs {exp}", b.fc);
        }
    }

    #[test]
    fn hill_attenuates_high_frequencies_more() {
        let mut p = flat(400.0, 5.0, GroundClass::D);
        for s in p.iter_mut() {
            // Colline gaussienne de 15 m au milieu.
            s.ground += (15.0 * (-((s.x - 200.0) / 40.0).powi(2)).exp()) as f32;
        }
        let cfg = PathConfig {
            condition: Condition::Homogeneous,
            ..PathConfig::default()
        };
        let bands = octaves();
        let mut out = vec![0.0; bands.len()];
        let info = path_excess(
            &p,
            1.0,
            1.5,
            &bands,
            &cfg,
            &mut Scratch::default(),
            &mut out,
        );
        assert!(info.n_edges >= 1 && info.delta > 0.0);
        assert!(
            out[8] < out[1] - 10.0,
            "8 kHz {} vs 63 Hz {}",
            out[8],
            out[1]
        );
    }

    #[test]
    fn favorable_condition_reduces_screening() {
        let mut p = flat(1000.0, 5.0, GroundClass::D);
        for s in p.iter_mut() {
            s.ground += (8.0 * (-((s.x - 500.0) / 60.0).powi(2)).exp()) as f32;
        }
        let bands = octaves();
        let mut hom = vec![0.0; bands.len()];
        let mut fav = vec![0.0; bands.len()];
        let mut sc = Scratch::default();
        let base = PathConfig::default();
        path_excess(
            &p,
            1.0,
            1.5,
            &bands,
            &PathConfig {
                condition: Condition::Homogeneous,
                ..base
            },
            &mut sc,
            &mut hom,
        );
        path_excess(
            &p,
            1.0,
            1.5,
            &bands,
            &PathConfig {
                condition: Condition::Favorable,
                ..base
            },
            &mut sc,
            &mut fav,
        );
        assert!(
            fav[5] > hom[5] + 2.0,
            "1 kHz : fav {} hom {}",
            fav[5],
            hom[5]
        );
    }

    #[test]
    fn building_screen_and_forest() {
        let mut p = flat(300.0, 2.0, GroundClass::D);
        for s in p.iter_mut() {
            if (148.0..=152.0).contains(&s.x) {
                s.obstacle = 8.0;
            }
        }
        let cfg = PathConfig {
            condition: Condition::Homogeneous,
            ..PathConfig::default()
        };
        let bands = octaves();
        let mut screened = vec![0.0; bands.len()];
        let info = path_excess(
            &p,
            1.0,
            1.5,
            &bands,
            &cfg,
            &mut Scratch::default(),
            &mut screened,
        );
        // Bâtiment de 4 m d'épaisseur : écran épais à deux arêtes.
        assert_eq!(info.n_edges, 2);
        assert!(
            screened[6] < -12.0,
            "2 kHz derrière un bâtiment : {}",
            screened[6]
        );

        let mut f = flat(300.0, 2.0, GroundClass::C);
        for s in f.iter_mut() {
            if (50.0..250.0).contains(&s.x) {
                s.canopy = 15.0;
            }
        }
        let mut forest = vec![0.0; bands.len()];
        let info = path_excess(
            &f,
            1.0,
            1.5,
            &bands,
            &cfg,
            &mut Scratch::default(),
            &mut forest,
        );
        assert!((info.foliage_m - 200.0).abs() < 4.0);
        let mut open = vec![0.0; bands.len()];
        path_excess(
            &flat(300.0, 2.0, GroundClass::C),
            1.0,
            1.5,
            &bands,
            &cfg,
            &mut Scratch::default(),
            &mut open,
        );
        assert!((open[8] - forest[8] - 24.0).abs() < 0.5, "8 kHz forêt");
        assert!((open[1] - forest[1] - 4.0).abs() < 0.5, "63 Hz forêt");
    }
}
