//! Liukuva taajuusilmaisin ("sliding DFT bin").
//!
//! Sekoitetaan tulosignaali kompleksisella eksponentilla taajuudelle f ja
//! pidetään yllä viimeisten N näytteen summaa. Summan itseisarvon neliö on
//! taajuuden f teho N näytteen ikkunassa, ja se päivittyy vakioajassa
//! jokaisella näytteellä. Tämä vastaa Goertzelia, joka lasketaan uudelleen
//! joka näytteellä, mutta maksaa vain muutaman kertolaskun per näyte.

pub struct SlidingBin {
    n: usize,
    // Vaiheen eteneminen per näyte (yksikköympyrällä).
    step: (f64, f64),
    phasor: (f64, f64),
    ring_re: Vec<f32>,
    ring_im: Vec<f32>,
    pos: usize,
    sum: (f64, f64),
    renorm: u32,
}

impl SlidingBin {
    pub fn new(freq: f64, fs: f64, window: usize) -> Self {
        let w = std::f64::consts::TAU * freq / fs;
        SlidingBin {
            n: window.max(1),
            step: (w.cos(), -w.sin()),
            phasor: (1.0, 0.0),
            ring_re: vec![0.0; window.max(1)],
            ring_im: vec![0.0; window.max(1)],
            pos: 0,
            sum: (0.0, 0.0),
            renorm: 0,
        }
    }

    /// Syötä yksi näyte ja palauta ikkunan teho |Σ|² / N².
    /// Siniaallolle, jonka amplitudi on A ja taajuus f, tulos on A²/4.
    #[inline]
    pub fn push(&mut self, x: f32) -> f32 {
        let zr = x * self.phasor.0 as f32;
        let zi = x * self.phasor.1 as f32;
        self.sum.0 += (zr - self.ring_re[self.pos]) as f64;
        self.sum.1 += (zi - self.ring_im[self.pos]) as f64;
        self.ring_re[self.pos] = zr;
        self.ring_im[self.pos] = zi;
        self.pos += 1;
        if self.pos == self.n {
            self.pos = 0;
            // Liukulukuvirhe kertyy juoksevaan summaan; lasketaan se välillä tarkasti uudelleen.
            self.renorm += 1;
            if self.renorm >= 64 {
                self.renorm = 0;
                self.sum.0 = self.ring_re.iter().map(|&v| v as f64).sum();
                self.sum.1 = self.ring_im.iter().map(|&v| v as f64).sum();
            }
        }
        let (pr, pi) = self.phasor;
        let nr = pr * self.step.0 - pi * self.step.1;
        let ni = pr * self.step.1 + pi * self.step.0;
        // Pidetään osoitin yksikköympyrällä.
        let g = 1.5 - 0.5 * (nr * nr + ni * ni);
        self.phasor = (nr * g, ni * g);
        let inv = 1.0 / self.n as f64;
        ((self.sum.0 * self.sum.0 + self.sum.1 * self.sum.1) * inv * inv) as f32
    }

    pub fn reset(&mut self) {
        self.ring_re.iter_mut().for_each(|v| *v = 0.0);
        self.ring_im.iter_mut().for_each(|v| *v = 0.0);
        self.sum = (0.0, 0.0);
        self.pos = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::goertzel_power;

    #[test]
    fn matches_goertzel_on_window() {
        let fs = 48000.0;
        let n = 480;
        let x: Vec<f32> = (0..5000)
            .map(|i| {
                let t = i as f64 / fs;
                (0.6 * (std::f64::consts::TAU * 2000.0 * t).sin() + 0.3 * (std::f64::consts::TAU * 3100.0 * t).cos()) as f32
            })
            .collect();
        let mut bin = SlidingBin::new(2000.0, fs, n);
        let mut last = 0.0;
        for &v in &x {
            last = bin.push(v);
        }
        let reference = goertzel_power(&x[x.len() - n..], 2000.0, fs) as f32;
        assert!((last - reference).abs() < 1e-4, "{last} vs {reference}");
        assert!((last - 0.09).abs() < 0.01);
    }
}
