//! Encodage Terrarium (`h = R·256 + G + B/256 − 32768`) et génération de
//! tuiles de terrain PNG pour MapLibre (`raster-dem`, `encoding: "terrarium"`).

use crate::Result;
use crate::proj::Lambert93;
use crate::raster::MultiResDem;
use crate::tiles::{TileId, tile_frac_to_lonlat};

/// Altitude (m) → (R, G, B) Terrarium. Précision : 1/256 m.
#[inline]
pub fn encode(h: f32) -> [u8; 3] {
    let v = (f64::from(h) + 32768.0).clamp(0.0, 65_535.996);
    let i = v.floor();
    let r = (i / 256.0).floor();
    let g = i - r * 256.0;
    let b = ((v - i) * 256.0).floor();
    [r as u8, g as u8, b as u8]
}

/// (R, G, B) Terrarium → altitude (m).
#[inline]
pub fn decode(rgb: [u8; 3]) -> f32 {
    (f32::from(rgb[0]) * 256.0 + f32::from(rgb[1]) + f32::from(rgb[2]) / 256.0) - 32768.0
}

/// Nombre de nœuds par côté de la grille de projection (les coordonnées
/// Lambert-93 sont interpolées bilinéairement entre nœuds : erreur < 1 cm).
const GRID: usize = 17;

/// Génère la tuile Terrarium `tile` (côté `size` px) en PNG.
///
/// Hors de l'emprise du MNT, on prolonge la valeur du bord ; sans aucune
/// donnée, on écrit `fallback`.
pub fn render_tile(
    dem: &MultiResDem,
    proj: &Lambert93,
    tile: TileId,
    size: u32,
    fallback: f32,
) -> Result<Vec<u8>> {
    // 1. Grille GRID×GRID de points projetés en Lambert-93.
    let mut nodes = [(0.0f64, 0.0f64); GRID * GRID];
    for j in 0..GRID {
        for i in 0..GRID {
            let fx = f64::from(tile.x) + i as f64 / (GRID - 1) as f64;
            let fy = f64::from(tile.y) + j as f64 / (GRID - 1) as f64;
            let (lon, lat) = tile_frac_to_lonlat(fx, fy, tile.z);
            nodes[j * GRID + i] = proj.from_wgs84(lon, lat)?;
        }
    }
    // 2. Échantillonnage au centre de chaque pixel.
    let n = size as usize;
    let mut rgb = vec![0u8; n * n * 3];
    let step = (GRID - 1) as f64 / n as f64;
    for py in 0..n {
        let gy = (py as f64 + 0.5) * step;
        let j = (gy.floor() as usize).min(GRID - 2);
        let ty = gy - j as f64;
        for px in 0..n {
            let gx = (px as f64 + 0.5) * step;
            let i = (gx.floor() as usize).min(GRID - 2);
            let tx = gx - i as f64;
            let a = nodes[j * GRID + i];
            let b = nodes[j * GRID + i + 1];
            let c = nodes[(j + 1) * GRID + i];
            let d = nodes[(j + 1) * GRID + i + 1];
            let x = (a.0 * (1.0 - tx) + b.0 * tx) * (1.0 - ty) + (c.0 * (1.0 - tx) + d.0 * tx) * ty;
            let y = (a.1 * (1.0 - tx) + b.1 * tx) * (1.0 - ty) + (c.1 * (1.0 - tx) + d.1 * tx) * ty;
            let h = dem.sample_clamped(x, y).unwrap_or(fallback);
            let o = (py * n + px) * 3;
            rgb[o..o + 3].copy_from_slice(&encode(h));
        }
    }
    encode_png_rgb(&rgb, size)
}

/// Encode un buffer RGB8 carré en PNG (compression rapide).
pub fn encode_png_rgb(rgb: &[u8], size: u32) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(rgb.len() / 2);
    {
        let mut enc = png::Encoder::new(&mut out, size, size);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut w = enc.write_header()?;
        w.write_image_data(rgb)?;
        w.finish()?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BboxL93;
    use crate::raster::{MmapRaster, RasterHeader, RasterWriter};

    #[test]
    fn encode_decode_round_trip() {
        for h in [-50.0f32, 0.0, 494.7, 830.4, 4807.81] {
            let d = decode(encode(h));
            assert!((d - h).abs() <= 1.0 / 256.0 + 1e-3, "{h} → {d}");
        }
    }

    #[test]
    fn maplibre_reference_encoding() {
        // Formule du dépôt MapLibre : h = 3000 → R = ⌊35768/256⌋ = 139, G = 35768 mod 256 = 184.
        assert_eq!(encode(3000.0), [139, 184, 0]);
        assert_eq!(encode(0.0), [128, 0, 0]);
    }

    #[test]
    fn render_flat_tile() {
        let dir = tempfile::tempdir().unwrap();
        let proj = Lambert93::new().unwrap();
        let (cx, cy) = (707_460.0, 6_333_667.0);
        let h = RasterHeader::for_bbox(&BboxL93::around(cx, cy, 3000.0), 10.0);
        let p = dir.path().join("flat.smr");
        let mut w = RasterWriter::create(&p, h).unwrap();
        let data = vec![500.0f32; (h.width * h.height) as usize];
        w.write_block(0, 0, h.width as usize, h.height as usize, &data);
        w.finish().unwrap();
        let dem = MultiResDem::new(vec![MmapRaster::open(&p).unwrap()]);
        let (lon, lat) = proj.to_wgs84(cx, cy).unwrap();
        let t = TileId::from_lonlat(lon, lat, 14);
        let png = render_tile(&dem, &proj, t, 256, 0.0).unwrap();
        let dec = png::Decoder::new(std::io::Cursor::new(png));
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        r.next_frame(&mut buf).unwrap();
        assert!((decode([buf[0], buf[1], buf[2]]) - 500.0).abs() < 0.01);
    }
}
