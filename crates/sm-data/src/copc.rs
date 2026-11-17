//! Nuages de points COPC lus par HTTP Range : en-tête LAS 1.4 et VLR `copc`.
//! Lecture de l'octree à la demande (niveaux de détail et emprise utiles),
//! décodage LAZ des nœuds (crate `laz`, Rust pur).

use crate::http::{GeoClient, Service};
use crate::{DataError, Result};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

/// Octets lus pour l'en-tête et le premier VLR.
pub const HEADER_PROBE_LEN: u64 = 4096;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CopcHeader {
    pub version: (u8, u8),
    pub point_format: u8,
    pub point_record_len: u16,
    pub point_count: u64,
    pub header_size: u16,
    pub vlr_count: u32,
    pub has_copc_vlr: bool,
    pub bounds: [f64; 6],
    /// Taille totale du fichier (Content-Range).
    pub total_len: Option<u64>,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}
fn f64_at(b: &[u8], o: usize) -> Option<f64> {
    Some(f64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Analyse l'en-tête LAS 1.4 public (375 octets) et le premier VLR.
pub fn parse_header(b: &[u8]) -> Result<CopcHeader> {
    let err = || DataError::Parse("COPC : en-tête LAS tronqué".into());
    if b.get(0..4) != Some(b"LASF") {
        return Err(DataError::Parse("COPC : signature LASF absente".into()));
    }
    let header_size = u16_at(b, 94).ok_or_else(err)?;
    let legacy = u64::from(u32_at(b, 107).ok_or_else(err)?);
    let minor = *b.get(25).ok_or_else(err)?;
    let point_count = if minor >= 4 {
        u64_at(b, 247).ok_or_else(err)?
    } else {
        legacy
    };
    // Bornes : max X, min X, max Y, min Y, max Z, min Z à partir de 179.
    let mut bounds = [0.0; 6];
    for (i, v) in bounds.iter_mut().enumerate() {
        *v = f64_at(b, 179 + 8 * i).ok_or_else(err)?;
    }
    // Premier VLR : 2 octets réservés, puis user_id[16] ; COPC impose « copc ».
    let vlr = usize::from(header_size);
    let has_copc_vlr = b
        .get(vlr + 2..vlr + 18)
        .is_some_and(|uid| uid.starts_with(b"copc"));
    Ok(CopcHeader {
        version: (*b.get(24).ok_or_else(err)?, minor),
        point_format: *b.get(104).ok_or_else(err)? & 0x3F,
        point_record_len: u16_at(b, 105).ok_or_else(err)?,
        point_count,
        header_size,
        vlr_count: u32_at(b, 100).ok_or_else(err)?,
        has_copc_vlr,
        bounds,
        total_len: None,
    })
}

/// Lit l'en-tête d'un fichier COPC distant (une seule requête Range).
pub async fn read_header(
    client: &GeoClient,
    url: &str,
    cancel: &CancellationToken,
) -> Result<CopcHeader> {
    let body = client
        .get_range(Service::Copc, url, 0, HEADER_PROBE_LEN - 1, cancel)
        .await?;
    if body.status != 206 {
        return Err(DataError::Parse(format!(
            "COPC : HTTP Range non respecté (statut {})",
            body.status
        )));
    }
    let mut h = parse_header(&body.bytes)?;
    h.total_len = body.total_len;
    Ok(h)
}

// ------------------------------------------------------------------ octree

/// VLR `copc` (info) : cube racine, espacement, page racine de la hiérarchie.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CopcInfo {
    pub center: [f64; 3],
    pub halfsize: f64,
    /// Espacement des points au niveau racine (m).
    pub spacing: f64,
    pub root_hier_offset: u64,
    pub root_hier_size: u64,
}

/// Échelle et décalage des coordonnées entières LAS.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LasTransform {
    pub scale: [f64; 3],
    pub offset: [f64; 3],
}

/// Fichier COPC ouvert : en-tête, info d'octree, VLR LAZ.
#[derive(Debug, Clone)]
pub struct CopcFile {
    pub url: String,
    pub header: CopcHeader,
    pub info: CopcInfo,
    pub transform: LasTransform,
    pub laz_vlr: Vec<u8>,
}

/// Entrée de hiérarchie (32 octets) : nœud (d, x, y, z) → bloc de points, ou
/// page de hiérarchie enfant si `point_count == −1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NodeEntry {
    pub depth: i32,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub offset: u64,
    pub byte_size: i32,
    pub point_count: i32,
}

impl NodeEntry {
    /// Emprise horizontale du nœud [xmin, ymin, xmax, ymax].
    pub fn bounds_xy(&self, info: &CopcInfo) -> [f64; 4] {
        let size = 2.0 * info.halfsize / f64::from(1u32 << self.depth.clamp(0, 30));
        let x0 = info.center[0] - info.halfsize + f64::from(self.x) * size;
        let y0 = info.center[1] - info.halfsize + f64::from(self.y) * size;
        [x0, y0, x0 + size, y0 + size]
    }
}

/// Point LiDAR décodé (L93, altitude IGN69).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LidarPoint {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Classe ASPRS : 2 sol, 3/4/5 végétation basse/moyenne/haute, 6 bâti…
    pub class: u8,
}

/// Analyse l'info COPC et le VLR LAZ dans la zone des VLR (octets 0 → début des points).
pub fn parse_vlrs(b: &[u8]) -> Result<(CopcHeader, CopcInfo, LasTransform, Vec<u8>)> {
    let header = parse_header(b)?;
    let err = || DataError::Parse("COPC : VLR tronqués".into());
    let mut transform = LasTransform {
        scale: [0.0; 3],
        offset: [0.0; 3],
    };
    for i in 0..3 {
        transform.scale[i] = f64_at(b, 131 + 8 * i).ok_or_else(err)?;
        transform.offset[i] = f64_at(b, 155 + 8 * i).ok_or_else(err)?;
    }
    let mut o = usize::from(header.header_size);
    let (mut info, mut laz) = (None, None);
    for _ in 0..header.vlr_count {
        let uid = b.get(o + 2..o + 18).ok_or_else(err)?;
        let rid = u16_at(b, o + 18).ok_or_else(err)?;
        let len = usize::from(u16_at(b, o + 20).ok_or_else(err)?);
        let data = b.get(o + 54..o + 54 + len).ok_or_else(err)?;
        if uid.starts_with(b"copc") && rid == 1 {
            info = Some(CopcInfo {
                center: [
                    f64_at(data, 0).ok_or_else(err)?,
                    f64_at(data, 8).ok_or_else(err)?,
                    f64_at(data, 16).ok_or_else(err)?,
                ],
                halfsize: f64_at(data, 24).ok_or_else(err)?,
                spacing: f64_at(data, 32).ok_or_else(err)?,
                root_hier_offset: u64_at(data, 40).ok_or_else(err)?,
                root_hier_size: u64_at(data, 48).ok_or_else(err)?,
            });
        } else if uid.starts_with(b"laszip encoded") && rid == 22204 {
            laz = Some(data.to_vec());
        }
        o += 54 + len;
    }
    match (info, laz) {
        (Some(i), Some(l)) => Ok((header, i, transform, l)),
        _ => Err(DataError::Parse(
            "COPC : VLR « copc » ou « laszip » absent".into(),
        )),
    }
}

/// Entrées d'une page de hiérarchie.
pub fn parse_hierarchy_page(b: &[u8]) -> Vec<NodeEntry> {
    b.as_chunks::<32>()
        .0
        .iter()
        .filter_map(|e| {
            let i = |o: usize| u32_at(e, o).map(|v| v as i32);
            Some(NodeEntry {
                depth: i(0)?,
                x: i(4)?,
                y: i(8)?,
                z: i(12)?,
                offset: u64_at(e, 16)?,
                byte_size: i(24)?,
                point_count: i(28)?,
            })
        })
        .collect()
}

/// Décode un bloc de points LAZ (formats 6 à 8) d'un nœud.
pub fn decode_chunk(file: &CopcFile, chunk: &[u8], count: usize) -> Result<Vec<LidarPoint>> {
    use laz::record::{LayeredPointRecordDecompressor, RecordDecompressor};
    let perr = |e: String| DataError::Parse(format!("COPC/LAZ : {e}"));
    let vlr = laz::LazVlr::read_from(file.laz_vlr.as_slice()).map_err(|e| perr(e.to_string()))?;
    let mut dec = LayeredPointRecordDecompressor::new(std::io::Cursor::new(chunk));
    dec.set_fields_from(vlr.items())
        .map_err(|e| perr(e.to_string()))?;
    let len = usize::from(file.header.point_record_len);
    if len < 17 {
        return Err(perr(format!("taille d'enregistrement {len}")));
    }
    let mut buf = vec![0u8; len];
    let t = &file.transform;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        dec.decompress_next(&mut buf)
            .map_err(|e| perr(e.to_string()))?;
        let iv = |o: usize| {
            f64::from(i32::from_le_bytes([
                buf[o],
                buf[o + 1],
                buf[o + 2],
                buf[o + 3],
            ]))
        };
        out.push(LidarPoint {
            x: iv(0) * t.scale[0] + t.offset[0],
            y: iv(4) * t.scale[1] + t.offset[1],
            z: iv(8) * t.scale[2] + t.offset[2],
            class: buf[16],
        });
    }
    Ok(out)
}

impl CopcFile {
    /// Ouvre un COPC distant : en-tête puis zone des VLR (2 requêtes Range).
    pub async fn open(client: &GeoClient, url: &str, cancel: &CancellationToken) -> Result<Self> {
        let probe = client
            .get_range(Service::Copc, url, 0, HEADER_PROBE_LEN - 1, cancel)
            .await?;
        if probe.status != 206 {
            return Err(DataError::Parse(format!(
                "COPC : HTTP Range non respecté (statut {})",
                probe.status
            )));
        }
        // Les VLR se terminent au début des points (offset 96 de l'en-tête).
        let end = u32_at(&probe.bytes, 96)
            .map(u64::from)
            .ok_or_else(|| DataError::Parse("COPC : offset des points".into()))?;
        let bytes = if end <= HEADER_PROBE_LEN {
            probe.bytes.to_vec()
        } else {
            client
                .get_range(Service::Copc, url, 0, end - 1, cancel)
                .await?
                .bytes
                .to_vec()
        };
        let (header, info, transform, laz_vlr) = parse_vlrs(&bytes)?;
        Ok(Self {
            url: url.to_owned(),
            header: CopcHeader {
                total_len: probe.total_len,
                ..header
            },
            info,
            transform,
            laz_vlr,
        })
    }

    /// Nœuds contenant des points, jusqu'à la profondeur `max_depth`, dont
    /// l'emprise coupe `bbox` [xmin, ymin, xmax, ymax].
    pub async fn nodes(
        &self,
        client: &GeoClient,
        bbox: [f64; 4],
        max_depth: i32,
        cancel: &CancellationToken,
    ) -> Result<Vec<NodeEntry>> {
        let hits = |e: &NodeEntry| {
            let b = e.bounds_xy(&self.info);
            b[0] < bbox[2] && b[2] > bbox[0] && b[1] < bbox[3] && b[3] > bbox[1]
        };
        let mut pages = vec![(self.info.root_hier_offset, self.info.root_hier_size)];
        let mut out = Vec::new();
        while let Some((off, size)) = pages.pop() {
            if size == 0 {
                continue;
            }
            let page = client
                .get_range(Service::Copc, &self.url, off, off + size - 1, cancel)
                .await?;
            for e in parse_hierarchy_page(&page.bytes) {
                if e.depth > max_depth || !hits(&e) {
                    continue;
                }
                if e.point_count == -1 {
                    pages.push((e.offset, u64::try_from(e.byte_size).unwrap_or(0)));
                } else if e.point_count > 0 && e.byte_size > 0 {
                    out.push(e);
                }
            }
        }
        Ok(out)
    }

    /// Profondeur à laquelle l'espacement des points atteint `spacing_m`.
    pub fn depth_for_spacing(&self, spacing_m: f64) -> i32 {
        let r = (self.info.spacing / spacing_m.max(1e-3)).max(1.0);
        r.log2().ceil() as i32
    }

    /// Télécharge et décode les points d'un nœud.
    pub async fn points(
        &self,
        client: &GeoClient,
        e: &NodeEntry,
        cancel: &CancellationToken,
    ) -> Result<Vec<LidarPoint>> {
        let size = u64::try_from(e.byte_size).unwrap_or(0);
        if size == 0 {
            return Ok(Vec::new());
        }
        let chunk = client
            .get_range(
                Service::Copc,
                &self.url,
                e.offset,
                e.offset + size - 1,
                cancel,
            )
            .await?;
        decode_chunk(
            self,
            &chunk.bytes,
            usize::try_from(e.point_count).unwrap_or(0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_header() -> Vec<u8> {
        let mut b = vec![0u8; 600];
        b[0..4].copy_from_slice(b"LASF");
        b[24] = 1;
        b[25] = 4;
        b[94..96].copy_from_slice(&375u16.to_le_bytes());
        b[100..104].copy_from_slice(&3u32.to_le_bytes());
        b[104] = 6 | 0x40; // bit de compression LAZ
        b[105..107].copy_from_slice(&30u16.to_le_bytes());
        b[247..255].copy_from_slice(&28_715_324u64.to_le_bytes());
        b[179..187].copy_from_slice(&702_000.0f64.to_le_bytes());
        b[375 + 2..375 + 6].copy_from_slice(b"copc");
        b
    }

    #[test]
    fn parses_las14_copc_header() {
        let h = parse_header(&fake_header()).unwrap();
        assert_eq!(h.version, (1, 4));
        assert_eq!(h.point_format, 6);
        assert_eq!(h.point_count, 28_715_324);
        assert!(h.has_copc_vlr);
        assert_eq!(h.bounds[0], 702_000.0);
    }

    #[test]
    fn rejects_non_las() {
        assert!(parse_header(b"PK\x03\x04").is_err());
    }

    #[tokio::test]
    #[ignore = "réseau : COPC LiDAR HD par HTTP Range"]
    async fn live_copc_header() {
        let c = GeoClient::public().unwrap();
        let url = "https://data.geopf.fr/telechargement/download/LiDARHD-NUALID/NUALHD_1-0__LAZ_LAMB93_LP_2025-02-28/LHD_FXX_0701_6321_PTS_LAMB93_IGN69.copc.laz";
        let h = read_header(&c, url, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(h.version, (1, 4));
        assert_eq!(h.point_format, 6);
        assert!(h.has_copc_vlr);
        assert!(h.point_count > 1_000_000);
        assert!(h.total_len.unwrap_or(0) > 100_000_000);
    }

    #[tokio::test]
    #[ignore = "réseau : octree COPC et décodage LAZ"]
    async fn live_copc_points() {
        let c = GeoClient::public().unwrap();
        let url = "https://data.geopf.fr/telechargement/download/LiDARHD-NUALID/NUALHD_1-0__LAZ_LAMB93_LP_2025-02-28/LHD_FXX_0701_6321_PTS_LAMB93_IGN69.copc.laz";
        let cancel = CancellationToken::new();
        let f = CopcFile::open(&c, url, &cancel).await.unwrap();
        assert!(f.info.halfsize > 400.0 && f.info.spacing > 0.0);
        let b = [701_400.0, 6_320_400.0, 701_600.0, 6_320_600.0];
        let nodes = f.nodes(&c, b, 2, &cancel).await.unwrap();
        assert!(!nodes.is_empty());
        let pts = f.points(&c, &nodes[0], &cancel).await.unwrap();
        assert_eq!(pts.len(), nodes[0].point_count as usize);
        let p = pts[pts.len() / 2];
        assert!((700_000.0..702_000.0).contains(&p.x), "{p:?}");
        assert!((6_320_000.0..6_322_000.0).contains(&p.y), "{p:?}");
        assert!(pts.iter().any(|p| p.class == 2));
    }
}
