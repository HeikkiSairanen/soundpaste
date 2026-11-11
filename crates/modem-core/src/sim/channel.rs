//! One audio path from speaker to microphone, plus a helper for one-way transfer.

use super::noise::{Interference, InterferencePlayer, NoiseKind, NoiseSource};
use crate::dsp::conv::Convolver;
use crate::dsp::resample::Resampler;
use crate::rng::Rng;
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct PathConfig {
    /// Nominal sample rate of the transmitter.
    pub fs_tx: f64,
    /// Nominal sample rate of the receiver.
    pub fs_rx: f64,
    /// Receiver clock error in parts per million relative to the transmitter.
    pub clock_ppm: f64,
    /// Room impulse response at the receiver's rate. Empty = no echo.
    pub ir: Vec<f32>,
    /// Linear gain (attenuation < 1).
    pub gain: f32,
    /// Fixed delay in seconds (sound travel time and device buffers).
    pub delay_s: f64,
}

impl PathConfig {
    pub fn ideal(fs: f64) -> Self {
        PathConfig { fs_tx: fs, fs_rx: fs, clock_ppm: 0.0, ir: Vec::new(), gain: 1.0, delay_s: 0.0 }
    }
}

/// Streaming audio path. The number of output samples follows the rate ratio.
pub struct Path {
    resampler: Resampler,
    convolver: Convolver,
    delay: VecDeque<f32>,
    gain: f32,
    tmp: Vec<f32>,
    tmp2: Vec<f32>,
}

impl Path {
    pub fn new(cfg: &PathConfig) -> Self {
        // If the receiver's clock runs faster, it takes more samples of the same sound.
        let fs_rx_actual = cfg.fs_rx * (1.0 + cfg.clock_ppm * 1e-6);
        let ir: &[f32] = if cfg.ir.is_empty() { &[1.0] } else { &cfg.ir };
        let convolver = Convolver::new(ir);
        let want = (cfg.delay_s * cfg.fs_rx).round() as usize;
        // The convolution's own buffering delay counts towards the requested delay.
        let extra = want.saturating_sub(convolver.latency());
        Path {
            resampler: Resampler::new(cfg.fs_tx, fs_rx_actual),
            convolver,
            delay: std::iter::repeat(0.0).take(extra).collect(),
            gain: cfg.gain,
            tmp: Vec::new(),
            tmp2: Vec::new(),
        }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.tmp.clear();
        self.resampler.process(input, &mut self.tmp);
        self.tmp2.clear();
        self.convolver.process(&self.tmp, &mut self.tmp2);
        for &v in &self.tmp2 {
            self.delay.push_back(v * self.gain);
            out.push(self.delay.pop_front().unwrap());
        }
    }
}

/// Settings for a one-way transfer: one transmission, one receiver.
#[derive(Clone, Debug)]
pub struct OneWay {
    pub fs_tx: f64,
    pub fs_rx: f64,
    pub clock_ppm: f64,
    pub ir: Vec<f32>,
    /// Signal-to-noise ratio at the microphone (dB). Both powers are measured over the whole
    /// band 0..fs/2, and the signal power over the duration of the transmission after the echo.
    pub snr_db: f64,
    pub noise: NoiseKind,
    /// Signal RMS at the microphone (full scale 1.0). A small value
    /// models attenuation, a large one (> ~0.5) clipping.
    pub rx_level: f32,
    /// Silence before and after the transmission.
    pub lead_s: f64,
    pub tail_s: f64,
    pub interference: Vec<Interference>,
}

impl OneWay {
    pub fn clean(fs: f64) -> Self {
        OneWay {
            fs_tx: fs,
            fs_rx: fs,
            clock_ppm: 0.0,
            ir: Vec::new(),
            snr_db: 60.0,
            noise: NoiseKind::White,
            rx_level: 0.1,
            lead_s: 0.3,
            tail_s: 0.5,
            interference: Vec::new(),
        }
    }
}

/// Pass the transmission through the channel and return the microphone signal.
pub fn run_oneway(tx: &[f32], cfg: &OneWay, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let mut path = Path::new(&PathConfig {
        fs_tx: cfg.fs_tx,
        fs_rx: cfg.fs_rx,
        clock_ppm: cfg.clock_ppm,
        ir: cfg.ir.clone(),
        gain: 1.0,
        delay_s: 0.0,
    });
    let lead = (cfg.lead_s * cfg.fs_tx) as usize;
    let tail = ((cfg.tail_s + cfg.ir.len() as f64 / cfg.fs_rx) * cfg.fs_tx) as usize + 2 * Convolver::BLOCK;
    let mut input = vec![0.0f32; lead];
    input.extend_from_slice(tx);
    input.extend(std::iter::repeat(0.0).take(tail));
    let mut rx = Vec::with_capacity(input.len() + 1024);
    for chunk in input.chunks(4096) {
        path.process(chunk, &mut rx);
    }
    // Signal power over the duration of the transmission (no silent edges).
    let active = tx.len() as f64 / cfg.fs_tx;
    let energy: f64 = rx.iter().map(|&v| (v as f64) * (v as f64)).sum();
    let sig_rms = (energy / (active * cfg.fs_rx).max(1.0)).sqrt().max(1e-12);
    let scale = cfg.rx_level as f64 / sig_rms;
    let noise_rms = cfg.rx_level as f64 / crate::dsp::from_db20(cfg.snr_db);
    for v in rx.iter_mut() {
        *v *= scale as f32;
    }
    NoiseSource::new(cfg.noise.clone(), noise_rms as f32, rng.next_u64()).add(&mut rx);
    InterferencePlayer::new(cfg.interference.clone(), cfg.fs_rx, rng.next_u64()).add(&mut rx);
    for v in rx.iter_mut() {
        *v = v.clamp(-1.0, 1.0);
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::rms;

    #[test]
    fn snr_is_what_was_asked() {
        let fs = 48000.0;
        let tx: Vec<f32> = (0..48000).map(|i| (i as f32 * 0.3).sin() * 0.5).collect();
        let mut cfg = OneWay::clean(fs);
        cfg.snr_db = 10.0;
        cfg.lead_s = 1.0;
        let rx = run_oneway(&tx, &cfg, 1);
        let noise = rms(&rx[..40000]) as f64;
        let both = rms(&rx[50000..90000]) as f64;
        let sig = (both * both - noise * noise).sqrt();
        let snr = 20.0 * (sig / noise).log10();
        assert!((snr - 10.0).abs() < 0.5, "{snr}");
        assert!((sig - 0.1).abs() < 0.01);
    }

    #[test]
    fn path_delay_and_rate_change() {
        let cfg = PathConfig { fs_tx: 44100.0, fs_rx: 48000.0, clock_ppm: 0.0, ir: vec![], gain: 0.5, delay_s: 0.1 };
        let mut p = Path::new(&cfg);
        let mut x = vec![0.0f32; 44100];
        x[0] = 1.0;
        let mut y = Vec::new();
        p.process(&x, &mut y);
        assert!((y.len() as i64 - 48000).abs() < 40);
        let peak = y.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        // 0.1 s delay + group delay of the interpolation filter (a few samples).
        assert!((peak as i64 - 4800).abs() < 30, "{peak}");
    }
}
