//! Géologie BRGM : scan WMS (affichage) et lithologie simplifiée au
//! 1/1 000 000 en WFS (GeoJSON, Lambert-93) pour distinguer roche dure et
//! formations meubles.

use crate::http::{GeoClient, Service};
use crate::{BboxL93, DataError, Result};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub const LAYER_LITHO: &str = "ms:LITHO_1M_SIMPLIFIEE";

/// Nature mécanique d'une formation, déduite de sa description.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RockKind {
    /// Roche consolidée (granite, basalte, calcaire, gneiss, grès…).
    Hard,
    /// Formations meubles (alluvions, limons, argiles, sables, marnes…).
    Soft,
    /// Description mêlant les deux (ex. « calcaires, marnes et gypse »).
    Mixed,
}

/// Classe une description lithologique BRGM (mots-clés, insensible à la casse).
pub fn rock_kind(descr: &str) -> RockKind {
    let d = descr.to_lowercase();
    const HARD: [&str; 22] = [
        "granit",
        "granodiorit",
        "diorit",
        "gabbro",
        "basalt",
        "andésit",
        "rhyolit",
        "volcan",
        "plutoni",
        "gneiss",
        "micaschist",
        "schist",
        "quartzit",
        "amphibolit",
        "calcaire",
        "dolomi",
        "grès",
        "conglomér",
        "métamorph",
        "migmatit",
        "ophiolit",
        "roches cristallines",
    ];
    const SOFT: [&str; 12] = [
        "alluvion",
        "limon",
        "argile",
        "sable",
        "marne",
        "colluvion",
        "moraine",
        "loess",
        "tourbe",
        "éboulis",
        "dépôts",
        "formations superficielles",
    ];
    let hard = HARD.iter().any(|k| d.contains(k));
    let soft = SOFT.iter().any(|k| d.contains(k));
    match (hard, soft) {
        (true, false) => RockKind::Hard,
        (true, true) => RockKind::Mixed,
        _ => RockKind::Soft,
    }
}

/// Polygone lithologique (anneaux en Lambert-93).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lithology {
    pub descr: String,
    pub kind: RockKind,
    pub rings: Vec<Vec<(f64, f64)>>,
}

pub fn litho_url(base: &str, bbox: &BboxL93) -> String {
    format!(
        "{base}/geologie?SERVICE=WFS&VERSION=2.0.0&REQUEST=GetFeature&TYPENAMES={LAYER_LITHO}\
         &BBOX={},{},{},{},urn:ogc:def:crs:EPSG::2154&SRSNAME=EPSG:2154&OUTPUTFORMAT=geojson",
        bbox.xmin, bbox.ymin, bbox.xmax, bbox.ymax
    )
}

/// Analyse la réponse GeoJSON (Polygon / MultiPolygon).
pub fn parse_lithology(bytes: &[u8]) -> Result<Vec<Lithology>> {
    let v: Value = serde_json::from_slice(bytes)?;
    let feats = v
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| DataError::Parse("BRGM : pas de `features`".into()))?;
    let ring = |r: &Value| -> Vec<(f64, f64)> {
        r.as_array()
            .map(|pts| {
                pts.iter()
                    .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for f in feats {
        let descr = f
            .pointer("/properties/DESCR")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let g = f.get("geometry");
        let ty = g.and_then(|g| g.get("type")).and_then(Value::as_str);
        let coords = g.and_then(|g| g.get("coordinates"));
        let polys: Vec<&Value> = match (ty, coords.and_then(Value::as_array)) {
            (Some("Polygon"), Some(_)) => coords.into_iter().collect(),
            (Some("MultiPolygon"), Some(a)) => a.iter().collect(),
            _ => continue,
        };
        let rings: Vec<Vec<(f64, f64)>> = polys
            .iter()
            .filter_map(|p| p.as_array())
            .flat_map(|rs| rs.iter().map(ring))
            .filter(|r| r.len() >= 3)
            .collect();
        if !rings.is_empty() {
            out.push(Lithology {
                kind: rock_kind(&descr),
                descr,
                rings,
            });
        }
    }
    Ok(out)
}

/// Lithologie simplifiée (1/1 000 000) intersectant l'emprise.
pub async fn fetch_lithology(
    client: &GeoClient,
    bbox: &BboxL93,
    cancel: &CancellationToken,
) -> Result<Vec<Lithology>> {
    let url = litho_url(&client.endpoints().brgm, bbox);
    parse_lithology(&client.get(Service::Brgm, &url, cancel).await?.bytes)
}

pub const LAYER_GEOL50: &str = "SCAN_D_GEOL50";

pub fn getmap_url(base: &str, layer: &str, bbox: &BboxL93, w: u32, h: u32) -> String {
    format!(
        "{base}/geologie?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS={layer}&STYLES=\
         &CRS=EPSG:2154&BBOX={},{},{},{}&WIDTH={w}&HEIGHT={h}&FORMAT=image/png",
        bbox.xmin, bbox.ymin, bbox.xmax, bbox.ymax
    )
}

pub async fn get_map(
    client: &GeoClient,
    layer: &str,
    bbox: &BboxL93,
    w: u32,
    h: u32,
    cancel: &CancellationToken,
) -> Result<Bytes> {
    let url = getmap_url(&client.endpoints().brgm, layer, bbox, w, h);
    Ok(client.get(Service::Brgm, &url, cancel).await?.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rock_kinds() {
        assert_eq!(rock_kind("Granites et granodiorites"), RockKind::Hard);
        assert_eq!(rock_kind("Calcaires, marnes et gypse"), RockKind::Mixed);
        assert_eq!(rock_kind("Alluvions récentes"), RockKind::Soft);
        assert_eq!(rock_kind("Basaltes"), RockKind::Hard);
    }

    #[test]
    fn parses_geojson_multipolygon() {
        let j = br#"{"type":"FeatureCollection","features":[{"type":"Feature",
            "properties":{"DESCR":"Basaltes"},"geometry":{"type":"MultiPolygon","coordinates":
            [[[[0,0],[10,0],[10,10],[0,0]]],[[[20,20],[30,20],[30,30],[20,20]]]]}}]}"#;
        let l = parse_lithology(j).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].kind, RockKind::Hard);
        assert_eq!(l[0].rings.len(), 2);
    }

    #[tokio::test]
    #[ignore = "réseau : WFS lithologie BRGM"]
    async fn live_lithology() {
        let c = GeoClient::public().unwrap();
        let l = fetch_lithology(
            &c,
            &BboxL93::new(695000.0, 6315000.0, 707000.0, 6327000.0),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(!l.is_empty());
        assert!(l.iter().all(|x| x.rings[0][0].0 > 100_000.0));
    }

    #[test]
    fn url_snapshot() {
        let u = getmap_url(
            "https://geoservices.brgm.fr",
            LAYER_GEOL50,
            &BboxL93::new(700000.0, 6320000.0, 701000.0, 6321000.0),
            256,
            256,
        );
        insta::assert_snapshot!(u, @"https://geoservices.brgm.fr/geologie?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=SCAN_D_GEOL50&STYLES=&CRS=EPSG:2154&BBOX=700000,6320000,701000,6321000&WIDTH=256&HEIGHT=256&FORMAT=image/png");
    }

    #[tokio::test]
    #[ignore = "réseau : WMS BRGM"]
    async fn live_png() {
        let c = GeoClient::public().unwrap();
        let b = get_map(
            &c,
            LAYER_GEOL50,
            &BboxL93::new(700000.0, 6320000.0, 701000.0, 6321000.0),
            256,
            256,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(&b[1..4], b"PNG");
    }
}
