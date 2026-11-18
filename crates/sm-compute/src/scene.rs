//! Scène de calcul chargée depuis un pack de zone (lecture locale uniquement) :
//! MNT et MNH memory-mappés, bâti et occupation du sol rasterisés, bâtiments,
//! population, routes, lieux sensibles, ZNIEFF, météo.

use crate::mvt;
use crate::{ComputeError, Result};
use serde_json::Value;
use sm_core::ground::GroundClass;
use sm_data::openmeteo::MeteoForecast;
use sm_data::pack::finalize::open_dem;
use sm_data::pack::manifest::Manifest;
use sm_geo::raster::{MmapRaster, MultiResDem};
use sm_geo::tiles::tile_frac_to_lonlat;
use sm_geo::{BboxL93, Lambert93};
use std::path::{Path, PathBuf};

/// Raster en mémoire, ordre ligne, origine en haut à gauche (Lambert-93).
#[derive(Debug, Clone)]
pub struct Grid<T> {
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub w: usize,
    pub h: usize,
    pub data: Vec<T>,
}

impl<T: Copy> Grid<T> {
    pub fn new(b: &BboxL93, cell: f64, fill: T) -> Self {
        let w = (b.width() / cell).ceil().max(1.0) as usize;
        let h = (b.height() / cell).ceil().max(1.0) as usize;
        Self {
            x0: b.xmin,
            y0: b.ymax,
            cell,
            w,
            h,
            data: vec![fill; w * h],
        }
    }

    #[inline]
    pub fn index(&self, x: f64, y: f64) -> Option<usize> {
        let c = ((x - self.x0) / self.cell).floor();
        let r = ((self.y0 - y) / self.cell).floor();
        if c < 0.0 || r < 0.0 {
            return None;
        }
        let (c, r) = (c as usize, r as usize);
        (c < self.w && r < self.h).then_some(r * self.w + c)
    }

    #[inline]
    pub fn get(&self, x: f64, y: f64) -> Option<T> {
        self.index(x, y).map(|i| self.data[i])
    }

    /// Remplissage pair-impair (tous les anneaux ensemble : trous gérés).
    pub fn fill_polygon(&mut self, rings: &[Vec<(f64, f64)>], v: T) {
        let (mut ymin, mut ymax) = (f64::MAX, f64::MIN);
        for p in rings.iter().flatten() {
            ymin = ymin.min(p.1);
            ymax = ymax.max(p.1);
        }
        if ymin > ymax {
            return;
        }
        let r0 = (((self.y0 - ymax) / self.cell).floor().max(0.0)) as usize;
        let r1 = (((self.y0 - ymin) / self.cell).ceil().max(0.0) as usize).min(self.h);
        let mut xs: Vec<f64> = Vec::new();
        for row in r0..r1 {
            let y = self.y0 - (row as f64 + 0.5) * self.cell;
            xs.clear();
            for ring in rings {
                for e in ring.windows(2) {
                    let ((x1, y1), (x2, y2)) = (e[0], e[1]);
                    if (y1 <= y && y2 > y) || (y2 <= y && y1 > y) {
                        xs.push(x1 + (y - y1) / (y2 - y1) * (x2 - x1));
                    }
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            for pair in xs.as_chunks::<2>().0 {
                let c0 = ((pair[0] - self.x0) / self.cell - 0.5).ceil().max(0.0) as usize;
                let c1 = (((pair[1] - self.x0) / self.cell - 0.5).floor() + 1.0).max(0.0) as usize;
                let c1 = c1.min(self.w);
                for c in c0..c1 {
                    self.data[row * self.w + c] = v;
                }
            }
        }
    }

    /// Trace un segment épais (demi-largeur `hw`).
    pub fn draw_segment(&mut self, a: (f64, f64), b: (f64, f64), hw: f64, v: T) {
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        let n = (len / (self.cell * 0.5)).ceil().max(1.0) as usize;
        let k = (hw / self.cell).ceil() as i64;
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            for dy in -k..=k {
                for dx in -k..=k {
                    let (px, py) = (x + dx as f64 * self.cell, y + dy as f64 * self.cell);
                    if (px - x).hypot(py - y) <= hw.max(self.cell * 0.5)
                        && let Some(i) = self.index(px, py)
                    {
                        self.data[i] = v;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Building {
    pub id: String,
    /// Anneaux en Lambert-93 (le premier est l'extérieur).
    pub rings: Vec<Vec<(f64, f64)>>,
    pub height: f64,
    pub residential: bool,
    pub dwellings: u32,
    pub centroid: (f64, f64),
    pub ground: f64,
    /// Habitants estimés (Filosofi réparti au prorata des logements).
    pub inhabitants: f64,
    /// Habitation hors carreau Filosofi peuplé (résidence secondaire, carreau
    /// absent) : habitants estimés à `HOUSEHOLD_SIZE` par logement.
    pub inhabitants_estimated: bool,
    pub usage: String,
}

/// Taille moyenne des ménages (INSEE, France, ≈ 2,2 personnes).
pub const HOUSEHOLD_SIZE: f64 = 2.2;

#[derive(Debug, Clone)]
pub struct SensitivePlace {
    pub name: String,
    pub kind: String,
    pub pos: (f64, f64),
}

#[derive(Debug, Clone)]
pub struct Znieff {
    pub name: String,
    pub level: u8,
    pub rings: Vec<Vec<(f64, f64)>>,
}

#[derive(Debug, Clone)]
pub struct Road {
    pub pts: Vec<(f64, f64)>,
    /// Importance BD TOPO (1 = autoroute/nationale … 5 = desserte).
    pub importance: u8,
}

pub struct Scene {
    pub zone_id: String,
    pub name: String,
    pub dir: PathBuf,
    /// Origine des coordonnées locales = emplacement de la sono (L93).
    pub origin: (f64, f64),
    pub radius: f64,
    pub proj: Lambert93,
    pub dem: MultiResDem,
    pub canopy: MultiResDem,
    /// Hauteur des bâtiments (pas de 0,25 m).
    pub bati: Grid<u8>,
    /// Classe de sol (index de `GroundClass`).
    pub landcover: Grid<u8>,
    pub landcover_source: String,
    pub buildings: Vec<Building>,
    pub sensitive: Vec<SensitivePlace>,
    pub znieff: Vec<Znieff>,
    pub roads: Vec<Road>,
    pub meteo: Option<MeteoForecast>,
    pub fallback_z: f32,
    pub meteo_date: String,
    /// Végétation (voxels LiDAR HD) et rugosité, si téléchargées.
    pub vegetation: Option<crate::vegetation::VegetationGrid>,
    /// État du sol horaire (humidité, neige, pluie), si téléchargé.
    pub soil: Option<sm_data::openmeteo::SoilState>,
    /// Ajustements de la grille de sol (falaises, géologie).
    pub ground_adjust: crate::ground_state::GroundAdjust,
}

impl std::fmt::Debug for Scene {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scene")
            .field("zone_id", &self.zone_id)
            .field("buildings", &self.buildings.len())
            .finish()
    }
}

fn rings_of(geom: &Value) -> Vec<Vec<(f64, f64)>> {
    let to_ring = |r: &Value| -> Vec<(f64, f64)> {
        r.as_array()
            .map(|pts| {
                pts.iter()
                    .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
                    .collect()
            })
            .unwrap_or_default()
    };
    let coords = &geom["coordinates"];
    match geom["type"].as_str() {
        Some("Polygon") => coords
            .as_array()
            .map(|a| a.iter().map(to_ring).collect())
            .unwrap_or_default(),
        Some("MultiPolygon") => coords
            .as_array()
            .map(|polys| {
                polys
                    .iter()
                    .filter_map(Value::as_array)
                    .flat_map(|p| p.iter().map(to_ring))
                    .collect()
            })
            .unwrap_or_default(),
        Some("LineString") => vec![to_ring(coords)],
        Some("MultiLineString") => coords
            .as_array()
            .map(|a| a.iter().map(to_ring).collect())
            .unwrap_or_default(),
        Some("Point") => coords
            .as_array()
            .and_then(|p| Some(vec![vec![(p.first()?.as_f64()?, p.get(1)?.as_f64()?)]]))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn centroid(rings: &[Vec<(f64, f64)>]) -> (f64, f64) {
    let pts: Vec<&(f64, f64)> = rings
        .first()
        .map(|r| r.iter().collect())
        .unwrap_or_default();
    if pts.is_empty() {
        return (0.0, 0.0);
    }
    let n = pts.len() as f64;
    (
        pts.iter().map(|p| p.0).sum::<f64>() / n,
        pts.iter().map(|p| p.1).sum::<f64>() / n,
    )
}

pub fn point_in_rings(rings: &[Vec<(f64, f64)>], x: f64, y: f64) -> bool {
    let mut inside = false;
    for ring in rings {
        for e in ring.windows(2) {
            let ((x1, y1), (x2, y2)) = (e[0], e[1]);
            if ((y1 > y) != (y2 > y)) && x < x1 + (y - y1) / (y2 - y1) * (x2 - x1) {
                inside = !inside;
            }
        }
    }
    inside
}

fn read_features(dir: &Path, name: &str) -> Vec<Value> {
    std::fs::read(dir.join(format!("raw/wfs/{name}.geojson")))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["features"].as_array().cloned())
        .unwrap_or_default()
}

fn str_prop<'a>(p: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| p.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()))
}

/// Correspondance OCS GE (`code_cs`) → classe de sol Nordtest (ADR 0006).
pub fn ocsge_class(code: &str) -> GroundClass {
    match code {
        "CS1.1.1.1" => GroundClass::G,                   // zones bâties
        "CS1.1.1.2" => GroundClass::H,                   // zones non bâties imperméables (routes)
        "CS1.1.2.1" => GroundClass::F,                   // matériaux minéraux (graviers, carrières)
        "CS1.1.2.2" => GroundClass::E,                   // matériaux composites
        "CS1.2.1" => GroundClass::E,                     // sols nus
        "CS1.2.2" => GroundClass::H,                     // surfaces d'eau
        "CS1.2.3" => GroundClass::A,                     // névés, glaciers
        c if c.starts_with("CS2.1.1") => GroundClass::C, // forêts
        _ => GroundClass::D,                             // landes, herbacées, vignes, vergers…
    }
}

impl Scene {
    /// Charge la scène d'une zone finalisée.
    pub fn load(dir: &Path) -> Result<Scene> {
        let t0 = std::time::Instant::now();
        let manifest = Manifest::load(dir)?;
        let products = manifest
            .products
            .clone()
            .ok_or_else(|| ComputeError::Invalid("zone non finalisée".into()))?;
        let proj = Lambert93::new()?;
        let dem = open_dem(dir, &products)?;
        let canopy = MultiResDem::new(
            products
                .rings
                .iter()
                .filter(|r| r.layer == "mnh")
                .filter_map(|r| MmapRaster::open(&dir.join(&r.file)).ok())
                .collect(),
        );
        let spec = &manifest.spec;
        let origin = (spec.center_x, spec.center_y);
        let radius = spec.radius_m;
        let bbox = spec.bbox();
        let fallback_z = products.elevation_range.map_or(0.0, |(a, b)| (a + b) / 2.0);

        // Bâtiments (L93, attributs BD TOPO).
        let mut bati = Grid::new(&bbox, (radius / 2500.0).max(2.0), 0u8);
        let mut buildings = Vec::new();
        for f in read_features(dir, "batiment") {
            let rings = rings_of(&f["geometry"]);
            if rings.is_empty() {
                continue;
            }
            let p = &f["properties"];
            let height = p["hauteur"]
                .as_f64()
                .filter(|h| *h > 0.0)
                .or_else(|| p["nombre_d_etages"].as_f64().map(|n| n * 3.0))
                .unwrap_or(4.0);
            bati.fill_polygon(&rings, (height / 0.25).round().clamp(1.0, 255.0) as u8);
            let c = centroid(&rings);
            buildings.push(Building {
                id: str_prop(p, &["cleabs"]).unwrap_or("").to_owned(),
                residential: sm_data::wfs::is_residential(p),
                dwellings: p["nombre_de_logements"].as_u64().unwrap_or(0) as u32,
                usage: str_prop(p, &["usage_1"]).unwrap_or("").to_owned(),
                ground: f64::from(dem.sample_clamped(c.0, c.1).unwrap_or(fallback_z)),
                centroid: c,
                rings,
                height,
                inhabitants: 0.0,
                inhabitants_estimated: false,
            });
        }

        // Population : Filosofi 200 m réparti sur les habitations du carreau.
        for f in read_features(dir, "filosofi_200m") {
            let rings = rings_of(&f["geometry"]);
            let ind = f["properties"]["ind"].as_f64().unwrap_or(0.0);
            if rings.is_empty() || ind <= 0.0 {
                continue;
            }
            let inside: Vec<usize> = buildings
                .iter()
                .enumerate()
                .filter(|(_, b)| {
                    b.residential && point_in_rings(&rings, b.centroid.0, b.centroid.1)
                })
                .map(|(i, _)| i)
                .collect();
            let weight: f64 = inside
                .iter()
                .map(|&i| f64::from(buildings[i].dwellings.max(1)))
                .sum();
            for i in inside {
                buildings[i].inhabitants += ind * f64::from(buildings[i].dwellings.max(1)) / weight;
            }
        }

        // Habitations sans habitant fiscal (résidences secondaires, carreaux
        // absents) : occupées potentiellement le soir de la fête.
        for b in buildings
            .iter_mut()
            .filter(|b| b.residential && b.inhabitants <= 0.0)
        {
            b.inhabitants = HOUSEHOLD_SIZE * f64::from(b.dwellings.max(1));
            b.inhabitants_estimated = true;
        }

        // Occupation du sol.
        let mut landcover = Grid::new(
            &bbox,
            (radius / 1000.0).clamp(5.0, 10.0),
            GroundClass::D.index(),
        );
        let mut lc_source = products.landcover.clone();
        if lc_source == "ocsge" {
            let mut n = 0usize;
            for it in manifest.items.iter().filter(|i| i.id.starts_with("ocsge/")) {
                let Ok(bytes) = std::fs::read(dir.join(&it.dest)) else {
                    continue;
                };
                if bytes.is_empty() {
                    continue;
                }
                let parts: Vec<u32> = it
                    .id
                    .split('/')
                    .skip(1)
                    .filter_map(|s| s.parse().ok())
                    .collect();
                let [z, tx, ty] = parts[..] else { continue };
                let Some(layers) = mvt::decode(&bytes) else {
                    continue;
                };
                for layer in layers.iter().filter(|l| l.name == "occupation_sol") {
                    let ext = f64::from(layer.extent.max(1));
                    for feat in &layer.features {
                        let Some(code) = feat
                            .props
                            .iter()
                            .find(|(k, _)| k == "code_cs")
                            .map(|(_, v)| v)
                        else {
                            continue;
                        };
                        let cls = ocsge_class(code);
                        if cls == GroundClass::D {
                            continue; // valeur par défaut déjà présente
                        }
                        let rings: Vec<Vec<(f64, f64)>> = feat
                            .rings
                            .iter()
                            .map(|r| {
                                r.iter()
                                    .filter_map(|(px, py)| {
                                        let (lon, lat) = tile_frac_to_lonlat(
                                            f64::from(tx) + px / ext,
                                            f64::from(ty) + py / ext,
                                            z as u8,
                                        );
                                        proj.from_wgs84(lon, lat).ok()
                                    })
                                    .collect()
                            })
                            .collect();
                        landcover.fill_polygon(&rings, cls.index());
                        n += 1;
                    }
                }
            }
            tracing::info!(polygones = n, "occupation du sol OCS GE rasterisée");
        } else {
            // Sans OCS GE : forêts déduites du MNH (> 5 m).
            lc_source = format!("{lc_source} + MNH");
            for r in 0..landcover.h {
                for c in 0..landcover.w {
                    let x = landcover.x0 + (c as f64 + 0.5) * landcover.cell;
                    let y = landcover.y0 - (r as f64 + 0.5) * landcover.cell;
                    if canopy.sample(x, y).is_some_and(|h| h > 5.0) {
                        landcover.data[r * landcover.w + c] = GroundClass::C.index();
                    }
                }
            }
        }
        for name in ["surface_hydrographique", "plan_d_eau"] {
            for f in read_features(dir, name) {
                landcover.fill_polygon(&rings_of(&f["geometry"]), GroundClass::H.index());
            }
        }
        let mut roads = Vec::new();
        for f in read_features(dir, "troncon_de_route") {
            let p = &f["properties"];
            let importance = p["importance"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| p["importance"].as_u64().map(|v| v as u8))
                .unwrap_or(5);
            for line in rings_of(&f["geometry"]) {
                let hw = match importance {
                    1 | 2 => 5.0,
                    3 => 3.5,
                    _ => 2.5,
                };
                if str_prop(p, &["nature"])
                    .is_none_or(|n| !n.contains("Chemin") && !n.contains("Sentier"))
                {
                    for s in line.windows(2) {
                        landcover.draw_segment(s[0], s[1], hw, GroundClass::H.index());
                    }
                }
                roads.push(Road {
                    pts: line,
                    importance,
                });
            }
        }
        for b in &buildings {
            landcover.fill_polygon(&b.rings, GroundClass::G.index());
        }

        // Lieux sensibles.
        let mut sensitive = Vec::new();
        for f in read_features(dir, "zone_d_activite_ou_d_interet") {
            let p = &f["properties"];
            let cat = str_prop(p, &["categorie"]).unwrap_or("");
            let nature = str_prop(p, &["nature", "nature_detaillee"]).unwrap_or("");
            let lower = format!("{cat} {nature}").to_lowercase();
            let keep = cat == "Santé"
                || cat.contains("enseignement")
                || [
                    "élevage", "elevage", "hôpital", "ehpad", "retraite", "école", "crèche",
                    "haras",
                ]
                .iter()
                .any(|k| lower.contains(k));
            if keep {
                let rings = rings_of(&f["geometry"]);
                sensitive.push(SensitivePlace {
                    name: str_prop(p, &["toponyme", "nom"])
                        .unwrap_or(nature)
                        .to_owned(),
                    kind: if nature.is_empty() {
                        cat.to_owned()
                    } else {
                        nature.to_owned()
                    },
                    pos: centroid(&rings),
                });
            }
        }
        let mut znieff = Vec::new();
        for (name, level) in [("znieff1", 1u8), ("znieff2", 2u8)] {
            for f in read_features(dir, name) {
                let p = &f["properties"];
                znieff.push(Znieff {
                    name: str_prop(
                        p,
                        &[
                            "nom",
                            "nom_site",
                            "lb_zn",
                            "nom_znieff",
                            "nom_zone",
                            "nom_zn",
                        ],
                    )
                    .unwrap_or("ZNIEFF")
                    .to_owned(),
                    level,
                    rings: rings_of(&f["geometry"]),
                });
            }
        }
        let meteo = std::fs::read(dir.join("meteo.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        tracing::info!(
            ms = t0.elapsed().as_millis() as u64,
            batiments = buildings.len(),
            "scène chargée"
        );
        let ground_data = crate::ground_state::GroundData::load(dir);
        let ground_adjust = crate::ground_state::adjust_landcover(
            &mut landcover,
            &dem,
            ground_data.lithology.as_deref(),
        );
        Ok(Scene {
            zone_id: manifest.id.clone(),
            name: spec.name.clone(),
            dir: dir.to_path_buf(),
            origin,
            radius,
            proj,
            dem,
            canopy,
            bati,
            landcover,
            landcover_source: lc_source,
            buildings,
            sensitive,
            znieff,
            roads,
            meteo,
            fallback_z,
            meteo_date: spec.meteo_date.clone(),
            soil: ground_data.soil,
            ground_adjust,
            vegetation: match crate::vegetation::VegetationGrid::load(dir) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("végétation LiDAR illisible : {e}");
                    None
                }
            },
        })
    }

    /// État du sol à l'heure `hour` de la date de la zone.
    pub fn ground_state(&self, hour: u8) -> Option<crate::ground_state::GroundState> {
        crate::ground_state::state_at(self.soil.as_ref()?, &self.meteo_date, hour)
    }

    pub fn lonlat_to_local(&self, lon: f64, lat: f64) -> Option<(f64, f64)> {
        let (x, y) = self.proj.from_wgs84(lon, lat).ok()?;
        Some((x - self.origin.0, y - self.origin.1))
    }

    pub fn local_to_lonlat(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        self.proj
            .to_wgs84(x + self.origin.0, y + self.origin.1)
            .ok()
    }

    /// Altitude du sol en coordonnées absolues L93.
    pub fn ground_abs(&self, x: f64, y: f64) -> f64 {
        f64::from(self.dem.sample_clamped(x, y).unwrap_or(self.fallback_z))
    }

    pub fn ground_local(&self, x: f64, y: f64) -> f64 {
        self.ground_abs(x + self.origin.0, y + self.origin.1)
    }

    /// Point en coordonnées locales → absolues.
    pub fn abs(&self, x: f64, y: f64) -> (f64, f64) {
        (x + self.origin.0, y + self.origin.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_fill_with_hole() {
        let mut g = Grid::new(&BboxL93::new(0.0, 0.0, 10.0, 10.0), 1.0, 0u8);
        let outer = vec![(1.0, 1.0), (9.0, 1.0), (9.0, 9.0), (1.0, 9.0), (1.0, 1.0)];
        let hole = vec![(4.0, 4.0), (6.0, 4.0), (6.0, 6.0), (4.0, 6.0), (4.0, 4.0)];
        g.fill_polygon(&[outer, hole], 7);
        assert_eq!(g.get(2.5, 2.5), Some(7));
        assert_eq!(g.get(5.0, 5.0), Some(0));
        assert_eq!(g.get(0.5, 0.5), Some(0));
        let filled = g.data.iter().filter(|v| **v == 7).count();
        assert_eq!(filled, 64 - 4);
    }

    #[test]
    fn ocsge_mapping() {
        assert_eq!(ocsge_class("CS2.1.1.2"), GroundClass::C);
        assert_eq!(ocsge_class("CS1.2.2"), GroundClass::H);
        assert_eq!(ocsge_class("CS2.2.1"), GroundClass::D);
    }

    #[test]
    fn segment_drawing() {
        let mut g = Grid::new(&BboxL93::new(0.0, 0.0, 20.0, 20.0), 1.0, 0u8);
        g.draw_segment((2.0, 10.0), (18.0, 10.0), 1.5, 1);
        assert_eq!(g.get(10.0, 10.0), Some(1));
        assert_eq!(g.get(10.0, 11.0), Some(1));
        assert_eq!(g.get(10.0, 14.0), Some(0));
    }
}
