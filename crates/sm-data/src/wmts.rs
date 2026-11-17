//! Tuiles raster WMTS Géoplateforme (Web Mercator).

use crate::Result;
use crate::http::{GeoClient, Service};
use bytes::Bytes;
use serde::Serialize;
use sm_geo::TileId;
use tokio_util::sync::CancellationToken;

/// Couche WMTS connue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WmtsLayer {
    /// Identifiant court utilisé dans les URL `soundmap://` et dans le cache.
    pub id: &'static str,
    pub name: &'static str,
    pub format: &'static str,
    pub ext: &'static str,
    pub matrix_set: &'static str,
    pub min_zoom: u8,
    pub max_zoom: u8,
}

pub const ORTHO: WmtsLayer = WmtsLayer {
    id: "ortho",
    name: "ORTHOIMAGERY.ORTHOPHOTOS",
    format: "image/jpeg",
    ext: "jpg",
    matrix_set: "PM_0_19",
    min_zoom: 0,
    max_zoom: 19,
};
pub const PLAN_IGN: WmtsLayer = WmtsLayer {
    id: "plan",
    name: "GEOGRAPHICALGRIDSYSTEMS.PLANIGNV2",
    format: "image/png",
    ext: "png",
    matrix_set: "PM_0_19",
    min_zoom: 0,
    max_zoom: 19,
};
pub const LIDAR_SHADOW: WmtsLayer = WmtsLayer {
    id: "lidar-shadow",
    name: "IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.SHADOW",
    format: "image/png",
    ext: "png",
    matrix_set: "PM_0_18",
    min_zoom: 0,
    max_zoom: 18,
};
pub const OCSGE_COUVERTURE: WmtsLayer = WmtsLayer {
    id: "ocsge",
    name: "OCSGE.COUVERTURE.2021-2023",
    format: "image/png",
    ext: "png",
    matrix_set: "PM_6_16",
    min_zoom: 6,
    max_zoom: 16,
};
pub const COSIA: WmtsLayer = WmtsLayer {
    id: "cosia",
    name: "IGNF_COSIA_2021-2023",
    format: "image/png",
    ext: "png",
    matrix_set: "PM_6_18",
    min_zoom: 6,
    max_zoom: 18,
};

pub const ALL: [WmtsLayer; 5] = [ORTHO, PLAN_IGN, LIDAR_SHADOW, OCSGE_COUVERTURE, COSIA];

pub fn layer_by_id(id: &str) -> Option<WmtsLayer> {
    ALL.iter().copied().find(|l| l.id == id)
}

pub fn tile_url(base: &str, layer: &WmtsLayer, t: TileId) -> String {
    format!(
        "{base}/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER={}&STYLE=normal\
         &TILEMATRIXSET={}&TILEMATRIX={}&TILEROW={}&TILECOL={}&FORMAT={}",
        layer.name, layer.matrix_set, t.z, t.y, t.x, layer.format
    )
}

/// Télécharge une tuile ; `None` si la tuile n'existe pas (404).
pub async fn get_tile(
    client: &GeoClient,
    layer: &WmtsLayer,
    t: TileId,
    cancel: &CancellationToken,
) -> Result<Option<Bytes>> {
    let url = tile_url(&client.endpoints().geopf, layer, t);
    match client.get(Service::Wmts, &url, cancel).await {
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
        let u = tile_url("https://data.geopf.fr", &ORTHO, TileId::new(14, 8332, 5951));
        insta::assert_snapshot!(u, @"https://data.geopf.fr/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=ORTHOIMAGERY.ORTHOPHOTOS&STYLE=normal&TILEMATRIXSET=PM_0_19&TILEMATRIX=14&TILEROW=5951&TILECOL=8332&FORMAT=image/jpeg");
    }

    #[test]
    fn lookup() {
        assert_eq!(layer_by_id("plan"), Some(PLAN_IGN));
        assert_eq!(layer_by_id("nope"), None);
    }

    #[tokio::test]
    #[ignore = "réseau : WMTS Géoplateforme"]
    async fn live_ortho_tile_is_jpeg() {
        let c = GeoClient::public().unwrap();
        let t = get_tile(
            &c,
            &ORTHO,
            TileId::new(14, 8332, 5951),
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&t[..2], &[0xFF, 0xD8]);
    }
}
