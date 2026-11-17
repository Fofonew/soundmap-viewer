//! `sm-compute` : orchestration des calculs.
//!
//! - [`scene`] : scène chargée depuis un pack de zone (lecture locale) ;
//! - [`polar`] : grille polaire, méthode d'ingénierie, caches incrémentaux ;
//! - [`pe`] : basses fréquences par GFPE, fusion, enveloppe de scénarios ;
//! - [`impact`], [`sizing`], [`optimize`] : nuisances, dimensionnement, optimiseur ;
//! - [`render`], [`export`] : cartes, isophones, GeoTIFF, GeoJSON, rapport HTML.
//!
//! Les calculs longs sont **annulables** et publient leur **progression**.

pub mod auralize;
pub mod calibrate;
pub mod export;
pub mod ground_state;
pub mod impact;
pub mod meteo;
pub mod mvt;
pub mod optimize;
pub mod pe;
pub mod polar;
pub mod profile;
pub mod project;
pub mod receiver;
pub mod render;
pub mod scene;
pub mod sizing;
pub mod spots;
pub mod valley;
pub mod vegetation;

#[derive(Debug, thiserror::Error)]
pub enum ComputeError {
    #[error("données : {0}")]
    Data(#[from] sm_data::DataError),
    #[error("géo : {0}")]
    Geo(#[from] sm_geo::GeoError),
    #[error("E/S : {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("calcul annulé")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, ComputeError>;

use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Jeton d'annulation partagé entre l'UI et le calcul.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Instantané de progression d'un job.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

impl Progress {
    pub fn fraction(self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.done as f64 / self.total as f64
        }
    }
}

/// Compteur de progression partagé (lock-free).
#[derive(Debug, Clone, Default)]
pub struct ProgressCounter {
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

impl ProgressCounter {
    pub fn new(total: u64) -> Self {
        let c = Self::default();
        c.total.store(total, Ordering::Relaxed);
        c
    }
    pub fn inc(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
    }
    pub fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }
    pub fn snapshot(&self) -> Progress {
        Progress {
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_flag_is_shared() {
        let a = CancelFlag::new();
        let b = a.clone();
        b.cancel();
        assert!(a.is_cancelled());
    }

    #[test]
    fn progress_fraction() {
        let p = ProgressCounter::new(4);
        p.inc(1);
        assert_eq!(p.snapshot().fraction(), 0.25);
        assert_eq!(ProgressCounter::new(0).snapshot().fraction(), 1.0);
    }
}
