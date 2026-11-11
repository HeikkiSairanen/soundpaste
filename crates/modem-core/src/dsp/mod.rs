//! Signaalinkäsittelyn perusosat: FFT, liukuva taajuusilmaisin, apufunktiot.

pub mod conv;
pub mod fft;
pub mod resample;
pub mod sliding;

/// Tehollisarvo (RMS).
pub fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    let s: f64 = x.iter().map(|&v| (v as f64) * (v as f64)).sum();
    (s / x.len() as f64).sqrt() as f32
}

/// Tehosuhde desibeleinä.
pub fn db10(ratio: f64) -> f64 {
    10.0 * ratio.max(1e-30).log10()
}

/// Amplitudisuhde desibeleistä.
pub fn from_db20(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Goertzel-algoritmi: yhden taajuuden teho lohkossa, |X(f)|² / N².
/// Halvempi kuin FFT, kun kiinnostavia taajuuksia on vain muutama.
pub fn goertzel_power(x: &[f32], freq: f64, fs: f64) -> f64 {
    let w = std::f64::consts::TAU * freq / fs;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s0 = v as f64 + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let p = s1 * s1 + s2 * s2 - c * s1 * s2;
    let n = x.len().max(1) as f64;
    p / (n * n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goertzel_finds_tone() {
        let fs = 48000.0;
        let x: Vec<f32> = (0..480).map(|i| (std::f64::consts::TAU * 2000.0 * i as f64 / fs).sin() as f32).collect();
        let on = goertzel_power(&x, 2000.0, fs);
        let off = goertzel_power(&x, 3000.0, fs);
        // Siniaallon amplitudi 1 -> |X|²/N² = 1/4.
        assert!((on - 0.25).abs() < 0.01, "{on}");
        assert!(off < 1e-6, "{off}");
    }
}
