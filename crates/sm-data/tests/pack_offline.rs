#![allow(clippy::unwrap_used)] // code de test
//! Cycle complet d'un pack contre un serveur simulé : plan → téléchargement
//! avec pannes → reprise → finalisation → réouverture hors ligne.

use sm_data::pack::{self, Category, Manifest, OpenZone, ZoneSpec, ZoneStatus};
use sm_data::{CancellationToken, Endpoints, GeoClient, HttpConfig, Service};
use sm_geo::{Lambert93, TileId};
use std::collections::HashMap;
use std::time::Duration;
use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// Répond un BIL float32 de la taille demandée : pente douce autour de 500 m.
struct Bil;
impl Respond for Bil {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let q: HashMap<String, String> = req.url.query_pairs().into_owned().collect();
        let w: usize = q.get("WIDTH").and_then(|v| v.parse().ok()).unwrap_or(0);
        let h: usize = q.get("HEIGHT").and_then(|v| v.parse().ok()).unwrap_or(0);
        let is_mns = q.get("LAYERS").is_some_and(|l| l.contains("MNS"));
        let base = if is_mns { 505.0f32 } else { 500.0 };
        let body: Vec<u8> = (0..w * h)
            .flat_map(|i| (base + (i % w) as f32 * 0.001).to_le_bytes())
            .collect();
        ResponseTemplate::new(200).set_body_bytes(body)
    }
}

const METEO: &str = r#"{"latitude":44.1,"longitude":3.1,"elevation":357.0,"hourly":{"time":["2026-11-17T22:00"],"temperature_2m":[3.1],"temperature_20m":[4.0],"temperature_50m":[5.2],"temperature_100m":[6.0],"temperature_150m":[6.2],"wind_speed_10m":[1.2],"wind_direction_10m":[200],"cloud_cover":[5],"relative_humidity_2m":[88]}}"#;

async fn mock_services(mns_failures: u64) -> MockServer {
    let server = MockServer::start().await;
    // Pannes : les requêtes MNS échouent `mns_failures` fois.
    Mock::given(method("GET"))
        .and(path("/wms-r"))
        .and(query_param(
            "LAYERS",
            "IGNF_LIDAR-HD_MNS_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93",
        ))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(mns_failures)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/wms-r"))
        .respond_with(Bil)
        .mount(&server)
        .await;
    let buildings = include_str!("../../../tests/data/batiments_millau_extrait.geojson");
    Mock::given(method("GET"))
        .and(path("/wfs/ows"))
        .and(query_param("TYPENAMES", "BDTOPO_V3:batiment"))
        .respond_with(ResponseTemplate::new(200).set_body_string(buildings))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/wfs/ows"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"type":"FeatureCollection","features":[]}"#),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/meteofrance"))
        .respond_with(ResponseTemplate::new(200).set_body_string(METEO))
        .mount(&server)
        .await;
    // OCS GE absente → repli CoSIA attendu.
    Mock::given(method("GET"))
        .and(path_regex("^/tms/"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/wmts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0xFF, 0xD8, 0xFF, 0xE0]))
        .mount(&server)
        .await;
    server
}

fn client(base: &str) -> GeoClient {
    let mut cfg = HttpConfig {
        max_retries: 1,
        base_backoff: Duration::from_millis(1),
        ..HttpConfig::default()
    };
    for s in Service::ALL {
        cfg.limits.insert(s, (10_000, 32));
    }
    GeoClient::new(Endpoints::all_at(base), cfg).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_resume_finalize_reopen() {
    let proj = Lambert93::new().unwrap();
    let spec = ZoneSpec::new(
        "Test hors ligne",
        707_460.0,
        6_333_667.0,
        600.0,
        "2026-11-17",
    );
    let rings = spec.effective_rings();
    assert_eq!(rings.len(), 2);

    // MNS : anneau 0,5 m (2×2 blocs) + anneau 1 m (1200 px → 2×2) = 8 blocs,
    // chacun tenté 2 fois (1 retry) au premier passage.
    let server = mock_services(16).await;
    let c = client(&server.uri());
    let items = pack::build_plan(&spec, c.endpoints(), &proj).unwrap();
    let n_mns = items
        .iter()
        .filter(|i| i.id.starts_with("relief/mns/"))
        .count();
    assert_eq!(n_mns, 8);
    let est = pack::estimate(&items);
    assert!(
        est.categories
            .iter()
            .any(|c| c.category == Category::Relief)
    );

    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(spec.id());
    let mut m = Manifest::new(spec.clone(), items);
    let cancel = CancellationToken::new();

    // Passage 1 : les blocs MNS échouent, le reste passe.
    let r1 = pack::download(&c, &dir, &mut m, &|_| {}, &cancel)
        .await
        .unwrap();
    assert_eq!(r1.failed.len(), n_mns, "{:?}", r1.failed);
    assert_eq!(m.status, ZoneStatus::Partial);
    // Le repli CoSIA n'est pas lancé tant qu'il reste des échecs.
    assert!(!m.items.iter().any(|i| i.id.starts_with("cosia/")));
    assert!(pack::finalize(&dir, &mut m, &|_| {}).is_err());

    // Passage 2 (reprise, après « redémarrage ») : seuls les blocs MNS sont
    // re-téléchargés, puis le repli CoSIA est ajouté.
    let mut m = Manifest::load(&dir).unwrap();
    let r2 = pack::download(&c, &dir, &mut m, &|_| {}, &cancel)
        .await
        .unwrap();
    assert!(r2.failed.is_empty(), "{:?}", r2.failed);
    let n_cosia = m
        .items
        .iter()
        .filter(|i| i.id.starts_with("cosia/"))
        .count();
    assert!(n_cosia > 0);
    assert_eq!(r2.fetched_items, n_mns + n_cosia);
    assert_eq!(m.status, ZoneStatus::Downloaded);

    // Finalisation puis réouverture.
    let steps = std::sync::Mutex::new(Vec::new());
    pack::finalize(&dir, &mut m, &|s| steps.lock().unwrap().push(s.to_owned())).unwrap();
    assert_eq!(m.status, ZoneStatus::Complete);
    let p = m.products.as_ref().unwrap();
    assert_eq!(p.landcover, "cosia");
    assert_eq!(p.buildings.total, 15);
    assert_eq!(p.rings.iter().filter(|r| r.layer == "mnh").count(), 2);
    assert!(p.terrain_tiles > 0);
    assert!(!dir.join("raw/mnt").exists(), "blocs bruts supprimés");

    let t0 = std::time::Instant::now();
    let z = OpenZone::open(&dir).unwrap();
    assert!(t0.elapsed() < Duration::from_secs(2));
    let (lon, lat) = proj.to_wgs84(spec.center_x, spec.center_y).unwrap();
    let h = z.elevation_at(lon, lat).unwrap();
    assert!((h - 500.0).abs() < 2.0, "{h}");
    // MNH ≈ 5 m partout (MNS = MNT + 5).
    let mnh = sm_geo::MmapRaster::open(&dir.join("rasters/mnh_r0.smr")).unwrap();
    assert!((mnh.get(10, 10).unwrap() - 5.0).abs() < 1e-3);
    // Terrain z17 (au-delà de la pré-génération) : calculé à la demande puis en cache.
    let t = TileId::from_lonlat(lon, lat, 17);
    let png = z.terrain_tile(t).unwrap().unwrap();
    assert_eq!(&png[1..4], b"PNG");
    assert!(dir.join(format!("terrain/17/{}/{}.png", t.x, t.y)).exists());
    // Liste des zones.
    let zones = pack::list_zones(root.path());
    assert_eq!(zones.len(), 1);
    assert_eq!(zones[0].status, ZoneStatus::Complete);
}

#[tokio::test]
async fn cancel_stops_download_and_keeps_partial_state() {
    let proj = Lambert93::new().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;
    let c = client(&server.uri());
    let spec = ZoneSpec::new("annul", 707_460.0, 6_333_667.0, 600.0, "2026-11-17");
    let items = pack::build_plan(&spec, c.endpoints(), &proj).unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(spec.id());
    let mut m = Manifest::new(spec, items);
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        c2.cancel();
    });
    let t0 = std::time::Instant::now();
    let r = pack::download(&c, &dir, &mut m, &|_| {}, &cancel).await;
    assert!(matches!(r, Err(sm_data::DataError::Cancelled)));
    assert!(t0.elapsed() < Duration::from_secs(5));
    assert_eq!(Manifest::load(&dir).unwrap().status, ZoneStatus::Partial);
}
