//! Plan de téléchargement d'un pack et estimation du volume et du temps.

use super::spec::ZoneSpec;
use crate::http::{Endpoints, Service};
use crate::wfs::layers;
use crate::wmsr::{self, ElevationLayer};
use crate::{BboxL93, Result, openmeteo, tms, wmts};
use serde::{Deserialize, Serialize};
use sm_geo::tiles::tiles_covering_disk;
use sm_geo::{Lambert93, TileId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Relief,
    Vecteurs,
    OccupationSol,
    Meteo,
    Ortho,
}

/// Comment récupérer un item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Fetch {
    /// GET simple. `expect_len` valide la taille (BIL) ; `allow_missing`
    /// accepte un 404 (tuile hors couverture).
    Bytes {
        url: String,
        service: Service,
        expect_len: Option<u64>,
        allow_missing: bool,
    },
    /// Couche WFS complète dans l'emprise (pagination).
    WfsLayer { typename: String, bbox: BboxL93 },
}

/// Bloc d'altitude (pour l'assemblage à la finalisation).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BlockRef {
    pub layer: ElevationLayer,
    pub ring: usize,
    /// Position du bloc en pixels dans le raster de l'anneau.
    pub col0: u32,
    pub row0: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    pub id: String,
    pub category: Category,
    pub fetch: Fetch,
    /// Chemin relatif au dossier de la zone.
    pub dest: String,
    pub est_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<BlockRef>,
}

/// Volume estimé des couches WFS, en octets par km² (ordre de grandeur rural).
fn wfs_bytes_per_km2(typename: &str) -> u64 {
    match typename {
        layers::BATIMENT => 150_000,
        layers::ROUTE => 80_000,
        layers::VEGETATION => 60_000,
        layers::FILOSOFI => 30_000,
        layers::ZNIEFF1 | layers::ZNIEFF2 => 40_000,
        _ => 8_000,
    }
}

/// Couches vectorielles du pack.
pub const WFS_LAYERS: [(&str, &str); 11] = [
    ("batiment", layers::BATIMENT),
    ("zone_de_vegetation", layers::VEGETATION),
    ("surface_hydrographique", layers::SURFACE_HYDRO),
    ("plan_d_eau", layers::PLAN_D_EAU),
    ("troncon_de_route", layers::ROUTE),
    ("troncon_de_voie_ferree", layers::VOIE_FERREE),
    ("zone_d_activite_ou_d_interet", layers::ZONE_ACTIVITE),
    ("parc_ou_reserve", layers::PARC_RESERVE),
    ("filosofi_200m", layers::FILOSOFI),
    ("znieff1", layers::ZNIEFF1),
    ("znieff2", layers::ZNIEFF2),
];

const ORTHO_TILE_BYTES: u64 = 20_000;
const OCSGE_TILE_BYTES: u64 = 4_000;
const COSIA_TILE_BYTES: u64 = 25_000;

/// Construit le plan complet d'une zone.
pub fn build_plan(spec: &ZoneSpec, ep: &Endpoints, proj: &Lambert93) -> Result<Vec<PlanItem>> {
    let mut items = Vec::new();
    let (cx, cy, radius) = (spec.center_x, spec.center_y, spec.radius_m);

    // 1. Relief : blocs WMS-R par couche et par anneau.
    for layer in &spec.elevation_layers {
        for (ri, ring) in spec.effective_rings().iter().enumerate() {
            let rb = spec.ring_bbox(ring);
            let w_px = (rb.width() / ring.cell_m).round() as u32;
            let h_px = (rb.height() / ring.cell_m).round() as u32;
            let bp = spec.block_px.max(1);
            for by in 0..h_px.div_ceil(bp) {
                for bx in 0..w_px.div_ceil(bp) {
                    let (col0, row0) = (bx * bp, by * bp);
                    let w = bp.min(w_px - col0);
                    let h = bp.min(h_px - row0);
                    let xmin = rb.xmin + f64::from(col0) * ring.cell_m;
                    let ymax = rb.ymax - f64::from(row0) * ring.cell_m;
                    let bbox = BboxL93::new(
                        xmin,
                        ymax - f64::from(h) * ring.cell_m,
                        xmin + f64::from(w) * ring.cell_m,
                        ymax,
                    );
                    if bbox.distance_to(cx, cy) > radius {
                        continue; // coin du carré hors du disque d'étude
                    }
                    let len = u64::from(w) * u64::from(h) * 4;
                    items.push(PlanItem {
                        id: format!("relief/{}/r{ri}/{bx}_{by}", layer.short()),
                        category: Category::Relief,
                        fetch: Fetch::Bytes {
                            url: wmsr::getmap_url(&ep.geopf, *layer, &bbox, w as usize, h as usize),
                            service: Service::WmsR,
                            expect_len: Some(len),
                            allow_missing: false,
                        },
                        dest: format!("raw/{}/r{ri}/{bx}_{by}.bil", layer.short()),
                        est_bytes: len,
                        block: Some(BlockRef {
                            layer: *layer,
                            ring: ri,
                            col0,
                            row0,
                            w,
                            h,
                        }),
                    });
                }
            }
        }
    }

    // 2. Vecteurs WFS (une couche par requête, pagination côté client).
    let bbox = spec.bbox();
    for (name, typename) in WFS_LAYERS {
        items.push(PlanItem {
            id: format!("wfs/{name}"),
            category: Category::Vecteurs,
            fetch: Fetch::WfsLayer {
                typename: typename.to_owned(),
                bbox,
            },
            dest: format!("raw/wfs/{name}.geojson"),
            est_bytes: (bbox.area_km2() * wfs_bytes_per_km2(typename) as f64) as u64,
            block: None,
        });
    }

    // 3. Météo (Open-Meteo, AROME).
    let (lon, lat) = proj.to_wgs84(cx, cy)?;
    items.push(PlanItem {
        id: "meteo".into(),
        category: Category::Meteo,
        fetch: Fetch::Bytes {
            url: openmeteo::forecast_url(&ep.open_meteo, lat, lon, &spec.meteo_date),
            service: Service::OpenMeteo,
            expect_len: None,
            allow_missing: false,
        },
        dest: "raw/meteo.json".into(),
        est_bytes: 12_000,
        block: None,
    });

    // 4. Occupation du sol : tuiles vectorielles OCS GE.
    for t in tiles_covering_disk(proj, cx, cy, radius, spec.ocsge_zoom)? {
        items.push(tile_item(
            Category::OccupationSol,
            "ocsge",
            t,
            "pbf",
            tms::tile_url(&ep.geopf, &tms::OCSGE_COUVERTURE, t),
            Service::Tms,
            OCSGE_TILE_BYTES,
        ));
    }

    // 5. Ortho haut zoom, pour une vue satellite fluide hors ligne.
    for lvl in &spec.ortho {
        let r = lvl.radius_m.min(radius);
        for t in tiles_covering_disk(proj, cx, cy, r, lvl.zoom)? {
            items.push(tile_item(
                Category::Ortho,
                "ortho",
                t,
                "jpg",
                wmts::tile_url(&ep.geopf, &wmts::ORTHO, t),
                Service::Wmts,
                ORTHO_TILE_BYTES,
            ));
        }
    }
    Ok(items)
}

fn tile_item(
    category: Category,
    dir: &str,
    t: TileId,
    ext: &str,
    url: String,
    service: Service,
    est_bytes: u64,
) -> PlanItem {
    PlanItem {
        id: format!("{dir}/{}/{}/{}", t.z, t.x, t.y),
        category,
        fetch: Fetch::Bytes {
            url,
            service,
            expect_len: None,
            allow_missing: true,
        },
        dest: format!("{dir}/{}/{}/{}.{ext}", t.z, t.x, t.y),
        est_bytes,
        block: None,
    }
}

/// Repli CoSIA (PNG) quand l'OCS GE est absente sur la zone.
pub fn cosia_items(spec: &ZoneSpec, ep: &Endpoints, proj: &Lambert93) -> Result<Vec<PlanItem>> {
    let z = spec.ocsge_zoom.min(wmts::COSIA.max_zoom);
    Ok(
        tiles_covering_disk(proj, spec.center_x, spec.center_y, spec.radius_m, z)?
            .into_iter()
            .map(|t| {
                tile_item(
                    Category::OccupationSol,
                    "cosia",
                    t,
                    "png",
                    wmts::tile_url(&ep.geopf, &wmts::COSIA, t),
                    Service::Wmts,
                    COSIA_TILE_BYTES,
                )
            })
            .collect(),
    )
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CategoryEstimate {
    pub category: Category,
    pub items: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Estimate {
    pub categories: Vec<CategoryEstimate>,
    pub total_items: usize,
    pub total_bytes: u64,
    /// Octets sur disque après finalisation (MNH calculé, terrain, FlatGeobuf).
    pub disk_bytes: u64,
    pub est_seconds: f64,
}

/// Débit descendant effectif supposé (octets/s) pour l'estimation du temps.
/// Mesuré le 17/11/2026 : ~1,1 Mo/s sur un pack de 5 km (débit partagé entre
/// requêtes, latence du WMS-R) ; 2 Mo/s sur une requête isolée.
pub const ASSUMED_BANDWIDTH: f64 = 1.5e6;

/// Latence typique par requête (s), mesurée le 17/11/2026.
fn latency(svc: Service) -> f64 {
    match svc {
        Service::WmsR => 1.0,
        Service::Wfs => 1.5,
        Service::OpenMeteo => 0.5,
        _ => 0.15,
    }
}

/// Estimation du volume et du temps : services en parallèle, chacun borné
/// par son quota (req/s), sa concurrence et la latence ; le tout borné par
/// le débit réseau.
pub fn estimate(items: &[PlanItem]) -> Estimate {
    use std::collections::BTreeMap;
    let mut cats: BTreeMap<Category, (usize, u64)> = BTreeMap::new();
    let mut per_svc: BTreeMap<String, (Service, usize)> = BTreeMap::new();
    let mut relief_bytes = 0u64;
    for it in items {
        let e = cats.entry(it.category).or_default();
        e.0 += 1;
        e.1 += it.est_bytes;
        if it.category == Category::Relief {
            relief_bytes += it.est_bytes;
        }
        let (svc, reqs) = match &it.fetch {
            Fetch::Bytes { service, .. } => (*service, 1),
            // ~1 page par tranche de 5000 objets ; on compte 2 requêtes.
            Fetch::WfsLayer { .. } => (Service::Wfs, 2),
        };
        per_svc.entry(format!("{svc:?}")).or_insert((svc, 0)).1 += reqs;
    }
    let total_bytes: u64 = cats.values().map(|c| c.1).sum();
    let svc_time = per_svc
        .values()
        .map(|(svc, n)| {
            let (rps, conc) = svc.default_limits();
            let n = *n as f64;
            (n / f64::from(rps)).max(n * latency(*svc) / conc as f64)
        })
        .fold(0.0, f64::max);
    let est_seconds = svc_time.max(total_bytes as f64 / ASSUMED_BANDWIDTH) + 5.0;
    Estimate {
        categories: cats
            .into_iter()
            .map(|(category, (items, bytes))| CategoryEstimate {
                category,
                items,
                bytes,
            })
            .collect(),
        total_items: items.len(),
        total_bytes,
        // MNH calculé (½ du relief MNT+MNS) et blocs bruts supprimés.
        disk_bytes: total_bytes + relief_bytes / 2,
        est_seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(radius: f64) -> Vec<PlanItem> {
        let spec = ZoneSpec::new("t", 707_460.0, 6_333_667.0, radius, "2026-11-17");
        build_plan(&spec, &Endpoints::default(), &Lambert93::new().unwrap()).unwrap()
    }

    #[test]
    fn relief_blocks_for_5km() {
        let p = plan(5000.0);
        let relief: Vec<_> = p
            .iter()
            .filter(|i| i.category == Category::Relief)
            .collect();
        // Par couche : 2×2 blocs à 0,5 m + 4×4 à 1 m + 2×2 à 5 m = 24 ; ×2 couches.
        assert_eq!(relief.len(), 48);
        let bytes: u64 = relief.iter().map(|i| i.est_bytes).sum();
        assert_eq!(bytes, 48 * 4_000_000);
    }

    #[test]
    fn corner_blocks_outside_disk_are_skipped() {
        // Rayon 2,9 km, anneau 5 m : 1160 px → blocs de 1000 + 160 px par côté.
        // Le petit bloc de coin commence à (2100, 2100) m du centre, soit
        // 2970 m > 2900 m : il est exclu.
        let p = plan(2900.0);
        let r = p
            .iter()
            .filter(|i| i.id.starts_with("relief/mnt/r2/"))
            .count();
        assert_eq!(r, 3);
        // Avec des blocs de 1000 px à 1 m sur ±10 km, les coins sont exclus.
        let mut spec = ZoneSpec::new("t", 707_460.0, 6_333_667.0, 10_000.0, "2026-11-17");
        spec.rings = vec![super::super::spec::Ring {
            half_size_m: 10_000.0,
            cell_m: 1.0,
        }];
        spec.elevation_layers = vec![ElevationLayer::Mnt];
        let p = build_plan(&spec, &Endpoints::default(), &Lambert93::new().unwrap()).unwrap();
        let n = p.iter().filter(|i| i.category == Category::Relief).count();
        assert!(n < 400 && n > 300, "{n}");
    }

    #[test]
    fn plan_contents_and_estimate() {
        let p = plan(5000.0);
        assert_eq!(
            p.iter()
                .filter(|i| i.category == Category::Vecteurs)
                .count(),
            11
        );
        assert_eq!(
            p.iter().filter(|i| i.category == Category::Meteo).count(),
            1
        );
        let ortho = p.iter().filter(|i| i.category == Category::Ortho).count();
        assert!((1500..4000).contains(&ortho), "{ortho}");
        let ids: std::collections::HashSet<_> = p.iter().map(|i| &i.id).collect();
        assert_eq!(ids.len(), p.len(), "identifiants uniques");
        let e = estimate(&p);
        assert_eq!(e.total_items, p.len());
        assert!(
            e.total_bytes > 200_000_000 && e.total_bytes < 400_000_000,
            "{}",
            e.total_bytes
        );
        assert!(
            e.est_seconds > 60.0 && e.est_seconds < 600.0,
            "{}",
            e.est_seconds
        );
    }
}
