//! Praticabilité d'un site de fête : clairière (pas de bois, vigne, verger,
//! lande ligneuse) et accès carrossable (BD TOPO), autour d'un candidat du
//! criblage. Géométries en Lambert-93.
//!
//! Recherche dans un rayon `search_m` (pas de 25 m) du point qui :
//! - est dégagé : disque de rayon `clear_radius_m` échantillonné tous les
//!   25 m, hors bois et forêts (et, au choix, vignes, vergers, landes, haies) ;
//! - a un accès : distance à la voie admise la plus proche ≤ `max_access_m`
//!   (route, route empierrée, chemin au choix ; sentiers, escaliers, pistes
//!   cyclables et voies « physiquement impossibles » toujours exclus).
//!
//! Coût = distance d'accès + pénalité de type de voie + 0,3 × déplacement.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Type d'accès, du meilleur au moins bon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessKind {
    /// Route revêtue.
    Route,
    /// Route empierrée (piste carrossable).
    Piste,
    /// Chemin (agricole ou forestier, 4×4 conseillé).
    Chemin,
}

impl AccessKind {
    /// Pénalité ajoutée à la distance d'accès (m).
    fn penalty_m(self) -> f64 {
        match self {
            Self::Route => 0.0,
            Self::Piste => 50.0,
            Self::Chemin => 120.0,
        }
    }
}

/// Classe un tronçon BD TOPO ; `None` si non carrossable.
pub fn access_kind(props: &Value) -> Option<(AccessKind, bool)> {
    let nature = props.get("nature").and_then(Value::as_str).unwrap_or("");
    let acces = props
        .get("acces_vehicule_leger")
        .and_then(Value::as_str)
        .unwrap_or("");
    if acces.contains("impossible") {
        return None;
    }
    let restricted = acces.contains("Restreint") || acces.contains("ayants");
    let kind = match nature {
        n if n.starts_with("Route à") || n == "Bretelle" || n == "Rond-point" => AccessKind::Route,
        "Route empierrée" => AccessKind::Piste,
        "Chemin" => AccessKind::Chemin,
        // Type autoroutier : pas d'arrêt possible ; sentier, escalier, etc.
        _ => return None,
    };
    Some((kind, restricted))
}

/// Végétation ou culture incompatible avec une fête (BD TOPO `zone_de_vegetation`).
/// Bois et forêts fermées bloquent toujours ; forêt ouverte (clairsemée), vignes
/// et vergers, landes et haies selon `spec`.
pub fn blocks_site(props: &Value, spec: &SiteSpec) -> bool {
    let nature = props.get("nature").and_then(Value::as_str).unwrap_or("");
    if nature == "Forêt ouverte" {
        return spec.block_open_forest;
    }
    nature.starts_with("Forêt")
        || matches!(nature, "Bois" | "Peupleraie")
        || (spec.block_crops && matches!(nature, "Verger" | "Vigne"))
        || (spec.block_scrub && matches!(nature, "Lande ligneuse" | "Haie"))
}

type Ring = Vec<(f64, f64)>;

/// Polygone avec trous et boîte englobante.
#[derive(Debug, Clone)]
pub struct Poly {
    outer: Ring,
    holes: Vec<Ring>,
    bb: (f64, f64, f64, f64),
}

fn ring_of(v: &Value) -> Option<Ring> {
    let r: Ring = v
        .as_array()?
        .iter()
        .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
        .collect();
    (r.len() >= 3).then_some(r)
}

fn poly_of(rings: &Value) -> Option<Poly> {
    let a = rings.as_array()?;
    let outer = ring_of(a.first()?)?;
    let bb = outer
        .iter()
        .fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |b, p| {
            (b.0.min(p.0), b.1.min(p.1), b.2.max(p.0), b.3.max(p.1))
        });
    Some(Poly {
        outer,
        holes: a.iter().skip(1).filter_map(ring_of).collect(),
        bb,
    })
}

/// Polygones d'une géométrie GeoJSON (Polygon ou MultiPolygon).
pub fn polygons(geom: &Value) -> Vec<Poly> {
    let c = geom.get("coordinates");
    match geom.get("type").and_then(Value::as_str) {
        Some("Polygon") => c.and_then(poly_of).into_iter().collect(),
        Some("MultiPolygon") => c
            .and_then(Value::as_array)
            .map(|ps| ps.iter().filter_map(poly_of).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Lignes d'une géométrie GeoJSON (LineString ou MultiLineString).
pub fn lines(geom: &Value) -> Vec<Ring> {
    let c = geom.get("coordinates");
    let line = |v: &Value| -> Option<Ring> {
        let r: Ring = v
            .as_array()?
            .iter()
            .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
            .collect();
        (r.len() >= 2).then_some(r)
    };
    match geom.get("type").and_then(Value::as_str) {
        Some("LineString") => c.and_then(line).into_iter().collect(),
        Some("MultiLineString") => c
            .and_then(Value::as_array)
            .map(|ls| ls.iter().filter_map(line).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn in_ring(r: &Ring, x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut j = r.len() - 1;
    for i in 0..r.len() {
        let (xi, yi) = r[i];
        let (xj, yj) = r[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

impl Poly {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.bb.0
            && x <= self.bb.2
            && y >= self.bb.1
            && y <= self.bb.3
            && in_ring(&self.outer, x, y)
            && !self.holes.iter().any(|h| in_ring(h, x, y))
    }
}

fn seg_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

/// Voie carrossable.
#[derive(Debug, Clone)]
pub struct Road {
    pub kind: AccessKind,
    pub restricted: bool,
    pub line: Ring,
}

/// Données locales autour d'un candidat.
#[derive(Debug, Clone, Default)]
pub struct SiteData {
    pub blocked: Vec<Poly>,
    pub roads: Vec<Road>,
}

impl SiteData {
    /// Végétation bloquante et voies admises selon `spec`.
    pub fn from_features(vegetation: &[Value], roads: &[Value], spec: &SiteSpec) -> Self {
        let blocked = vegetation
            .iter()
            .filter(|f| f.get("properties").is_some_and(|p| blocks_site(p, spec)))
            .filter_map(|f| f.get("geometry"))
            .flat_map(polygons)
            .collect();
        let roads = roads
            .iter()
            .filter_map(|f| {
                let (kind, restricted) = access_kind(f.get("properties")?)?;
                if !spec.allows(kind) || (restricted && !spec.allow_restricted) {
                    return None;
                }
                Some((kind, restricted, lines(f.get("geometry")?)))
            })
            .flat_map(|(kind, restricted, ls)| {
                ls.into_iter().map(move |line| Road {
                    kind,
                    restricted,
                    line,
                })
            })
            .collect();
        Self { blocked, roads }
    }

    pub fn is_blocked(&self, x: f64, y: f64) -> bool {
        self.blocked.iter().any(|p| p.contains(x, y))
    }

    /// Voie la plus intéressante : (coût, distance, voie).
    fn best_access(&self, x: f64, y: f64) -> Option<(f64, f64, &Road)> {
        self.roads
            .iter()
            .map(|r| {
                let d = r
                    .line
                    .windows(2)
                    .map(|w| seg_dist((x, y), w[0], w[1]))
                    .fold(f64::MAX, f64::min);
                let cost = d + r.kind.penalty_m() + if r.restricted { 100.0 } else { 0.0 };
                (cost, d, r)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// Part (%) de végétation bloquante dans un disque de rayon `r`.
    pub fn blocked_pct(&self, x: f64, y: f64, r: f64) -> f64 {
        let step = r / 6.0;
        let (mut n, mut b) = (0u32, 0u32);
        for i in -6..=6 {
            for j in -6..=6 {
                let (dx, dy) = (f64::from(i) * step, f64::from(j) * step);
                if dx.hypot(dy) <= r {
                    n += 1;
                    if self.is_blocked(x + dx, y + dy) {
                        b += 1;
                    }
                }
            }
        }
        100.0 * f64::from(b) / f64::from(n.max(1))
    }
}

/// Critères du site, réglables dans l'interface.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteSpec {
    /// Rayon de la clairière exigée (m).
    pub clear_radius_m: f64,
    /// Distance maximale à une voie admise (m).
    pub max_access_m: f64,
    /// Déplacement maximal autour du candidat (m).
    pub search_m: f64,
    /// Voies admises : route revêtue, route empierrée, chemin.
    pub access_route: bool,
    pub access_piste: bool,
    pub access_chemin: bool,
    /// Voies à accès restreint (ayants droit) admises.
    pub allow_restricted: bool,
    /// Vignes et vergers excluent le site.
    pub block_crops: bool,
    /// Landes ligneuses et haies excluent le site.
    pub block_scrub: bool,
    /// Forêt ouverte (couvert clairsemé, BD TOPO « Forêt ouverte ») exclut le site.
    pub block_open_forest: bool,
}

impl Default for SiteSpec {
    fn default() -> Self {
        Self {
            clear_radius_m: 50.0,
            max_access_m: 300.0,
            search_m: 250.0,
            access_route: true,
            access_piste: true,
            access_chemin: true,
            allow_restricted: true,
            block_crops: true,
            block_scrub: true,
            block_open_forest: true,
        }
    }
}

impl SiteSpec {
    pub fn allows(&self, kind: AccessKind) -> bool {
        match kind {
            AccessKind::Route => self.access_route,
            AccessKind::Piste => self.access_piste,
            AccessKind::Chemin => self.access_chemin,
        }
    }

    /// Bornes raisonnables (une saisie hors limites ne doit pas faire exploser
    /// la grille de recherche ni la requête BD TOPO).
    pub fn clamped(mut self) -> Self {
        self.clear_radius_m = self.clear_radius_m.clamp(10.0, 300.0);
        self.max_access_m = self.max_access_m.clamp(0.0, 2000.0);
        self.search_m = self.search_m.clamp(0.0, 1000.0);
        self
    }
}

/// Points d'échantillonnage d'un disque de rayon `r` : le centre puis des
/// cercles tous les 25 m au plus, points espacés de 25 m au plus.
fn clearing_probe(r: f64) -> Vec<(f64, f64)> {
    let rings = (r / 25.0).ceil().max(2.0) as u32;
    let mut pts = vec![(0.0, 0.0)];
    for k in 1..=rings {
        let rho = r * f64::from(k) / f64::from(rings);
        let n = ((std::f64::consts::TAU * rho / 25.0).ceil() as u32).max(6);
        let phase = if k % 2 == 0 { 0.5 } else { 0.0 };
        for i in 0..n {
            let a = (f64::from(i) + phase) * std::f64::consts::TAU / f64::from(n);
            pts.push((rho * a.cos(), rho * a.sin()));
        }
    }
    pts
}

/// Site retenu autour d'un candidat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub x: f64,
    pub y: f64,
    pub shift_m: f64,
    pub access: AccessKind,
    pub access_restricted: bool,
    pub access_m: f64,
    /// Végétation haute ou cultures pérennes dans 150 m (%).
    pub wooded_pct: f64,
}

/// Pourquoi un candidat est écarté.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reject {
    /// Aucune clairière assez grande.
    Wooded,
    /// Clairières, mais aucune proche d'une voie carrossable.
    NoAccess,
}

/// Meilleur site autour de (cx, cy).
pub fn best_site(data: &SiteData, cx: f64, cy: f64, spec: &SiteSpec) -> Result<Site, Reject> {
    let ring = clearing_probe(spec.clear_radius_m);
    let step = 25.0;
    let k = (spec.search_m / step).floor() as i32;
    let mut any_clear = false;
    let mut best: Option<(f64, Site)> = None;
    for i in -k..=k {
        for j in -k..=k {
            let (dx, dy) = (f64::from(i) * step, f64::from(j) * step);
            let shift = dx.hypot(dy);
            if shift > spec.search_m {
                continue;
            }
            let (x, y) = (cx + dx, cy + dy);
            if ring.iter().any(|&(ox, oy)| data.is_blocked(x + ox, y + oy)) {
                continue;
            }
            any_clear = true;
            let Some((cost, d, road)) = data.best_access(x, y) else {
                continue;
            };
            if d > spec.max_access_m {
                continue;
            }
            let score = cost + 0.3 * shift;
            if best.as_ref().is_none_or(|b| score < b.0) {
                best = Some((
                    score,
                    Site {
                        x,
                        y,
                        shift_m: shift,
                        access: road.kind,
                        access_restricted: road.restricted,
                        access_m: d,
                        wooded_pct: 0.0,
                    },
                ));
            }
        }
    }
    match best {
        Some((_, mut s)) => {
            s.wooded_pct = data.blocked_pct(s.x, s.y, 150.0);
            Ok(s)
        }
        None if any_clear => Err(Reject::NoAccess),
        None => Err(Reject::Wooded),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64, nature: &str) -> Value {
        json!({"properties": {"nature": nature}, "geometry": {"type": "Polygon",
            "coordinates": [[[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]]}})
    }

    fn road(x0: f64, y0: f64, x1: f64, y1: f64, nature: &str, acces: &str) -> Value {
        json!({"properties": {"nature": nature, "acces_vehicule_leger": acces},
            "geometry": {"type": "LineString", "coordinates": [[x0, y0, 0.0], [x1, y1, 0.0]]}})
    }

    #[test]
    fn classification() {
        let p = |n: &str, a: &str| json!({"nature": n, "acces_vehicule_leger": a});
        assert_eq!(
            access_kind(&p("Route à 1 chaussée", "Libre")),
            Some((AccessKind::Route, false))
        );
        assert_eq!(
            access_kind(&p("Chemin", "Restreint aux ayants droit")),
            Some((AccessKind::Chemin, true))
        );
        assert_eq!(access_kind(&p("Sentier", "Physiquement impossible")), None);
        assert_eq!(access_kind(&p("Type autoroutier", "A péage")), None);
        assert!(blocks_site(
            &json!({"nature": "Forêt fermée de feuillus"}),
            &SiteSpec::default()
        ));
        assert!(!blocks_site(
            &json!({"nature": "Prairie"}),
            &SiteSpec::default()
        ));
        let open_ok = SiteSpec {
            block_open_forest: false,
            ..SiteSpec::default()
        };
        assert!(blocks_site(
            &json!({"nature": "Forêt ouverte"}),
            &SiteSpec::default()
        ));
        assert!(!blocks_site(&json!({"nature": "Forêt ouverte"}), &open_ok));
        assert!(blocks_site(
            &json!({"nature": "Forêt fermée mixte"}),
            &open_ok
        ));
        assert!(blocks_site(&json!({"nature": "Bois"}), &open_ok));
    }

    #[test]
    fn forest_everywhere_is_rejected() {
        let d = SiteData::from_features(
            &[square(-1000.0, -1000.0, 1000.0, 1000.0, "Bois")],
            &[road(-500.0, 0.0, 500.0, 0.0, "Route à 1 chaussée", "Libre")],
            &SiteSpec::default(),
        );
        assert_eq!(
            best_site(&d, 0.0, 0.0, &SiteSpec::default()).unwrap_err(),
            Reject::Wooded
        );
    }

    #[test]
    fn clearing_without_road_is_rejected() {
        let d = SiteData::from_features(
            &[],
            &[road(
                0.0,
                2000.0,
                100.0,
                2000.0,
                "Route à 1 chaussée",
                "Libre",
            )],
            &SiteSpec::default(),
        );
        assert_eq!(
            best_site(&d, 0.0, 0.0, &SiteSpec::default()).unwrap_err(),
            Reject::NoAccess
        );
        let d = SiteData::from_features(
            &[],
            &[road(
                -500.0,
                0.0,
                500.0,
                0.0,
                "Sentier",
                "Physiquement impossible",
            )],
            &SiteSpec::default(),
        );
        assert_eq!(
            best_site(&d, 0.0, 0.0, &SiteSpec::default()).unwrap_err(),
            Reject::NoAccess
        );
    }

    #[test]
    fn moves_to_clearing_next_to_road() {
        // Bois au sud (y < 100), clairière au nord, route en y = 260.
        let d = SiteData::from_features(
            &[square(
                -1000.0,
                -1000.0,
                1000.0,
                100.0,
                "Forêt fermée mixte",
            )],
            &[road(
                -500.0,
                260.0,
                500.0,
                260.0,
                "Route empierrée",
                "Libre",
            )],
            &SiteSpec::default(),
        );
        let s = best_site(&d, 0.0, 0.0, &SiteSpec::default()).unwrap();
        assert!(s.y >= 150.0 && s.y <= 250.0, "{s:?}");
        assert_eq!(s.access, AccessKind::Piste);
        assert!(s.access_m <= 110.0, "{s:?}");
        assert!(!d.is_blocked(s.x, s.y));
    }

    #[test]
    fn hole_in_forest_is_clear() {
        let mut f = square(-1000.0, -1000.0, 1000.0, 1000.0, "Bois");
        f["geometry"]["coordinates"]
            .as_array_mut()
            .unwrap()
            .push(json!([
                [-150.0, -150.0],
                [150.0, -150.0],
                [150.0, 150.0],
                [-150.0, 150.0],
                [-150.0, -150.0]
            ]));
        let d = SiteData::from_features(
            &[f],
            &[road(-500.0, 0.0, 500.0, 0.0, "Chemin", "Libre")],
            &SiteSpec::default(),
        );
        assert!(!d.is_blocked(0.0, 0.0));
        assert!(d.is_blocked(500.0, 500.0));
        let s = best_site(&d, 0.0, 0.0, &SiteSpec::default()).unwrap();
        assert_eq!(s.access, AccessKind::Chemin);
    }

    #[test]
    fn criteria_are_honoured() {
        // Vigne partout, chemin à accès restreint en y = 0.
        let vigne = [square(-1000.0, -1000.0, 1000.0, 1000.0, "Vigne")];
        let chemin = [road(
            -500.0,
            0.0,
            500.0,
            0.0,
            "Chemin",
            "Restreint aux ayants droit",
        )];
        let strict = SiteSpec::default();
        let d = SiteData::from_features(&vigne, &chemin, &strict);
        assert_eq!(
            best_site(&d, 0.0, 0.0, &strict).unwrap_err(),
            Reject::Wooded
        );
        let vines_ok = SiteSpec {
            block_crops: false,
            ..SiteSpec::default()
        };
        let d = SiteData::from_features(&vigne, &chemin, &vines_ok);
        assert!(best_site(&d, 0.0, 0.0, &vines_ok).is_ok());
        // Chemins exclus, ou accès restreint refusé : plus d'accès.
        for spec in [
            SiteSpec {
                access_chemin: false,
                ..vines_ok
            },
            SiteSpec {
                allow_restricted: false,
                ..vines_ok
            },
        ] {
            let d = SiteData::from_features(&vigne, &chemin, &spec);
            assert_eq!(
                best_site(&d, 0.0, 0.0, &spec).unwrap_err(),
                Reject::NoAccess
            );
        }
    }

    #[test]
    fn larger_clearing_is_stricter() {
        // Bosquet de 20 m à 80 m du centre : gêne une clairière de 100 m, pas de 50 m.
        let veg = [square(75.0, -10.0, 95.0, 10.0, "Bois")];
        let rd = [road(-500.0, 0.0, 500.0, 0.0, "Route à 1 chaussée", "Libre")];
        let spec = |r: f64| SiteSpec {
            clear_radius_m: r,
            search_m: 0.0,
            ..SiteSpec::default()
        };
        let d = SiteData::from_features(&veg, &rd, &spec(50.0));
        assert!(best_site(&d, 0.0, 0.0, &spec(50.0)).is_ok());
        assert_eq!(
            best_site(&d, 0.0, 0.0, &spec(100.0)).unwrap_err(),
            Reject::Wooded
        );
        assert!(clearing_probe(100.0).len() > clearing_probe(50.0).len());
    }
}
