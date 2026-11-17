//! Non-régression face à NoiseModelling 6.0.0 (CNOSSOS-EU) : les niveaux de
//! référence sont versionnés dans `tests/noisemodelling/out/` (voir
//! docs/VALIDATION-NOISEMODELLING.md pour les écarts attendus et leur origine).

#[path = "../examples/compare_nm.rs"]
mod compare_nm;

use std::path::Path;

#[test]
fn laeq_stays_close_to_noisemodelling() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/noisemodelling/out");
    let gaps = compare_nm::compare(&dir, false).expect("CSV NoiseModelling");
    assert_eq!(gaps.len(), 34);
    for g in &gaps {
        // Homogène : même famille de méthodes → ±3 dB(A).
        // Favorable : écarts de méthode documentés (sol dur > +6 dB chez
        // CNOSSOS, rayons courbes par transformation du relief) → ±6 dB(A).
        let tol = if g.favorable { 6.0 } else { 3.0 };
        assert!(
            g.dlaeq.abs() <= tol,
            "{} {} à {} m : Δ LAeq {:+.1} dB",
            g.scene,
            if g.favorable { "F" } else { "H" },
            g.d,
            g.dlaeq
        );
    }
}
