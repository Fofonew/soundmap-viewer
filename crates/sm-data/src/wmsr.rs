//! Relief LiDAR HD et RGE ALTI via le WMS-Raster Géoplateforme, en float32 brut (BIL).

use crate::http::{GeoClient, Service};
use crate::{BboxL93, DataError, Result};
use serde::{Deserialize, Serialize};
use sm_geo::raster::NODATA;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ElevationLayer {
    /// MNT LiDAR HD (sol nu).
    Mnt,
    /// MNS LiDAR HD (surface avec le sursol).
    Mns,
    /// MNH LiDAR HD (hauteur du sursol).
    Mnh,
    /// RGE ALTI (repli hors couverture LiDAR HD).
    RgeAlti,
}

impl ElevationLayer {
    pub fn layer_name(self) -> &'static str {
        match self {
            ElevationLayer::Mnt => "IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93",
            ElevationLayer::Mns => "IGNF_LIDAR-HD_MNS_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93",
            ElevationLayer::Mnh => "IGNF_LIDAR-HD_MNH_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93",
            ElevationLayer::RgeAlti => "ELEVATION.ELEVATIONGRIDCOVERAGE.HIGHRES",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            ElevationLayer::Mnt => "mnt",
            ElevationLayer::Mns => "mns",
            ElevationLayer::Mnh => "mnh",
            ElevationLayer::RgeAlti => "rgealti",
        }
    }
}

/// Grille d'altitudes renvoyée par un GetMap (ordre ligne, du nord au sud).
#[derive(Debug, Clone, PartialEq)]
pub struct ElevationGrid {
    pub bbox: BboxL93,
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

impl ElevationGrid {
    /// (min, max) des valeurs valides.
    pub fn min_max(&self) -> Option<(f32, f32)> {
        self.data
            .iter()
            .copied()
            .filter(|v| *v != NODATA && v.is_finite())
            .fold(None, |acc, v| match acc {
                None => Some((v, v)),
                Some((a, b)) => Some((a.min(v), b.max(v))),
            })
    }
    pub fn nodata_count(&self) -> usize {
        self.data.iter().filter(|v| **v == NODATA).count()
    }
    pub fn cell(&self) -> f64 {
        self.bbox.width() / self.width as f64
    }
}

/// URL GetMap WMS 1.3.0 en BIL float32 (EPSG:2154, axes x/y).
pub fn getmap_url(base: &str, layer: ElevationLayer, bbox: &BboxL93, w: usize, h: usize) -> String {
    format!(
        "{base}/wms-r?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS={}&STYLES=normal\
         &CRS=EPSG:2154&BBOX={},{},{},{}&WIDTH={w}&HEIGHT={h}&FORMAT=image/x-bil;bits=32",
        layer.layer_name(),
        bbox.xmin,
        bbox.ymin,
        bbox.xmax,
        bbox.ymax
    )
}

/// Décode un buffer BIL float32 little-endian.
pub fn decode_bil(bytes: &[u8], w: usize, h: usize) -> Result<Vec<f32>> {
    if bytes.len() != w * h * 4 {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]).into_owned();
        return Err(DataError::Parse(format!(
            "BIL : {} octets reçus, {} attendus ({head})",
            bytes.len(),
            w * h * 4
        )));
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect())
}

/// Télécharge une grille d'altitude `w×h` sur `bbox`.
pub async fn get_elevation(
    client: &GeoClient,
    layer: ElevationLayer,
    bbox: &BboxL93,
    w: usize,
    h: usize,
    cancel: &CancellationToken,
) -> Result<ElevationGrid> {
    let url = getmap_url(&client.endpoints().geopf, layer, bbox, w, h);
    let body = client.get(Service::WmsR, &url, cancel).await?;
    let data = decode_bil(&body.bytes, w, h)?;
    Ok(ElevationGrid {
        bbox: *bbox,
        width: w,
        height: h,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_matches_validated_request() {
        let u = getmap_url(
            "https://data.geopf.fr",
            ElevationLayer::Mnt,
            &BboxL93::new(700000.0, 6320000.0, 701000.0, 6321000.0),
            1000,
            1000,
        );
        insta::assert_snapshot!(u, @"https://data.geopf.fr/wms-r?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93&STYLES=normal&CRS=EPSG:2154&BBOX=700000,6320000,701000,6321000&WIDTH=1000&HEIGHT=1000&FORMAT=image/x-bil;bits=32");
    }

    #[test]
    fn decode_bil_and_stats() {
        let vals = [494.7f32, 830.4, NODATA, 600.0];
        let bytes: Vec<u8> = vals.iter().flat_map(|v| v.to_le_bytes()).collect();
        let data = decode_bil(&bytes, 2, 2).unwrap();
        let g = ElevationGrid {
            bbox: BboxL93::new(0.0, 0.0, 2.0, 2.0),
            width: 2,
            height: 2,
            data,
        };
        assert_eq!(g.min_max(), Some((494.7, 830.4)));
        assert_eq!(g.nodata_count(), 1);
    }

    #[test]
    fn decode_bil_rejects_xml_error() {
        let e = decode_bil(b"<ServiceException>oops</ServiceException>", 10, 10).unwrap_err();
        assert!(e.to_string().contains("ServiceException"));
    }

    #[tokio::test]
    #[ignore = "réseau : WMS-R Géoplateforme"]
    async fn live_mnt_1km2() {
        let c = GeoClient::public().unwrap();
        let g = get_elevation(
            &c,
            ElevationLayer::Mnt,
            &BboxL93::new(700000.0, 6320000.0, 701000.0, 6321000.0),
            1000,
            1000,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let (lo, hi) = g.min_max().unwrap();
        assert!(
            (490.0..500.0).contains(&lo) && (825.0..835.0).contains(&hi),
            "{lo} {hi}"
        );
        assert_eq!(g.nodata_count(), 0);
    }
}
