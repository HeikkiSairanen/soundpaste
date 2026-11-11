//! Asema: modeemi ja linkkikerros yhdessä äänilohkojen käsittelijänä.
//!
//! [`Station::process`] saa mikrofonin näytelohkon ja täyttää kaiuttimen
//! näytelohkon. Kaikki muu (vastaanotin, tilakone, lähetteen muodostus)
//! tapahtuu sen sisällä. Aika on käsiteltyjen näytteiden määrä, joten asema
//! toimii samalla tavalla selaimen AudioWorkletissa, komentorivillä ja
//! simulaattorissa.

use crate::link::{Action, Event, Link, LinkConfig, Quality, SendError};
use crate::phy::{Demodulator, Modulator, PhyEvent, PhyKind};
use std::collections::VecDeque;

/// Fyysisen kerroksen laskurit vianetsintään ja mittauksiin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StationStats {
    pub preambles: u32,
    pub frames_ok: u32,
    pub frames_failed: u32,
    pub frames_sent: u32,
    /// Virheenkorjauksen korjaamat tavut yhteensä.
    pub corrected: u32,
    /// Viimeisimmän ehjän kehyksen laatu.
    pub last_snr_db: f32,
    pub last_rms: f32,
}

pub struct Station {
    fs: f64,
    link: Link,
    modulator: Box<dyn Modulator>,
    demod: Box<dyn Demodulator>,
    /// Soittoa odottavat näytteet.
    tx: VecDeque<f32>,
    transmitting: bool,
    n: u64,
    events: VecDeque<Event>,
    phy_events: Vec<PhyEvent>,
    stats: StationStats,
    /// Tulotason mittari: liukuva tehollisarvo (noin 100 ms) ja huippu.
    level_sq: f32,
    peak: f32,
}

impl Station {
    pub fn new(kind: PhyKind, fs: f64, cfg: LinkConfig, seed: u64) -> Self {
        Station {
            fs,
            link: Link::new(cfg, seed),
            modulator: kind.modulator(fs),
            demod: kind.demodulator(fs),
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

    pub fn sample_rate(&self) -> f64 {
        self.fs
    }

    /// Aloita viestin lähetys.
    pub fn send(&mut self, data: &[u8]) -> Result<u16, SendError> {
        let now = self.now_ms();
        self.link.send(now, data)
    }

    /// Peruuta käynnissä oleva lähetys.
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

    /// Mikrofonin taso: (tehollisarvo, huippu viimeisimmässä lohkossa).
    pub fn input_level(&self) -> (f32, f32) {
        (self.level_sq.sqrt(), self.peak)
    }

    pub fn max_message(&self) -> usize {
        self.link.config().max_message
    }

    /// Arvio siirron kestosta sekunteina, kun kaikki onnistuu ensimmäisellä kerralla.
    pub fn estimate_seconds(&self, len: usize) -> f64 {
        let cfg = self.link.config();
        let turn = (cfg.response_delay + 200) as f64 / 1000.0;
        let d = |payload: usize| self.modulator.duration(payload);
        let listen = cfg.listen as f64 / 1000.0;
        let n = len.div_ceil(cfg.chunk);
        if n == 0 || (cfg.inline_start && n == 1) {
            return listen + d(11 + len) + turn + d(7);
        }
        let last = len - (n - 1) * cfg.chunk;
        let data = (n - 1) as f64 * (d(4 + cfg.chunk) + cfg.frame_gap as f64 / 1000.0) + d(4 + last);
        listen + d(11) + turn + d(6) + turn + data + turn + d(7)
    }

    fn drain_actions(&mut self) {
        while let Some(action) = self.link.poll_action() {
            match action {
                Action::Transmit(bytes) => {
                    self.tx.extend(self.modulator.modulate(&bytes));
                    self.transmitting = true;
                    self.stats.frames_sent += 1;
                }
                Action::AbortTx => {
                    self.tx.clear();
                    self.transmitting = false;
                }
                Action::Event(e) => self.events.push_back(e),
            }
        }
    }

    /// Käsittele yksi äänilohko: `input` mikrofonista, `output` kaiuttimeen.
    /// Lohkot ovat yhtä pitkät.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        debug_assert_eq!(input.len(), output.len());
        // Tulotason mittari.
        if !input.is_empty() {
            let sq = input.iter().map(|v| v * v).sum::<f32>() / input.len() as f32;
            let a = (input.len() as f32 / (0.1 * self.fs as f32)).min(1.0);
            self.level_sq += a * (sq - self.level_sq);
            self.peak = input.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        }

        self.phy_events.clear();
        self.demod.push(input, &mut self.phy_events);
        self.n += input.len() as u64;
        let now = self.now_ms();
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
                    self.link.on_frame(end, &payload, Quality::from_stats(stats.snr_db, stats.rms, stats.clipped));
                }
                PhyEvent::Failed { .. } => {
                    self.stats.frames_failed += 1;
                    self.link.on_rx_failed(now);
                }
            }
        }
        self.link.tick(now, self.demod.receiving());
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
}
