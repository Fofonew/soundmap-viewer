//! Détection de vallée autour de la sono et paramètres d'écoulement nocturne
//! (lac d'air froid, vent catabatique) pour un scénario dédié.
//!
//! - Indice de position topographique TPI = z(sono) − z̄(anneau 300–1500 m).
//!   TPI < −10 m : fond de vallée ou cuvette.
//! - Direction aval : plan ajusté au MNT sur un disque de 1,5 km (les versants
//!   transverses se compensent, il reste la pente le long de la vallée).
//! - Lac d'air froid : épaisseur 0,6·|TPI| (20–150 m), déficit 0,04 K/m de
//!   l'épaisseur (2–8 K) ; jet catabatique u_max = 1 + 15·sin(pente) (1–4 m/s),
//!   maximum à 10 m. Ordres de grandeur : Whiteman (2000), ch. 8–9.

use crate::scene::Scene;
use serde::Serialize;
use sm_core::atmosphere::ValleyFlow;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ValleyAnalysis {
    /// Indice de position topographique (m) : < 0 en creux.
    pub tpi_m: f64,
    /// Pente régionale (°) et direction vers l'aval (°).
    pub slope_deg: f64,
    pub downslope_toward_deg: f64,
    pub valley: bool,
}

pub fn analyze(scene: &Scene) -> ValleyAnalysis {
    let (ox, oy) = scene.origin;
    let z = |x: f64, y: f64| scene.dem.sample_clamped(x, y).map(f64::from);
    let z0 = z(ox, oy).unwrap_or(f64::from(scene.fallback_z));
    let mut ring = Vec::new();
    for r in [300.0, 600.0, 1000.0, 1500.0] {
        let r: f64 = f64::min(r, scene.radius);
        for k in 0..24 {
            let a = f64::from(k) * 15f64.to_radians();
            if let Some(v) = z(ox + r * a.sin(), oy + r * a.cos()) {
                ring.push(v);
            }
        }
    }
    let tpi = if ring.is_empty() {
        0.0
    } else {
        z0 - ring.iter().sum::<f64>() / ring.len() as f64
    };
    // Plan z = a + b·x + c·y (moindres carrés) sur une grille de 150 m.
    let rad = 1500f64.min(scene.radius);
    let (mut n, mut sx, mut sy, mut sz, mut sxx, mut syy, mut sxy, mut sxz, mut syz) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let step = 150.0;
    let k = (rad / step) as i32;
    for i in -k..=k {
        for j in -k..=k {
            let (x, y) = (f64::from(i) * step, f64::from(j) * step);
            if x.hypot(y) > rad {
                continue;
            }
            let Some(v) = z(ox + x, oy + y) else { continue };
            n += 1.0;
            sx += x;
            sy += y;
            sz += v;
            sxx += x * x;
            syy += y * y;
            sxy += x * y;
            sxz += x * v;
            syz += y * v;
        }
    }
    let (gx, gy) = if n > 3.0 {
        // Centré : covariances.
        let (mx, my, mz) = (sx / n, sy / n, sz / n);
        let (cxx, cyy, cxy) = (sxx / n - mx * mx, syy / n - my * my, sxy / n - mx * my);
        let (cxz, cyz) = (sxz / n - mx * mz, syz / n - my * mz);
        let det = cxx * cyy - cxy * cxy;
        if det.abs() > 1e-9 {
            ((cxz * cyy - cyz * cxy) / det, (cyz * cxx - cxz * cxy) / det)
        } else {
            (0.0, 0.0)
        }
    } else {
        (0.0, 0.0)
    };
    let slope = gx.hypot(gy).atan().to_degrees();
    // Vers l'aval : opposé au gradient (0° = nord, sens horaire).
    let toward = (-gx).atan2(-gy).to_degrees().rem_euclid(360.0);
    ValleyAnalysis {
        tpi_m: tpi,
        slope_deg: slope,
        downslope_toward_deg: toward,
        valley: tpi < -10.0,
    }
}

impl ValleyAnalysis {
    /// Paramètres d'écoulement nocturne pour le scénario « vallée ».
    pub fn flow(&self) -> ValleyFlow {
        let depth = (0.6 * self.tpi_m.abs()).clamp(20.0, 150.0);
        ValleyFlow {
            pool_depth_m: depth,
            pool_dt_k: (0.04 * depth).clamp(2.0, 8.0),
            kat_umax: (1.0 + 15.0 * self.slope_deg.to_radians().sin()).clamp(1.0, 4.0),
            kat_height_m: 10.0,
            kat_toward_deg: self.downslope_toward_deg,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_scales_with_relief() {
        let a = ValleyAnalysis {
            tpi_m: -80.0,
            slope_deg: 3.0,
            downslope_toward_deg: 200.0,
            valley: true,
        };
        let f = a.flow();
        assert!((f.pool_depth_m - 48.0).abs() < 1e-9);
        assert!((f.pool_dt_k - 2.0).abs() < 1e-9);
        assert!(f.kat_umax > 1.5 && f.kat_umax < 2.0);
        assert_eq!(f.kat_toward_deg, 200.0);
    }
}
