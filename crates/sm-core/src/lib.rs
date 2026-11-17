//! `sm-core` : physique pure de SoundMap, sans aucune I/O.
//!
//! Ce crate doit rester compilable en `wasm32-unknown-unknown`.
//! Les équations sont documentées dans `docs/PHYSIQUE.md`.

pub mod atmosphere;
pub mod bands;
pub mod diffraction;
pub mod engineering;
pub mod faddeeva;
pub mod foliage;
pub mod gfpe;
pub mod ground;
pub mod source;
pub mod units;
pub mod weighting;

pub use bands::{Band, BandSet};
pub use num_complex::Complex64 as C64;
pub use units::{Db, Hz, Meters};
pub use weighting::{a_weighting, c_weighting};
