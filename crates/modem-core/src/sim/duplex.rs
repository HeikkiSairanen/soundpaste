//! Two-way simulation: two stations in the same room.
//!
//! ```text
//!   station A ──output delay──► room A→B ──┐
//!      ▲  └──────► self-leak A→A ──┐        ├─► + noise, interference, clipping ─► station B
//!      │                           ▼        │
//!      └── + noise, interference ◄── room B→A ◄──output delay── station B (+ self-leak B→B)
//! ```
//!
//! Each station has its own sample rate, clock and audio latencies.
//! The simulation advances in 10 ms steps; the stations hear each other's audio only
//! on the next step, which corresponds to a small extra delay.

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
    /// Audio output delay (buffers before the speaker) and input delay (from the microphone to the program).
    pub out_latency_s: f64,
    pub in_latency_s: f64,
    /// Room impulse response from the other station's speaker to this station's microphone.
    pub ir: Vec<f32>,
    /// RMS of the received signal at the microphone.
    pub level: f32,
    /// Leak of the station's own speaker into its own microphone: impulse response and level. Level 0 =
    /// does not hear itself (headphones).
    pub self_ir: Vec<f32>,
    pub self_level: f32,
    pub noise: NoiseKind,
    pub noise_rms: f32,
    pub interference: Vec<Interference>,
    pub link: LinkConfig,
}

#[derive(Clone, Debug)]
pub struct DuplexConfig {
    /// The bands of both stations: one waveform, or a band ladder (lowest first).
    pub kinds: Vec<PhyKind>,
    pub sides: [Side; 2],
    /// Clock error of station B relative to station A.
    pub clock_ppm: f64,
    pub seed: u64,
}

/// RMS of the transmission against which levels are scaled.
const TX_RMS: f32 = 0.45;

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
        let station = |i: usize, seed: u64| Station::with_bands(&cfg.kinds, cfg.sides[i].fs, cfg.sides[i].link.clone(), seed);
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

    /// Advance 10 ms.
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
            // Microphone i hears the other station through the room and its own speaker as a leak.
            self.tmp.clear();
            self.mics[i].cross.process(&outputs[1 - i], &mut self.tmp);
            let cross = std::mem::take(&mut self.tmp);
            let mut own = Vec::new();
            self.mics[i].own.process(&outputs[i], &mut own);
            // The output counts of the paths can differ by a sample because of clock error: sum
            // into the queue so that each path keeps its own timeline.
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

    /// Run until the condition holds or the time limit is exceeded. Returns whether the condition held.
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

    /// Has the transmission ended (successfully or not)?
    pub fn settled(&self, i: usize) -> bool {
        self.delivered_ms(i).is_some() || self.failed(i)
    }

    /// Debug log from both stations.
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
    /// An ordinary laptop in a quiet room.
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
        DuplexConfig { kinds: vec![PhyKind::DEFAULT], sides: [side(48000.0, 1), side(48000.0, 2)], clock_ppm: 0.0, seed }
    }

    #[test]
    fn normal_session_over_audio() {
        let mut d = Duplex::new(&room(1));
        d.run(0.5);
        d.stations[0].send(LINK).unwrap();
        assert!(d.run_until(15.0, |d| d.settled(0)), "{}", d.log());
        assert_eq!(d.received(1), vec![LINK.to_vec()], "{}", d.log());
        let ms = d.delivered_ms(0).expect("not delivered");
        // Inline start (default): about 2.6 s.
        assert!((2000..3600).contains(&ms), "took {ms} ms\n{}", d.log());
        // Neither interpreted its own echo as a message.
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
        // DESIGN.md section 6: about 4.1 s.
        let ms = d.delivered_ms(0).expect("not delivered");
        assert!((3200..5200).contains(&ms), "took {ms} ms\n{}", d.log());
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
    fn ultrasonic_longest_message_between_44k_and_48k() {
        // Longer frames than v1: the receiver's timeouts must still hold, and the
        // 44.1 kHz side's band edge sits right under its Nyquist frequency.
        let mut cfg = room(7);
        cfg.kinds = vec![PhyKind::Mfsk(&crate::phy::mfsk::ULTRASONIC)];
        cfg.sides[1].fs = 44100.0;
        cfg.sides[1].ir = synth_ir(44100.0, 0.35, 8.0, 17);
        cfg.sides[1].self_ir = synth_ir(44100.0, 0.35, 14.0, 18);
        cfg.clock_ppm = 100.0;
        let mut d = Duplex::new(&cfg);
        d.run(0.3);
        let msg: Vec<u8> = (0..512).map(|i| (i * 31 + 5) as u8).collect();
        d.stations[0].send(&msg).unwrap();
        assert!(d.run_until(120.0, |d| d.settled(0)), "{}", d.log());
        assert_eq!(d.received(1), vec![msg], "{}", d.log());
        assert!(d.delivered_ms(0).is_some(), "{}", d.log());
    }

    #[test]
    fn normal_and_ultrasonic_stations_do_not_hear_each_other() {
        let mut d = Duplex::new(&room(8));
        d.stations[1] = Station::new(PhyKind::Mfsk(&crate::phy::mfsk::ULTRASONIC), 48000.0, LinkConfig::default(), 99);
        d.run(0.3);
        d.stations[1].send(LINK).unwrap();
        assert!(d.run_until(20.0, |d| d.settled(1)), "{}", d.log());
        assert!(d.events[1].iter().any(|e| matches!(e, Event::Failed { reason: FailReason::NoReceiver, .. })), "{}", d.log());
        assert!(d.received(0).is_empty());
        assert_eq!(d.stations[0].stats().preambles, 0);
    }

    // ---------------------------------------------------------------- band ladder

    fn ladder_room(seed: u64) -> DuplexConfig {
        DuplexConfig { kinds: crate::phy::LADDER.to_vec(), ..room(seed) }
    }

    /// A microphone path that stops at `fc`: the room response through a sharp low-pass
    /// filter (Blackman-windowed sinc, 801 taps).
    fn cut_at(ir: &[f32], fc: f64, fs: f64) -> Vec<f32> {
        let n = 801;
        let m = (n - 1) as f64;
        let h: Vec<f64> = (0..n)
            .map(|i| {
                let x = i as f64 - m / 2.0;
                let sinc = if x == 0.0 { 2.0 * fc / fs } else { (std::f64::consts::TAU * fc / fs * x).sin() / (std::f64::consts::PI * x) };
                let w = 0.42 - 0.5 * (std::f64::consts::TAU * i as f64 / m).cos() + 0.08 * (2.0 * std::f64::consts::TAU * i as f64 / m).cos();
                sinc * w
            })
            .collect();
        let mut out = vec![0.0f32; ir.len() + n - 1];
        for (i, &x) in ir.iter().enumerate() {
            for (k, &hk) in h.iter().enumerate() {
                out[i + k] += x * hk as f32;
            }
        }
        out
    }

    /// The band the sender ended on (its last band event).
    fn sender_band(d: &Duplex, i: usize) -> Option<u8> {
        d.events[i].iter().rev().find_map(|e| match e {
            Event::Band { band, sending: true, .. } => Some(*band),
            _ => None,
        })
    }

    fn ladder_transfer(cfg: &DuplexConfig) -> (Duplex, u8) {
        let mut d = Duplex::new(cfg);
        d.run(0.4);
        d.stations[0].send(LINK).unwrap();
        assert!(d.run_until(40.0, |d| d.settled(0)), "{}", d.log());
        assert_eq!(d.received(1), vec![LINK.to_vec()], "{}", d.log());
        assert!(d.delivered_ms(0).is_some(), "{}", d.log());
        let band = sender_band(&d, 0).expect("no band chosen");
        (d, band)
    }

    #[test]
    fn ladder_climbs_to_the_top_band_in_an_open_room() {
        let (d, band) = ladder_transfer(&ladder_room(11));
        assert_eq!(band, 3, "{}", d.log());
        // The receiver heard the data on the same band.
        assert!(d.events[1].iter().any(|e| matches!(e, Event::Band { band: 3, sending: false, lo_hz: 17600, hi_hz: 20000 })), "{}", d.log());
    }

    #[test]
    fn ladder_stops_under_the_receivers_cutoff() {
        // B's microphone stops at 14.6 kHz (between rungs 1 and 2), then at 9 kHz (above the
        // normal band only).
        for (fc, expected) in [(14_600.0, 1), (9_000.0, 0)] {
            let mut cfg = ladder_room(12);
            cfg.sides[1].ir = cut_at(&cfg.sides[1].ir, fc, 48000.0);
            let (d, band) = ladder_transfer(&cfg);
            assert_eq!(band, expected, "cutoff {fc} Hz\n{}", d.log());
        }
    }

    #[test]
    fn ladder_answers_on_the_band_the_return_direction_carries() {
        // A -> B carries everything, but A's own microphone stops at 17.4 kHz: the data goes
        // on the top band and B's answers on band 2 (EXPERIMENTS.md entry 9).
        let mut cfg = ladder_room(13);
        cfg.sides[0].ir = cut_at(&cfg.sides[0].ir, 17_400.0, 48000.0);
        let (d, band) = ladder_transfer(&cfg);
        assert_eq!(band, 3, "{}", d.log());
        assert!(d.log().contains("replies on 2"), "{}", d.log());
        assert!(d.log().lines().any(|l| l.starts_with("A Log(\"<- DONE") && l.contains("band 2")), "{}", d.log());
    }

    #[test]
    fn ladder_quiet_data_with_answers_on_the_normal_band() {
        // The Mac -> PC case of entry 9: B's speaker carries nothing above 9 kHz (A's
        // microphone path is cut there), but A -> B carries every band.
        let mut cfg = ladder_room(16);
        cfg.sides[0].ir = cut_at(&cfg.sides[0].ir, 9_000.0, 48000.0);
        let (d, band) = ladder_transfer(&cfg);
        assert_eq!(band, 3, "{}", d.log());
        assert!(d.log().contains("replies on 0"), "{}", d.log());
    }

    #[test]
    fn ladder_station_receives_from_a_single_band_station() {
        // An older station (v1 only, inline start) sends to a ladder station.
        let mut d = Duplex::new(&ladder_room(14));
        d.stations[0] = Station::new(PhyKind::DEFAULT, 48000.0, LinkConfig::default(), 5);
        d.run(0.3);
        d.stations[0].send(LINK).unwrap();
        assert!(d.run_until(15.0, |d| d.settled(0)), "{}", d.log());
        assert_eq!(d.received(1), vec![LINK.to_vec()], "{}", d.log());
        assert!(d.delivered_ms(0).is_some(), "{}", d.log());
    }

    /// Every band either station transmitted on, from the log ("bands 3", "bands 0+1+2+3").
    fn bands_sent(d: &Duplex, i: usize) -> std::collections::BTreeSet<u8> {
        d.events[i]
            .iter()
            .filter_map(|e| match e {
                Event::Log(s) if s.starts_with("-> ") => Some(s.clone()),
                _ => None,
            })
            .flat_map(|s| s.split("bands ").nth(1).map(|b| b.trim_end_matches(')').split('+').map(|x| x.parse::<u8>().unwrap()).collect::<Vec<_>>()).unwrap_or_else(|| vec![0]))
            .collect()
    }

    #[test]
    fn ultrasonic_only_station_and_ladder_station_talk_silently() {
        // The page's "ultrasonic handshake" switch: a single-band v1-us station. Both
        // directions, and neither station may make a sound outside 17.6–20 kHz.
        let us = PhyKind::Mfsk(&crate::phy::mfsk::ULTRASONIC);
        for silent in [0, 1] {
            let mut d = Duplex::new(&ladder_room(17 + silent as u64));
            d.stations[silent] = Station::new(us, 48000.0, LinkConfig::default(), 5);
            d.run(0.3);
            d.stations[0].send(LINK).unwrap();
            assert!(d.run_until(30.0, |d| d.settled(0)), "{}", d.log());
            assert_eq!(d.received(1), vec![LINK.to_vec()], "{}", d.log());
            assert!(d.delivered_ms(0).is_some(), "{}", d.log());
            assert_eq!(bands_sent(&d, silent), [0].into(), "the silent station has one band, its own\n{}", d.log());
            // The ladder station answered or sent only on its top band (3), apart from the
            // first START's probes when it is the sender.
            let ladder = 1 - silent;
            let sent: Vec<_> = d.events[ladder].iter().filter_map(|e| match e { Event::Log(s) if s.starts_with("-> ") => Some(s.clone()), _ => None }).collect();
            for s in &sent {
                assert!(s.contains("bands 3") || s.contains("bands 0+1+2+3"), "{s}\n{}", d.log());
            }
        }
    }

    #[test]
    fn ladder_both_press_send_and_both_messages_arrive() {
        let mut d = Duplex::new(&ladder_room(15));
        d.run(0.4);
        d.stations[0].send(b"from A to B").unwrap();
        d.run(0.35);
        d.stations[1].send(b"from B to A").unwrap();
        assert!(d.run_until(60.0, |d| d.settled(0) && d.settled(1)), "{}", d.log());
        assert_eq!(d.received(1), vec![b"from A to B".to_vec()], "{}", d.log());
        assert_eq!(d.received(0), vec![b"from B to A".to_vec()], "{}", d.log());
    }

    #[test]
    fn both_press_send_and_both_messages_arrive() {
        let mut d = Duplex::new(&room(3));
        d.run(0.4);
        d.stations[0].send(b"from A to B").unwrap();
        d.run(0.35);
        d.stations[1].send("from B to A, café niño".as_bytes()).unwrap();
        assert!(d.run_until(40.0, |d| d.settled(0) && d.settled(1)), "{}", d.log());
        assert_eq!(d.received(1), vec![b"from A to B".to_vec()], "{}", d.log());
        assert_eq!(d.received(0), vec!["from B to A, café niño".as_bytes().to_vec()], "{}", d.log());
    }

    #[test]
    fn no_receiver_in_the_room() {
        let mut cfg = room(4);
        cfg.sides[0].level = 0.0; // A does not hear B
        cfg.sides[1].level = 0.0; // nor B A
        let mut d = Duplex::new(&cfg);
        d.stations[0].send(LINK).unwrap();
        assert!(d.run_until(15.0, |d| d.settled(0)), "{}", d.log());
        assert!(d.events[0].iter().any(|e| matches!(e, Event::Failed { reason: FailReason::NoReceiver, .. })), "{}", d.log());
        assert!(d.time_s() < 10.0, "took {} s", d.time_s());
        assert!(d.received(1).is_empty());
    }

    #[test]
    fn loud_interference_during_transfer_is_survived() {
        let mut cfg = room(5);
        // Thumps and speech at the receiver's microphone in the middle of the data.
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
        // A new message gets through after the cancellation.
        d.run(0.5);
        d.stations[0].send(b"next").unwrap();
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
