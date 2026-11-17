//! Optimiseur : position, azimut et configuration des subs (tel quel,
//! cardioïde, end-fire). Recherche grossière par la méthode d'ingénierie sur
//! les récepteurs (façades des habitations + centre du dancefloor) ; les
//! meilleurs candidats sont ensuite vérifiés au GFPE par l'application.

use crate::impact::{emergence, facade_points, residual_dba, source_level_for_emergence};
use crate::polar::Air;
use crate::project::{Project, SpeakerSpec, centroid, group_heights};
use crate::render::Metric;
use crate::scene::Scene;
use crate::{CancelFlag, ProgressCounter};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sm_core::atmosphere::{iso9613_1_alpha, sound_speed};
use sm_core::engineering::{BandSpec, PathConfig, Scratch, path_excess};
use sm_core::source::{Program, Source, SpeakerKind, band_energy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubConfig {
    AsIs,
    Cardioid,
    EndFire,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizeSpec {
    pub search_radius_m: f64,
    pub position_step_m: f64,
    pub azimuth_step_deg: f64,
    pub configs: Vec<SubConfig>,
}

impl Default for OptimizeSpec {
    fn default() -> Self {
        Self {
            search_radius_m: 60.0,
            position_step_m: 30.0,
            azimuth_step_deg: 15.0,
            configs: vec![SubConfig::AsIs, SubConfig::Cardioid, SubConfig::EndFire],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub dx: f64,
    pub dy: f64,
    /// Rotation appliquée au système (°).
    pub rotation_deg: f64,
    /// Azimut absolu résultant de la première enceinte (°).
    pub azimuth_deg: f64,
    pub config: SubConfig,
    pub dancefloor_dba: f64,
    pub dancefloor_dbc: f64,
    pub worst_emergence_dba: f64,
    pub margin_db: f64,
    pub score: f64,
    /// Enceintes du candidat (WGS84), prêtes à appliquer au projet.
    pub speakers: Vec<SpeakerSpec>,
    /// Vérification GFPE (scénario prévision) : émergence max et marge.
    #[serde(default)]
    pub gfpe_worst_emergence_dba: Option<f64>,
    #[serde(default)]
    pub gfpe_margin_db: Option<f64>,
}

/// Distance quart d'onde à 63 Hz pour les montages cardioïde / end-fire.
fn quarter_wave(c: f64) -> f64 {
    c / 63.0 / 4.0
}

/// Construit la disposition d'un candidat (coordonnées locales).
pub fn layout(
    base: &[Source],
    dx: f64,
    dy: f64,
    rot_deg: f64,
    cfg: SubConfig,
    c: f64,
) -> Vec<Source> {
    let Some((cx, cy)) = centroid(base) else {
        return Vec::new();
    };
    let (s, co) = rot_deg.to_radians().sin_cos();
    let mut out: Vec<Source> = base
        .iter()
        .map(|src| {
            let (x, y) = (src.x - cx, src.y - cy);
            // Rotation horaire (azimut) autour du barycentre puis translation.
            Source {
                x: cx + dx + x * co + y * s,
                y: cy + dy - x * s + y * co,
                azimuth_deg: (src.azimuth_deg + rot_deg).rem_euclid(360.0),
                ..src.clone()
            }
        })
        .collect();
    let subs: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, s)| s.model.kind == SpeakerKind::Sub)
        .map(|(i, _)| i)
        .collect();
    if subs.is_empty() {
        return out;
    }
    let az = out[subs[0]].azimuth_deg.to_radians();
    let (ax, ay) = (az.sin(), az.cos());
    let d = quarter_wave(c);
    match cfg {
        SubConfig::AsIs => {}
        SubConfig::Cardioid => {
            // Moitié arrière : décalée de d vers l'arrière, inversée, retardée de d/c.
            if subs.len() == 1 {
                let mut rear = out[subs[0]].clone();
                rear.x -= ax * d;
                rear.y -= ay * d;
                rear.polarity = -out[subs[0]].polarity;
                rear.delay_s += d / c;
                out.push(rear);
            } else {
                for &i in subs.iter().skip(subs.len() / 2) {
                    out[i].x -= ax * d;
                    out[i].y -= ay * d;
                    out[i].polarity = -out[i].polarity;
                    out[i].delay_s += d / c;
                }
            }
        }
        SubConfig::EndFire => {
            // Subs alignés sur l'axe, espacés de d ; retard croissant vers l'avant.
            // Ligne centrée sur le barycentre des subs.
            let ns = subs.len() as f64;
            let bx = subs.iter().map(|&i| out[i].x).sum::<f64>() / ns;
            let by = subs.iter().map(|&i| out[i].y).sum::<f64>() / ns;
            let n = subs.len().max(2);
            if subs.len() == 1 {
                let mut front = out[subs[0]].clone();
                front.x += ax * d;
                front.y += ay * d;
                front.delay_s += d / c;
                out.push(front);
            } else {
                for (k, &i) in subs.iter().enumerate() {
                    let off = (k as f64 - (n - 1) as f64 / 2.0) * d;
                    out[i].x = bx + ax * off;
                    out[i].y = by + ay * off;
                    out[i].delay_s = (k as f64) * d / c;
                }
            }
        }
    }
    out
}

/// Convertit des sources locales en enceintes du projet (WGS84).
pub fn to_specs(scene: &Scene, base: &[SpeakerSpec], sources: &[Source]) -> Vec<SpeakerSpec> {
    sources
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let (lon, lat) = scene.local_to_lonlat(s.x, s.y)?;
            let tmpl = base.get(i.min(base.len().saturating_sub(1)))?;
            Some(SpeakerSpec {
                id: if i < base.len() {
                    tmpl.id.clone()
                } else {
                    format!("{}-{}", tmpl.id, i)
                },
                name: if i < base.len() {
                    tmpl.name.clone()
                } else {
                    format!("{} (ajout)", s.model.name)
                },
                model: s.model.clone(),
                lon,
                lat,
                height_m: s.height,
                azimuth_deg: s.azimuth_deg,
                tilt_deg: s.tilt_deg,
                delay_ms: s.delay_s * 1000.0,
                polarity: if s.polarity < 0.0 { -1 } else { 1 },
                gain_db: s.gain_db - 0.0,
                muted: false,
            })
        })
        .collect()
}

struct Rx {
    x: f64,
    y: f64,
    res_a: f64,
    zr: f64,
}

#[allow(clippy::too_many_arguments)]
pub fn optimize(
    scene: &Scene,
    project: &Project,
    sources: &[Source],
    program: &Program,
    bands: &[BandSpec],
    cfg: &PathConfig,
    air: Air,
    spec: &OptimizeSpec,
    cancel: &CancelFlag,
    progress: &ProgressCounter,
) -> Vec<Candidate> {
    if sources.is_empty() {
        return Vec::new();
    }
    let c = sound_speed(air.t_c);
    let wa = Metric::A.weights(bands);
    let wc = Metric::C.weights(bands);
    let alpha: Vec<f64> = bands
        .iter()
        .map(|b| iso9613_1_alpha(b.fc, air.t_c, air.rh, 101.325))
        .collect();
    let thr = project
        .settings
        .thresholds_db
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let thr = if thr.is_finite() { thr } else { 5.0 };
    // Un récepteur par habitation : la façade la plus proche de la sono.
    let receivers: Vec<Rx> = scene
        .buildings
        .iter()
        .filter(|b| b.residential)
        .filter_map(|b| {
            let p = facade_points(&b.rings).into_iter().min_by(|a, bb| {
                let da = (a.0 - scene.origin.0).hypot(a.1 - scene.origin.1);
                let db = (bb.0 - scene.origin.0).hypot(bb.1 - scene.origin.1);
                da.total_cmp(&db)
            })?;
            let (x, y) = (p.0 - scene.origin.0, p.1 - scene.origin.1);
            (x.hypot(y) < scene.radius * 0.95).then(|| Rx {
                x,
                y,
                res_a: residual_dba(scene, project, p.0, p.1),
                zr: scene.ground_local(x, y) + 1.5,
            })
        })
        .collect();
    let df = project.dancefloor_local(scene).map(|p| {
        let n = p.len() as f64;
        (
            p.iter().map(|q| q.0).sum::<f64>() / n,
            p.iter().map(|q| q.1).sum::<f64>() / n,
        )
    });
    let (hs_sub, hs_top) = group_heights(sources);
    let steps = (spec.search_radius_m / spec.position_step_m).floor() as i64;
    let mut positions = Vec::new();
    for iy in -steps..=steps {
        for ix in -steps..=steps {
            let (dx, dy) = (
                ix as f64 * spec.position_step_m,
                iy as f64 * spec.position_step_m,
            );
            if dx.hypot(dy) <= spec.search_radius_m + 1e-6 {
                positions.push((dx, dy));
            }
        }
    }
    let base_c = centroid(sources).unwrap_or((0.0, 0.0));
    let base_az = sources.first().map_or(0.0, |s| s.azimuth_deg);
    // Niveau dancefloor de la configuration actuelle : la contrainte est
    // « pas moins que min(cible, actuel - 1 dB) », pour ne pas sacrifier la
    // marge à une cible que le système ne peut de toute façon pas atteindre.
    let reference = df.map(|d| {
        let mut sc = Scratch::default();
        let nb = bands.len();
        let path = crate::profile::extract_path(
            scene,
            scene.abs(base_c.0, base_c.1),
            scene.abs(d.0, d.1),
            1.0,
        );
        let mut a = vec![0.0; nb];
        let mut b = vec![0.0; nb];
        path_excess(&path, hs_sub, 1.5, bands, cfg, &mut sc, &mut a);
        path_excess(&path, hs_top, 1.5, bands, cfg, &mut sc, &mut b);
        let zr = scene.ground_local(d.0, d.1) + 1.5;
        let r = (d.0 - base_c.0).hypot(d.1 - base_c.1);
        let lvl = |w: &[f64]| {
            let mut e = 0.0;
            for (k, band) in bands.iter().enumerate() {
                let (es, et) = band_energy(
                    sources,
                    [d.0, d.1, zr],
                    band.fc,
                    band.lo,
                    band.hi,
                    band.width_thirds,
                    program,
                    c,
                    3,
                );
                let l = 10.0
                    * (es * 10f64.powf(a[k] / 10.0) + et * 10f64.powf(b[k] / 10.0))
                        .max(1e-12)
                        .log10()
                    - alpha[k] * r;
                e += 10f64.powf((l + w[k]) / 10.0);
            }
            10.0 * e.max(1e-12).log10()
        };
        (
            project.settings.target.dba.min(lvl(&wa) - 1.0),
            project.settings.target.dbc.min(lvl(&wc) - 1.0),
        )
    });
    let mut all: Vec<Candidate> = positions
        .par_iter()
        .flat_map_iter(|&(dx, dy)| {
            if cancel.is_cancelled() {
                return Vec::new();
            }
            let pos = (base_c.0 + dx, base_c.1 + dy);
            let pos_abs = scene.abs(pos.0, pos.1);
            let mut sc = Scratch::default();
            let nb = bands.len();
            // Excès par récepteur (subs, têtes) depuis cette position.
            let mut ex = vec![0.0f64; receivers.len() * 2 * nb];
            let mut out = vec![0.0; nb];
            for (k, rx) in receivers.iter().enumerate() {
                let step = ((rx.x - pos.0).hypot(rx.y - pos.1) / 300.0).clamp(3.0, 15.0);
                let path =
                    crate::profile::extract_path(scene, pos_abs, scene.abs(rx.x, rx.y), step);
                for (g, hs) in [hs_sub, hs_top].into_iter().enumerate() {
                    path_excess(&path, hs, 1.5, bands, cfg, &mut sc, &mut out);
                    ex[(k * 2 + g) * nb..(k * 2 + g + 1) * nb].copy_from_slice(&out);
                }
            }
            let df_ex: Option<(Vec<f64>, Vec<f64>, f64)> = df.map(|d| {
                let path = crate::profile::extract_path(scene, pos_abs, scene.abs(d.0, d.1), 1.0);
                let mut a = vec![0.0; nb];
                let mut b = vec![0.0; nb];
                path_excess(&path, hs_sub, 1.5, bands, cfg, &mut sc, &mut a);
                path_excess(&path, hs_top, 1.5, bands, cfg, &mut sc, &mut b);
                (a, b, scene.ground_local(d.0, d.1) + 1.5)
            });
            let level =
                |srcs: &[Source], x: f64, y: f64, zr: f64, exs: &[f64], ext: &[f64], w: &[f64]| {
                    let r = (x - pos.0).hypot(y - pos.1);
                    let mut e = 0.0;
                    for (b, band) in bands.iter().enumerate() {
                        let (es, et) = band_energy(
                            srcs,
                            [x, y, zr],
                            band.fc,
                            band.lo,
                            band.hi,
                            band.width_thirds,
                            program,
                            c,
                            3,
                        );
                        let l = 10.0
                            * (es * 10f64.powf(exs[b] / 10.0) + et * 10f64.powf(ext[b] / 10.0))
                                .max(1e-12)
                                .log10()
                            - alpha[b] * r;
                        e += 10f64.powf((l + w[b]) / 10.0);
                    }
                    10.0 * e.max(1e-12).log10()
                };
            let mut res = Vec::new();
            let mut az = 0.0;
            while az < 360.0 - 1e-9 {
                for &conf in &spec.configs {
                    let srcs = layout(sources, dx, dy, az, conf, c);
                    let mut worst = f64::MIN;
                    let mut margin = f64::INFINITY;
                    for (k, rx) in receivers.iter().enumerate() {
                        let la = level(
                            &srcs,
                            rx.x,
                            rx.y,
                            rx.zr,
                            &ex[k * 2 * nb..(k * 2 + 1) * nb],
                            &ex[(k * 2 + 1) * nb..(k * 2 + 2) * nb],
                            &wa,
                        );
                        worst = worst.max(emergence(la, rx.res_a));
                        margin = margin.min(source_level_for_emergence(rx.res_a, thr) - la);
                    }
                    let (dfa, dfc) = df_ex
                        .as_ref()
                        .zip(df)
                        .map(|((a, b, z), d)| {
                            (
                                level(&srcs, d.0, d.1, *z, a, b, &wa),
                                level(&srcs, d.0, d.1, *z, a, b, &wc),
                            )
                        })
                        .unwrap_or((f64::NAN, f64::NAN));
                    let deficit = match reference {
                        Some((ra, rc)) if dfa.is_finite() => {
                            (ra - dfa).max(0.0) + (rc - dfc).max(0.0)
                        }
                        _ => 0.0,
                    };
                    let margin = if margin.is_finite() { margin } else { 99.0 };
                    res.push(Candidate {
                        dx,
                        dy,
                        rotation_deg: az,
                        azimuth_deg: (base_az + az).rem_euclid(360.0),
                        config: conf,
                        dancefloor_dba: dfa,
                        dancefloor_dbc: dfc,
                        worst_emergence_dba: if worst > f64::MIN { worst } else { 0.0 },
                        margin_db: margin,
                        score: margin - 3.0 * deficit,
                        speakers: Vec::new(),
                        gfpe_worst_emergence_dba: None,
                        gfpe_margin_db: None,
                    });
                }
                az += spec.azimuth_step_deg;
            }
            progress.inc(1);
            res
        })
        .collect();
    all.sort_by(|a, b| b.score.total_cmp(&a.score));
    all.truncate(10);
    for cand in &mut all {
        let srcs = layout(sources, cand.dx, cand.dy, cand.rotation_deg, cand.config, c);
        let back: Vec<Source> = srcs
            .iter()
            .map(|s| Source {
                gain_db: s.gain_db - project.settings.master_gain_db,
                ..s.clone()
            })
            .collect();
        cand.speakers = to_specs(scene, &project.speakers, &back);
    }
    all
}

/// Nombre de positions explorées (pour la progression).
pub fn position_count(spec: &OptimizeSpec) -> u64 {
    let steps = (spec.search_radius_m / spec.position_step_m).floor() as i64;
    let mut n = 0;
    for iy in -steps..=steps {
        for ix in -steps..=steps {
            if (ix as f64 * spec.position_step_m).hypot(iy as f64 * spec.position_step_m)
                <= spec.search_radius_m + 1e-6
            {
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use sm_core::source::preset;

    fn sub(x: f64, y: f64) -> Source {
        Source {
            x,
            y,
            z: 0.5,
            height: 0.5,
            azimuth_deg: 0.0,
            tilt_deg: 0.0,
            delay_s: 0.0,
            polarity: 1.0,
            gain_db: 0.0,
            model: preset("sub18").unwrap(),
        }
    }

    #[test]
    fn cardioid_layout_rejects_rear() {
        let base = vec![sub(-0.4, 0.0), sub(0.4, 0.0)];
        let card = layout(&base, 0.0, 0.0, 0.0, SubConfig::Cardioid, 343.0);
        assert_eq!(card.len(), 2);
        assert!(card.iter().any(|s| s.polarity < 0.0));
        let p = sm_core::source::program("rose");
        let e = |srcs: &[Source], y: f64| {
            band_energy(srcs, [0.0, y, 0.5], 63.0, 56.0, 70.7, 1, &p, 343.0, 3).0
        };
        let front_rear = 10.0 * (e(&card, 200.0) / e(&card, -200.0)).log10();
        assert!(front_rear > 10.0, "{front_rear}");
    }

    #[test]
    fn rotation_moves_azimuth() {
        let base = vec![sub(0.0, 0.0), sub(1.0, 0.0)];
        let r = layout(&base, 10.0, 0.0, 90.0, SubConfig::AsIs, 343.0);
        assert!((r[0].azimuth_deg - 90.0).abs() < 1e-9);
        assert!((r[0].x - 10.0).abs() < 1e-9 || (r[1].x - 10.0).abs() < 1.0);
    }

    #[test]
    fn endfire_aligns_subs_on_axis() {
        let base = vec![sub(-1.0, 0.0), sub(0.0, 0.0), sub(1.0, 0.0)];
        let ef = layout(&base, 0.0, 0.0, 0.0, SubConfig::EndFire, 343.0);
        assert!(ef.iter().all(|s| s.x.abs() < 1e-9));
        assert!(ef[2].delay_s > ef[0].delay_s);
    }
}
