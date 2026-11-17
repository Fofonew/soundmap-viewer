//! Finalisation d'un pack téléchargé : assemblage des rasters `.smr`, MNH
//! local, FlatGeobuf, GeoJSON WGS84 pour l'UI et pré-génération du terrain.

use super::manifest::{Manifest, Products, RingProduct, VectorProduct, ZoneStatus};
use super::plan::WFS_LAYERS;
use crate::tile_cache::write_atomic_sync;
use crate::wfs::{building_stats, is_residential};
use crate::wmsr::{ElevationLayer, decode_bil};
use crate::{DataError, Result, openmeteo};
use serde_json::{Value, json};
use sm_geo::raster::{MmapRaster, MultiResDem, RasterHeader, RasterWriter};
use sm_geo::tiles::{l93_bbox_to_lonlat, tiles_covering_disk};
use sm_geo::{Lambert93, TileId, terrarium};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

pub fn ring_file(layer: ElevationLayer, ring: usize) -> String {
    format!("rasters/{}_r{ring}.smr", layer.short())
}

/// Finalise le pack (opération CPU, à lancer hors du thread UI).
pub fn finalize(
    dir: &Path,
    manifest: &mut Manifest,
    on_step: &(dyn Fn(&str) + Send + Sync),
) -> Result<()> {
    if manifest.status == ZoneStatus::Partial {
        return Err(DataError::Invalid(
            "pack incomplet : terminer le téléchargement avant la finalisation".into(),
        ));
    }
    let t0 = Instant::now();
    let proj = Lambert93::new()?;
    let spec = manifest.spec.clone();
    let rings = spec.effective_rings();
    let mut products = Products::default();
    std::fs::create_dir_all(dir.join("rasters"))?;

    // 1. Assemblage des blocs en rasters tuilés par anneau.
    on_step("assemblage des rasters");
    for layer in &spec.elevation_layers {
        for (ri, ring) in rings.iter().enumerate() {
            let bbox = spec.ring_bbox(ring);
            let header = RasterHeader::for_bbox(&bbox, ring.cell_m);
            let file = ring_file(*layer, ri);
            let mut w = RasterWriter::create(&dir.join(&file), header)?;
            for it in &manifest.items {
                let Some(b) = it.block.filter(|b| b.layer == *layer && b.ring == ri) else {
                    continue;
                };
                let raw = std::fs::read(dir.join(&it.dest))?;
                let data = decode_bil(&raw, b.w as usize, b.h as usize)?;
                w.write_block(
                    i64::from(b.col0),
                    i64::from(b.row0),
                    b.w as usize,
                    b.h as usize,
                    &data,
                );
            }
            w.finish()?;
            products.rings.push(RingProduct {
                layer: layer.short().into(),
                cell_m: ring.cell_m,
                bbox: header.bbox(),
                file,
            });
        }
    }

    // 2. MNH = max(0, MNS − MNT), calculé localement (ADR 0004).
    let has = |l| spec.elevation_layers.contains(&l);
    if has(ElevationLayer::Mnt) && has(ElevationLayer::Mns) {
        on_step("calcul du MNH");
        for (ri, ring) in rings.iter().enumerate() {
            let mnt = MmapRaster::open(&dir.join(ring_file(ElevationLayer::Mnt, ri)))?;
            let mns = MmapRaster::open(&dir.join(ring_file(ElevationLayer::Mns, ri)))?;
            let h = *mnt.header();
            let file = ring_file(ElevationLayer::Mnh, ri);
            let mut w = RasterWriter::create(&dir.join(&file), h)?;
            for row in 0..h.height {
                for col in 0..h.width {
                    if let (Some(t), Some(s)) = (mnt.get(col, row), mns.get(col, row)) {
                        w.set(col, row, (s - t).max(0.0));
                    }
                }
            }
            w.finish()?;
            products.rings.push(RingProduct {
                layer: "mnh".into(),
                cell_m: ring.cell_m,
                bbox: h.bbox(),
                file,
            });
        }
    }
    // Les blocs bruts ne servent plus : on libère la place.
    for layer in &spec.elevation_layers {
        let _ = std::fs::remove_dir_all(dir.join("raw").join(layer.short()));
    }

    // 3. Vecteurs : FlatGeobuf (L93) + bâtiments WGS84 pour l'UI.
    on_step("conversion des vecteurs");
    std::fs::create_dir_all(dir.join("vectors"))?;
    for (name, _) in WFS_LAYERS {
        let src = dir.join(format!("raw/wfs/{name}.geojson"));
        let Ok(bytes) = std::fs::read(&src) else {
            continue;
        };
        let fc: Value = serde_json::from_slice(&bytes)?;
        let feats = fc["features"].as_array().cloned().unwrap_or_default();
        let file = format!("vectors/{name}.fgb");
        if !feats.is_empty()
            && let Err(e) = write_fgb(&bytes, name, &dir.join(&file))
        {
            tracing::warn!(name, err = %e, "FlatGeobuf non écrit (GeoJSON conservé)");
        }
        if name == "batiment" {
            products.buildings = building_stats(&feats);
            let ui = buildings_ui(&feats, &proj)?;
            write_atomic_sync(&dir.join("ui/buildings.geojson"), &serde_json::to_vec(&ui)?)?;
            products.buildings_ui = Some("ui/buildings.geojson".into());
        }
        products.vectors.push(VectorProduct {
            name: name.into(),
            file,
            count: feats.len(),
        });
    }

    // 4. Météo normalisée.
    if let Ok(raw) = std::fs::read(dir.join("raw/meteo.json")) {
        match openmeteo::parse(&raw) {
            Ok(f) => {
                write_atomic_sync(&dir.join("meteo.json"), &serde_json::to_vec_pretty(&f)?)?;
                products.meteo = Some("meteo.json".into());
            }
            Err(e) => tracing::warn!(err = %e, "météo illisible"),
        }
    }

    // 5. Occupation du sol et ortho : inventaire.
    let non_empty = |prefix: &str| {
        manifest
            .items
            .iter()
            .filter(|i| i.id.starts_with(prefix))
            .filter(|i| std::fs::metadata(dir.join(&i.dest)).is_ok_and(|m| m.len() > 0))
            .count()
    };
    products.landcover = if non_empty("ocsge/") > 0 {
        "ocsge".into()
    } else if non_empty("cosia/") > 0 {
        "cosia".into()
    } else {
        "absente".into()
    };
    products.ortho_tiles = non_empty("ortho/");
    let mut zooms: Vec<u8> = spec.ortho.iter().map(|o| o.zoom).collect();
    zooms.sort_unstable();
    zooms.dedup();
    products.ortho_zooms = zooms;

    // 6. Emprises et terrain Terrarium pré-généré.
    let ll = l93_bbox_to_lonlat(&proj, &spec.bbox())?;
    products.lonlat_bounds = [ll.west, ll.south, ll.east, ll.north];
    let (clon, clat) = proj.to_wgs84(spec.center_x, spec.center_y)?;
    products.center_lonlat = [clon, clat];
    let dem = open_dem(dir, &products)?;
    products.elevation_range = elevation_range(&dem);
    let fallback = products.elevation_range.map_or(0.0, |(a, b)| (a + b) / 2.0);
    on_step("génération du terrain 3D");
    let mut tiles = Vec::new();
    for z in 8..=spec.terrain_pregen_max_zoom {
        tiles.extend(tiles_covering_disk(
            &proj,
            spec.center_x,
            spec.center_y,
            spec.radius_m,
            z,
        )?);
    }
    products.terrain_tiles = render_terrain_tiles(dir, &dem, &proj, &tiles, fallback)?;
    products.terrain_pregen_max_zoom = spec.terrain_pregen_max_zoom;

    products.finalize_seconds = t0.elapsed().as_secs_f64();
    manifest.products = Some(products);
    manifest.status = ZoneStatus::Complete;
    manifest.save(dir)?;
    on_step("terminé");
    Ok(())
}

/// Ouvre le MNT multi-résolution d'un pack.
pub fn open_dem(dir: &Path, products: &Products) -> Result<MultiResDem> {
    let rings = products
        .rings
        .iter()
        .filter(|r| r.layer == "mnt")
        .map(|r| MmapRaster::open(&dir.join(&r.file)))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(MultiResDem::new(rings))
}

fn elevation_range(dem: &MultiResDem) -> Option<(f32, f32)> {
    // L'anneau le plus grossier couvre toute la zone : on l'échantillonne.
    let r = dem.rings().last()?;
    let h = r.header();
    let step = (h.width.max(h.height) / 512).max(1);
    let mut acc: Option<(f32, f32)> = None;
    for row in (0..h.height).step_by(step as usize) {
        for col in (0..h.width).step_by(step as usize) {
            if let Some(v) = r.get(col, row) {
                acc = Some(acc.map_or((v, v), |(a, b)| (a.min(v), b.max(v))));
            }
        }
    }
    acc
}

/// Rend des tuiles Terrarium en parallèle (threads natifs) dans `terrain/`.
fn render_terrain_tiles(
    dir: &Path,
    dem: &MultiResDem,
    proj: &Lambert93,
    tiles: &[TileId],
    fallback: f32,
) -> Result<usize> {
    let next = AtomicUsize::new(0);
    let written = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    let first_err: std::sync::Mutex<Option<DataError>> = std::sync::Mutex::new(None);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(t) = tiles.get(i) else { break };
                    let res = terrarium::render_tile(dem, proj, *t, 256, fallback)
                        .map_err(DataError::from)
                        .and_then(|png| write_atomic_sync(&terrain_path(dir, *t), &png));
                    match res {
                        Ok(()) => {
                            written.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            if let Ok(mut g) = first_err.lock() {
                                g.get_or_insert(e);
                            }
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = first_err.into_inner().ok().flatten() {
        return Err(e);
    }
    Ok(written.into_inner())
}

pub fn terrain_path(dir: &Path, t: TileId) -> std::path::PathBuf {
    dir.join(format!("terrain/{}/{}/{}.png", t.z, t.x, t.y))
}

fn write_fgb(geojson: &[u8], name: &str, out: &Path) -> Result<()> {
    use flatgeobuf::{FgbCrs, FgbWriter, FgbWriterOptions, GeometryType};
    use geozero::GeozeroDatasource;
    use geozero::geojson::GeoJsonReader;
    let opts = FgbWriterOptions {
        crs: FgbCrs {
            code: 2154,
            ..Default::default()
        },
        ..Default::default()
    };
    let err = |e: &dyn std::fmt::Display| DataError::Parse(format!("FlatGeobuf {name} : {e}"));
    let mut fgb =
        FgbWriter::create_with_options(name, GeometryType::Unknown, opts).map_err(|e| err(&e))?;
    let mut cursor = std::io::Cursor::new(geojson);
    GeoJsonReader(&mut cursor)
        .process(&mut fgb)
        .map_err(|e| err(&e))?;
    let mut buf = Vec::new();
    fgb.write(&mut buf).map_err(|e| err(&e))?;
    write_atomic_sync(out, &buf)
}

/// Hauteur d'extrusion : `hauteur`, sinon étages × 3 m, sinon 4 m.
fn building_height(p: &Value) -> f64 {
    p.get("hauteur")
        .and_then(Value::as_f64)
        .filter(|h| *h > 0.0)
        .or_else(|| {
            p.get("nombre_d_etages")
                .and_then(Value::as_f64)
                .filter(|n| *n > 0.0)
                .map(|n| n * 3.0)
        })
        .unwrap_or(4.0)
}

fn reproject_coords(v: &Value, proj: &Lambert93) -> Result<Value> {
    match v {
        Value::Array(a) if a.first().is_some_and(Value::is_number) => {
            let x = a.first().and_then(Value::as_f64).unwrap_or(0.0);
            let y = a.get(1).and_then(Value::as_f64).unwrap_or(0.0);
            let (lon, lat) = proj.to_wgs84(x, y)?;
            // 7 décimales ≈ 1 cm : suffisant et compact.
            let r = |f: f64| (f * 1e7).round() / 1e7;
            Ok(json!([r(lon), r(lat)]))
        }
        Value::Array(a) => Ok(Value::Array(
            a.iter()
                .map(|c| reproject_coords(c, proj))
                .collect::<Result<Vec<_>>>()?,
        )),
        other => Ok(other.clone()),
    }
}

/// Bâtiments en WGS84 avec les seuls attributs utiles à l'UI.
pub fn buildings_ui(features: &[Value], proj: &Lambert93) -> Result<Value> {
    let mut out = Vec::with_capacity(features.len());
    for f in features {
        let (Some(g), Some(p)) = (f.get("geometry"), f.get("properties")) else {
            continue;
        };
        let Some(coords) = g.get("coordinates") else {
            continue;
        };
        out.push(json!({
            "type": "Feature",
            "geometry": { "type": g.get("type").cloned().unwrap_or(Value::Null), "coordinates": reproject_coords(coords, proj)? },
            "properties": {
                "id": p.get("cleabs").cloned().unwrap_or(Value::Null),
                "usage": p.get("usage_1").cloned().unwrap_or(Value::Null),
                "nature": p.get("nature").cloned().unwrap_or(Value::Null),
                "logements": p.get("nombre_de_logements").cloned().unwrap_or(Value::Null),
                "h": building_height(p),
                "res": is_residential(p),
            }
        }));
    }
    Ok(json!({ "type": "FeatureCollection", "features": out }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buildings_ui_from_fixture() {
        let fc: Value = serde_json::from_str(include_str!(
            "../../../../tests/data/batiments_millau_extrait.geojson"
        ))
        .unwrap();
        let feats = fc["features"].as_array().unwrap();
        let ui = buildings_ui(feats, &Lambert93::new().unwrap()).unwrap();
        let f0 = &ui["features"][0];
        let c = &f0["geometry"]["coordinates"][0][0][0];
        let lon = c[0].as_f64().unwrap();
        let lat = c[1].as_f64().unwrap();
        assert!(
            (3.0..3.2).contains(&lon) && (44.0..44.2).contains(&lat),
            "{lon} {lat}"
        );
        assert!(f0["properties"]["h"].as_f64().unwrap() > 0.0);
        let n_res = ui["features"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["properties"]["res"] == true)
            .count();
        assert!(n_res >= 1);
    }

    #[test]
    fn fgb_written_from_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = include_bytes!("../../../../tests/data/batiments_millau_extrait.geojson");
        let out = dir.path().join("b.fgb");
        write_fgb(bytes, "batiment", &out).unwrap();
        let b = std::fs::read(out).unwrap();
        assert_eq!(&b[..3], b"fgb");
    }

    #[test]
    fn height_fallbacks() {
        assert_eq!(building_height(&json!({"hauteur": 7.5})), 7.5);
        assert_eq!(
            building_height(&json!({"hauteur": null, "nombre_d_etages": 2})),
            6.0
        );
        assert_eq!(building_height(&json!({})), 4.0);
    }
}
