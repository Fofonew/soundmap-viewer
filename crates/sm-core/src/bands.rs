//! Bandes de fréquence d'octave et de tiers d'octave (IEC 61260-1, base 10).
//!
//! Fréquence médiane exacte : `fm = 1000 · 10^(n/10)`, avec `n` entier pour les
//! tiers d'octave et `n` multiple de 3 pour les octaves.

use crate::units::Hz;
use serde::{Deserialize, Serialize};

/// Une bande : indice `n` (en tiers d'octave, relatif à 1 kHz) et largeur.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub n: i32,
    /// Largeur en tiers d'octave : 1 (tiers) ou 3 (octave).
    pub width_thirds: u8,
}

/// Fréquences nominales en tiers d'octave, de n = -16 (25 Hz) à n = 9 (8 kHz).
const NOMINAL_THIRDS: [f64; 26] = [
    25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0,
    630.0, 800.0, 1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0,
];
const N_MIN: i32 = -16;
const N_MAX: i32 = 9;

impl Band {
    /// Fréquence médiane exacte (base 10).
    pub fn center(self) -> Hz {
        Hz(1000.0 * 10f64.powf(f64::from(self.n) / 10.0))
    }

    /// Fréquence nominale (libellé normalisé).
    pub fn nominal(self) -> Hz {
        usize::try_from(self.n - N_MIN)
            .ok()
            .and_then(|i| NOMINAL_THIRDS.get(i).copied())
            .map_or_else(|| self.center(), Hz)
    }

    /// Bornes inférieure et supérieure exactes.
    pub fn edges(self) -> (Hz, Hz) {
        let half = f64::from(self.width_thirds) / 20.0;
        let fc = self.center().0;
        (Hz(fc * 10f64.powf(-half)), Hz(fc * 10f64.powf(half)))
    }
}

/// Jeu de bandes de calcul.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BandSet {
    /// Octaves de 31,5 Hz à 8 kHz (9 bandes).
    Octave,
    /// Tiers d'octave de 25 Hz à 8 kHz (26 bandes).
    ThirdOctave,
}

impl BandSet {
    pub fn bands(self) -> Vec<Band> {
        match self {
            BandSet::ThirdOctave => (N_MIN..=N_MAX)
                .map(|n| Band { n, width_thirds: 1 })
                .collect(),
            BandSet::Octave => (-15..=N_MAX)
                .step_by(3)
                .map(|n| Band { n, width_thirds: 3 })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn third_octave_count_and_range() {
        let b = BandSet::ThirdOctave.bands();
        assert_eq!(b.len(), 26);
        assert_eq!(b[0].nominal().0, 25.0);
        assert_eq!(b[25].nominal().0, 8000.0);
        assert_relative_eq!(b[0].center().0, 25.1189, epsilon = 1e-3);
    }

    #[test]
    fn octave_nominals() {
        let nom: Vec<f64> = BandSet::Octave
            .bands()
            .iter()
            .map(|b| b.nominal().0)
            .collect();
        assert_eq!(
            nom,
            vec![
                31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0
            ]
        );
    }

    #[test]
    fn octave_edges_ratio_and_geometric_center() {
        let (lo, hi) = Band {
            n: 0,
            width_thirds: 3,
        }
        .edges();
        assert_relative_eq!(hi.0 / lo.0, 10f64.powf(0.3), epsilon = 1e-12);
        assert_relative_eq!((lo.0 * hi.0).sqrt(), 1000.0, epsilon = 1e-9);
    }
}
