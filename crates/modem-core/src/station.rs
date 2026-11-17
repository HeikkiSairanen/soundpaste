//! Station: modem and link layer together as a processor of audio blocks.
//!
//! [`Station::process`] takes a block of microphone samples and fills a block of
//! speaker samples. Everything else (receiver, state machine, building the transmission)
//! happens inside it. Time is the number of samples processed, so the station
//! works the same way in the browser's AudioWorklet, on the command line and
//! in the simulator.
//!
//! A station can run a band ladder (DESIGN.md 5.9): one modulator and one receiver
//! per band, all receivers listening at once. The link layer names the bands of
//! every transmission and hears the band of every frame.

use crate::link::{Action, Event, Link, LinkConfig, Quality, SendError};
use crate::phy::{Demodulator, Modulator, PhyEvent, PhyKind};
use std::collections::VecDeque;

/// Physical layer counters for debugging and measurements.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StationStats {
    pub preambles: u32,
    pub frames_ok: u32,
    pub frames_failed: u32,
    pub frames_sent: u32,
    /// Total bytes corrected by error correction.
    pub corrected: u32,
    /// Quality of the latest intact frame.
    pub last_snr_db: f32,
    pub last_rms: f32,
}

pub struct Station {
    kinds: Vec<PhyKind>,
    fs: f64,
    link: Link,
    modulators: Vec<Box<dyn Modulator>>,
    demods: Vec<Box<dyn Demodulator>>,
    /// Samples waiting to be played.
    tx: VecDeque<f32>,
    transmitting: bool,
    n: u64,
    events: VecDeque<Event>,
    phy_events: Vec<PhyEvent>,
    stats: StationStats,
    /// Input level meter: sliding RMS (about 100 ms) and peak.
    level_sq: f32,
    peak: f32,
}

impl Station {
    /// A station on a single waveform.
    pub fn new(kind: PhyKind, fs: f64, cfg: LinkConfig, seed: u64) -> Self {
        Self::with_bands(&[kind], fs, cfg, seed)
    }

    /// A station on a band ladder, lowest (most robust) band first. With one band
    /// this is [`Station::new`].
    pub fn with_bands(kinds: &[PhyKind], fs: f64, cfg: LinkConfig, seed: u64) -> Self {
        assert!(!kinds.is_empty() && kinds.len() <= 8);
        let cfg = LinkConfig { bands: kinds.len() as u8, ..cfg };
        Station {
            kinds: kinds.to_vec(),
            fs,
            link: Link::new(cfg, seed),
            modulators: kinds.iter().map(|k| k.modulator(fs)).collect(),
            demods: kinds.iter().map(|k| k.demodulator(fs)).collect(),
            tx: VecDeque::new(),
            transmitting: false,
            n: 0,
            events: VecDeque::new(),
            phy_events: Vec::new(),
            stats: StationStats::default(),
            level_sq: 0.0,
            peak: 0.0,
        }
    }

    fn ms(&self, sample: u64) -> u64 {
        (sample as f64 * 1000.0 / self.fs) as u64
    }

    pub fn now_ms(&self) -> u64 {
        self.ms(self.n)
    }

    /// The waveform of band 0 (the only one without a ladder).
    pub fn phy(&self) -> PhyKind {
        self.kinds[0]
    }

    /// The bands the station listens on, lowest first.
    pub fn bands(&self) -> &[PhyKind] {
        &self.kinds
    }

    pub fn sample_rate(&self) -> f64 {
        self.fs
    }

    /// Start sending a message.
    pub fn send(&mut self, data: &[u8]) -> Result<u16, SendError> {
        let now = self.now_ms();
        self.link.send(now, data)
    }

    /// Cancel the transmission in progress.
    pub fn cancel(&mut self) {
        let now = self.now_ms();
        self.link.cancel(now);
        self.drain_actions();
    }

    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    pub fn is_sending(&self) -> bool {
        self.link.is_sending()
    }

    pub fn is_receiving(&self) -> bool {
        self.link.is_receiving()
    }

    pub fn stats(&self) -> StationStats {
        self.stats
    }

    /// Microphone level: (RMS, peak in the latest block).
    pub fn input_level(&self) -> (f32, f32) {
        (self.level_sq.sqrt(), self.peak)
    }

    pub fn max_message(&self) -> usize {
        self.link.config().max_message
    }

    /// Estimate of the transfer duration in seconds when everything succeeds on the first try.
    pub fn estimate_seconds(&self, len: usize) -> f64 {
        let cfg = self.link.config();
        let turn = (cfg.response_delay + 200) as f64 / 1000.0;
        // With a ladder: the START goes out on every band at once (as long as the longest
        // copy), and the rest is assumed to happen on the top band.
        let top = self.modulators.last().unwrap();
        let d = |payload: usize| top.duration(payload);
        let start = self.modulators.iter().map(|m| m.duration(11)).fold(0.0, f64::max);
        let ladder = self.kinds.len() > 1;
        let start_ack = if ladder { 7 + self.kinds.len() } else { 6 };
        let listen = cfg.listen as f64 / 1000.0;
        let n = len.div_ceil(cfg.chunk);
        if n == 0 || (cfg.inline_start && !ladder && n == 1) {
            return listen + d(11 + len) + turn + d(7);
        }
        let last = len - (n - 1) * cfg.chunk;
        let data = (n - 1) as f64 * (d(4 + cfg.chunk) + cfg.frame_gap as f64 / 1000.0) + d(4 + last);
        listen + start + turn + d(start_ack) + turn + data + turn + d(7)
    }

    fn drain_actions(&mut self) {
        while let Some(action) = self.link.poll_action() {
            match action {
                Action::Transmit(bytes, mask) => {
                    let bands: Vec<usize> = (0..self.kinds.len()).filter(|b| mask & (1 << b) != 0).collect();
                    let bands = if bands.is_empty() { vec![0] } else { bands };
                    if let [b] = bands[..] {
                        self.tx.extend(self.modulators[b].modulate(&bytes));
                    } else {
                        // Simultaneous copies on several bands, aligned to end together so that
                        // the receiver hears them all at once. Each copy at 1/√n of its level:
                        // the total stays near one frame's, and a copy that is heard at that
                        // level has margin at full level.
                        let waves: Vec<Vec<f32>> = bands.iter().map(|&b| self.modulators[b].modulate(&bytes)).collect();
                        let len = waves.iter().map(Vec::len).max().unwrap();
                        let scale = 1.0 / (waves.len() as f32).sqrt();
                        let mut mix = vec![0.0f32; len];
                        for w in &waves {
                            for (m, v) in mix[len - w.len()..].iter_mut().zip(w) {
                                *m += scale * v;
                            }
                        }
                        self.tx.extend(mix.into_iter().map(|v| v.clamp(-0.95, 0.95)));
                    }
                    self.transmitting = true;
                    self.stats.frames_sent += 1;
                }
                Action::AbortTx => {
                    self.tx.clear();
                    self.transmitting = false;
                }
                Action::Event(Event::Band { band, sending, .. }) => {
                    let (lo, hi) = self.kinds.get(band as usize).map_or((0.0, 0.0), |k| k.band_hz());
                    self.events.push_back(Event::Band { band, sending, lo_hz: lo.round() as u32, hi_hz: hi.round() as u32 });
                }
                Action::Event(e) => self.events.push_back(e),
            }
        }
    }

    /// Process one audio block: `input` from the microphone, `output` to the speaker.
    /// The blocks are the same length.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        debug_assert_eq!(input.len(), output.len());
        // Input level meter.
        if !input.is_empty() {
            let sq = input.iter().map(|v| v * v).sum::<f32>() / input.len() as f32;
            let a = (input.len() as f32 / (0.1 * self.fs as f32)).min(1.0);
            self.level_sq += a * (sq - self.level_sq);
            self.peak = input.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        }

        self.n += input.len() as u64;
        let now = self.now_ms();
        for band in 0..self.demods.len() {
            self.phy_events.clear();
            self.demods[band].push(input, &mut self.phy_events);
            self.handle_phy_events(band as u8, now);
        }
        let busy = self.demods.iter().any(|d| d.receiving());
        self.link.tick(now, busy);
        self.drain_actions();

        for o in output.iter_mut() {
            *o = self.tx.pop_front().unwrap_or(0.0);
        }
        if self.transmitting && self.tx.is_empty() {
            self.transmitting = false;
            self.link.on_tx_done(now);
            self.drain_actions();
        }
    }

    fn handle_phy_events(&mut self, band: u8, now: u64) {
        for ev in std::mem::take(&mut self.phy_events) {
            match ev {
                PhyEvent::Detected { .. } => {
                    self.stats.preambles += 1;
                    self.link.on_preamble(now);
                }
                PhyEvent::Frame { payload, stats, .. } => {
                    self.stats.frames_ok += 1;
                    self.stats.corrected += stats.corrected;
                    self.stats.last_snr_db = stats.snr_db;
                    self.stats.last_rms = stats.rms;
                    let end = self.ms(stats.end_sample).min(now);
                    self.link.on_frame_band(end, &payload, Quality::from_stats(stats.snr_db, stats.rms, stats.clipped), band);
                }
                PhyEvent::Failed { reason, stats, .. } => {
                    self.stats.frames_failed += 1;
                    // For debugging: rejected frames show up in the log with their levels.
                    let on = if self.kinds.len() > 1 { format!(" band {band}") } else { String::new() };
                    self.events.push_back(Event::Log(format!(
                        "<- frame rejected ({}){on}, snr={:.0} dB level={:.0} dBFS clipped={:.0} %",
                        match reason {
                            crate::phy::FailReason::Header => "header",
                            crate::phy::FailReason::Checksum => "checksum",
                        },
                        stats.snr_db,
                        20.0 * stats.rms.max(1e-7).log10(),
                        stats.clipped * 100.0
                    )));
                    // Only a frame whose header decoded is certainly a real transmission.
                    if reason == crate::phy::FailReason::Checksum {
                        self.link.on_rx_failed(now);
                    }
                }
            }
        }
    }
}
