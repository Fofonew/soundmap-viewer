//! Atténuation par le feuillage dense (ISO 9613-2:1996, annexe A, tableau A.1).
//!
//! Pour une longueur traversée df : 10 ≤ df < 20 m → valeur forfaitaire ;
//! 20 ≤ df ≤ 200 m → dB/m × df. Au-delà de 200 m, on plafonne à 200 m.
//! Les basses fréquences (< 125 Hz) ne sont quasiment pas atténuées.

/// Octaves 63 Hz … 8 kHz : (forfait 10–20 m en dB, dB/m pour 20–200 m).
const TABLE: [(f64, f64, f64); 8] = [
    (63.0, 0.0, 0.02),
    (125.0, 0.0, 0.03),
    (250.0, 1.0, 0.04),
    (500.0, 1.0, 0.05),
    (1000.0, 1.0, 0.06),
    (2000.0, 1.0, 0.08),
    (4000.0, 2.0, 0.09),
    (8000.0, 3.0, 0.12),
];

fn coeffs(f: f64) -> (f64, f64) {
    if f <= TABLE[0].0 {
        // En dessous de 63 Hz : extrapolation prudente à 0.
        let t = (f / TABLE[0].0).clamp(0.0, 1.0);
        return (0.0, TABLE[0].2 * t);
    }
    for w in TABLE.windows(2) {
        let (a, b) = (w[0], w[1]);
        if f <= b.0 {
            let t = (f / a.0).log2() / (b.0 / a.0).log2();
            return (a.1 + t * (b.1 - a.1), a.2 + t * (b.2 - a.2));
        }
    }
    (TABLE[7].1, TABLE[7].2)
}

/// Atténuation (dB, positive) pour `df` mètres de feuillage traversés.
pub fn foliage_db(f: f64, df: f64) -> f64 {
    let (flat, per_m) = coeffs(f);
    if df < 10.0 {
        0.0
    } else if df < 20.0 {
        flat
    } else {
        per_m * df.min(200.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_values() {
        assert_eq!(foliage_db(1000.0, 5.0), 0.0);
        assert_eq!(foliage_db(1000.0, 15.0), 1.0);
        assert!((foliage_db(1000.0, 100.0) - 6.0).abs() < 1e-9);
        assert!((foliage_db(8000.0, 500.0) - 24.0).abs() < 1e-9);
        assert!(foliage_db(31.5, 100.0) < foliage_db(63.0, 100.0));
    }
}
