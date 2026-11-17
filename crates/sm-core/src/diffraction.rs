//! Diffraction par le relief et le bâti : chemin « élastique » (enveloppe
//! convexe supérieure du profil entre la source et le récepteur) et
//! atténuation CNOSSOS-EU (directive 2015/996, §2.5.6), qui généralise la
//! formule de Maekawa (1968) à plusieurs arêtes.

/// Résultat du calcul de chemin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiffractionPath {
    /// Différence de marche δ (m) : > 0 si la vue directe est obstruée,
    /// < 0 (marge de dégagement) sinon.
    pub delta: f64,
    /// Distance entre la première et la dernière arête (m), 0 si une seule.
    pub e: f64,
    /// Première et dernière arête (x, z) ; `None` si la vue est dégagée.
    pub first_edge: Option<(f64, f64)>,
    pub last_edge: Option<(f64, f64)>,
    pub n_edges: usize,
}

#[inline]
fn cross(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

#[inline]
fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// Chemin de diffraction de `s` à `r` au-dessus des obstacles `pts`
/// (abscisses croissantes, strictement entre s et r). `hull` est un tampon
/// réutilisé pour éviter les allocations.
pub fn rubber_band(
    s: (f64, f64),
    r: (f64, f64),
    pts: impl Iterator<Item = (f64, f64)>,
    hull: &mut Vec<(f64, f64)>,
) -> DiffractionPath {
    hull.clear();
    hull.push(s);
    // Point le plus proche de la ligne de visée (pour δ < 0).
    let (mut best_clear, mut best_pt) = (f64::MAX, None);
    let dx = r.0 - s.0;
    for p in pts.chain(std::iter::once(r)) {
        if p.0 > s.0 && p.0 < r.0 && dx > 0.0 {
            let line = s.1 + (r.1 - s.1) * (p.0 - s.0) / dx;
            let clear = line - p.1;
            if clear < best_clear {
                best_clear = clear;
                best_pt = Some(p);
            }
        }
        // Enveloppe supérieure (chaîne monotone d'Andrew).
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) >= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    let direct = dist(s, r);
    if hull.len() <= 2 {
        // Vue dégagée : marge négative par le point le plus critique.
        let delta = match best_pt {
            Some(o) => direct - (dist(s, o) + dist(o, r)),
            None => f64::NEG_INFINITY,
        };
        return DiffractionPath {
            delta,
            e: 0.0,
            first_edge: None,
            last_edge: None,
            n_edges: 0,
        };
    }
    let path: f64 = hull.windows(2).map(|w| dist(w[0], w[1])).sum();
    let first = hull[1];
    let last = hull[hull.len() - 2];
    let e: f64 = hull[1..hull.len() - 1]
        .windows(2)
        .map(|w| dist(w[0], w[1]))
        .sum();
    DiffractionPath {
        delta: path - direct,
        e,
        first_edge: Some(first),
        last_edge: Some(last),
        n_edges: hull.len() - 2,
    }
}

/// Atténuation par diffraction (dB, positive) selon CNOSSOS-EU :
/// Δdif = 10·Ch·log10(3 + 40/λ·C''·δ) si 40δ/λ·C'' ≥ −2, sinon 0, avec
/// C'' = (1 + (5λ/e)²)/(1/3 + (5λ/e)²) pour plusieurs arêtes, plafonnée à 25 dB.
pub fn delta_dif_db(delta: f64, lambda: f64, e: f64) -> f64 {
    let c2 = if e > 0.0 {
        let q = (5.0 * lambda / e).powi(2);
        (1.0 + q) / (1.0 / 3.0 + q)
    } else {
        1.0
    };
    let arg = 40.0 / lambda * c2 * delta;
    if arg < -2.0 {
        0.0
    } else {
        (10.0 * (3.0 + arg).log10()).clamp(0.0, 25.0)
    }
}

/// Formule de Maekawa d'origine (écran mince), pour la validation :
/// A = 10·log10(3 + 20N), N = 2δ/λ (nombre de Fresnel).
pub fn maekawa_db(fresnel_n: f64) -> f64 {
    10.0 * (3.0 + 20.0 * fresnel_n).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maekawa_reference_points() {
        // N = 0 → 4,8 dB (rasant) ; N = 1 → 13,6 dB ; N = 10 → 23,1 dB.
        assert!((maekawa_db(0.0) - 4.77).abs() < 0.01);
        assert!((maekawa_db(1.0) - 13.62).abs() < 0.01);
        assert!((maekawa_db(10.0) - 23.08).abs() < 0.01);
    }

    #[test]
    fn single_edge_equals_maekawa() {
        let lambda = 343.0 / 500.0;
        for n in [0.1, 1.0, 5.0] {
            let delta = n * lambda / 2.0;
            assert!((delta_dif_db(delta, lambda, 0.0) - maekawa_db(n)).abs() < 1e-9);
        }
    }

    #[test]
    fn thin_screen_geometry() {
        // Source et récepteur à 1 m, écran de 5 m au milieu, 20 m de distance.
        let mut h = Vec::new();
        let p = rubber_band((0.0, 1.0), (20.0, 1.0), [(10.0, 5.0)].into_iter(), &mut h);
        let expected = 2.0 * (10f64.hypot(4.0)) - 20.0;
        assert!((p.delta - expected).abs() < 1e-9);
        assert_eq!(p.n_edges, 1);
        assert_eq!(p.first_edge, Some((10.0, 5.0)));
    }

    #[test]
    fn clear_line_of_sight_gives_negative_delta() {
        let mut h = Vec::new();
        let p = rubber_band(
            (0.0, 10.0),
            (100.0, 10.0),
            [(30.0, 2.0), (50.0, 8.0), (70.0, 1.0)].into_iter(),
            &mut h,
        );
        assert!(p.delta < 0.0 && p.n_edges == 0);
        // Le point critique est la crête à 8 m (dégagement 2 m).
        let expected = 100.0 - (50f64.hypot(2.0) * 2.0);
        assert!((p.delta - expected).abs() < 1e-9);
        assert_eq!(delta_dif_db(p.delta, 0.1, 0.0), 0.0);
    }

    #[test]
    fn double_edge_more_attenuation_than_single() {
        let mut h = Vec::new();
        let one = rubber_band((0.0, 1.0), (100.0, 1.0), [(50.0, 10.0)].into_iter(), &mut h);
        let two = rubber_band(
            (0.0, 1.0),
            (100.0, 1.0),
            [(40.0, 10.0), (60.0, 10.0)].into_iter(),
            &mut h,
        );
        assert_eq!(two.n_edges, 2);
        assert!((two.e - 20.0).abs() < 1e-9);
        let l = 343.0 / 1000.0;
        assert!(delta_dif_db(two.delta, l, two.e) > delta_dif_db(one.delta, l, 0.0));
        assert!(delta_dif_db(two.delta, l, two.e) <= 25.0);
    }
}
