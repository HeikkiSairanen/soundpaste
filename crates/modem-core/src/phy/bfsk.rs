//! v0: karkea 2-FSK-modeemi kehityssyklin pystyttämistä varten.
//!
//! Tarkoituksella yksinkertaisin toimiva ratkaisu, ei hyvä modeemi:
//! - kaksi taajuutta (0 = 2000 Hz, 1 = 3000 Hz), 100 symbolia sekunnissa
//! - ei virheenkorjausta, vain CRC-16
//! - synkronointi: vuorotteleva 1010...-alkumerkki ja 16-bittinen tahdistussana
//!
//! Kehys bitteinä (MSB ensin):
//! `alkumerkki 32 b | tahdistussana 16 b | pituus 8 b | data | CRC-16`
//!
//! Vastaanotin laskee kummankin taajuuden tehon yhden symbolin mittaisessa
//! liukuvassa ikkunassa ja tekee bittipäätöksen kahdeksassa eri vaiheessa
//! symbolia kohti. Vaihe, jossa tahdistussana löytyy suurimmalla marginaalilla,
//! lukitaan kehyksen ajaksi.

use super::{Demodulator, FailReason, Modulator, PhyEvent, RxStats};
use crate::crc::crc16;
use crate::dsp::sliding::SlidingBin;

const BAUD: f64 = 100.0;
const F0: f64 = 2000.0;
const F1: f64 = 3000.0;
const PREAMBLE_BITS: usize = 32;
const SYNC: u16 = 0x2DD4;
const AMPLITUDE: f32 = 0.5;
const PHASES: usize = 8;
const MAX_PAYLOAD: usize = 255;

pub struct BfskModulator {
    fs: f64,
}

impl BfskModulator {
    pub fn new(fs: f64) -> Self {
        BfskModulator { fs }
    }

    fn bits(&self, payload: &[u8]) -> Vec<bool> {
        let mut bits = Vec::new();
        for i in 0..PREAMBLE_BITS {
            bits.push(i % 2 == 0);
        }
        for i in (0..16).rev() {
            bits.push(SYNC >> i & 1 == 1);
        }
        for b in self.coded(payload) {
            for i in (0..8).rev() {
                bits.push(b >> i & 1 == 1);
            }
        }
        // Kaksi täytebittiä, jotta viimeisen oikean symbolin ikkuna ei osu häntään.
        bits.push(false);
        bits.push(true);
        bits
    }
}

impl Modulator for BfskModulator {
    fn max_payload(&self) -> usize {
        MAX_PAYLOAD
    }

    fn coded(&self, payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() <= MAX_PAYLOAD);
        let mut out = vec![payload.len() as u8];
        out.extend_from_slice(payload);
        let c = crc16(&out);
        out.extend_from_slice(&c.to_be_bytes());
        out
    }

    fn modulate(&self, payload: &[u8]) -> Vec<f32> {
        let bits = self.bits(payload);
        let total = (bits.len() as f64 * self.fs / BAUD).round() as usize;
        let mut out = Vec::with_capacity(total);
        // Jatkuvavaiheinen FSK: vaihe ei hyppää symbolirajalla, joten spektri pysyy kapeana.
        let mut phase = 0.0f64;
        for n in 0..total {
            let sym = ((n as f64 * BAUD / self.fs) as usize).min(bits.len() - 1);
            let f = if bits[sym] { F1 } else { F0 };
            phase += std::f64::consts::TAU * f / self.fs;
            out.push(phase.sin() as f32 * AMPLITUDE);
        }
        // Pehmeä alku ja loppu (5 ms), jotta kaiutin ei naksahda.
        let ramp = (0.005 * self.fs) as usize;
        for i in 0..ramp.min(total / 2) {
            let g = 0.5 - 0.5 * (std::f32::consts::PI * i as f32 / ramp as f32).cos();
            out[i] *= g;
            out[total - 1 - i] *= g;
        }
        out
    }

    fn duration(&self, payload_len: usize) -> f64 {
        (PREAMBLE_BITS + 16 + (payload_len + 3) * 8 + 2) as f64 / BAUD
    }
}

enum State {
    Search,
    /// Tahdistussana löytyi jostakin vaiheesta; odotetaan loput vaiheet ja valitaan paras.
    Choosing { best_phase: usize, best_quality: f32, hops_left: usize },
    Locked { phase: usize, bits: Vec<bool>, need: Option<usize> },
}

pub struct BfskDemodulator {
    bin0: SlidingBin,
    bin1: SlidingBin,
    hop_len: f64,
    next_hop: f64,
    hop_index: u64,
    n: u64,
    reg: [u32; PHASES],
    margins: [[f32; 16]; PHASES],
    margin_pos: [usize; PHASES],
    state: State,
    // Kehyksen aikaiset tilastot.
    start: u64,
    sum_sq: f64,
    clipped: u64,
    count: u64,
    sum_hi: f64,
    sum_lo: f64,
}

impl BfskDemodulator {
    pub fn new(fs: f64) -> Self {
        let sym = fs / BAUD;
        let win = sym.round() as usize;
        BfskDemodulator {
            bin0: SlidingBin::new(F0, fs, win),
            bin1: SlidingBin::new(F1, fs, win),
            hop_len: sym / PHASES as f64,
            next_hop: sym,
            hop_index: 0,
            n: 0,
            reg: [0; PHASES],
            margins: [[0.0; 16]; PHASES],
            margin_pos: [0; PHASES],
            state: State::Search,
            start: 0,
            sum_sq: 0.0,
            clipped: 0,
            count: 0,
            sum_hi: 0.0,
            sum_lo: 0.0,
        }
    }

    fn stats(&self) -> RxStats {
        let n = self.count.max(1) as f64;
        RxStats {
            start_sample: self.start,
            end_sample: self.n,
            snr_db: crate::dsp::db10((self.sum_hi - self.sum_lo).max(0.0) / self.sum_lo.max(1e-30)) as f32,
            rms: (self.sum_sq / n).sqrt() as f32,
            clipped: (self.clipped as f64 / n) as f32,
            corrected: 0,
        }
    }

    fn lock(&mut self, phase: usize, events: &mut Vec<PhyEvent>) {
        self.state = State::Locked { phase, bits: Vec::new(), need: None };
        // Alkumerkki ja tahdistussana ovat jo menneet: 48 symbolia taaksepäin.
        let back = (48.0 * self.hop_len * PHASES as f64) as u64;
        self.start = self.n.saturating_sub(back);
        self.sum_sq = 0.0;
        self.clipped = 0;
        self.count = 0;
        self.sum_hi = 0.0;
        self.sum_lo = 0.0;
        events.push(PhyEvent::Detected { at: self.start });
    }

    fn hop(&mut self, e0: f32, e1: f32, events: &mut Vec<PhyEvent>) {
        let p = (self.hop_index % PHASES as u64) as usize;
        self.hop_index += 1;
        let bit = e1 > e0;
        let margin = (e1 - e0).abs() / (e1 + e0 + 1e-20);

        // Haku: jokainen vaihe pitää omaa siirtorekisteriään.
        if matches!(self.state, State::Search | State::Choosing { .. }) {
            self.reg[p] = (self.reg[p] << 1) | bit as u32;
            self.margins[p][self.margin_pos[p]] = margin;
            self.margin_pos[p] = (self.margin_pos[p] + 1) % 16;
            let hit = self.reg[p] as u16 == SYNC;
            let quality: f32 = self.margins[p].iter().sum();
            match &mut self.state {
                State::Search if hit => {
                    self.state = State::Choosing { best_phase: p, best_quality: quality, hops_left: PHASES - 1 };
                }
                State::Choosing { best_phase, best_quality, hops_left } => {
                    if hit && quality > *best_quality {
                        *best_phase = p;
                        *best_quality = quality;
                    }
                    *hops_left -= 1;
                    if *hops_left == 0 {
                        let bp = *best_phase;
                        self.lock(bp, events);
                    }
                }
                _ => {}
            }
            return;
        }

        let State::Locked { phase, bits, need } = &mut self.state else { return };
        if p != *phase {
            return;
        }
        bits.push(bit);
        self.sum_hi += e0.max(e1) as f64;
        self.sum_lo += e0.min(e1) as f64;
        if need.is_none() && bits.len() == 8 {
            let len = bits.iter().fold(0usize, |a, &b| a << 1 | b as usize);
            *need = Some((1 + len + 2) * 8);
        }
        if Some(bits.len()) == *need {
            let raw: Vec<u8> = bits.chunks(8).map(|c| c.iter().fold(0u8, |a, &b| a << 1 | b as u8)).collect();
            let body = raw.len() - 2;
            let ok = crc16(&raw[..body]) == u16::from_be_bytes([raw[body], raw[body + 1]]);
            let stats = self.stats();
            events.push(if ok {
                PhyEvent::Frame { payload: raw[1..body].to_vec(), raw, stats }
            } else {
                PhyEvent::Failed { reason: FailReason::Checksum, raw, stats }
            });
            self.reset();
        }
    }
}

impl Demodulator for BfskDemodulator {
    fn push(&mut self, samples: &[f32], events: &mut Vec<PhyEvent>) {
        for &x in samples {
            let e0 = self.bin0.push(x);
            let e1 = self.bin1.push(x);
            self.n += 1;
            if matches!(self.state, State::Locked { .. }) {
                self.sum_sq += (x as f64) * (x as f64);
                self.count += 1;
                if x.abs() >= 0.99 {
                    self.clipped += 1;
                }
            }
            if self.n as f64 >= self.next_hop {
                self.next_hop += self.hop_len;
                self.hop(e0, e1, events);
            }
        }
    }

    fn receiving(&self) -> bool {
        !matches!(self.state, State::Search)
    }

    fn reset(&mut self) {
        self.state = State::Search;
        self.reg = [0; PHASES];
    }

    fn position(&self) -> u64 {
        self.n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(fs: f64, x: &[f32], chunk: usize) -> Vec<PhyEvent> {
        let mut d = BfskDemodulator::new(fs);
        let mut ev = Vec::new();
        for c in x.chunks(chunk) {
            d.push(c, &mut ev);
        }
        ev
    }

    #[test]
    fn clean_roundtrip_at_both_rates() {
        for fs in [44100.0, 48000.0] {
            let m = BfskModulator::new(fs);
            let payload = "https://sairanen.org/ ääkköset 🎵".as_bytes();
            let mut x = vec![0.0; 1234];
            x.extend(m.modulate(payload));
            x.extend(vec![0.0; 5000]);
            let ev = decode_all(fs, &x, 128);
            let got = ev.iter().find_map(|e| if let PhyEvent::Frame { payload, .. } = e { Some(payload.clone()) } else { None });
            assert_eq!(got.as_deref(), Some(payload), "fs={fs}");
            assert!((m.duration(payload.len()) - (x.len() - 6234) as f64 / fs).abs() < 0.01);
        }
    }

    #[test]
    fn empty_and_max_payload() {
        let fs = 48000.0;
        let m = BfskModulator::new(fs);
        for len in [0usize, 255] {
            let payload: Vec<u8> = (0..len).map(|i| (i * 7) as u8).collect();
            let mut x = m.modulate(&payload);
            x.extend(vec![0.0; 2000]);
            let ev = decode_all(fs, &x, 1000);
            assert!(ev.iter().any(|e| matches!(e, PhyEvent::Frame { payload: p, .. } if *p == payload)), "len={len}");
        }
    }

    #[test]
    fn two_frames_back_to_back() {
        let fs = 48000.0;
        let m = BfskModulator::new(fs);
        let mut x = m.modulate(b"eka");
        x.extend(vec![0.0; 4800]);
        x.extend(m.modulate(b"toka"));
        x.extend(vec![0.0; 4800]);
        let frames: Vec<Vec<u8>> = decode_all(fs, &x, 4096)
            .into_iter()
            .filter_map(|e| if let PhyEvent::Frame { payload, .. } = e { Some(payload) } else { None })
            .collect();
        assert_eq!(frames, vec![b"eka".to_vec(), b"toka".to_vec()]);
    }
}
