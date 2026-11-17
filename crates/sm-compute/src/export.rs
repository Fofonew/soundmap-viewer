//! Exports : GeoTIFF float32 (Lambert-93), GeoJSON, rapport HTML autonome.

use crate::impact::ImpactReport;
use crate::project::Project;
use crate::render::Raster;
use crate::scene::Scene;
use crate::sizing::SizingReport;
use crate::{ComputeError, Result};
use serde_json::{Value, json};
use std::io::Write;
use tiff::encoder::{TiffEncoder, colortype};
use tiff::tags::Tag;

/// Écrit un GeoTIFF float32 en EPSG:2154 (nodata = −9999).
pub fn geotiff<W: Write + std::io::Seek>(scene: &Scene, r: &Raster, out: W) -> Result<()> {
    let err = |e: tiff::TiffError| ComputeError::Invalid(format!("GeoTIFF : {e}"));
    let mut enc = TiffEncoder::new(out).map_err(err)?;
    let mut img = enc
        .new_image::<colortype::Gray32Float>(r.w as u32, r.h as u32)
        .map_err(err)?;
    let x0 = scene.origin.0 + r.x0;
    let y0 = scene.origin.1 + r.y0;
    let e = img.encoder();
    // ModelPixelScale, ModelTiepoint, GeoKeyDirectory (GeoTIFF 1.0).
    e.write_tag(Tag::Unknown(33550), &[r.cell, r.cell, 0.0][..])
        .map_err(err)?;
    e.write_tag(Tag::Unknown(33922), &[0.0, 0.0, 0.0, x0, y0, 0.0][..])
        .map_err(err)?;
    let keys: [u16; 16] = [
        1, 1, 0, 3, // version, révision, nombre de clés
        1024, 0, 1, 1, // GTModelType = projeté
        1025, 0, 1, 1, // GTRasterType = PixelIsArea
        3072, 0, 1, 2154, // ProjectedCRS = EPSG:2154
    ];
    e.write_tag(Tag::Unknown(34735), &keys[..]).map_err(err)?;
    e.write_tag(Tag::Unknown(42113), "-9999").map_err(err)?; // GDAL_NODATA
    let data: Vec<f32> = r
        .data
        .iter()
        .map(|v| if v.is_finite() { *v } else { -9999.0 })
        .collect();
    img.write_data(&data).map_err(err)?;
    Ok(())
}

/// GeoJSON : isophones + bâtiments évalués.
pub fn geojson(isophones: &Value, impact: Option<&ImpactReport>) -> Value {
    let mut feats: Vec<Value> = isophones["features"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if let Some(i) = impact {
        for b in &i.buildings {
            feats.push(json!({
                "type": "Feature",
                "geometry": { "type": "Point", "coordinates": [b.lon, b.lat] },
                "properties": {
                    "kind": "habitation", "id": b.id, "niveau_dba": b.level_dba, "niveau_dbc": b.level_dbc,
                    "residuel_dba": b.residual_dba, "emergence_dba": b.emergence_dba,
                    "emergence_dbc": b.emergence_dbc, "classe": b.class, "habitants": b.inhabitants,
                    "marge_db": b.margin_db
                }
            }));
        }
    }
    json!({ "type": "FeatureCollection", "features": feats })
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= c.len() {
                s.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Rapport HTML autonome (imprimable en PDF depuis n'importe quel navigateur).
pub fn html_report(
    project: &Project,
    scene: &Scene,
    impact: Option<&ImpactReport>,
    sizing: Option<&SizingReport>,
    map_png: Option<&[u8]>,
    method: &str,
    uncertainty: &str,
) -> String {
    let mut h = String::new();
    h.push_str("<!doctype html><html lang=\"fr\"><head><meta charset=\"utf-8\"><title>SoundMap – ");
    h.push_str(&esc(&project.name));
    h.push_str("</title><style>body{font-family:system-ui,sans-serif;max-width:960px;margin:2em auto;color:#222}\
        table{border-collapse:collapse;width:100%;font-size:13px}td,th{border-bottom:1px solid #ddd;padding:4px 6px;text-align:left}\
        th{background:#f4f2ee}.n{text-align:right;font-variant-numeric:tabular-nums}.warn{color:#b3261e}\
        img{max-width:100%;border:1px solid #ccc}small{color:#666}</style></head><body>");
    h.push_str(&format!(
        "<h1>Étude d'impact sonore simplifiée – {}</h1><p>Zone « {} », rayon {:.1} km. Usage : recherche (aucun référentiel réglementaire intégré ; seuils choisis par l'utilisateur).</p>",
        esc(&project.name),
        esc(&scene.name),
        scene.radius / 1000.0
    ));
    h.push_str(&format!(
        "<h2>Méthode</h2><p>{}</p><p><strong>Incertitude :</strong> {}</p>",
        esc(method),
        esc(uncertainty)
    ));
    h.push_str("<h2>Système</h2><table><tr><th>Enceinte</th><th>Modèle</th><th class=n>Hauteur</th><th class=n>Azimut</th><th class=n>Retard</th><th class=n>Polarité</th><th class=n>Gain</th></tr>");
    for s in &project.speakers {
        h.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td class=n>{:.1} m</td><td class=n>{:.0}°</td><td class=n>{:.2} ms</td><td class=n>{}</td><td class=n>{:+.1} dB</td></tr>",
            esc(&s.name),
            esc(&s.model.name),
            s.height_m,
            s.azimuth_deg,
            s.delay_ms,
            if s.polarity < 0 { "−" } else { "+" },
            s.gain_db
        ));
    }
    h.push_str("</table>");
    if let Some(png) = map_png {
        h.push_str("<h2>Carte</h2><img alt=\"Carte des niveaux\" src=\"data:image/png;base64,");
        h.push_str(&base64(png));
        h.push_str("\">");
    }
    if let Some(z) = sizing {
        h.push_str(&format!(
            "<h2>Dancefloor</h2><p>Centre : {:.1} dB(A) / {:.1} dB(C) (cible {:.0} / {:.0}). Homogénéité : écart-type {:.1} dB, étendue {:.1} dB.</p>\
             <p>Pour atteindre la cible : {} subs, {} têtes, {:.0} W d'amplification (headroom {:+.1} dB(A), {:+.1} dB(C)).</p>",
            z.center_dba, z.center_dbc, z.target_dba, z.target_dbc, z.uniformity_std_db, z.uniformity_range_db,
            z.subs_needed, z.tops_needed, z.power_w, z.headroom_dba, z.headroom_dbc
        ));
    }
    if let Some(i) = impact {
        h.push_str(&format!(
            "<h2>Impact</h2><p>Seuils d'émergence : {} dB. Marge globale avant le premier seuil : <strong class=\"{}\">{:+.1} dB</strong>. Habitants dans la zone : {:.0}.</p>",
            i.thresholds_db.iter().map(|t| format!("{t:.0}")).collect::<Vec<_>>().join(" / "),
            if i.margin_db < 0.0 { "warn" } else { "" },
            i.margin_db,
            i.inhabitants_total
        ));
        h.push_str("<table><tr><th>Classe d'émergence</th><th class=n>Bâtiments</th><th class=n>Habitants</th></tr>");
        for c in &i.classes {
            h.push_str(&format!(
                "<tr><td>{}</td><td class=n>{}</td><td class=n>{:.0}</td></tr>",
                esc(&c.label),
                c.buildings,
                c.inhabitants
            ));
        }
        h.push_str("</table><h3>Habitations les plus exposées</h3><table><tr><th>Bâtiment</th><th class=n>Distance</th><th class=n>dB(A)</th><th class=n>dB(C)</th><th class=n>Résiduel</th><th class=n>Émergence A</th><th class=n>Émergence C</th><th class=n>Marge</th></tr>");
        for b in i.buildings.iter().take(20) {
            h.push_str(&format!(
                "<tr><td>{} <small>({:.5}, {:.5})</small></td><td class=n>{:.0} m</td><td class=n>{:.1}</td><td class=n>{:.1}</td><td class=n>{:.1}</td><td class=n>{:+.1}</td><td class=n>{:+.1}</td><td class=n>{:+.1}</td></tr>",
                esc(&b.usage), b.lat, b.lon, b.distance_m, b.level_dba, b.level_dbc, b.residual_dba,
                b.emergence_dba, b.emergence_dbc, b.margin_db
            ));
        }
        h.push_str("</table>");
        if !i.sensitive.is_empty() {
            h.push_str("<h3>Lieux sensibles</h3><table><tr><th>Lieu</th><th>Type</th><th class=n>dB(A)</th><th class=n>Émergence</th></tr>");
            for s in &i.sensitive {
                h.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td class=n>{:.1}</td><td class=n>{:+.1}</td></tr>",
                    esc(&s.name),
                    esc(&s.kind),
                    s.level_dba,
                    s.emergence_dba
                ));
            }
            h.push_str("</table>");
        }
        if !i.fauna.is_empty() {
            h.push_str("<h3>Faune (ZNIEFF)</h3><table><tr><th>Zone</th><th class=n>Type</th><th class=n>Surface > 45 dB(A)</th><th class=n>Max</th></tr>");
            for f in &i.fauna {
                h.push_str(&format!(
                    "<tr><td>{}</td><td class=n>{}</td><td class=n>{:.2} km²</td><td class=n>{:.1} dB(A)</td></tr>",
                    esc(&f.name), f.level, f.area_above_45_km2, f.max_dba
                ));
            }
            h.push_str("</table>");
        }
    }
    h.push_str("<p><small>Données : IGN Géoplateforme (LiDAR HD, BD TOPO, OCS GE), INSEE Filosofi, PatriNat (ZNIEFF), Météo-France AROME via Open-Meteo. Généré par SoundMap.</small></p></body></html>");
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
