//! Background noise and sudden interference.

use crate::rng::Rng;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum NoiseKind {
    /// Flat spectrum over the whole band.
    White,
    /// Power is weighted towards low frequencies (about -3 dB/octave), like
    /// typical room noise (air conditioning, fans, traffic).
    Pink,
    /// A real recording (same sample rate as the receiver),
    /// looped from a random position.
    Recorded(Arc<Vec<f32>>),
}

pub struct NoiseSource {
    kind: NoiseKind,
    rms: f32,
    rng: Rng,
    pink: [f32; 7],
    pos: usize,
    rec_gain: f32,
}

impl NoiseSource {
    pub fn new(kind: NoiseKind, rms: f32, seed: u64) -> Self {
        let mut rng = Rng::new(seed ^ 0x5EED_0153);
        let (pos, rec_gain) = match &kind {
            NoiseKind::Recorded(buf) if !buf.is_empty() => {
                let r = crate::dsp::rms(buf).max(1e-12);
                (rng.below(buf.len() as u64) as usize, 1.0 / r)
            }
            _ => (0, 1.0),
        };
        NoiseSource { kind, rms, rng, pink: [0.0; 7], pos, rec_gain }
    }

    pub fn set_rms(&mut self, rms: f32) {
        self.rms = rms;
    }

    /// Add noise to a block.
    pub fn add(&mut self, out: &mut [f32]) {
        if self.rms <= 0.0 {
            return;
        }
        match &self.kind {
            NoiseKind::White => {
                for v in out {
                    *v += self.rng.gauss() * self.rms;
                }
            }
            NoiseKind::Pink => {
                // Paul Kellet's filter: pink noise from white noise
                // with seven first-order filters. The output RMS is about 1/0.11
                // before scaling when the input RMS is 1.
                let b = &mut self.pink;
                for v in out {
                    let w = self.rng.gauss();
                    b[0] = 0.99886 * b[0] + w * 0.0555179;
                    b[1] = 0.99332 * b[1] + w * 0.0750759;
                    b[2] = 0.96900 * b[2] + w * 0.1538520;
                    b[3] = 0.86650 * b[3] + w * 0.3104856;
                    b[4] = 0.55000 * b[4] + w * 0.5329522;
                    b[5] = -0.7616 * b[5] - w * 0.0168980;
                    let pink = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
                    b[6] = w * 0.115926;
                    *v += pink * 0.329 * self.rms;
                }
            }
            NoiseKind::Recorded(buf) => {
                if buf.is_empty() {
                    return;
                }
                let g = self.rec_gain * self.rms;
                for v in out {
                    *v += buf[self.pos] * g;
                    self.pos += 1;
                    if self.pos == buf.len() {
                        self.pos = 0;
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InterferenceKind {
    /// Speech-like: noise pulsing at syllable rate (about 4 Hz) with its power
    /// in the speech band.
    Babble,
    /// Thump: a sudden wideband impact that decays in about 50 ms.
    Click,
}

/// Scheduled interference at the microphone.
#[derive(Clone, Copy, Debug)]
pub struct Interference {
    pub kind: InterferenceKind,
    pub start_s: f64,
    pub duration_s: f64,
    /// RMS of the peak level.
    pub rms: f32,
}

pub struct InterferencePlayer {
    items: Vec<Interference>,
    fs: f64,
    n: u64,
    rng: Rng,
    lp: [f32; 2],
}

impl InterferencePlayer {
    pub fn new(items: Vec<Interference>, fs: f64, seed: u64) -> Self {
        InterferencePlayer { items, fs, n: 0, rng: Rng::new(seed ^ 0xBAB_B1E), lp: [0.0; 2] }
    }

    pub fn add(&mut self, out: &mut [f32]) {
        if self.items.is_empty() {
            self.n += out.len() as u64;
            return;
        }
        for v in out {
            let t = self.n as f64 / self.fs;
            self.n += 1;
            for it in &self.items {
                let dt = t - it.start_s;
                if dt < 0.0 || dt > it.duration_s {
                    continue;
                }
                let w = self.rng.gauss();
                match it.kind {
                    InterferenceKind::Babble => {
                        // Two cascaded low-pass filters (~3 kHz) limit the power to the speech band.
                        let a = (1.0 - (-std::f64::consts::TAU * 3000.0 / self.fs).exp()) as f32;
                        self.lp[0] += a * (w - self.lp[0]);
                        self.lp[1] += a * (self.lp[0] - self.lp[1]);
                        let syll = 0.55 + 0.45 * (std::f64::consts::TAU * 4.0 * dt).sin() as f32;
                        *v += self.lp[1] * 2.6 * syll * it.rms;
                    }
                    InterferenceKind::Click => {
                        *v += w * it.rms * (-dt / 0.012).exp() as f32;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::rms;

    #[test]
    fn noise_levels_match_request() {
        for kind in [NoiseKind::White, NoiseKind::Pink] {
            let mut src = NoiseSource::new(kind.clone(), 0.1, 3);
            let mut buf = vec![0.0; 200_000];
            src.add(&mut buf);
            let r = rms(&buf[20_000..]);
            assert!((r - 0.1).abs() < 0.02, "{kind:?}: {r}");
        }
    }

    #[test]
    fn pink_has_more_low_frequency_power() {
        let mut src = NoiseSource::new(NoiseKind::Pink, 0.1, 4);
        let mut buf = vec![0.0; 96_000];
        src.add(&mut buf);
        let fs = 48000.0;
        let avg = |f: f64| -> f64 { buf.chunks(4800).map(|c| crate::dsp::goertzel_power(c, f, fs)).sum::<f64>() };
        assert!(avg(200.0) > 5.0 * avg(6400.0));
    }
}
