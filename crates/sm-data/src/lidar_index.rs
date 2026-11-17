//! Index des dalles LiDAR HD (WFS `IGNF_LIDAR-HD_METADONNEE:metadata`) et URL COPC.

use crate::http::GeoClient;
use crate::wfs::{self, layers};
use crate::{BboxL93, Result};
use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LidarTile {
    /// URL du nuage COPC (`url_npl`).
    pub url_copc: Option<String>,
    pub url_mnt: Option<String>,
    pub point_count: Option<u64>,
    pub acquisition_start: Option<String>,
    pub acquisition_end: Option<String>,
    pub bbox: Option<BboxL93>,
}

fn bbox_of(geom: &Value) -> Option<BboxL93> {
    let mut b = BboxL93::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut stack = vec![geom.get("coordinates")?];
    while let Some(v) = stack.pop() {
        let a = v.as_array()?;
        if let (Some(x), Some(y)) = (
            a.first().and_then(Value::as_f64),
            a.get(1).and_then(Value::as_f64),
        ) {
            b.xmin = b.xmin.min(x);
            b.ymin = b.ymin.min(y);
            b.xmax = b.xmax.max(x);
            b.ymax = b.ymax.max(y);
        } else {
            stack.extend(a.iter());
        }
    }
    (b.xmin <= b.xmax).then_some(b)
}

pub fn parse_tiles(features: &[Value]) -> Vec<LidarTile> {
    let s = |p: &Value, k: &str| p.get(k).and_then(Value::as_str).map(str::to_owned);
    features
        .iter()
        .filter_map(|f| {
            let p = f.get("properties")?;
            Some(LidarTile {
                url_copc: s(p, "url_npl"),
                url_mnt: s(p, "url_mnt"),
                point_count: p.get("nombre_points").and_then(Value::as_u64),
                acquisition_start: s(p, "date_debut_acquisition"),
                acquisition_end: s(p, "date_fin_acquisition"),
                bbox: f.get("geometry").and_then(bbox_of),
            })
        })
        .collect()
}

pub async fn tiles_in_bbox(
    client: &GeoClient,
    bbox: &BboxL93,
    cancel: &CancellationToken,
) -> Result<Vec<LidarTile>> {
    let feats = wfs::get_all(client, layers::LIDAR_INDEX, bbox, cancel).await?;
    Ok(parse_tiles(&feats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_feature() {
        let f = serde_json::json!({
            "type": "Feature",
            "geometry": {"type": "Polygon", "coordinates": [[[701000.0, 6320000.0], [702000.0, 6320000.0], [702000.0, 6321000.0], [701000.0, 6320000.0]]]},
            "properties": {"url_npl": "https://x/LHD.copc.laz", "nombre_points": 28715324}
        });
        let t = parse_tiles(&[f]);
        assert_eq!(t[0].point_count, Some(28_715_324));
        assert_eq!(
            t[0].bbox,
            Some(BboxL93::new(701000.0, 6320000.0, 702000.0, 6321000.0))
        );
    }

    #[tokio::test]
    #[ignore = "réseau : WFS index LiDAR HD"]
    async fn live_index_has_copc_urls() {
        let c = GeoClient::public().unwrap();
        let t = tiles_in_bbox(
            &c,
            &BboxL93::new(700500.0, 6320500.0, 701500.0, 6321500.0),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(!t.is_empty());
        assert!(t.iter().any(|t| {
            t.url_copc
                .as_deref()
                .is_some_and(|u| u.ends_with(".copc.laz"))
        }));
    }
}
