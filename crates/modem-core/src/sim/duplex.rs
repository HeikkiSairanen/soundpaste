//! Kaksisuuntainen simulaatio: kaksi asemaa samassa huoneessa.
//!
//! ```text
//!   asema A ──lähtöviive──► huone A→B ──┐
//!      ▲  └──────► oma vuoto A→A ──┐     ├─► + melu, häiriöt, leikkaus ─► asema B
//!      │                           ▼     │
//!      └── + melu, häiriöt ◄── huone B→A ◄──lähtöviive── asema B (+ oma vuoto B→B)
//! ```
//!
//! Kummallakin asemalla on oma näytteenottotaajuus, kello ja ääniviiveet.
//! Simulaatio etenee 10 ms:n askelin; asemat näkevät toistensa äänen vasta
//! seuraavalla askeleella, mikä vastaa pientä lisäviivettä.

use super::channel::{Path, PathConfig};
use super::ir::energy;
use super::noise::{Interference, InterferencePlayer, NoiseKind, NoiseSource};
use crate::link::{Event, LinkConfig};
use crate::phy::PhyKind;
use crate::rng::Rng;
use crate::station::Station;
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct Side {
    pub fs: f64,
    /// Äänen lähtöviive (puskurit ennen kaiutinta) ja tuloviive (mikrofonista ohjelmalle).
    pub out_latency_s: f64,
    pub in_latency_s: f64,
    /// Huoneen impulssivaste toisen aseman kaiuttimesta tämän aseman mikrofoniin.
    pub ir: Vec<f32>,
    /// Vastaanotetun signaalin tehollisarvo mikrofonissa.
    pub level: f32,
    /// Oman kaiuttimen vuoto omaan mikrofoniin: impulssivaste ja taso. Taso 0 =
    /// ei kuule itseään (kuulokkeet).
    pub self_ir: Vec<f32>,
    pub self_level: f32,
    pub noise: NoiseKind,
    pub noise_rms: f32,
    pub interference: Vec<Interference>,
    pub link: LinkConfig,
}

#[derive(Clone, Debug)]
pub struct DuplexConfig {
    pub kind: PhyKind,
    pub sides: [Side; 2],
    /// Aseman B kellon virhe suhteessa asemaan A.
    pub clock_ppm: f64,
    pub seed: u64,
}

/// Lähetteen tehollisarvo, jota vasten tasot skaalataan.
const TX_RMS: f32 = 0.25;

struct Mic {
    cross: Path,
    own: Path,
    noise: NoiseSource,
    interference: InterferencePlayer,
    fifo: VecDeque<f32>,
    block: usize,
}

pub struct Duplex {
    pub stations: [Station; 2],
    pub events: [Vec<Event>; 2],
    mics: [Mic; 2],
    time_s: f64,
    tmp: Vec<f32>,
}

impl Duplex {
    pub fn new(cfg: &DuplexConfig) -> Self {
        let mut rng = Rng::new(cfg.seed);
        let station = |i: usize, seed: u64| Station::new(cfg.kind, cfg.sides[i].fs, cfg.sides[i].link.clone(), seed);
        let stations = [station(0, rng.next_u64()), station(1, rng.next_u64())];
        let mic = |i: usize, rng: &mut Rng| {
            let me = &cfg.sides[i];
            let other = &cfg.sides[1 - i];
            let ppm = if i == 1 { cfg.clock_ppm } else { -cfg.clock_ppm };
            let gain = |ir: &Vec<f32>, level: f32| level / (TX_RMS * (if ir.is_empty() { 1.0 } else { energy(ir).sqrt() as f32 }));
            Mic {
                cross: Path::new(&PathConfig {
                    fs_tx: other.fs,
                    fs_rx: me.fs,
                    clock_ppm: ppm,
                    ir: me.ir.clone(),
                    gain: gain(&me.ir, me.level),
                    delay_s: other.out_latency_s + me.in_latency_s,
                }),
                own: Path::new(&PathConfig {
                    fs_tx: me.fs,
                    fs_rx: me.fs,
                    clock_ppm: 0.0,
                    ir: me.self_ir.clone(),
                    gain: gain(&me.self_ir, me.self_level),
                    delay_s: me.out_latency_s + me.in_latency_s,
                }),
                noise: NoiseSource::new(me.noise.clone(), me.noise_rms, rng.next_u64()),
                interference: InterferencePlayer::new(me.interference.clone(), me.fs, rng.next_u64()),
                fifo: VecDeque::new(),
                block: (me.fs / 100.0).round() as usize,
            }
        };
        let mics = [mic(0, &mut rng), mic(1, &mut rng)];
        Duplex { stations, events: [Vec::new(), Vec::new()], mics, time_s: 0.0, tmp: Vec::new() }
    }

    pub fn time_s(&self) -> f64 {
        self.time_s
    }

    /// Etene 10 ms.
    pub fn step(&mut self) {
        let mut outputs: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
        for i in 0..2 {
            let mic = &mut self.mics[i];
            let n = mic.block;
            let mut input: Vec<f32> = (0..n).map(|_| mic.fifo.pop_front().unwrap_or(0.0)).collect();
            mic.noise.add(&mut input);
            mic.interference.add(&mut input);
            input.iter_mut().for_each(|v| *v = v.clamp(-1.0, 1.0));
            let mut out = vec![0.0f32; n];
            self.stations[i].process(&input, &mut out);
            while let Some(e) = self.stations[i].poll_event() {
                self.events[i].push(e);
            }
            outputs[i] = out;
        }
        for i in 0..2 {
            // Mikrofoni i kuulee toisen aseman huoneen läpi ja oman kaiuttimensa vuotona.
            self.tmp.clear();
            self.mics[i].cross.process(&outputs[1 - i], &mut self.tmp);
            let cross = std::mem::take(&mut self.tmp);
            let mut own = Vec::new();
            self.mics[i].own.process(&outputs[i], &mut own);
            // Polkujen lähtömäärä voi heittää näytteen kellovirheen takia: summataan
            // jonoon niin, että kumpikin polku säilyttää oman aikajanansa.
            let mic = &mut self.mics[i];
            let base = mic.fifo.len();
            let need = base + cross.len().max(own.len());
            mic.fifo.resize(need, 0.0);
            for (k, v) in cross.iter().enumerate() {
                mic.fifo[base + k] += v;
            }
            for (k, v) in own.iter().enumerate() {
                mic.fifo[base + k] += v;
            }
            self.tmp = cross;
        }
        self.time_s += 0.01;
    }

    pub fn run(&mut self, seconds: f64) {
        let until = self.time_s + seconds;
        while self.time_s < until {
            self.step();
        }
    }

    /// Aja, kunnes ehto täyttyy tai aikaraja ylittyy. Palauttaa, täyttyikö ehto.
    pub fn run_until(&mut self, max_seconds: f64, cond: impl Fn(&Duplex) -> bool) -> bool {
        let until = self.time_s + max_seconds;
        while !cond(self) {
            if self.time_s >= until {
                return false;
            }
            self.step();
        }
        true
    }

    pub fn received(&self, i: usize) -> Vec<Vec<u8>> {
        self.events[i].iter().filter_map(|e| if let Event::Received { data, .. } = e { Some(data.clone()) } else { None }).collect()
    }

    pub fn delivered_ms(&self, i: usize) -> Option<u64> {
        self.events[i].iter().find_map(|e| if let Event::Delivered { elapsed, .. } = e { Some(*elapsed) } else { None })
    }

    pub fn failed(&self, i: usize) -> bool {
        self.events[i].iter().any(|e| matches!(e, Event::Failed { .. }))
    }

    /// Onko lähetys päättynyt (onnistuen tai epäonnistuen)?
    pub fn settled(&self, i: usize) -> bool {
        self.delivered_ms(i).is_some() || self.failed(i)
    }

    /// Vianetsintäloki molemmilta asemilta.
    pub fn log(&self) -> String {
        let mut s = String::new();
        for i in 0..2 {
            for e in &self.events[i] {
                s += &format!("{} {e:?}\n", ["A", "B"][i]);
            }
        }
        s
    }
}

impl Side {
    /// Tavallinen kannettava hiljaisessa huoneessa.
    pub fn typical(fs: f64) -> Side {
        Side {
            fs,
            out_latency_s: 0.08,
            in_latency_s: 0.04,
            ir: Vec::new(),
            level: 0.03,
            self_ir: Vec::new(),
            self_level: 0.15,
            noise: NoiseKind::White,
            noise_rms: 0.003,
            interference: Vec::new(),
            link: LinkConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::{AbortReason, FailReason};
    use crate::sim::{synth_ir, InterferenceKind};

    const LINK: &[u8] = b"https://sairanen.org/soundpaste/?v=Zk3mQ9xT7bLw2RfN8cYh5VpJd";

    fn room(seed: u64) -> DuplexConfig {
        let side = |fs: f64, s: u64| Side { ir: synth_ir(fs, 0.35, 8.0, seed + s), self_ir: synth_ir(fs, 0.35, 14.0, seed + s + 50), ..Side::typical(fs) };
        DuplexConfig { kind: PhyKind::DEFAULT, sides: [side(48000.0, 1), side(48000.0, 2)], clock_ppm: 0.0, seed }
    }

    #[test]
    fn normal_session_over_audio() {
        let mut d = Duplex::new(&room(1));
        d.run(0.5);
        d.stations[0].send(LINK).unwrap();
        assert!(d.run_until(15.0, |d| d.settled(0)), "{}", d.log());
        assert_eq!(d.received(1), vec![LINK.to_vec()], "{}", d.log());
        let ms = d.delivered_ms(0).expect("ei perillä");
        // Pikalähetys (oletus): noin 2,6 s.
        assert!((2000..3600).contains(&ms), "kesti {ms} ms\n{}", d.log());
        // Kumpikaan ei tulkinnut omaa kaikuaan viestiksi.
        assert!(d.received(0).is_empty());
        d.run(1.0);
        assert!(!d.stations[0].is_sending() && !d.stations[1].is_receiving());
    }

    #[test]
    fn handshake_session_matches_design_estimate() {
        let mut cfg = room(1);
        for s in cfg.sides.iter_mut() {
            s.link.inline_start = false;
        }
        let mut d = Duplex::new(&cfg);
        d.run(0.5);
        d.stations[0].send(LINK).unwrap();
        assert!(d.run_until(15.0, |d| d.settled(0)), "{}", d.log());
        assert_eq!(d.received(1), vec![LINK.to_vec()], "{}", d.log());
        // DESIGN.md luku 6: noin 4,1 s.
        let ms = d.delivered_ms(0).expect("ei perillä");
        assert!((3200..5200).contains(&ms), "kesti {ms} ms\n{}", d.log());
    }

    #[test]
    fn different_sample_rates_clock_error_and_long_latencies() {
        let mut cfg = room(2);
        cfg.sides[1].fs = 44100.0;
        cfg.sides[1].ir = synth_ir(44100.0, 0.35, 8.0, 7);
        cfg.sides[1].self_ir = synth_ir(44100.0, 0.35, 14.0, 8);
        cfg.clock_ppm = 150.0;
        cfg.sides[0].out_latency_s = 0.30;
        cfg.sides[1].in_latency_s = 0.25;
        let mut d = Duplex::new(&cfg);
        d.run(0.3);
        let msg: Vec<u8> = (0..300).map(|i| (i * 13) as u8).collect();
        d.stations[1].send(&msg).unwrap();
        assert!(d.run_until(40.0, |d| d.settled(1)), "{}", d.log());
        assert_eq!(d.received(0), vec![msg], "{}", d.log());
        assert!(d.delivered_ms(1).is_some());
    }

    #[test]
    fn both_press_send_and_both_messages_arrive() {
        let mut d = Duplex::new(&room(3));
        d.run(0.4);
        d.stations[0].send(b"A:lta B:lle").unwrap();
        d.run(0.35);
        d.stations[1].send("B:ltä A:lle ääkkösin".as_bytes()).unwrap();
        assert!(d.run_until(40.0, |d| d.settled(0) && d.settled(1)), "{}", d.log());
        assert_eq!(d.received(1), vec![b"A:lta B:lle".to_vec()], "{}", d.log());
        assert_eq!(d.received(0), vec!["B:ltä A:lle ääkkösin".as_bytes().to_vec()], "{}", d.log());
    }

    #[test]
    fn no_receiver_in_the_room() {
        let mut cfg = room(4);
        cfg.sides[0].level = 0.0; // A ei kuule B:tä
        cfg.sides[1].level = 0.0; // eikä B A:ta
        let mut d = Duplex::new(&cfg);
        d.stations[0].send(LINK).unwrap();
        assert!(d.run_until(15.0, |d| d.settled(0)), "{}", d.log());
        assert!(d.events[0].iter().any(|e| matches!(e, Event::Failed { reason: FailReason::NoReceiver, .. })), "{}", d.log());
        assert!(d.time_s() < 10.0, "kesti {} s", d.time_s());
        assert!(d.received(1).is_empty());
    }

    #[test]
    fn loud_interference_during_transfer_is_survived() {
        let mut cfg = room(5);
        // Kolahduksia ja puhetta vastaanottajan mikrofonissa kesken datan.
        cfg.sides[1].interference = vec![
            Interference { kind: InterferenceKind::Click, start_s: 2.2, duration_s: 0.08, rms: 0.5 },
            Interference { kind: InterferenceKind::Babble, start_s: 3.0, duration_s: 1.5, rms: 0.3 },
            Interference { kind: InterferenceKind::Click, start_s: 5.1, duration_s: 0.08, rms: 0.5 },
        ];
        let mut d = Duplex::new(&cfg);
        d.run(0.5);
        let msg: Vec<u8> = (0..400).map(|i| (i * 7 + 1) as u8).collect();
        d.stations[0].send(&msg).unwrap();
        assert!(d.run_until(60.0, |d| d.settled(0)), "{}", d.log());
        assert_eq!(d.received(1), vec![msg], "{}", d.log());
    }

    #[test]
    fn cancel_and_sender_disappearing() {
        let mut d = Duplex::new(&room(6));
        d.run(0.3);
        d.stations[0].send(&vec![7u8; 500]).unwrap();
        assert!(d.run_until(10.0, |d| d.events[1].iter().any(|e| matches!(e, Event::RxProgress { have: 2, .. }))), "{}", d.log());
        d.stations[0].cancel();
        assert!(d.run_until(5.0, |d| d.events[1].iter().any(|e| matches!(e, Event::RxAborted { reason: AbortReason::Cancelled }))), "{}", d.log());
        assert!(d.events[0].contains(&Event::Cancelled));
        assert!(d.received(1).is_empty());
        // Uusi viesti menee perille peruutuksen jälkeen.
        d.run(0.5);
        d.stations[0].send(b"uusi").unwrap();
        assert!(d.run_until(15.0, |d| d.received(1).len() == 1), "{}", d.log());
    }

    #[test]
    fn headphones_no_self_echo() {
        let mut cfg = room(7);
        cfg.sides[0].self_level = 0.0;
        cfg.sides[1].self_level = 0.0;
        let mut d = Duplex::new(&cfg);
        d.run(0.2);
        d.stations[0].send(LINK).unwrap();
        assert!(d.run_until(20.0, |d| d.settled(0)), "{}", d.log());
        assert_eq!(d.received(1), vec![LINK.to_vec()], "{}", d.log());
    }
}
