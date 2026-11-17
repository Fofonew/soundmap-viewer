//! Géocodage Géoplateforme (`/geocodage/search`), qui renvoie aussi x/y en Lambert-93.

use crate::http::{GeoClient, Service, urlencode};
use crate::{DataError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeocodeResult {
    pub label: String,
    pub context: Option<String>,
    pub kind: Option<String>,
    pub lon: f64,
    pub lat: f64,
    /// Lambert-93 si fourni par le service.
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub score: Option<f64>,
}

pub fn search_url(base: &str, q: &str, limit: usize) -> String {
    format!("{base}/geocodage/search?q={}&limit={limit}", urlencode(q))
}

pub fn parse(bytes: &[u8]) -> Result<Vec<GeocodeResult>> {
    let v: Value = serde_json::from_slice(bytes)?;
    let feats = v
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::Parse("géocodage : pas de `features`".into()))?;
    Ok(feats
        .iter()
        .filter_map(|f| {
            let c = f.pointer("/geometry/coordinates")?.as_array()?;
            let p = f.get("properties")?;
            Some(GeocodeResult {
                label: p.get("label")?.as_str()?.to_owned(),
                context: p.get("context").and_then(Value::as_str).map(str::to_owned),
                kind: p.get("type").and_then(Value::as_str).map(str::to_owned),
                lon: c.first()?.as_f64()?,
                lat: c.get(1)?.as_f64()?,
                x: p.get("x").and_then(Value::as_f64),
                y: p.get("y").and_then(Value::as_f64),
                score: p.get("score").and_then(Value::as_f64),
            })
        })
        .collect())
}

pub async fn search(
    client: &GeoClient,
    q: &str,
    limit: usize,
    cancel: &CancellationToken,
) -> Result<Vec<GeocodeResult>> {
    let url = search_url(&client.endpoints().geopf, q, limit);
    let body = client.get(Service::Geocode, &url, cancel).await?;
    parse(&body.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MILLAU: &str = r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Point","coordinates":[3.093182,44.10197]},"properties":{"label":"Millau","score":0.9495,"type":"municipality","x":707460.4,"y":6333667.33,"context":"12, Aveyron, Occitanie"}}],"query":"Millau"}"#;

    #[test]
    fn parses_reference_response() {
        let r = parse(MILLAU.as_bytes()).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].label, "Millau");
        assert_eq!(r[0].x, Some(707460.4));
        assert_eq!(r[0].kind.as_deref(), Some("municipality"));
    }

    #[test]
    fn url_encodes_query() {
        assert_eq!(
            search_url("https://data.geopf.fr", "rue de l'Église Millau", 5),
            "https://data.geopf.fr/geocodage/search?q=rue%20de%20l%27%C3%89glise%20Millau&limit=5"
        );
    }

    #[tokio::test]
    #[ignore = "réseau : géocodage Géoplateforme"]
    async fn live_millau() {
        let c = GeoClient::public().unwrap();
        let r = search(&c, "Millau", 1, &CancellationToken::new())
            .await
            .unwrap();
        assert!((r[0].lon - 3.09).abs() < 0.05);
        assert!(r[0].x.is_some());
    }
}
