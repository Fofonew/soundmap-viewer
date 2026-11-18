//! Gendarmeries et commissariats (BD TOPO `zone_d_activite_ou_d_interet`,
//! natures « Gendarmerie » et « Poste ou hôtel de police »), par requête WFS
//! filtrée (CQL) sur une emprise Lambert-93.

use crate::http::{GeoClient, Service, urlencode};
use crate::{BboxL93, DataError, Result};
use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub const NATURES: [&str; 2] = ["Gendarmerie", "Poste ou hôtel de police"];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PoliceStation {
    pub name: String,
    /// « Gendarmerie » ou « Poste ou hôtel de police ».
    pub nature: String,
    /// Centre de l'emprise (Lambert-93).
    pub x: f64,
    pub y: f64,
}

pub fn police_url(base: &str, bbox: &BboxL93) -> String {
    let natures = NATURES
        .iter()
        .map(|n| format!("'{n}'"))
        .collect::<Vec<_>>()
        .join(",");
    let cql = format!(
        "nature IN ({natures}) AND BBOX(geometrie,{},{},{},{},'EPSG:2154')",
        bbox.xmin, bbox.ymin, bbox.xmax, bbox.ymax
    );
    format!(
        "{base}/wfs/ows?SERVICE=WFS&VERSION=2.0.0&REQUEST=GetFeature\
         &TYPENAMES=BDTOPO_V3:zone_d_activite_ou_d_interet&OUTPUTFORMAT=application/json\
         &SRSNAME=EPSG:2154&COUNT=500&CQL_FILTER={}",
        urlencode(&cql)
    )
}

/// Centre de la boîte englobante de toutes les coordonnées d'une géométrie.
fn center(geom: &Value) -> Option<(f64, f64)> {
    fn walk(v: &Value, b: &mut (f64, f64, f64, f64)) {
        if let Some(a) = v.as_array() {
            if let (Some(x), Some(y)) = (
                a.first().and_then(Value::as_f64),
                a.get(1).and_then(Value::as_f64),
            ) {
                *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            } else {
                for c in a {
                    walk(c, b);
                }
            }
        }
    }
    let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    walk(geom.get("coordinates")?, &mut b);
    (b.0 <= b.2).then(|| ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0))
}

pub fn parse(bytes: &[u8]) -> Result<Vec<PoliceStation>> {
    let v: Value = serde_json::from_slice(bytes)?;
    let feats = v
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::Parse("WFS police : pas de `features`".into()))?;
    Ok(feats
        .iter()
        .filter_map(|f| {
            let p = f.get("properties")?;
            let nature = p.get("nature")?.as_str()?.to_owned();
            let name = p
                .get("toponyme")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(&nature)
                .to_owned();
            let (x, y) = center(f.get("geometry")?)?;
            Some(PoliceStation { name, nature, x, y })
        })
        .collect())
}

pub async fn fetch(
    client: &GeoClient,
    bbox: &BboxL93,
    cancel: &CancellationToken,
) -> Result<Vec<PoliceStation>> {
    let url = police_url(&client.endpoints().geopf, bbox);
    let body = client.get(Service::Wfs, &url, cancel).await?;
    parse(&body.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_filters_natures_and_bbox() {
        let u = police_url("https://data.geopf.fr", &BboxL93::new(1.0, 2.0, 3.0, 4.0));
        assert!(u.contains("TYPENAMES=BDTOPO_V3:zone_d_activite_ou_d_interet"));
        assert!(u.contains("CQL_FILTER=nature%20IN%20"));
        assert!(u.contains("Gendarmerie"));
        assert!(!u.contains(' '));
    }

    #[test]
    fn parses_center_and_name() {
        let body = r#"{"type":"FeatureCollection","features":[
            {"properties":{"nature":"Gendarmerie","toponyme":"Gendarmerie du Caylar"},
             "geometry":{"type":"MultiPolygon","coordinates":[[[[0,0],[10,0],[10,20],[0,20],[0,0]]]]}},
            {"properties":{"nature":"Poste ou hôtel de police","toponyme":null},
             "geometry":{"type":"Polygon","coordinates":[[[100,100],[110,100],[110,110],[100,100]]]}}]}"#;
        let s = parse(body.as_bytes()).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name, "Gendarmerie du Caylar");
        assert_eq!((s[0].x, s[0].y), (5.0, 10.0));
        assert_eq!(s[1].name, "Poste ou hôtel de police");
    }

    #[tokio::test]
    #[ignore = "réseau : Géoplateforme"]
    async fn live_police_larzac() {
        let c = GeoClient::public().unwrap();
        let bbox = BboxL93::new(690_000.0, 6_300_000.0, 750_000.0, 6_360_000.0);
        let s = fetch(&c, &bbox, &CancellationToken::new()).await.unwrap();
        assert!(s.iter().any(|p| p.name.contains("Caylar")), "{s:?}");
    }
}
