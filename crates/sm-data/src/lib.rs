//! `sm-data` : accès aux données publiques et packs de zone locaux.
//!
//! Tous les clients passent par [`http::GeoClient`] : User-Agent générique,
//! limitation de débit par service, retries avec backoff, annulation.
//! Les endpoints sont décrits (et ont été vérifiés) dans `docs/ENDPOINTS.md`.

pub mod altimetrie;
pub mod arome_grib;
pub mod brgm;
pub mod copc;
pub mod geocode;
pub mod http;
pub mod lidar_index;
pub mod openmeteo;
pub mod pack;
pub mod tile_cache;
pub mod tms;
pub mod wfs;
pub mod wmsr;
pub mod wmts;

pub use bytes::Bytes;
pub use http::{Endpoints, GeoClient, HttpConfig, Service};
pub use sm_geo::BboxL93;
pub use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("HTTP : {0}")]
    Http(#[from] reqwest::Error),
    #[error("HTTP {status} sur {url} : {body}")]
    Status {
        status: u16,
        url: String,
        body: String,
    },
    #[error("opération annulée")]
    Cancelled,
    #[error("E/S : {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON : {0}")]
    Json(#[from] serde_json::Error),
    #[error("réponse inattendue : {0}")]
    Parse(String),
    #[error("requête invalide : {0}")]
    Invalid(String),
    #[error("géo : {0}")]
    Geo(#[from] sm_geo::GeoError),
}

impl DataError {
    /// Vrai pour une réponse 404 (tuile absente, par exemple).
    pub fn is_not_found(&self) -> bool {
        matches!(self, DataError::Status { status: 404, .. })
    }
}

pub type Result<T> = std::result::Result<T, DataError>;
