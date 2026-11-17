//! Vecteurs WFS 2.0 Géoplateforme (GeoJSON), avec pagination.
//!
//! Contraintes du service : 2 couches maximum par requête, 5000 objets par
//! page par défaut (`CountDefault` des capacités, vérifié le 17/11/2026).

use crate::http::{GeoClient, Service};
use crate::{BboxL93, DataError, Result};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub const PAGE_SIZE: usize = 5000;
pub const MAX_TYPENAMES: usize = 2;

/// Couches WFS utilisées par SoundMap.
pub mod layers {
    pub const BATIMENT: &str = "BDTOPO_V3:batiment";
    pub const VEGETATION: &str = "BDTOPO_V3:zone_de_vegetation";
    pub const SURFACE_HYDRO: &str = "BDTOPO_V3:surface_hydrographique";
    pub const PLAN_D_EAU: &str = "BDTOPO_V3:plan_d_eau";
    pub const ROUTE: &str = "BDTOPO_V3:troncon_de_route";
    pub const VOIE_FERREE: &str = "BDTOPO_V3:troncon_de_voie_ferree";
    pub const ZONE_ACTIVITE: &str = "BDTOPO_V3:zone_d_activite_ou_d_interet";
    pub const PARC_RESERVE: &str = "BDTOPO_V3:parc_ou_reserve";
    pub const FILOSOFI: &str = "INSEE.FILOSOFI.INDICATORS:carreaux_200m";
    pub const ZNIEFF1: &str = "patrinat_znieff1:znieff1";
    pub const ZNIEFF2: &str = "patrinat_znieff2:znieff2";
    pub const LIDAR_INDEX: &str = "IGNF_LIDAR-HD_METADONNEE:metadata";
}

/// URL GetFeature (GeoJSON, géométries en Lambert-93).
pub fn getfeature_url(
    base: &str,
    typenames: &[&str],
    bbox: &BboxL93,
    count: usize,
    start: usize,
) -> Result<String> {
    if typenames.is_empty() || typenames.len() > MAX_TYPENAMES {
        return Err(DataError::Invalid(format!(
            "WFS : 1 à {MAX_TYPENAMES} couches par requête, {} demandées",
            typenames.len()
        )));
    }
    Ok(format!(
        "{base}/wfs/ows?SERVICE=WFS&VERSION=2.0.0&REQUEST=GetFeature&TYPENAMES={}\
         &OUTPUTFORMAT=application/json&SRSNAME=EPSG:2154&BBOX={},{},{},{},EPSG:2154\
         &COUNT={count}&STARTINDEX={start}",
        typenames.join(","),
        bbox.xmin,
        bbox.ymin,
        bbox.xmax,
        bbox.ymax
    ))
}

/// Une page de résultats.
#[derive(Debug, Clone)]
pub struct FeaturePage {
    pub features: Vec<Value>,
    pub number_matched: Option<u64>,
}

pub fn parse_page(bytes: &[u8]) -> Result<FeaturePage> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| {
        DataError::Parse(format!(
            "WFS : JSON invalide ({e}) : {}",
            String::from_utf8_lossy(&bytes[..bytes.len().min(200)])
        ))
    })?;
    let features = match v.get("features") {
        Some(Value::Array(a)) => a.clone(),
        _ => return Err(DataError::Parse("WFS : pas de tableau `features`".into())),
    };
    Ok(FeaturePage {
        features,
        number_matched: v.get("numberMatched").and_then(Value::as_u64),
    })
}

pub async fn get_page(
    client: &GeoClient,
    typename: &str,
    bbox: &BboxL93,
    count: usize,
    start: usize,
    cancel: &CancellationToken,
) -> Result<FeaturePage> {
    let url = getfeature_url(&client.endpoints().geopf, &[typename], bbox, count, start)?;
    let body = client.get(Service::Wfs, &url, cancel).await?;
    parse_page(&body.bytes)
}

/// Tous les objets d'une couche dans l'emprise (pages successives).
pub async fn get_all(
    client: &GeoClient,
    typename: &str,
    bbox: &BboxL93,
    cancel: &CancellationToken,
) -> Result<Vec<Value>> {
    let mut all = Vec::new();
    loop {
        let page = get_page(client, typename, bbox, PAGE_SIZE, all.len(), cancel).await?;
        let n = page.features.len();
        all.extend(page.features);
        let done = n < PAGE_SIZE || page.number_matched.is_some_and(|m| all.len() as u64 >= m);
        if done {
            return Ok(all);
        }
    }
}

/// Assemble des objets en FeatureCollection GeoJSON.
pub fn feature_collection(features: Vec<Value>) -> Value {
    serde_json::json!({ "type": "FeatureCollection", "features": features })
}

/// Résumé des bâtiments BD TOPO.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BuildingStats {
    pub total: usize,
    pub residential: usize,
    pub dwellings: u64,
}

/// `usage_1` ou `usage_2` = « Résidentiel ».
pub fn is_residential(props: &Value) -> bool {
    ["usage_1", "usage_2"]
        .iter()
        .any(|k| props.get(*k).and_then(Value::as_str) == Some("Résidentiel"))
}

pub fn building_stats(features: &[Value]) -> BuildingStats {
    let mut s = BuildingStats::default();
    for f in features {
        s.total += 1;
        if let Some(p) = f.get("properties") {
            if is_residential(p) {
                s.residential += 1;
            }
            s.dwellings += p
                .get("nombre_de_logements")
                .and_then(Value::as_u64)
                .unwrap_or(0);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn url_snapshot() {
        let u = getfeature_url(
            "https://data.geopf.fr",
            &[layers::BATIMENT],
            &BboxL93::new(700000.0, 6320000.0, 703000.0, 6323000.0),
            5000,
            0,
        )
        .unwrap();
        insta::assert_snapshot!(u, @"https://data.geopf.fr/wfs/ows?SERVICE=WFS&VERSION=2.0.0&REQUEST=GetFeature&TYPENAMES=BDTOPO_V3:batiment&OUTPUTFORMAT=application/json&SRSNAME=EPSG:2154&BBOX=700000,6320000,703000,6323000,EPSG:2154&COUNT=5000&STARTINDEX=0");
    }

    #[test]
    fn rejects_three_layers() {
        let b = BboxL93::new(0.0, 0.0, 1.0, 1.0);
        assert!(getfeature_url("x", &["a", "b", "c"], &b, 1, 0).is_err());
        assert!(getfeature_url("x", &["a", "b"], &b, 1, 0).is_ok());
    }

    #[test]
    fn stats_from_fixture() {
        let fc: Value = serde_json::from_str(include_str!(
            "../../../tests/data/batiments_millau_extrait.geojson"
        ))
        .unwrap();
        let feats = fc["features"].as_array().unwrap();
        let s = building_stats(feats);
        assert_eq!(s.total, feats.len());
        assert!(s.residential >= 1);
    }

    #[tokio::test]
    async fn paginates_until_short_page() {
        let server = MockServer::start().await;
        let feat = serde_json::json!({"type":"Feature","properties":{},"geometry":null});
        let full: Vec<Value> = vec![feat.clone(); PAGE_SIZE];
        Mock::given(method("GET"))
            .and(query_param("STARTINDEX", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"type":"FeatureCollection","features":full,"numberMatched":5003}),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("STARTINDEX", "5000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"type":"FeatureCollection","features":[feat.clone(),feat.clone(),feat]}),
            ))
            .mount(&server)
            .await;
        let c = GeoClient::new(
            crate::Endpoints::all_at(&server.uri()),
            crate::HttpConfig::default(),
        )
        .unwrap();
        let all = get_all(
            &c,
            layers::BATIMENT,
            &BboxL93::new(0.0, 0.0, 1.0, 1.0),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(all.len(), 5003);
    }

    #[tokio::test]
    #[ignore = "réseau : WFS Géoplateforme"]
    async fn live_buildings_and_layers() {
        let c = GeoClient::public().unwrap();
        let cancel = CancellationToken::new();
        let b = BboxL93::new(700000.0, 6320000.0, 701000.0, 6321000.0);
        let feats = get_all(&c, layers::BATIMENT, &b, &cancel).await.unwrap();
        assert!(!feats.is_empty());
        let p = &feats[0]["properties"];
        assert!(p.get("usage_1").is_some() && p.get("hauteur").is_some());
        for l in [
            layers::VEGETATION,
            layers::ROUTE,
            layers::FILOSOFI,
            layers::ZNIEFF1,
        ] {
            let page = get_page(&c, l, &b, 10, 0, &cancel).await.unwrap();
            tracing::info!(l, n = page.features.len());
        }
    }
}
