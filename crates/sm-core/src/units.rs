//! Newtypes d'unités physiques : les unités sont explicites dans les types.

use core::fmt;
use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};
use serde::{Deserialize, Serialize};

macro_rules! unit {
    ($(#[$m:meta])* $name:ident, $suffix:literal) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub f64);

        impl $name {
            #[inline]
            pub const fn new(v: f64) -> Self {
                Self(v)
            }
            #[inline]
            pub const fn value(self) -> f64 {
                self.0
            }
        }
        impl Add for $name {
            type Output = Self;
            #[inline]
            fn add(self, o: Self) -> Self {
                Self(self.0 + o.0)
            }
        }
        impl Sub for $name {
            type Output = Self;
            #[inline]
            fn sub(self, o: Self) -> Self {
                Self(self.0 - o.0)
            }
        }
        impl AddAssign for $name {
            #[inline]
            fn add_assign(&mut self, o: Self) {
                self.0 += o.0
            }
        }
        impl SubAssign for $name {
            #[inline]
            fn sub_assign(&mut self, o: Self) {
                self.0 -= o.0
            }
        }
        impl Neg for $name {
            type Output = Self;
            #[inline]
            fn neg(self) -> Self {
                Self(-self.0)
            }
        }
        impl Mul<f64> for $name {
            type Output = Self;
            #[inline]
            fn mul(self, k: f64) -> Self {
                Self(self.0 * k)
            }
        }
        impl Div<f64> for $name {
            type Output = Self;
            #[inline]
            fn div(self, k: f64) -> Self {
                Self(self.0 / k)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match f.precision() {
                    Some(p) => write!(f, "{:.*} {}", p, self.0, $suffix),
                    None => write!(f, "{} {}", self.0, $suffix),
                }
            }
        }
    };
}

unit!(
    /// Niveau ou écart en décibels.
    Db,
    "dB"
);
unit!(
    /// Fréquence en hertz.
    Hz,
    "Hz"
);
unit!(
    /// Longueur en mètres.
    Meters,
    "m"
);

impl Db {
    /// Somme énergétique (incohérente) de niveaux : `10·log10(Σ 10^(Li/10))`.
    pub fn energy_sum<I: IntoIterator<Item = Db>>(levels: I) -> Db {
        let s: f64 = levels.into_iter().map(|l| 10f64.powf(l.0 / 10.0)).sum();
        if s <= 0.0 {
            Db(f64::NEG_INFINITY)
        } else {
            Db(10.0 * s.log10())
        }
    }
}

impl Hz {
    /// Longueur d'onde pour une célérité `c` donnée (m/s).
    pub fn wavelength(self, c: f64) -> Meters {
        Meters(c / self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn energy_sum_two_equal_levels_adds_3db() {
        let s = Db::energy_sum([Db(90.0), Db(90.0)]);
        assert_relative_eq!(s.0, 93.0103, epsilon = 1e-3);
    }

    #[test]
    fn energy_sum_empty_is_minus_infinity() {
        assert!(Db::energy_sum([]).0.is_infinite());
    }

    #[test]
    fn display_with_precision() {
        assert_eq!(format!("{:.1}", Db(3.04159)), "3.0 dB");
        assert_eq!(format!("{}", Meters(2.0)), "2 m");
    }

    #[test]
    fn wavelength_63hz() {
        assert_relative_eq!(Hz(63.0).wavelength(343.0).0, 5.444, epsilon = 1e-3);
    }
}
