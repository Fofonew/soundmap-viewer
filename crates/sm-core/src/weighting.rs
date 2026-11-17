//! Pondérations fréquentielles A et C (IEC 61672-1:2013, annexe E).

use crate::units::{Db, Hz};

const F1: f64 = 20.598_997;
const F2: f64 = 107.652_65;
const F3: f64 = 737.862_23;
const F4: f64 = 12_194.217;
/// Constantes de normalisation : A(1 kHz) = C(1 kHz) = 0 dB.
const A1000: f64 = -2.000;
const C1000: f64 = -0.062;

/// Pondération A en dB à la fréquence `f`.
pub fn a_weighting(f: Hz) -> Db {
    let f2 = f.0 * f.0;
    let num = F4 * F4 * f2 * f2;
    let den = (f2 + F1 * F1) * ((f2 + F2 * F2) * (f2 + F3 * F3)).sqrt() * (f2 + F4 * F4);
    Db(20.0 * (num / den).log10() - A1000)
}

/// Pondération C en dB à la fréquence `f`.
pub fn c_weighting(f: Hz) -> Db {
    let f2 = f.0 * f.0;
    let num = F4 * F4 * f2;
    let den = (f2 + F1 * F1) * (f2 + F4 * F4);
    Db(20.0 * (num / den).log10() - C1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bands::BandSet;

    /// Valeurs tabulées IEC 61672-1 (tableau 3), tiers d'octave 25 Hz – 8 kHz.
    const A_TABLE: [f64; 26] = [
        -44.7, -39.4, -34.6, -30.2, -26.2, -22.5, -19.1, -16.1, -13.4, -10.9, -8.6, -6.6, -4.8,
        -3.2, -1.9, -0.8, 0.0, 0.6, 1.0, 1.2, 1.3, 1.2, 1.0, 0.5, -0.1, -1.1,
    ];
    const C_TABLE: [f64; 26] = [
        -4.4, -3.0, -2.0, -1.3, -0.8, -0.5, -0.3, -0.2, -0.1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, -0.1, -0.2, -0.3, -0.5, -0.8, -1.3, -2.0, -3.0,
    ];

    #[test]
    fn a_weighting_matches_iec_table() {
        for (b, expected) in BandSet::ThirdOctave.bands().iter().zip(A_TABLE) {
            let got = a_weighting(b.center()).0;
            assert!(
                (got - expected).abs() <= 0.051,
                "{} Hz : {got:.2} ≠ {expected}",
                b.nominal().0
            );
        }
    }

    #[test]
    fn c_weighting_matches_iec_table() {
        for (b, expected) in BandSet::ThirdOctave.bands().iter().zip(C_TABLE) {
            let got = c_weighting(b.center()).0;
            assert!(
                (got - expected).abs() <= 0.051,
                "{} Hz : {got:.2} ≠ {expected}",
                b.nominal().0
            );
        }
    }

    #[test]
    fn zero_at_1khz() {
        assert!(a_weighting(Hz(1000.0)).0.abs() < 0.01);
        assert!(c_weighting(Hz(1000.0)).0.abs() < 0.01);
    }
}
