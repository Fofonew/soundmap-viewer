//! Fonction de Faddeeva `w(z) = exp(−z²)·erfc(−iz)`.
//!
//! Algorithme W4 de J. Humlíček (1982), « Optimized computation of the Voigt
//! and complex probability functions », JQSRT 27(4) : 4 régions, précision
//! relative ~1e-4, largement suffisante pour l'effet de sol (on travaille au
//! dixième de dB). Pour Im z < 0 : w(z) = 2·exp(−z²) − w(−z).

use num_complex::Complex64 as C64;

/// w(z) pour tout z complexe.
pub fn faddeeva(z: C64) -> C64 {
    if z.im >= 0.0 {
        humlicek_w4(z)
    } else {
        2.0 * (-(z * z)).exp() - humlicek_w4(-z)
    }
}

/// W4 de Humlíček, valable pour Im z ≥ 0.
fn humlicek_w4(z: C64) -> C64 {
    let (x, y) = (z.re, z.im);
    // t = y − i·x = −i·z
    let t = C64::new(y, -x);
    let s = x.abs() + y;
    if s >= 15.0 {
        t * 0.564_189_6 / (0.5 + t * t)
    } else if s >= 5.5 {
        let u = t * t;
        t * (1.410_474 + u * 0.564_189_6) / (0.75 + u * (3.0 + u))
    } else if y >= 0.195 * x.abs() - 0.176 {
        (16.4955 + t * (20.209_33 + t * (11.964_82 + t * (3.778_987 + t * 0.564_223_6))))
            / (16.4955 + t * (38.823_63 + t * (39.271_21 + t * (21.692_74 + t * (6.699_398 + t)))))
    } else {
        let u = t * t;
        let num = t
            * (36_183.31
                - u * (3_321.990_5
                    - u * (1_540.787
                        - u * (219.031_3 - u * (35.766_83 - u * (1.320_522 - u * 0.564_19))))));
        let den = 32_066.6
            - u * (24_322.84
                - u * (9_022.228
                    - u * (2_186.181 - u * (364.219_1 - u * (61.570_37 - u * (1.841_439 - u))))));
        u.exp() - num / den
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: C64, b: C64, tol: f64) -> bool {
        (a - b).norm() <= tol * b.norm().max(1e-3)
    }

    #[test]
    fn reference_values() {
        // Valeurs de référence (Abramowitz & Stegun, tables 7.9 ; Dawson pour l'axe réel).
        let cases = [
            (C64::new(0.0, 0.0), C64::new(1.0, 0.0)),
            (C64::new(0.0, 1.0), C64::new(0.427_583_8, 0.0)), // e·erfc(1)
            (C64::new(1.0, 0.0), C64::new(0.367_879_4, 0.607_157_7)),
            (C64::new(1.0, 1.0), C64::new(0.304_744_2, 0.208_218_1)),
        ];
        for (z, w) in cases {
            let got = faddeeva(z);
            assert!(close(got, w, 2e-3), "w({z}) = {got}, attendu {w}");
        }
    }

    /// Série entière exacte : w(z) = Σ (iz)ⁿ / Γ(n/2 + 1).
    fn series(z: C64) -> C64 {
        let iz = C64::new(0.0, 1.0) * z;
        let mut sum = C64::new(0.0, 0.0);
        let mut pow = C64::new(1.0, 0.0);
        // Γ(n/2+1) par récurrence séparée pour n pair et impair.
        let (mut g_even, mut g_odd) = (1.0f64, std::f64::consts::PI.sqrt() / 2.0);
        for n in 0..120 {
            let g = if n % 2 == 0 { g_even } else { g_odd };
            sum += pow / g;
            pow *= iz;
            if n % 2 == 0 {
                g_even *= f64::from(n / 2 + 1);
            } else {
                g_odd *= f64::from(n / 2) + 1.5;
            }
        }
        sum
    }

    #[test]
    fn matches_power_series_near_origin() {
        // Couvre les régions 3 et 4 de l'algorithme (|x| + y < 5,5).
        for (x, y) in [
            (0.2, 0.05),
            (1.5, 0.1),
            (2.5, 0.02),
            (0.5, 1.5),
            (2.0, 0.8),
            (3.0, 0.5),
            (1.2, -0.3),
        ] {
            let z = C64::new(x, y);
            let s = series(z);
            assert!(
                close(faddeeva(z), s, 3e-3),
                "w({z}) = {} ; série {s}",
                faddeeva(z)
            );
        }
    }

    #[test]
    fn large_argument_asymptote() {
        // w(z) ≈ i/(√π z) pour |z| grand.
        let z = C64::new(30.0, 20.0);
        let asym = C64::new(0.0, 1.0) / (std::f64::consts::PI.sqrt() * z);
        assert!(close(faddeeva(z), asym, 1e-3));
    }

    #[test]
    fn lower_half_plane_symmetry() {
        // w(z̄) = conj(w(−z))
        let z = C64::new(0.7, -0.4);
        let a = faddeeva(z.conj());
        let b = faddeeva(-z).conj();
        assert!(close(a, b, 1e-6));
    }
}
