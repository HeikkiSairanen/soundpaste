//! Synteettinen huoneen impulssivaste.
//!
//! Suora ääni on yksikköimpulssi hetkellä 0. Sen perään tulee muutama erillinen
//! varhainen heijastus ja eksponentiaalisesti vaimeneva kohinahäntä, joka on
//! tavanomainen malli diffuusille jälkikaiunnalle. Parametrit:
//! - RT60: aika, jossa kaiku vaimenee 60 dB
//! - DRR: suoran äänen ja kaiun energiasuhde desibeleinä (direct-to-reverberant)
//!
//! Oikeat mitatut impulssivasteet korvaavat tämän, kun niitä on (ks. korpus).

use crate::rng::Rng;

pub fn synth_ir(fs: f64, rt60_s: f64, drr_db: f64, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed ^ 0x1A2B_3C4D);
    let len = ((rt60_s * fs) as usize).max(1);
    let mut h = vec![0.0f32; len];
    // -60 dB amplitudissa = kerroin 10^-3 = exp(-6.908).
    let decay = 6.908 / (rt60_s * fs);
    let start = (0.002 * fs) as usize;
    for (n, v) in h.iter_mut().enumerate().skip(start) {
        *v = rng.gauss() * (-(decay * n as f64)).exp() as f32;
    }
    // Muutama erillinen varhainen heijastus (pöytä, seinä, näyttö).
    for _ in 0..4 {
        let t = rng.range(0.001, 0.02);
        let n = (t * fs) as usize;
        if n < len {
            h[n] += rng.range(-1.0, 1.0) as f32 * 8.0 * (-(decay * n as f64)).exp() as f32;
        }
    }
    let tail_energy: f64 = h.iter().map(|&v| (v as f64) * (v as f64)).sum();
    if tail_energy > 0.0 {
        let target = 10f64.powf(-drr_db / 10.0);
        let g = (target / tail_energy).sqrt() as f32;
        h.iter_mut().for_each(|v| *v *= g);
    }
    h[0] = 1.0;
    h
}

/// Impulssivasteen energia (Σ h²). Valkoisen signaalin teho kasvaa tällä kertoimella.
pub fn energy(h: &[f32]) -> f64 {
    h.iter().map(|&v| (v as f64) * (v as f64)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drr_and_decay_are_as_requested() {
        let fs = 48000.0;
        let h = synth_ir(fs, 0.4, 6.0, 1);
        let tail = energy(&h[1..]);
        assert!((10.0 * (1.0 / tail).log10() - 6.0).abs() < 0.1);
        // Energia ensimmäisellä ja toisella puoliskolla: vaimenemisen pitää näkyä selvästi.
        let a = energy(&h[1..h.len() / 2]);
        let b = energy(&h[h.len() / 2..]);
        assert!(a > 100.0 * b);
    }
}
