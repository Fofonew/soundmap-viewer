use criterion::{Criterion, criterion_group, criterion_main};
use sm_data::pack::{ZoneSpec, build_plan, estimate};
use sm_data::{Endpoints, wmsr};
use sm_geo::Lambert93;
use std::hint::black_box;

fn benches(c: &mut Criterion) {
    let bytes: Vec<u8> = (0..1_000_000u32)
        .flat_map(|i| (500.0f32 + i as f32 * 1e-4).to_le_bytes())
        .collect();
    c.bench_function("decode_bil_1000x1000", |b| {
        b.iter(|| black_box(wmsr::decode_bil(&bytes, 1000, 1000).ok()))
    });

    let Ok(proj) = Lambert93::new() else { return };
    let spec = ZoneSpec::new("bench", 707_460.0, 6_333_667.0, 5000.0, "2026-11-17");
    let ep = Endpoints::default();
    c.bench_function("build_plan_5km", |b| {
        b.iter(|| black_box(build_plan(&spec, &ep, &proj).ok().map(|p| estimate(&p))))
    });
}

criterion_group!(data, benches);
criterion_main!(data);
