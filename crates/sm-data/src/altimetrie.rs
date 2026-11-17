//! API Altimétrie Géoplateforme : profils ponctuels rapides (prototypage).
//! Pour le calcul, préférer le WMS-R float32 ([`crate::wmsr`]).

use crate::http::{GeoClient, Service};
use crate::{DataError, Result};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

pub const RES_RGE_ALTI: &str = "ign_rge_alti_wld";
pub const RES_LIDAR_MNT: &str = "ign_lidar_hd_mnt_mono_wld";

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct ElevationPoint {
    pub lon: f64,
    pub lat: f64,
    pub z: f64,
}

#[derive(Deserialize)]
struct Resp {
    elevations: Vec<ElevationPoint>,
}

/// Profil entre deux points (lon, lat) échantillonné en `sampling` points.
pub fn line_url(
    base: &str,
    a: (f64, f64),
    b: (f64, f64),
    resource: &str,
    sampling: usize,
) -> String {
    format!(
        "{base}/altimetrie/1.0/calcul/alti/rest/elevationLine.json?lon={}%7C{}&lat={}%7C{}\
         &resource={resource}&sampling={sampling}",
        a.0, b.0, a.1, b.1
    )
}

pub fn parse(bytes: &[u8]) -> Result<Vec<ElevationPoint>> {
    let r: Resp =
        serde_json::from_slice(bytes).map_err(|e| DataError::Parse(format!("altimétrie : {e}")))?;
    Ok(r.elevations)
}

pub async fn elevation_line(
    client: &GeoClient,
    a: (f64, f64),
    b: (f64, f64),
    resource: &str,
    sampling: usize,
    cancel: &CancellationToken,
) -> Result<Vec<ElevationPoint>> {
    let url = line_url(&client.endpoints().geopf, a, b, resource, sampling);
    parse(&client.get(Service::Altimetrie, &url, cancel).await?.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_reference_response() {
        let r = parse(br#"{"elevations": [{"lon": 3.09, "lat": 44.1, "z": 354.63, "acc": "x"}, {"lon": 3.1, "lat": 44.11, "z": 763.11, "acc": "x"}], "height_differences": {"positive": 408.48, "negative": 0}}"#).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].z, 763.11);
    }

    #[tokio::test]
    #[ignore = "réseau : API altimétrie"]
    async fn live_profile() {
        let c = GeoClient::public().unwrap();
        let r = elevation_line(
            &c,
            (3.09, 44.10),
            (3.10, 44.11),
            RES_RGE_ALTI,
            3,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.len(), 3);
        assert!(r[0].z > 300.0);
    }
}
