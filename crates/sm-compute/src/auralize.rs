//! Auralisation : ce qu'on entend chez un riverain.
//!
//! Un extrait musical de synthèse (boucle techno : kick, basse, charleston,
//! clap) est filtré par la fonction de transfert dancefloor → point, en
//! pratique les écarts de niveau par bande ΔL_b = L_point,b − L_dancefloor,b.
//! L'égaliseur est appliqué dans le domaine fréquentiel, à phase nulle, avec un
//! gain interpolé linéairement en dB sur l'échelle log des fréquences entre les
//! centres de bande.
//! Le bruit résiduel (bruit rose) est ajouté au niveau relatif
//! L_résiduel − L_point (dB(A)), pour juger l'émergence à l'oreille.
//!
//! Deux extraits WAV (mono, 16 bits, 22 050 Hz) : dancefloor, et point à un
//! niveau d'écoute normalisé (la différence absolue de niveau est indiquée
//! à part). Approximation : les rapports de niveau sont fixés sur les valeurs
//! RMS non pondérées des signaux.

use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;
use serde::Serialize;

pub const SAMPLE_RATE: u32 = 22_050;

/// Générateur pseudo-aléatoire reproductible (xorshift).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }
}

/// Boucle de synthèse (BPM donné), crête normalisée à 1.
pub fn synth_loop(seconds: f32, bpm: f32) -> Vec<f32> {
    let sr = SAMPLE_RATE as f32;
    let n = (seconds * sr) as usize;
    let beat = 60.0 / bpm;
    let mut out = vec![0f32; n];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let tau = std::f32::consts::TAU;
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f32 / sr;
        let tb = t % beat;
        let pos = (t / beat) as u32;
        // Kick : sinus glissant 120 → 45 Hz, décroissance exponentielle.
        let f_k = 45.0 + 75.0 * (-tb * 30.0).exp();
        let phase_k = tau * (45.0 * tb + 75.0 / 30.0 * (1.0 - (-tb * 30.0).exp()));
        let kick = phase_k.sin() * (-tb * 7.0).exp() * if f_k > 0.0 { 1.0 } else { 0.0 };
        // Basse en contretemps (dent de scie 55 Hz adoucie).
        let tb2 = (t + beat / 2.0) % beat;
        let saw = 2.0 * ((55.0 * t) % 1.0) - 1.0;
        let bass = 0.45 * saw * (1.0 - (-tb2 * 60.0).exp()) * (-tb2 * 5.0).exp();
        // Charleston en contretemps : bruit bref.
        let hat = if tb2 < 0.04 {
            0.18 * rng.next() * (-tb2 * 120.0).exp()
        } else {
            0.0
        };
        // Clap sur les temps 2 et 4.
        let clap = if pos % 2 == 1 && tb < 0.12 {
            0.3 * rng.next() * (-tb * 30.0).exp()
        } else {
            0.0
        };
        *o = 0.9 * kick + bass + hat + clap;
    }
    // Charleston/clap : bruit blanc brut ; le filtrage par bande corrige le spectre.
    normalize_peak(&mut out, 1.0);
    out
}

/// Bruit rose (filtre de Paul Kellet), RMS unité.
pub fn pink_noise(n: usize, seed: u64) -> Vec<f32> {
    let mut rng = Rng(seed | 1);
    let (mut b0, mut b1, mut b2, mut b3, mut b4, mut b5, mut b6) =
        (0f32, 0f32, 0f32, 0f32, 0f32, 0f32, 0f32);
    let mut out: Vec<f32> = (0..n)
        .map(|_| {
            let w = rng.next();
            b0 = 0.99886 * b0 + w * 0.0555179;
            b1 = 0.99332 * b1 + w * 0.0750759;
            b2 = 0.96900 * b2 + w * 0.153_852;
            b3 = 0.86650 * b3 + w * 0.3104856;
            b4 = 0.55000 * b4 + w * 0.5329522;
            b5 = -0.7616 * b5 - w * 0.0168980;
            let v = b0 + b1 + b2 + b3 + b4 + b5 + b6 + w * 0.5362;
            b6 = w * 0.115926;
            v
        })
        .collect();
    let r = rms(&out).max(1e-9);
    out.iter_mut().for_each(|v| *v /= r);
    out
}

pub fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn normalize_peak(x: &mut [f32], peak: f32) {
    let m = x.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-9);
    x.iter_mut().for_each(|v| *v *= peak / m);
}

/// Gain (dB) à la fréquence f, interpolé entre les centres de bande (log f).
fn gain_at(f: f32, centers: &[f64], gains_db: &[f64]) -> f32 {
    if centers.is_empty() {
        return 0.0;
    }
    let lf = f64::from(f.max(1.0)).ln();
    let first = centers[0].ln();
    let last = centers[centers.len() - 1].ln();
    if lf <= first {
        return gains_db[0] as f32;
    }
    if lf >= last {
        return gains_db[gains_db.len() - 1] as f32;
    }
    for k in 0..centers.len() - 1 {
        let (a, b) = (centers[k].ln(), centers[k + 1].ln());
        if lf <= b {
            let t = (lf - a) / (b - a);
            return (gains_db[k] * (1.0 - t) + gains_db[k + 1] * t) as f32;
        }
    }
    0.0
}

/// Égaliseur à phase nulle (FFT du signal entier).
pub fn equalize(x: &[f32], centers: &[f64], gains_db: &[f64]) -> Vec<f32> {
    let n = x.len().next_power_of_two();
    let mut buf: Vec<Complex32> = x
        .iter()
        .map(|v| Complex32::new(*v, 0.0))
        .chain(std::iter::repeat(Complex32::new(0.0, 0.0)))
        .take(n)
        .collect();
    let mut planner = FftPlanner::<f32>::new();
    planner.plan_fft_forward(n).process(&mut buf);
    let df = SAMPLE_RATE as f32 / n as f32;
    for (i, v) in buf.iter_mut().enumerate() {
        let k = if i <= n / 2 { i } else { n - i };
        let g = 10f32.powf(gain_at(k as f32 * df, centers, gains_db) / 20.0);
        *v *= g;
    }
    planner.plan_fft_inverse(n).process(&mut buf);
    buf.iter().take(x.len()).map(|v| v.re / n as f32).collect()
}

/// WAV PCM 16 bits mono.
pub fn wav(x: &[f32]) -> Vec<u8> {
    let data_len = (x.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + x.len() * 2);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    b.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for v in x {
        b.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    b
}

#[derive(Debug, Clone, Serialize)]
pub struct Auralization {
    /// WAV encodés en base64.
    pub dancefloor_wav: String,
    pub point_wav: String,
    pub dancefloor_dba: f64,
    pub point_dba: f64,
    pub point_dbc: f64,
    pub residual_dba: f64,
    /// Écart de niveau réel dancefloor → point (dB(A)), non reproduit à l'écoute.
    pub level_drop_dba: f64,
    pub distance_m: f64,
}

/// Construit les deux extraits à partir des niveaux par bande au dancefloor
/// et au point.
pub fn render(
    centers: &[f64],
    l_df: &[f64],
    l_pt: &[f64],
    (df_dba, pt_dba, pt_dbc, residual_dba): (f64, f64, f64, f64),
    distance_m: f64,
) -> Auralization {
    let music = synth_loop(8.0, 138.0);
    let gains: Vec<f64> = l_pt.iter().zip(l_df).map(|(p, d)| p - d).collect();
    // Gains relatifs au maximum (le niveau absolu n'est pas reproduit).
    let gmax = gains.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let rel: Vec<f64> = gains.iter().map(|g| (g - gmax).max(-80.0)).collect();
    let mut point = equalize(&music, centers, &rel);
    let rp = rms(&point).max(1e-9);
    let noise = pink_noise(point.len(), 7);
    let k = rp * 10f32.powf(((residual_dba - pt_dba) / 20.0) as f32);
    for (p, nz) in point.iter_mut().zip(&noise) {
        *p += k * nz;
    }
    normalize_peak(&mut point, 0.5);
    let mut df = music;
    normalize_peak(&mut df, 0.5);
    Auralization {
        dancefloor_wav: crate::export::base64(&wav(&df)),
        point_wav: crate::export::base64(&wav(&point)),
        dancefloor_dba: df_dba,
        point_dba: pt_dba,
        point_dbc: pt_dbc,
        residual_dba,
        level_drop_dba: df_dba - pt_dba,
        distance_m,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band_energy(x: &[f32], lo: f32, hi: f32) -> f32 {
        let n = x.len().next_power_of_two();
        let mut buf: Vec<Complex32> = x
            .iter()
            .map(|v| Complex32::new(*v, 0.0))
            .chain(std::iter::repeat(Complex32::new(0.0, 0.0)))
            .take(n)
            .collect();
        FftPlanner::<f32>::new()
            .plan_fft_forward(n)
            .process(&mut buf);
        let df = SAMPLE_RATE as f32 / n as f32;
        buf.iter()
            .take(n / 2)
            .enumerate()
            .filter(|(i, _)| (lo..hi).contains(&(*i as f32 * df)))
            .map(|(_, v)| v.norm_sqr())
            .sum()
    }

    #[test]
    fn equalizer_applies_band_gains() {
        let x = pink_noise(SAMPLE_RATE as usize * 2, 3);
        let centers = [63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0];
        let gains = [0.0, 0.0, 0.0, -20.0, -40.0, -40.0, -40.0];
        let y = equalize(&x, &centers, &gains);
        let d = |lo, hi| 10.0 * (band_energy(&y, lo, hi) / band_energy(&x, lo, hi)).log10();
        assert!(d(50.0, 80.0).abs() < 0.5, "{}", d(50.0, 80.0));
        assert!(
            (d(1400.0, 2800.0) + 40.0).abs() < 1.0,
            "{}",
            d(1400.0, 2800.0)
        );
    }

    #[test]
    fn synth_has_bass_and_wav_header() {
        let x = synth_loop(2.0, 138.0);
        assert!(band_energy(&x, 30.0, 120.0) > band_energy(&x, 2000.0, 8000.0));
        let w = wav(&x);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(w.len(), 44 + x.len() * 2);
    }
}
