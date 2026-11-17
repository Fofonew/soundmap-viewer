//! Critère de sortie de la phase 0 :
//! `cargo run -p sm-data --example probe [lon lat]`
//!
//! Télécharge un MNT LiDAR HD de 1 km² en float32, les bâtiments de la zone,
//! une tuile ortho et le profil météo AROME, puis affiche un résumé.

use anyhow::Context;
use sm_data::{BboxL93, CancellationToken, GeoClient, openmeteo, wfs, wmsr, wmts};
use sm_geo::{Lambert93, TileId};
use std::time::Instant;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,sm_data=info".into()),
        )
        .init();

    let args: Vec<f64> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    // Par défaut : zone test d'ENDPOINTS.md, près de Millau.
    let (lon, lat) = match args.as_slice() {
        [lon, lat, ..] => (*lon, *lat),
        _ => (3.0130, 44.0040),
    };
    let proj = Lambert93::new()?;
    let (x, y) = proj.from_wgs84(lon, lat)?;
    let bbox = BboxL93::around((x / 10.0).round() * 10.0, (y / 10.0).round() * 10.0, 500.0);
    let client = GeoClient::public()?;
    let cancel = CancellationToken::new();
    let t_all = Instant::now();

    println!("SoundMap · probe");
    println!("  point     : {lon:.5}, {lat:.5}  →  L93 {x:.1}, {y:.1}");
    println!(
        "  emprise   : {:.0},{:.0} – {:.0},{:.0} (1 km²)",
        bbox.xmin, bbox.ymin, bbox.xmax, bbox.ymax
    );

    // Les quatre téléchargements sont indépendants : en parallèle.
    let t = Instant::now();
    let mnt = async {
        let t = Instant::now();
        let g = wmsr::get_elevation(
            &client,
            wmsr::ElevationLayer::Mnt,
            &bbox,
            1000,
            1000,
            &cancel,
        )
        .await;
        (g, t.elapsed())
    };
    let bat = async {
        let t = Instant::now();
        let f = wfs::get_all(&client, wfs::layers::BATIMENT, &bbox, &cancel).await;
        (f, t.elapsed())
    };
    let tile = TileId::from_lonlat(lon, lat, 17);
    let ortho = async {
        let t = Instant::now();
        let o = wmts::get_tile(&client, &wmts::ORTHO, tile, &cancel).await;
        (o, t.elapsed())
    };
    let today = sm_data::pack::today_utc();
    let meteo = async {
        let t = Instant::now();
        let m = openmeteo::fetch_day(&client, lat, lon, &today, &cancel).await;
        (m, t.elapsed())
    };
    let ((mnt, t_mnt), (bat, t_bat), (ortho, t_ortho), (meteo, t_meteo)) =
        tokio::join!(mnt, bat, ortho, meteo);
    let wall = t.elapsed();

    let mnt = mnt.context("MNT LiDAR HD")?;
    let (lo, hi) = mnt.min_max().unwrap_or((f32::NAN, f32::NAN));
    println!(
        "\nMNT LiDAR HD (WMS-R, BIL float32)          {:>7.0} ms",
        t_mnt.as_secs_f64() * 1e3
    );
    println!(
        "  {}×{} px à {:.1} m, {:.1} Mo, altitude {lo:.1}–{hi:.1} m, nodata {}",
        mnt.width,
        mnt.height,
        mnt.cell(),
        (mnt.data.len() * 4) as f64 / 1e6,
        mnt.nodata_count()
    );

    let bat = bat.context("bâtiments BD TOPO")?;
    let s = wfs::building_stats(&bat);
    println!(
        "\nBâtiments BD TOPO (WFS)                    {:>7.0} ms",
        t_bat.as_secs_f64() * 1e3
    );
    println!(
        "  {} bâtiments, dont {} d'habitation ({} logements renseignés)",
        s.total, s.residential, s.dwellings
    );

    let ortho = ortho.context("tuile ortho")?;
    println!(
        "\nTuile ortho (WMTS) z{}/{}/{}           {:>7.0} ms",
        tile.z,
        tile.x,
        tile.y,
        t_ortho.as_secs_f64() * 1e3
    );
    match ortho {
        Some(b) => println!(
            "  {:.1} ko, JPEG : {}",
            b.len() as f64 / 1e3,
            b.starts_with(&[0xFF, 0xD8])
        ),
        None => println!("  absente (404)"),
    }

    let (f, _) = meteo.context("météo Open-Meteo / AROME")?;
    println!(
        "\nProfil météo AROME (Open-Meteo) {today}  {:>7.0} ms",
        t_meteo.as_secs_f64() * 1e3
    );
    for hh in [22u8, 2] {
        let Some(p) = f.index_of_hour(hh).and_then(|i| f.profile(i)) else {
            continue;
        };
        let temps: Vec<String> = p
            .temperature
            .iter()
            .map(|(h, t)| format!("{h} m:{t:.1}°C"))
            .collect();
        let winds: Vec<String> = p
            .wind
            .iter()
            .map(|(h, u, _)| format!("{h} m:{u:.1}"))
            .collect();
        let grad = match (p.temperature.first(), p.temperature.last()) {
            (Some((z0, t0)), Some((z1, t1))) if z1 > z0 => (t1 - t0) / f64::from(z1 - z0) * 100.0,
            _ => f64::NAN,
        };
        println!("  {} T  {}", p.time, temps.join("  "));
        println!(
            "  {} U  {} (m/s)",
            " ".repeat(p.time.len()),
            winds.join("  ")
        );
        println!(
            "  {} gradient {grad:+.2} °C/100 m → {}, nébulosité {:.0} %",
            " ".repeat(p.time.len()),
            if grad > 0.0 {
                "INVERSION"
            } else {
                "décroissance normale"
            },
            p.cloud_cover.unwrap_or(f64::NAN)
        );
    }

    println!(
        "\nTotal : {:.2} s (téléchargements en parallèle : {:.2} s)",
        t_all.elapsed().as_secs_f64(),
        wall.as_secs_f64()
    );
    Ok(())
}
