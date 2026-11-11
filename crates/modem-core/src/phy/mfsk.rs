//! Multitone MFSK with tone hopping: the actual waveform (DESIGN.md section 4).
//!
//! ```text
//! | preamble (chirp) | gap | header | payload | tail |
//! ```
//!
//! - K parallel subchannels, each M-FSK: one of M tones is on.
//! - Tone hopping: symbol t uses tone group t mod G, so the echo of earlier
//!   symbols does not land on the candidate tones of the current symbol.
//! - Synchronisation: frequency chirp and matched filter, relative threshold (CFAR).
//! - Error correction: Reed–Solomon, uncertain bytes as erasures.
//!
//! All lengths are defined in seconds and frequencies in hertz, so the same
//! waveform is produced and decoded at any sample rate.
//! All parameters are in the [`MfskParams`] struct: a new mode (faster,
//! more robust or on a different band) is a new parameter set.

use super::{Demodulator, FailReason, Modulator, PhyEvent, RxStats};
use crate::crc::{crc16, crc8};
use crate::dsp::fft::{next_pow2, Fft};
use crate::rs::ReedSolomon;
use std::f64::consts::TAU;

pub const PROTOCOL_VERSION: u8 = 1;
/// Header: 3 data bytes + 9 parity bytes.
const HEADER_DATA: usize = 3;
const HEADER_LEN: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ByteLayout {
    /// Byte = adjacent subchannels in one symbol. A short disturbance ruins
    /// only a few bytes, but a bad subchannel hits many bytes.
    Freq,
    /// Byte = consecutive symbols in one subchannel. A bad subchannel ruins
    /// only its own bytes, but a short disturbance hits all subchannels.
    Time,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MfskParams {
    pub name: &'static str,
    /// Mode identifier in the header (5 bits).
    pub mode_id: u8,
    /// Lowest tone (Hz).
    pub f0: f64,
    /// Detection window (s); the tone spacing is 1/t_win.
    pub t_win: f64,
    /// Guard time before the window (s).
    pub t_guard: f64,
    /// Tones per subchannel and group (4 or 16).
    pub m: usize,
    /// Parallel subchannels.
    pub k: usize,
    /// Hop groups.
    pub g: usize,
    pub chirp_s: f64,
    pub gap_s: f64,
    pub tail_s: f64,
    /// At least (L+2)·num/den bytes of parity.
    pub parity_num: usize,
    pub parity_den: usize,
    pub layout: ByteLayout,
    /// A byte is marked uncertain when the power ratio of the strongest and second strongest
    /// tone is smaller than this. 0 = erasures disabled.
    pub erasure_ratio: f32,
    /// Preamble detection threshold: power of the correlation peak relative to the background.
    pub detect_threshold: f32,
    /// Preamble shape check: how large a share of the window's in-band energy
    /// is explained by the chirp (0..1). A click or speech exceeds the power threshold
    /// but does not look like a chirp. 0 = check off.
    pub chirp_fraction: f32,
    /// The detection window is advanced by this much to allow for timing error (s).
    pub timing_advance_s: f64,
    /// RMS of the transmission and the clipping limit for peaks (full scale 1.0).
    pub rms: f32,
    pub clip: f32,
}

pub const BASE: MfskParams = MfskParams {
    name: "v1",
    mode_id: 0,
    f0: 1000.0,
    t_win: 0.040,
    t_guard: 0.010,
    m: 4,
    k: 16,
    g: 4,
    chirp_s: 0.060,
    gap_s: 0.010,
    tail_s: 0.020,
    parity_num: 1,
    parity_den: 2,
    layout: ByteLayout::Freq,
    erasure_ratio: 2.0,
    detect_threshold: 25.0,
    chirp_fraction: 0.08,
    timing_advance_s: 0.001,
    rms: 0.45,
    clip: 0.9,
};

impl MfskParams {
    fn bps(&self) -> usize {
        self.m.trailing_zeros() as usize
    }
    fn units_per_byte(&self) -> usize {
        8 / self.bps()
    }
    pub fn tones(&self) -> usize {
        self.m * self.k * self.g
    }
    fn freq(&self, tone: usize) -> f64 {
        self.f0 + tone as f64 / self.t_win
    }
    pub fn band(&self) -> (f64, f64) {
        (self.f0, self.freq(self.tones()))
    }
    /// Interleaved tone map: the candidates of a subchannel are G tone spacings
    /// apart and the tones of the other groups fall between them.
    fn tone_index(&self, sub: usize, value: usize, sym: usize) -> usize {
        sub * self.m * self.g + value * self.g + sym % self.g
    }
    pub fn t_sym(&self) -> f64 {
        self.t_win + self.t_guard
    }
    fn symbols_for(&self, bytes: usize) -> usize {
        (bytes * self.units_per_byte()).div_ceil(self.k)
    }
    fn capacity(&self, symbols: usize) -> usize {
        symbols * self.k / self.units_per_byte()
    }
    /// Payload codeword length and amount of parity when there are `len` bytes of data.
    /// The space left over as symbol padding is used for extra parity.
    pub fn codeword(&self, len: usize) -> (usize, usize) {
        let base = len + 2;
        let parity = (base * self.parity_num).div_ceil(self.parity_den).max(4);
        let n = self.capacity(self.symbols_for(base + parity)).min(255);
        (n, n - base)
    }
    pub fn max_payload(&self) -> usize {
        let mut len = 0;
        while len < 250 && (len + 3) + ((len + 3) * self.parity_num).div_ceil(self.parity_den).max(4) <= 255 {
            len += 1;
        }
        len
    }
    /// In which symbol and subchannel part `unit` of byte `byte` travels.
    fn slot(&self, byte: usize, unit: usize, symbols: usize) -> (usize, usize) {
        let idx = byte * self.units_per_byte() + unit;
        match self.layout {
            ByteLayout::Freq => (idx / self.k, idx % self.k),
            ByteLayout::Time => (idx % symbols, idx / symbols),
        }
    }

    /// Bytes to symbols: `symbols` × K values in 0..M.
    fn map_bytes(&self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let symbols = self.symbols_for(bytes.len());
        // Unused slots are filled with a varying value so that every
        // subchannel sounds for the whole frame.
        let mut out: Vec<Vec<u8>> = (0..symbols).map(|t| (0..self.k).map(|s| ((t * 7 + s * 3) % self.m) as u8).collect()).collect();
        let bps = self.bps();
        for (b, &byte) in bytes.iter().enumerate() {
            for u in 0..self.units_per_byte() {
                let (t, s) = self.slot(b, u, symbols);
                out[t][s] = (byte >> (8 - bps * (u + 1))) & (self.m as u8 - 1);
            }
        }
        out
    }

    /// Symbol decisions back to bytes. The reliability of a byte is that of
    /// its weakest part.
    fn unmap_bytes(&self, n: usize, values: &[Vec<u8>], conf: &[Vec<f32>]) -> (Vec<u8>, Vec<f32>) {
        let symbols = values.len();
        let bps = self.bps();
        let mut bytes = vec![0u8; n];
        let mut confs = vec![f32::MAX; n];
        for b in 0..n {
            for u in 0..self.units_per_byte() {
                let (t, s) = self.slot(b, u, symbols);
                bytes[b] |= values[t][s] << (8 - bps * (u + 1));
                confs[b] = confs[b].min(conf[t][s]);
            }
        }
        (bytes, confs)
    }

    fn header_bytes(&self, len: usize) -> Vec<u8> {
        let mut h = vec![(PROTOCOL_VERSION << 5) | (self.mode_id & 0x1F), len as u8];
        h.push(crc8(&h));
        let parity = ReedSolomon::new(HEADER_LEN - HEADER_DATA).encode(&h);
        h.extend(parity);
        h
    }

    fn payload_bytes(&self, payload: &[u8]) -> Vec<u8> {
        let (_, parity) = self.codeword(payload.len());
        let mut cw = payload.to_vec();
        cw.extend_from_slice(&crc16(payload).to_be_bytes());
        let p = ReedSolomon::new(parity).encode(&cw);
        cw.extend(p);
        cw
    }

    /// Fixed pseudo-random initial phases of the tones: simultaneous tones
    /// do not add up into a single spike.
    fn tone_phases(&self) -> Vec<f64> {
        let mut rng = crate::rng::Rng::new(0x50AD_7A57);
        (0..self.tones()).map(|_| rng.range(0.0, TAU)).collect()
    }

    /// Phase of the preamble at time t (s): a linear chirp across the band.
    fn chirp_phase(&self, t: f64) -> f64 {
        let (lo, hi) = self.band();
        TAU * (lo * t + (hi - lo) * t * t / (2.0 * self.chirp_s))
    }
}

pub struct MfskModulator {
    p: MfskParams,
    fs: f64,
    phases: Vec<f64>,
}

impl MfskModulator {
    pub fn new(p: &MfskParams, fs: f64) -> Self {
        MfskModulator { p: p.clone(), fs, phases: p.tone_phases() }
    }
}

impl Modulator for MfskModulator {
    fn max_payload(&self) -> usize {
        self.p.max_payload()
    }

    fn coded(&self, payload: &[u8]) -> Vec<u8> {
        let mut out = self.p.header_bytes(payload.len());
        out.extend(self.p.payload_bytes(payload));
        out
    }

    fn duration(&self, payload_len: usize) -> f64 {
        let (n, _) = self.p.codeword(payload_len);
        let symbols = self.p.symbols_for(HEADER_LEN) + self.p.symbols_for(n);
        self.p.chirp_s + self.p.gap_s + symbols as f64 * self.p.t_sym() + self.p.tail_s
    }

    fn modulate(&self, payload: &[u8]) -> Vec<f32> {
        let p = &self.p;
        let fs = self.fs;
        assert!(payload.len() <= p.max_payload());
        let mut symbols = p.map_bytes(&p.header_bytes(payload.len()));
        symbols.extend(p.map_bytes(&p.payload_bytes(payload)));

        let chirp_len = (p.chirp_s * fs).round() as usize;
        let body_start = chirp_len + (p.gap_s * fs).round() as usize;
        let sym_at = |t: usize| body_start + (t as f64 * p.t_sym() * fs).round() as usize;
        let ramp = ((p.t_guard * 0.5).min(0.003) * fs) as usize;
        let body_end = sym_at(symbols.len()) + ramp;
        let mut out = vec![0.0f32; body_end + (p.tail_s * fs).round() as usize];

        // Preamble: constant-envelope chirp, soft edges.
        let edge = (0.002 * fs) as usize;
        let chirp_amp = p.rms * std::f32::consts::SQRT_2;
        for (i, v) in out[..chirp_len].iter_mut().enumerate() {
            let e = i.min(chirp_len - 1 - i);
            let g = if e < edge { 0.5 - 0.5 * (std::f32::consts::PI * e as f32 / edge as f32).cos() } else { 1.0 };
            *v = g * chirp_amp * p.chirp_phase(i as f64 / fs).sin() as f32;
        }

        // Symbols: K tones at a time. A tone rises at the start of the symbol and falls
        // only after the symbol (in the next guard time), so it is
        // steady for the duration of the detection window.
        for (t, sym) in symbols.iter().enumerate() {
            let (a, b) = (sym_at(t), sym_at(t + 1) + ramp);
            for (sub, &v) in sym.iter().enumerate() {
                let tone = p.tone_index(sub, v as usize, t);
                let w = TAU * p.freq(tone) / fs;
                let ph = self.phases[tone];
                for i in a..b {
                    let e = (i - a).min(b - 1 - i);
                    let g = if e < ramp { 0.5 - 0.5 * (std::f64::consts::PI * e as f64 / ramp as f64).cos() } else { 1.0 };
                    out[i] += (g * (w * i as f64 + ph).sin()) as f32;
                }
            }
        }
        // Scaling to the target power and clipping of peaks. According to measurements, clipping
        // 6 dB above the RMS does not weaken detection (DESIGN.md section 9).
        let scale = p.rms / (p.k as f32 / 2.0).sqrt();
        for v in out[body_start..body_end].iter_mut() {
            *v = (*v * scale).clamp(-p.clip, p.clip);
        }
        out
    }
}

/// Preamble detector: cross-correlation against an analytic template of the chirp
/// with an FFT, block by block. The result is the correlation power for every lag.
struct ChirpCorrelator {
    nfft: usize,
    lags: usize,
    /// FFT of the shape check (a window one preamble long) and the band limits as its frequency bins.
    shape_fft: Fft,
    shape_bins: (usize, usize),
    chirp_len: usize,
    fft: Fft,
    t_re: Vec<f32>,
    t_im: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl ChirpCorrelator {
    fn new(p: &MfskParams, fs: f64) -> Self {
        let lc = (p.chirp_s * fs).round() as usize;
        // The block must have room for the template, for the peak search one template length
        // ahead (see `search`) and for a reasonable step. A short block keeps
        // the detection delay small (48 kHz: 8192 samples = 170 ms).
        let nfft = next_pow2(lc + lc + (0.015 * fs) as usize + lc / 2);
        let fft = Fft::new(nfft);
        let mut t_re = vec![0.0f32; nfft];
        let mut t_im = vec![0.0f32; nfft];
        // Complex template e^{jφ(t)}: the magnitude of the correlation does not depend on the channel phase.
        for i in 0..lc {
            let ph = p.chirp_phase(i as f64 / fs);
            t_re[i] = ph.cos() as f32;
            t_im[i] = ph.sin() as f32;
        }
        fft.forward(&mut t_re, &mut t_im);
        let shape_n = next_pow2(lc);
        let (lo, hi) = p.band();
        let bin = |f: f64| (f * shape_n as f64 / fs).round() as usize;
        ChirpCorrelator { nfft, lags: nfft - lc + 1, shape_fft: Fft::new(shape_n), shape_bins: (bin(lo), bin(hi).min(shape_n / 2)), chirp_len: lc, fft, t_re, t_im, re: vec![0.0; nfft], im: vec![0.0; nfft] }
    }

    /// How large a share of the in-band energy of window `x` (one preamble long) is
    /// chirp when the peak correlation power was `peak`. For a clean chirp
    /// the result is 1, for noise and clicks close to zero.
    fn chirp_fraction(&self, x: &[f32], peak: f32) -> f32 {
        let n = self.shape_fft.len();
        let mut re = vec![0.0f32; n];
        let mut im = vec![0.0f32; n];
        re[..x.len()].copy_from_slice(x);
        self.shape_fft.forward(&mut re, &mut im);
        // Parseval: in-band energy in the time domain = (2/N) · Σ |X_k|² over the band's bins.
        let band: f64 = (self.shape_bins.0..=self.shape_bins.1).map(|k| (re[k] as f64).powi(2) + (im[k] as f64).powi(2)).sum::<f64>() * 2.0 / n as f64;
        // For a chirp with amplitude A: peak power = (A·L/2)² and energy = A²·L/2.
        (2.0 * peak as f64 / (band.max(1e-30) * self.chirp_len as f64)) as f32
    }

    /// `x`: nfft samples. Returns the power for lags 0..lags.
    fn power(&mut self, x: &[f32]) -> &[f32] {
        self.re.copy_from_slice(x);
        self.im.iter_mut().for_each(|v| *v = 0.0);
        self.fft.forward(&mut self.re, &mut self.im);
        for k in 0..self.nfft {
            // X · conj(T)
            let (xr, xi) = (self.re[k], self.im[k]);
            self.re[k] = xr * self.t_re[k] + xi * self.t_im[k];
            self.im[k] = xi * self.t_re[k] - xr * self.t_im[k];
        }
        self.fft.inverse(&mut self.re, &mut self.im);
        for k in 0..self.lags {
            self.re[k] = self.re[k] * self.re[k] + self.im[k] * self.im[k];
        }
        &self.re[..self.lags]
    }
}

struct Frame {
    /// Start of the preamble and start of the first symbol (absolute sample indices).
    t0: u64,
    sym0: u64,
    values: Vec<Vec<u8>>,
    conf: Vec<Vec<f32>>,
    /// Payload length read from the header once the header has been decoded.
    len: Option<usize>,
    header_raw: Vec<u8>,
    corrected: u32,
    sum_best: f64,
    sum_other: f64,
    sum_sq: f64,
    clipped: u64,
    samples: u64,
}

pub struct MfskDemodulator {
    p: MfskParams,
    fs: f64,
    corr: ChirpCorrelator,
    /// Sample buffer and the absolute index of its first sample.
    buf: Vec<f32>,
    buf_start: u64,
    n: u64,
    /// Search state: the next lag to process, the correlation power history before
    /// it and the limit before which we do not trigger (after a restart).
    search_pos: u64,
    hist: Vec<f32>,
    no_trigger_before: u64,
    frame: Option<Frame>,
    // Sample counts.
    n_win: usize,
    n_guard: usize,
    n_adv: usize,
    ref_win: usize,
    ref_gap: usize,
    peak_win: usize,
    chirp_len: usize,
    scratch: Vec<f32>,
}

impl MfskDemodulator {
    pub fn new(p: &MfskParams, fs: f64) -> Self {
        let n_win = (p.t_win * fs).round() as usize;
        MfskDemodulator {
            corr: ChirpCorrelator::new(p, fs),
            p: p.clone(),
            fs,
            buf: Vec::new(),
            buf_start: 0,
            n: 0,
            search_pos: 0,
            hist: Vec::new(),
            no_trigger_before: 0,
            frame: None,
            n_win,
            n_guard: (p.t_guard * fs).round() as usize,
            n_adv: (p.timing_advance_s * fs).round() as usize,
            ref_win: (0.060 * fs) as usize,
            ref_gap: (0.003 * fs) as usize,
            peak_win: (0.015 * fs) as usize,
            chirp_len: (p.chirp_s * fs).round() as usize,
            scratch: Vec::new(),
        }
    }

    fn sample(&self, abs: u64) -> usize {
        (abs - self.buf_start) as usize
    }

    fn trim(&mut self, keep_from: u64) {
        let keep_from = keep_from.min(self.n);
        if keep_from > self.buf_start + 16384 {
            let drop = self.sample(keep_from);
            self.buf.drain(..drop);
            self.buf_start = keep_from;
        }
    }

    fn restart_search(&mut self, at: u64) {
        self.frame = None;
        self.search_pos = at.saturating_sub(self.ref_win as u64).max(self.buf_start);
        self.no_trigger_before = at;
        self.hist.clear();
    }

    /// Search for the preamble in one block. Returns true if the block was processed
    /// (i.e. it is worth trying again), false if there are not enough samples yet.
    fn search(&mut self, events: &mut Vec<PhyEvent>) -> bool {
        let off = self.sample(self.search_pos);
        if self.buf.len() < off + self.corr.nfft {
            return false;
        }
        let h = self.hist.len();
        let lags = self.corr.lags;
        self.scratch.clear();
        self.scratch.extend_from_slice(&self.hist);
        let power = self.corr.power(&self.buf[off..off + self.corr.nfft]);
        self.scratch.extend_from_slice(power);
        let q = &self.scratch;
        // Cumulative sum for computing the background level.
        let mut cum = Vec::with_capacity(q.len() + 1);
        let mut acc = 0.0f64;
        cum.push(0.0);
        for &v in q.iter() {
            acc += v as f64;
            cum.push(acc);
        }
        // The threshold crossing can come from the chirp's own sidelobe, which precedes
        // the main peak by at most the template length. So the peak is searched from the crossing
        // to one template length ahead, and the rest of the block is left for the next round.
        let ahead = self.chirp_len + self.peak_win;
        let usable = lags - ahead;
        let mut found = None;
        let mut skip_until = 0usize;
        for i in 0..usable {
            let abs = self.search_pos + i as u64;
            let qi = h + i;
            if i < skip_until || abs < self.no_trigger_before || qi < self.ref_win {
                continue;
            }
            let (a, b) = (qi - self.ref_win, qi - self.ref_gap);
            let background = (cum[b] - cum[a]) / (b - a) as f64;
            if (q[qi] as f64) > self.p.detect_threshold as f64 * background && q[qi] > 1e-12 {
                // The strongest peak, and before it the earliest one that is almost as strong: the direct
                // sound arrives before the reflections.
                let win = &q[qi..qi + ahead];
                let (max_i, max) = win.iter().enumerate().fold((0, 0.0f32), |b, (j, &v)| if v > b.1 { (j, v) } else { b });
                // Shape check: is there really a chirp at the peak?
                if self.p.chirp_fraction > 0.0 {
                    let at = off + i + max_i;
                    let fraction = self.corr.chirp_fraction(&self.buf[at..at + self.chirp_len], max);
                    if fraction < self.p.chirp_fraction {
                        // A click or other disturbance: skip it and continue the search after it.
                        skip_until = i + max_i + self.ref_gap;
                        continue;
                    }
                }
                let from = max_i.saturating_sub(self.peak_win);
                let first = from + win[from..=max_i].iter().position(|&v| v >= 0.3 * max).unwrap_or(max_i - from);
                found = Some(self.search_pos + (i + first) as u64);
                break;
            }
        }
        match found {
            Some(t0) => {
                let chirp_len = (self.p.chirp_s * self.fs).round() as u64;
                let sym0 = t0 + chirp_len + (self.p.gap_s * self.fs).round() as u64;
                self.frame = Some(Frame {
                    t0,
                    sym0,
                    values: Vec::new(),
                    conf: Vec::new(),
                    len: None,
                    header_raw: Vec::new(),
                    corrected: 0,
                    sum_best: 0.0,
                    sum_other: 0.0,
                    sum_sq: 0.0,
                    clipped: 0,
                    samples: 0,
                });
                events.push(PhyEvent::Detected { at: t0 });
            }
            None => {
                // Move forward and keep the background level history.
                let keep_to = h + usable;
                let from = keep_to.saturating_sub(self.ref_win);
                self.hist = self.scratch[from..keep_to].to_vec();
                self.search_pos += usable as u64;
                self.trim(self.search_pos);
            }
        }
        true
    }

    /// Detect one symbol: the strongest candidate tone in each subchannel.
    fn detect_symbol(&mut self, t: usize) -> bool {
        let p = &self.p;
        let frame = self.frame.as_ref().unwrap();
        let start = frame.sym0 + (t as f64 * p.t_sym() * self.fs).round() as u64 + self.n_guard as u64 - self.n_adv as u64;
        if start + self.n_win as u64 > self.n {
            return false;
        }
        let a = self.sample(start);
        let win = &self.buf[a..a + self.n_win];
        let mut values = Vec::with_capacity(p.k);
        let mut confs = Vec::with_capacity(p.k);
        let (mut sum_best, mut sum_other) = (0.0f64, 0.0f64);
        for sub in 0..p.k {
            let (mut best, mut best_v, mut second, mut total) = (-1.0f64, 0usize, 0.0f64, 0.0f64);
            for v in 0..p.m {
                let e = tone_energy(win, p.freq(p.tone_index(sub, v, t)), self.fs);
                total += e;
                if e > best {
                    second = best.max(0.0);
                    best = e;
                    best_v = v;
                } else if e > second {
                    second = e;
                }
            }
            values.push(best_v as u8);
            confs.push((best / second.max(1e-30)).min(1e6) as f32);
            sum_best += best;
            sum_other += (total - best) / (p.m - 1) as f64;
        }
        let sq: f64 = win.iter().map(|&v| (v as f64) * (v as f64)).sum();
        let clipped = win.iter().filter(|v| v.abs() >= 0.99).count() as u64;
        let frame = self.frame.as_mut().unwrap();
        frame.values.push(values);
        frame.conf.push(confs);
        frame.sum_best += sum_best;
        frame.sum_other += sum_other;
        frame.sum_sq += sq;
        frame.clipped += clipped;
        frame.samples += self.n_win as u64;
        true
    }

    fn stats(&self, frame: &Frame, end: u64) -> RxStats {
        let n = frame.samples.max(1) as f64;
        RxStats {
            start_sample: frame.t0,
            end_sample: end,
            snr_db: crate::dsp::db10((frame.sum_best - frame.sum_other).max(0.0) / frame.sum_other.max(1e-30)) as f32,
            rms: (frame.sum_sq / n).sqrt() as f32,
            clipped: (frame.clipped as f64 / n) as f32,
            corrected: frame.corrected,
        }
    }

    /// Reed–Solomon decoding: first without erasures, then with the most uncertain bytes
    /// as erasures. `check` accepts the result (CRC).
    fn rs_decode(&self, bytes: &[u8], confs: &[f32], parity: usize, check: impl Fn(&[u8]) -> bool) -> Option<(Vec<u8>, u32)> {
        let rs = ReedSolomon::new(parity);
        let mut order: Vec<usize> = (0..bytes.len()).collect();
        order.sort_by(|&a, &b| confs[a].total_cmp(&confs[b]));
        let weak = if self.p.erasure_ratio > 0.0 { order.iter().take_while(|&&i| confs[i] < self.p.erasure_ratio).count() } else { 0 };
        let mut tries = vec![0usize];
        for frac in [2, 1] {
            let e = weak.min(parity.saturating_sub(2) / frac);
            if e > 0 && !tries.contains(&e) {
                tries.push(e);
            }
        }
        for e in tries {
            let mut cw = bytes.to_vec();
            if let Ok(fixed) = rs.decode(&mut cw, &order[..e]) {
                if check(&cw) {
                    return Some((cw, fixed as u32));
                }
            }
        }
        None
    }

    /// Process the unfinished frame. Returns false when more samples are needed.
    fn receive(&mut self, events: &mut Vec<PhyEvent>) -> bool {
        let header_syms = self.p.symbols_for(HEADER_LEN);
        let have = self.frame.as_ref().unwrap().values.len();
        let need = match self.frame.as_ref().unwrap().len {
            None => header_syms,
            Some(len) => header_syms + self.p.symbols_for(self.p.codeword(len).0),
        };
        if have < need {
            return self.detect_symbol(have);
        }
        let frame = self.frame.as_ref().unwrap();
        let sym_end = |t: usize| frame.sym0 + (t as f64 * self.p.t_sym() * self.fs).round() as u64;
        if frame.len.is_none() {
            // Header complete.
            let (bytes, confs) = self.p.unmap_bytes(HEADER_LEN, &frame.values[..header_syms], &frame.conf[..header_syms]);
            let decoded = self.rs_decode(&bytes, &confs, HEADER_LEN - HEADER_DATA, |cw| crc8(&cw[..2]) == cw[2]);
            let ok = decoded.as_ref().map_or(false, |(cw, _)| cw[0] == (PROTOCOL_VERSION << 5) | (self.p.mode_id & 0x1F) && (cw[1] as usize) <= self.p.max_payload());
            if !ok {
                // False alarm, foreign mode or badly corrupted header: continue
                // the search right after the preamble.
                let stats = self.stats(frame, sym_end(header_syms));
                let t0 = frame.t0;
                events.push(PhyEvent::Failed { reason: FailReason::Header, raw: bytes, stats });
                self.restart_search(t0 + (0.020 * self.fs) as u64);
                return true;
            }
            let (cw, fixed) = decoded.unwrap();
            let frame = self.frame.as_mut().unwrap();
            frame.len = Some(cw[1] as usize);
            frame.header_raw = bytes;
            frame.corrected += fixed;
            let t0 = frame.t0;
            self.trim(t0);
            return true;
        }
        // Payload complete.
        let len = frame.len.unwrap();
        let (n, parity) = self.p.codeword(len);
        let (bytes, confs) = self.p.unmap_bytes(n, &frame.values[header_syms..], &frame.conf[header_syms..]);
        let decoded = self.rs_decode(&bytes, &confs, parity, |cw| crc16(&cw[..len]) == u16::from_be_bytes([cw[len], cw[len + 1]]));
        let end = sym_end(need);
        let mut raw = frame.header_raw.clone();
        raw.extend_from_slice(&bytes);
        let mut stats = self.stats(frame, end);
        events.push(match decoded {
            Some((cw, fixed)) => {
                stats.corrected += fixed;
                PhyEvent::Frame { payload: cw[..len].to_vec(), raw, stats }
            }
            None => PhyEvent::Failed { reason: FailReason::Checksum, raw, stats },
        });
        self.restart_search(end);
        true
    }
}

/// Power of one tone in a window (|DFT|² / N²). The phase advances by recursion, so
/// sin/cos is computed only once per tone.
fn tone_energy(x: &[f32], freq: f64, fs: f64) -> f64 {
    let w = TAU * freq / fs;
    let (cw, sw) = (w.cos() as f32, w.sin() as f32);
    let (mut c, mut s) = (1.0f32, 0.0f32);
    let (mut re, mut im) = (0.0f32, 0.0f32);
    // Block-wise into an f64 sum so that the error of the f32 recursion and sum does not accumulate.
    let (mut re_acc, mut im_acc) = (0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        re += v * c;
        im += v * s;
        let nc = c * cw - s * sw;
        s = s * cw + c * sw;
        c = nc;
        if i & 255 == 255 {
            re_acc += re as f64;
            im_acc += im as f64;
            re = 0.0;
            im = 0.0;
            // Keep the phasor on the unit circle.
            let g = 1.5 - 0.5 * (c * c + s * s);
            c *= g;
            s *= g;
        }
    }
    re_acc += re as f64;
    im_acc += im as f64;
    let n = x.len().max(1) as f64;
    (re_acc * re_acc + im_acc * im_acc) / (n * n)
}

impl Demodulator for MfskDemodulator {
    fn push(&mut self, samples: &[f32], events: &mut Vec<PhyEvent>) {
        self.buf.extend_from_slice(samples);
        self.n += samples.len() as u64;
        loop {
            let progressed = if self.frame.is_some() { self.receive(events) } else { self.search(events) };
            if !progressed {
                break;
            }
        }
    }

    fn receiving(&self) -> bool {
        self.frame.is_some()
    }

    fn reset(&mut self) {
        let n = self.n;
        self.restart_search(n);
    }

    fn position(&self) -> u64 {
        self.n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(p: &MfskParams, fs: f64, x: &[f32], chunk: usize) -> Vec<Vec<u8>> {
        let mut d = MfskDemodulator::new(p, fs);
        let mut ev = Vec::new();
        for c in x.chunks(chunk) {
            d.push(c, &mut ev);
        }
        ev.into_iter().filter_map(|e| if let PhyEvent::Frame { payload, .. } = e { Some(payload) } else { None }).collect()
    }

    fn with_noise(x: &[f32], level: f32, seed: u64) -> Vec<f32> {
        let mut rng = crate::rng::Rng::new(seed);
        x.iter().map(|v| v + rng.gauss() * level).collect()
    }

    #[test]
    fn clean_roundtrip_at_both_rates_and_chunkings() {
        for fs in [44100.0, 48000.0] {
            let m = MfskModulator::new(&BASE, fs);
            let payload = "https://sairanen.org/ café niño 🎵".as_bytes();
            let mut x = vec![0.0; 5000];
            x.extend(m.modulate(payload));
            x.extend(vec![0.0; 20000]);
            let x = with_noise(&x, 0.001, 1);
            for chunk in [128, 1000, 50000] {
                assert_eq!(frames(&BASE, fs, &x, chunk), vec![payload.to_vec()], "fs={fs} chunk={chunk}");
            }
            assert!((m.duration(payload.len()) - (x.len() - 25000) as f64 / fs).abs() < 0.01);
        }
    }

    #[test]
    fn empty_and_max_payload() {
        let fs = 48000.0;
        let m = MfskModulator::new(&BASE, fs);
        assert!(m.max_payload() >= 160);
        for len in [0usize, 1, m.max_payload()] {
            let payload: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            let mut x = vec![0.0; 4000];
            x.extend(m.modulate(&payload));
            x.extend(vec![0.0; 20000]);
            assert_eq!(frames(&BASE, fs, &with_noise(&x, 0.001, 2), 4096), vec![payload], "len={len}");
        }
    }

    #[test]
    fn frames_in_a_burst_are_all_received() {
        let fs = 48000.0;
        let m = MfskModulator::new(&BASE, fs);
        let mut x = vec![0.0; 3000];
        let msgs: Vec<Vec<u8>> = (0..4u8).map(|i| vec![i; 10 + i as usize * 20]).collect();
        for msg in &msgs {
            x.extend(m.modulate(msg));
            x.extend(vec![0.0; 2400]); // 50 ms between frames
        }
        x.extend(vec![0.0; 20000]);
        assert_eq!(frames(&BASE, fs, &with_noise(&x, 0.001, 3), 512), msgs);
    }

    #[test]
    fn level_does_not_matter() {
        let fs = 48000.0;
        let m = MfskModulator::new(&BASE, fs);
        let mut x = vec![0.0; 6000];
        x.extend(m.modulate(b"gain"));
        x.extend(vec![0.0; 20000]);
        for gain in [1.0f32, 0.001] {
            let y: Vec<f32> = x.iter().map(|v| v * gain).collect();
            assert_eq!(frames(&BASE, fs, &with_noise(&y, 0.0001 * gain, 4), 1024), vec![b"gain".to_vec()], "gain={gain}");
        }
    }

    #[test]
    fn noise_alone_triggers_nothing() {
        let fs = 48000.0;
        let x = with_noise(&vec![0.0; 48000 * 20], 0.1, 5);
        let mut d = MfskDemodulator::new(&BASE, fs);
        let mut ev = Vec::new();
        d.push(&x, &mut ev);
        assert!(ev.is_empty(), "{} events", ev.len());
    }

    #[test]
    fn layouts_roundtrip_bytes() {
        for layout in [ByteLayout::Freq, ByteLayout::Time] {
            let p = MfskParams { layout, ..BASE };
            for n in [1usize, 12, 37, 100, 255] {
                let bytes: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
                let syms = p.map_bytes(&bytes);
                let conf: Vec<Vec<f32>> = syms.iter().map(|s| vec![9.0; s.len()]).collect();
                assert_eq!(p.unmap_bytes(n, &syms, &conf).0, bytes, "{layout:?} n={n}");
            }
        }
    }

    #[test]
    fn frame_durations_match_design() {
        let m = MfskModulator::new(&BASE, 48000.0);
        // DESIGN.md section 6: START (L=11) 0.49 s, DATA 60 bytes (L=64) 1.49 s.
        assert!((m.duration(11) - 0.49).abs() < 0.001, "{}", m.duration(11));
        assert!((m.duration(64) - 1.49).abs() < 0.001, "{}", m.duration(64));
    }
}
