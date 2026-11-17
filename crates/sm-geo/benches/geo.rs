use criterion::{Criterion, criterion_group, criterion_main};
use sm_geo::raster::{MmapRaster, MultiResDem, RasterHeader, RasterWriter};
use sm_geo::{BboxL93, Lambert93, TileId, terrarium};
use std::hint::black_box;

const CX: f64 = 707_460.0;
const CY: f64 = 6_333_667.0;

fn make_dem(dir: &std::path::Path) -> MultiResDem {
    let mut rings = Vec::new();
    for (i, (half, cell)) in [(500.0, 0.5), (2000.0, 1.0), (5000.0, 5.0)]
        .iter()
        .enumerate()
    {
        let h = RasterHeader::for_bbox(&BboxL93::around(CX, CY, *half), *cell);
        let p = dir.join(format!("r{i}.smr"));
        let Ok(mut w) = RasterWriter::create(&p, h) else {
            continue;
        };
        let data: Vec<f32> = (0..h.height)
            .flat_map(|r| (0..h.width).map(move |c| 500.0 + ((c ^ r) % 97) as f32))
            .collect();
        w.write_block(0, 0, h.width as usize, h.height as usize, &data);
        if w.finish().is_ok()
            && let Ok(r) = MmapRaster::open(&p)
        {
            rings.push(r);
        }
    }
    MultiResDem::new(rings)
}

fn benches(c: &mut Criterion) {
    let Ok(dir) = tempfile::tempdir() else { return };
    let dem = make_dem(dir.path());
    let Ok(proj) = Lambert93::new() else { return };

    c.bench_function("dem_sample_1M", |b| {
        b.iter(|| {
            let mut s = 0.0f32;
            for i in 0..1_000_000u32 {
                let x = CX - 4000.0 + f64::from(i % 1000) * 8.0;
                let y = CY - 4000.0 + f64::from(i / 1000) * 8.0;
                s += dem.sample(x, y).unwrap_or(0.0);
            }
            black_box(s)
        })
    });

    c.bench_function("proj_l93_to_wgs84_100k", |b| {
        b.iter(|| {
            for i in 0..100_000u32 {
                black_box(proj.to_wgs84(CX + f64::from(i), CY).ok());
            }
        })
    });

    let (lon, lat) = proj.to_wgs84(CX, CY).unwrap_or((3.09, 44.1));
    let tile = TileId::from_lonlat(lon, lat, 16);
    c.bench_function("terrarium_tile_256_z16", |b| {
        b.iter(|| black_box(terrarium::render_tile(&dem, &proj, tile, 256, 0.0).ok()))
    });
}

criterion_group!(geo, benches);
criterion_main!(geo);
