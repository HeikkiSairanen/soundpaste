//! Sliding frequency detector ("sliding DFT bin").
//!
//! The input signal is mixed with a complex exponential at frequency f and
//! the sum of the last N samples is maintained. The squared magnitude of the sum is
//! the power at frequency f in a window of N samples, and it updates in constant time
//! on every sample. This is equivalent to a Goertzel recomputed
//! on every sample, but costs only a few multiplications per sample.

pub struct SlidingBin {
    n: usize,
    // Phase advance per sample (on the unit circle).
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

    /// Feed one sample and return the window power |Σ|² / N².
    /// For a sine wave with amplitude A and frequency f the result is A²/4.
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
            // Floating-point error accumulates in the running sum; recompute it exactly now and then.
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
        // Keep the phasor on the unit circle.
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
