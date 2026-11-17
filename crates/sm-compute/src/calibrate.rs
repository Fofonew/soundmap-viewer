//! Calage sur mesures (sonomètre, CSV ou GPX) : ajustement bayésien sur grille
//! de la résistivité du sol (facteur sur σ), du gradient de célérité
//! (stabilité + vent) et d'un décalage global de niveau.
//!
//! Modèle : L_mesure = L_modèle(θ) + δ + ε, ε ~ N(0, s²), δ ~ N(0, τ²) intégré
//! analytiquement. Priors : log2(facteur σ) ~ N(0, 1) ; gradients uniformes
//! sur la grille. Postérieur = vraisemblance marginale × prior, normalisé.

use crate::meteo::forecast_profile;
use crate::polar::Air;
use crate::profile::extract_path;
use crate::project::Project;
use crate::receiver::levels_on_path;
use crate::render::{Metric, weighted_level};
use crate::scene::Scene;
use crate::{CancelFlag, ComputeError};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sm_core::atmosphere::sound_speed;
use sm_core::bands::BandSet;
use sm_core::engineering::{BandSpec, PathConfig, Refraction, Scratch};

/// Un point de mesure (WGS84) : LAeq et/ou LCeq.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub lon: f64,
    pub lat: f64,
    pub la: Option<f64>,
    pub lc: Option<f64>,
    pub label: Option<String>,
}

/// Paramètres calés, appliqués au projet (méthode d'ingénierie).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// Facteur multiplicatif sur la résistivité σ du sol.
    pub sigma_scale: f64,
    pub refraction: Refraction,
    /// Décalage global de niveau (dB), ajouté au gain général.
    pub offset_db: f64,
    /// Écart quadratique moyen après calage (dB) et nombre de mesures.
    pub rms_db: f64,
    pub n_points: usize,
}

/// Résumé du postérieur d'un paramètre.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ParamPosterior {
    pub mean: f64,
    pub sd: f64,
    /// Valeur du maximum a posteriori.
    pub map: f64,
}

/// Ajustement en un point de mesure.
#[derive(Debug, Clone, Serialize)]
pub struct PointFit {
    pub lon: f64,
    pub lat: f64,
    pub label: Option<String>,
    pub distance_m: f64,
    pub la: Option<f64>,
    pub lc: Option<f64>,
    pub model_la_before: f64,
    pub model_lc_before: f64,
    pub model_la_after: f64,
    pub model_lc_after: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalibrationReport {
    pub n_points: usize,
    pub n_ignored: usize,
    /// Écart quadratique moyen mesure − modèle (dB), avant et après.
    pub rms_before_db: f64,
    pub rms_after_db: f64,
    pub bias_before_db: f64,
    pub sigma_scale: ParamPosterior,
    pub grad_t: ParamPosterior,
    pub grad_w: ParamPosterior,
    pub offset_db: ParamPosterior,
    pub wind_toward_deg: f64,
    pub calibration: Calibration,
    pub points: Vec<PointFit>,
    pub notes: Vec<String>,
}

/// Réglages du calage.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CalibrationSpec {
    /// Incertitude de mesure + modèle par valeur (dB).
    pub sigma_db: f64,
    /// Écart-type a priori du décalage global (dB).
    pub prior_offset_db: f64,
}

impl Default for CalibrationSpec {
    fn default() -> Self {
        Self {
            sigma_db: 3.0,
            prior_offset_db: 3.0,
        }
    }
}

const SIGMA_SCALES: [f64; 9] = [0.25, 0.354, 0.5, 0.707, 1.0, 1.414, 2.0, 2.83, 4.0];
const GRAD_T: [f64; 6] = [0.0, 0.01, 0.02, 0.05, 0.1, 0.2];
const GRAD_W: [f64; 5] = [0.0, 0.01, 0.02, 0.05, 0.1];

// ---------------------------------------------------------------- lecture

fn parse_num(s: &str) -> Option<f64> {
    let t = s.trim().trim_matches('"').replace(',', ".");
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// CSV avec en-tête : colonnes lat/latitude, lon/lng/longitude,
/// la/laeq/dba, lc/lceq/dbc, et facultativement name/label/id.
/// Séparateur `,`, `;` ou tabulation ; virgule décimale acceptée avec `;`.
pub fn parse_csv(text: &str) -> Result<Vec<Measurement>, ComputeError> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| ComputeError::Invalid("CSV vide".into()))?;
    let sep = if header.contains(';') {
        ';'
    } else if header.contains('\t') {
        '\t'
    } else {
        ','
    };
    let cols: Vec<String> = header
        .split(sep)
        .map(|c| {
            c.trim()
                .trim_matches('"')
                .to_lowercase()
                .replace([' ', '_', '(', ')'], "")
        })
        .collect();
    let find = |names: &[&str]| cols.iter().position(|c| names.contains(&c.as_str()));
    let i_lat = find(&["lat", "latitude", "y"])
        .ok_or_else(|| ComputeError::Invalid("colonne latitude absente".into()))?;
    let i_lon = find(&["lon", "lng", "long", "longitude", "x"])
        .ok_or_else(|| ComputeError::Invalid("colonne longitude absente".into()))?;
    let i_la = find(&[
        "la",
        "laeq",
        "dba",
        "leqa",
        "leqdba",
        "spla",
        "lequivalenta",
    ]);
    let i_lc = find(&["lc", "lceq", "dbc", "leqc", "leqdbc", "splc"]);
    if i_la.is_none() && i_lc.is_none() {
        return Err(ComputeError::Invalid(
            "aucune colonne de niveau (LAeq / LCeq)".into(),
        ));
    }
    let i_label = find(&["name", "label", "id", "nom", "point"]);
    let mut out = Vec::new();
    for line in lines {
        let f: Vec<&str> = line.split(sep).collect();
        let get = |i: Option<usize>| i.and_then(|i| f.get(i)).and_then(|v| parse_num(v));
        let (Some(lat), Some(lon)) = (get(Some(i_lat)), get(Some(i_lon))) else {
            continue;
        };
        let (la, lc) = (get(i_la), get(i_lc));
        if la.is_none() && lc.is_none() {
            continue;
        }
        out.push(Measurement {
            lon,
            lat,
            la,
            lc,
            label: i_label
                .and_then(|i| f.get(i))
                .map(|s| s.trim().trim_matches('"').to_string()),
        });
    }
    Ok(out)
}

fn attr(tag: &str, name: &str) -> Option<f64> {
    let k = format!("{name}=");
    let i = tag.find(&k)? + k.len();
    let rest = &tag[i..];
    let q = rest.chars().next()?;
    let end = rest[1..].find(q)?;
    parse_num(&rest[1..=end])
}

fn inner<'a>(body: &'a str, names: &[&str]) -> Option<&'a str> {
    for n in names {
        for open in [format!("<{n}>"), format!(":{n}>")] {
            if let Some(i) = body.find(&open) {
                let rest = &body[i + open.len()..];
                if let Some(j) = rest.find("</") {
                    return Some(&rest[..j]);
                }
            }
        }
    }
    None
}

/// Premier nombre précédant `marker` (« 62,5 dB(A) ») ou suivant `key=`.
fn level_in_text(text: &str, markers: &[&str]) -> Option<f64> {
    let low = text.to_lowercase();
    for m in markers {
        if let Some(i) = low.find(m) {
            let before = low[..i].trim_end();
            let num: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if let Some(v) = parse_num(&num) {
                return Some(v);
            }
            let after = low[i + m.len()..].trim_start_matches([' ', '=', ':']);
            let num: String = after
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
                .collect();
            if let Some(v) = parse_num(&num) {
                return Some(v);
            }
        }
    }
    None
}

/// GPX : points `<wpt>` ou `<trkpt>` ; niveaux dans des balises d'extension
/// (`<laeq>`, `<la>`, `<dba>`, `<spl>`, `<lceq>`, `<lc>`, `<dbc>`) ou dans le
/// texte de `<name>`, `<desc>`, `<cmt>` (« 62,5 dB(A) », « LCeq=78 »).
pub fn parse_gpx(text: &str) -> Result<Vec<Measurement>, ComputeError> {
    let mut out = Vec::new();
    let mut rest = text;
    loop {
        let next = ["<wpt", "<trkpt"]
            .iter()
            .filter_map(|t| rest.find(t).map(|i| (i, *t)))
            .min_by_key(|(i, _)| *i);
        let Some((i, t)) = next else { break };
        let after = &rest[i..];
        let tag_end = after.find('>').unwrap_or(after.len());
        let tag = &after[..tag_end];
        let close = format!("</{}>", &t[1..]);
        let (body, adv) = if tag.ends_with('/') {
            ("", tag_end)
        } else {
            match after.find(&close) {
                Some(j) => (&after[tag_end..j], j + close.len()),
                None => (&after[tag_end..], after.len()),
            }
        };
        rest = &after[adv.max(1)..];
        let (Some(lat), Some(lon)) = (attr(tag, "lat"), attr(tag, "lon")) else {
            continue;
        };
        let text_fields: String = ["name", "desc", "cmt"]
            .iter()
            .filter_map(|n| inner(body, &[n]))
            .collect::<Vec<_>>()
            .join(" ");
        let la = inner(body, &["laeq", "la", "dba", "spl", "leq"])
            .and_then(parse_num)
            .or_else(|| level_in_text(&text_fields, &["db(a)", "dba", "laeq", "la="]));
        let lc = inner(body, &["lceq", "lc", "dbc"])
            .and_then(parse_num)
            .or_else(|| level_in_text(&text_fields, &["db(c)", "dbc", "lceq", "lc="]));
        if la.is_none() && lc.is_none() {
            continue;
        }
        out.push(Measurement {
            lon,
            lat,
            la,
            lc,
            label: inner(body, &["name"]).map(|s| s.trim().to_string()),
        });
    }
    if out.is_empty() {
        return Err(ComputeError::Invalid(
            "aucun point GPX avec un niveau (LAeq / LCeq)".into(),
        ));
    }
    Ok(out)
}

/// Choisit le lecteur selon l'extension ou le contenu.
pub fn parse_measurements(name: &str, text: &str) -> Result<Vec<Measurement>, ComputeError> {
    if name.to_lowercase().ends_with(".gpx") || text.trim_start().starts_with("<?xml") {
        parse_gpx(text)
    } else {
        parse_csv(text)
    }
}

// ---------------------------------------------------------------- calage

struct Prepared {
    m: Measurement,
    x: f64,
    y: f64,
    path: Vec<sm_core::engineering::PathSample>,
}

/// Niveaux (A, C) modélisés à un point pour une configuration donnée.
#[allow(clippy::too_many_arguments)]
fn model_ac(
    p: &Prepared,
    project: &Project,
    sources: &[sm_core::source::Source],
    program: &sm_core::source::Program,
    bands: &[BandSpec],
    w: (&[f64], &[f64]),
    cfg: &PathConfig,
    air: Air,
    scratch: &mut Scratch,
) -> (f64, f64) {
    let (lv, _) = levels_on_path(
        &p.path,
        sources,
        program,
        bands,
        cfg,
        air,
        p.x,
        p.y,
        project.settings.receiver_height_m,
        None,
        scratch,
    );
    (weighted_level(&lv, w.0), weighted_level(&lv, w.1))
}

/// Cale le modèle sur les mesures. Les points hors de la zone ou à moins de
/// 20 m de la sono sont ignorés.
pub fn calibrate(
    scene: &Scene,
    project: &Project,
    measurements: &[Measurement],
    spec: CalibrationSpec,
    cancel: &CancelFlag,
) -> Result<CalibrationReport, ComputeError> {
    // Modèle « avant » : réglages du projet sans calage antérieur.
    let mut base = project.clone();
    base.settings.calibration = None;
    let sources = base.sources(scene);
    if sources.is_empty() {
        return Err(ComputeError::Invalid("aucune enceinte active".into()));
    }
    let program = sm_core::source::programs()
        .into_iter()
        .find(|p| p.id == base.settings.program)
        .unwrap_or_else(|| sm_core::source::programs().remove(0));
    let bands: Vec<BandSpec> = BandSet::Octave
        .bands()
        .into_iter()
        .map(BandSpec::from_band)
        .collect();
    let wa = Metric::A.weights(&bands);
    let wc = Metric::C.weights(&bands);
    let forecast = forecast_profile(scene, base.settings.meteo_hour);
    let air = forecast.as_ref().map_or(
        Air {
            t_c: 10.0,
            rh: 80.0,
        },
        |p| Air {
            t_c: p.ground_temperature(),
            rh: p.rh,
        },
    );
    let c = sound_speed(air.t_c);
    let wind_toward = forecast
        .as_ref()
        .map(|p| (p.wind_from_deg(10.0) + 180.0).rem_euclid(360.0));
    let mut notes = Vec::new();
    if wind_toward.is_none() {
        notes.push("pas de prévision météo : composante de vent non calée".into());
    }

    let mut ignored = 0;
    let prepared: Vec<Prepared> = measurements
        .iter()
        .filter_map(|m| {
            let (x, y) = scene.lonlat_to_local(m.lon, m.lat)?;
            let d = x.hypot(y);
            if !(20.0..=scene.radius).contains(&d) {
                return None;
            }
            let step = (d / 500.0).max(2.0);
            Some(Prepared {
                m: m.clone(),
                x,
                y,
                path: extract_path(scene, scene.origin, scene.abs(x, y), step),
            })
        })
        .collect();
    ignored += measurements.len() - prepared.len();
    if prepared.is_empty() {
        return Err(ComputeError::Invalid(
            "aucune mesure exploitable dans la zone (hors zone ou à moins de 20 m)".into(),
        ));
    }
    let n_vals: usize = prepared
        .iter()
        .map(|p| usize::from(p.m.la.is_some()) + usize::from(p.m.lc.is_some()))
        .sum();

    let base_cfg = base.path_config(scene);
    let grad_w: &[f64] = if wind_toward.is_some() {
        &GRAD_W
    } else {
        &GRAD_W[..1]
    };
    let combos: Vec<(f64, f64, f64)> = SIGMA_SCALES
        .iter()
        .flat_map(|&s| {
            GRAD_T
                .iter()
                .flat_map(move |&gt| grad_w.iter().map(move |&gw| (s, gt, gw)))
        })
        .collect();
    let refr = |gt: f64, gw: f64| Refraction {
        grad_t: gt,
        grad_w: gw,
        wind_toward_deg: wind_toward.unwrap_or(0.0),
    };
    let cfg_for = |_: &Prepared, s: f64, gt: f64, gw: f64| PathConfig {
        sigma_scale: s * base_cfg.sigma_scale,
        refraction: Some(refr(gt, gw)),
        c,
        ..base_cfg
    };

    // Vraisemblance marginale (δ intégré) pour chaque combinaison.
    let w = 1.0 / spec.sigma_db.powi(2);
    let tau = 1.0 / spec.prior_offset_db.powi(2);
    let evals: Vec<Option<(f64, f64, f64, f64)>> = combos
        .par_iter()
        .map_init(Scratch::default, |sc, &(s, gt, gw)| {
            if cancel.is_cancelled() {
                return None;
            }
            let (mut sr, mut srr) = (0.0, 0.0);
            for p in &prepared {
                let (a, cc) = model_ac(
                    p,
                    &base,
                    &sources,
                    &program,
                    &bands,
                    (&wa, &wc),
                    &cfg_for(p, s, gt, gw),
                    air,
                    sc,
                );
                for (meas, model) in [(p.m.la, a), (p.m.lc, cc)] {
                    if let Some(v) = meas {
                        let r = v - model;
                        sr += r;
                        srr += r * r;
                    }
                }
            }
            let prec = n_vals as f64 * w + tau;
            let delta = sr * w / prec;
            let loglik = -0.5 * (srr * w - (sr * w).powi(2) / prec);
            let prior = -0.5 * s.log2().powi(2);
            Some((loglik + prior, delta, srr, sr))
        })
        .collect();
    if cancel.is_cancelled() {
        return Err(ComputeError::Cancelled);
    }
    let evals: Vec<(f64, f64, f64, f64)> = evals.into_iter().flatten().collect();
    let best = evals.iter().map(|e| e.0).fold(f64::NEG_INFINITY, f64::max);
    let weights: Vec<f64> = evals.iter().map(|e| (e.0 - best).exp()).collect();
    let z: f64 = weights.iter().sum();
    let post = |f: &dyn Fn(usize) -> f64| {
        let mean = weights
            .iter()
            .enumerate()
            .map(|(i, wi)| wi * f(i))
            .sum::<f64>()
            / z;
        let var = weights
            .iter()
            .enumerate()
            .map(|(i, wi)| wi * (f(i) - mean).powi(2))
            .sum::<f64>()
            / z;
        (mean, var.sqrt())
    };
    let i_map = weights
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(i, _)| i);
    let (s_map, gt_map, gw_map) = combos[i_map];
    let (ls_mean, ls_sd) = post(&|i| combos[i].0.log2());
    let (gt_mean, gt_sd) = post(&|i| combos[i].1);
    let (gw_mean, gw_sd) = post(&|i| combos[i].2);
    let (d_mean, d_sd) = post(&|i| evals[i].1);
    let d_map = evals[i_map].1;

    // Niveaux avant / après en chaque point.
    let mut sc = Scratch::default();
    let (mut rb, mut ra, mut bias) = (0.0, 0.0, 0.0);
    let points: Vec<PointFit> = prepared
        .iter()
        .map(|p| {
            let (b_a, b_c) = model_ac(
                p,
                &base,
                &sources,
                &program,
                &bands,
                (&wa, &wc),
                &base_cfg,
                air,
                &mut sc,
            );
            let (a_a, a_c) = model_ac(
                p,
                &base,
                &sources,
                &program,
                &bands,
                (&wa, &wc),
                &cfg_for(p, s_map, gt_map, gw_map),
                air,
                &mut sc,
            );
            let (a_a, a_c) = (a_a + d_map, a_c + d_map);
            for (meas, before, after) in [(p.m.la, b_a, a_a), (p.m.lc, b_c, a_c)] {
                if let Some(v) = meas {
                    rb += (v - before).powi(2);
                    ra += (v - after).powi(2);
                    bias += v - before;
                }
            }
            PointFit {
                lon: p.m.lon,
                lat: p.m.lat,
                label: p.m.label.clone(),
                distance_m: p.x.hypot(p.y),
                la: p.m.la,
                lc: p.m.lc,
                model_la_before: b_a,
                model_lc_before: b_c,
                model_la_after: a_a,
                model_lc_after: a_c,
            }
        })
        .collect();
    let nv = n_vals.max(1) as f64;
    let rms_after = (ra / nv).sqrt();
    if prepared.len() < 5 {
        notes.push(format!(
            "seulement {} points : paramètres peu contraints (écarts-types élevés)",
            prepared.len()
        ));
    }
    if gt_map == 0.0 && gw_map == 0.0 && rms_after > 2.0 * spec.sigma_db {
        notes.push(
            "mesures plus basses que le modèle même sans réfraction : réfraction vers le haut \
             (zone d'ombre) ou bruit résiduel non pris en compte"
                .into(),
        );
    }
    let calibration = Calibration {
        sigma_scale: s_map,
        refraction: refr(gt_map, gw_map),
        offset_db: d_map,
        rms_db: rms_after,
        n_points: prepared.len(),
    };
    Ok(CalibrationReport {
        n_points: prepared.len(),
        n_ignored: ignored,
        rms_before_db: (rb / nv).sqrt(),
        rms_after_db: rms_after,
        bias_before_db: bias / nv,
        sigma_scale: ParamPosterior {
            mean: 2f64.powf(ls_mean),
            sd: ls_sd,
            map: s_map,
        },
        grad_t: ParamPosterior {
            mean: gt_mean,
            sd: gt_sd,
            map: gt_map,
        },
        grad_w: ParamPosterior {
            mean: gw_mean,
            sd: gw_sd,
            map: gw_map,
        },
        offset_db: ParamPosterior {
            mean: d_mean,
            sd: d_sd,
            map: d_map,
        },
        wind_toward_deg: wind_toward.unwrap_or(0.0),
        calibration,
        points,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_semicolon_decimal_comma() {
        let t = "Nom;Latitude;Longitude;LAeq;LCeq\nP1;44,05;3,12;52,5;70\nP2;44,06;3,13;;65\n";
        let m = parse_csv(t).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].la, Some(52.5));
        assert_eq!(m[0].lon, 3.12);
        assert_eq!(m[1].la, None);
        assert_eq!(m[1].lc, Some(65.0));
        assert_eq!(m[0].label.as_deref(), Some("P1"));
    }

    #[test]
    fn csv_comma_with_db_a_header() {
        let t = "lat,lon,dB(A)\n44.0,3.0,48\nbad,line,1\n";
        let m = parse_csv(t).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].la, Some(48.0));
        assert!(parse_csv("lat,lon\n1,2\n").is_err());
    }

    #[test]
    fn gpx_extensions_and_text() {
        let t = r#"<?xml version="1.0"?><gpx><wpt lat="44.1" lon="3.2"><name>Ferme</name>
            <extensions><sm:laeq>51.2</sm:laeq><sm:lceq>68</sm:lceq></extensions></wpt>
            <trk><trkseg><trkpt lat='44.2' lon='3.3'><desc>LAeq=47,5 ; 66 dB(C)</desc></trkpt>
            <trkpt lat="44.3" lon="3.4"></trkpt></trkseg></trk></gpx>"#;
        let m = parse_gpx(t).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].la, m[0].lc), (Some(51.2), Some(68.0)));
        assert_eq!(m[0].label.as_deref(), Some("Ferme"));
        assert_eq!((m[1].la, m[1].lc), (Some(47.5), Some(66.0)));
    }
}
