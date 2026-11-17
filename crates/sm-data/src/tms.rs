//! Tuiles vectorielles TMS Géoplateforme (pbf, schéma XYZ).

use crate::Result;
use crate::http::{GeoClient, Service};
use bytes::Bytes;
use serde::Serialize;
use sm_geo::TileId;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TmsLayer {
    pub id: &'static str,
    pub name: &'static str,
    /// Nom de la `source-layer` (lu dans `metadata.json`, 17/11/2026).
    pub source_layer: &'static str,
    pub min_zoom: u8,
    pub max_zoom: u8,
}

/// Emprises des dalles de nuages de points LiDAR HD.
pub const LIDAR_HD_COVERAGE: TmsLayer = TmsLayer {
    id: "lidar-coverage",
    name: "IGNF_NUAGES-DE-POINTS-LIDAR-HD-produit",
    source_layer: "nuage_dalle",
    min_zoom: 10,
    max_zoom: 16,
};

/// OCS GE couverture 2021-2023 (attribut `code_cs`).
pub const OCSGE_COUVERTURE: TmsLayer = TmsLayer {
    id: "ocsge",
    name: "OCSGE-TMS.COUVERTURE.2021-2023",
    source_layer: "occupation_sol",
    min_zoom: 12,
    max_zoom: 16,
};

pub const ALL: [TmsLayer; 2] = [LIDAR_HD_COVERAGE, OCSGE_COUVERTURE];

pub fn layer_by_id(id: &str) -> Option<TmsLayer> {
    ALL.iter().copied().find(|l| l.id == id)
}

pub fn tile_url(base: &str, layer: &TmsLayer, t: TileId) -> String {
    format!(
        "{base}/tms/1.0.0/{}/{}/{}/{}.pbf",
        layer.name, t.z, t.x, t.y
    )
}

/// Télécharge une tuile pbf ; `None` si absente (404 ou 204).
pub async fn get_tile(
    client: &GeoClient,
    layer: &TmsLayer,
    t: TileId,
    cancel: &CancellationToken,
) -> Result<Option<Bytes>> {
    let url = tile_url(&client.endpoints().geopf, layer, t);
    match client.get(Service::Tms, &url, cancel).await {
        Ok(b) if b.status == 204 || b.bytes.is_empty() => Ok(None),
        Ok(b) => Ok(Some(b.bytes)),
        Err(e) if e.is_not_found() => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_snapshot() {
        let u = tile_url(
            "https://data.geopf.fr",
            &OCSGE_COUVERTURE,
            TileId::new(16, 33331, 23804),
        );
        insta::assert_snapshot!(u, @"https://data.geopf.fr/tms/1.0.0/OCSGE-TMS.COUVERTURE.2021-2023/16/33331/23804.pbf");
    }

    #[tokio::test]
    #[ignore = "réseau : TMS Géoplateforme"]
    async fn live_lidar_coverage_tile() {
        let c = GeoClient::public().unwrap();
        let t = get_tile(
            &c,
            &LIDAR_HD_COVERAGE,
            TileId::new(14, 8332, 5951),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(t.is_some_and(|b| !b.is_empty()));
    }
}
