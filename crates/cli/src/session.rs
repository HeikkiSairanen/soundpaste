//! Two-way session in the simulator: the bench run's session mode, and running a
//! single session for debugging.

use crate::args::Args;
use crate::bench::{self, Point, Profile};
use modem_core::link::{Event, LinkConfig, TxStatus};
use modem_core::phy::PhyKind;
use modem_core::rng::Rng;
use modem_core::sim::{synth_ir, Duplex, DuplexConfig, OneWay, Side};
use std::process::ExitCode;

/// SNR grid for session mode (sparser than in the one-way run, because a
/// session is ten times heavier to simulate).
pub const SESSION_GRID: &[f64] = &[-9.0, -6.0, -3.0, 0.0, 3.0, 6.0, 12.0, 18.0, 24.0, 30.0];

/// Build a two-station room from a one-way profile.
///
/// - A transmits at the profile's transmitter rate, B receives at the receiver rate.
/// - Each direction has its own room response (same profile, different seed).
/// - Each station hears its own speaker four times louder than the other's.
/// - Audio latencies are drawn at random: output 20–250 ms, input 10–150 ms.
/// - The profile's interference (thump, speech) hits B's microphone during the data frame.
pub fn duplex_config(kind: PhyKind, profile: &Profile, base: &OneWay, snr_db: f64, seed: u64, link: &LinkConfig) -> DuplexConfig {
    let mut rng = Rng::new(seed ^ 0xD0_91E5);
    let back = (profile.make)(snr_db, seed ^ 0x5A5A_1234);
    let noise_rms = base.rx_level / 10f32.powf(snr_db as f32 / 20.0);
    let mut side = |fs: f64, ir: &Vec<f32>, interference: Vec<modem_core::sim::Interference>| Side {
        fs,
        out_latency_s: rng.range(0.02, 0.25),
        in_latency_s: rng.range(0.01, 0.15),
        ir: ir.clone(),
        level: base.rx_level,
        self_ir: synth_ir(fs, 0.3, 12.0, rng.next_u64()),
        self_level: (base.rx_level * 4.0).min(0.7),
        noise: base.noise.clone(),
        noise_rms,
        interference,
        link: link.clone(),
    };
    // Interference is moved from the one-way run's timeline to the session's data frame.
    let interference = base
        .interference
        .iter()
        .map(|i| modem_core::sim::Interference { start_s: 2.3 + (i.start_s - base.lead_s), ..*i })
        .collect();
    // The return-direction response is computed for B's rate; if A's rate differs, the room
    // stretches in time by a few percent, which does not matter here.
    let a = side(base.fs_tx, &back.ir, Vec::new());
    let b = side(base.fs_rx, &base.ir, interference);
    DuplexConfig { kind, sides: [a, b], clock_ppm: base.clock_ppm, seed }
}

pub struct SessionOutcome {
    pub ok: bool,
    pub elapsed_ms: Option<u64>,
    pub data_and_polls: u32,
    pub duplex: Duplex,
}

/// One session: A sends a message to B. Success requires that A got the
/// confirmation and B showed exactly the right message exactly once.
pub fn run_session(cfg: &DuplexConfig, msg: &[u8]) -> SessionOutcome {
    let mut d = Duplex::new(cfg);
    d.run(0.5);
    d.stations[0].send(msg).expect("send did not start");
    d.run_until(90.0, |d| d.settled(0));
    d.run(0.3);
    let elapsed_ms = d.delivered_ms(0);
    let ok = elapsed_ms.is_some() && d.received(1) == vec![msg.to_vec()] && d.received(0).is_empty();
    let data_and_polls = d.events[0].iter().filter(|e| matches!(e, Event::Tx(TxStatus::Sending { .. } | TxStatus::Polling { .. }))).count() as u32;
    SessionOutcome { ok, elapsed_ms, data_and_polls, duplex: d }
}

pub fn session_point(kind: PhyKind, profile: &Profile, base: &OneWay, snr_db: f64, seed: u64, link: &LinkConfig) -> Point {
    let cfg = duplex_config(kind, profile, base, snr_db, seed, link);
    let out = run_session(&cfg, bench::LINK60.as_bytes());
    let b = out.duplex.stations[1].stats();
    Point {
        snr_db,
        n: 1,
        ok: out.ok as u32,
        detected: (b.preambles > 0) as u32,
        corrected: b.corrected as u64,
        times: if out.ok { vec![out.elapsed_ms.unwrap() as u32] } else { Vec::new() },
        frames_sent: out.data_and_polls as u64,
        ..Default::default()
    }
}

pub fn link_config(a: &Args) -> Result<(LinkConfig, String), String> {
    let cfg = LinkConfig { burst: a.num("burst", 0)?, inline_start: !a.has("handshake"), ..LinkConfig::default() };
    let desc = format!("{}{}", if cfg.burst == 0 { "burst".to_string() } else { format!("{} frames/ack", cfg.burst) }, if cfg.inline_start { ", inline start" } else { ", always handshake" });
    Ok((cfg, desc))
}

/// `soundpaste session`: run one session and print the events with timestamps.
pub fn cmd_session(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &["handshake", "quiet"])?;
    let kind = crate::phy_arg(&a)?;
    let snr: f64 = a.num("snr", 20.0)?;
    let seed: u64 = a.num("seed", 1)?;
    let name = a.str_or("profile", "room");
    let len: usize = a.num("len", 0)?;
    let text: Vec<u8> = match a.positional.first() {
        Some(t) => t.as_bytes().to_vec(),
        None if len > 0 => (0..len).map(|i| b'a' + (i % 26) as u8).collect(),
        None => bench::LINK60.as_bytes().to_vec(),
    };
    let (link, desc) = link_config(&a)?;
    let mut profiles = bench::synthetic_profiles();
    profiles.extend(bench::measured_profiles(true));
    let profile = profiles.iter().find(|p| p.name == name).ok_or_else(|| format!("unknown profile '{name}'"))?;
    let base = (profile.make)(snr, seed);
    let cfg = duplex_config(kind, profile, &base, snr, seed, &link);
    println!("SESSION {} ({desc}), profile {name}, SNR {snr} dB, seed {seed}, message {} bytes", kind.name(), text.len());
    for (i, s) in cfg.sides.iter().enumerate() {
        println!("  station {}: {} Hz, output latency {:.0} ms, input latency {:.0} ms", ["A", "B"][i], s.fs, s.out_latency_s * 1e3, s.in_latency_s * 1e3);
    }
    let mut d = Duplex::new(&cfg);
    let mut printed = [0usize; 2];
    let mut sent = false;
    while d.time_s() < 90.0 {
        if !sent && d.time_s() >= 0.5 {
            d.stations[0].send(&text).map_err(|e| format!("{e:?}"))?;
            sent = true;
        }
        d.step();
        for i in 0..2 {
            for e in &d.events[i][printed[i]..] {
                if !a.has("quiet") || !matches!(e, Event::Log(_)) {
                    let line = match e {
                        Event::Log(s) => s.clone(),
                        Event::Received { data, .. } => format!("RECEIVED: {}", String::from_utf8_lossy(data)),
                        other => format!("{other:?}"),
                    };
                    println!("{:7.2} s  {}  {line}", d.time_s(), ["A", "B"][i]);
                }
            }
            printed[i] = d.events[i].len();
        }
        if sent && d.settled(0) && !d.stations[1].is_receiving() && d.time_s() > 1.0 {
            d.run(0.5);
            break;
        }
    }
    let ok = d.delivered_ms(0).is_some() && d.received(1) == vec![text.clone()];
    println!("{}", match d.delivered_ms(0) {
        Some(ms) if ok => format!("DELIVERED {:.2} s", ms as f64 / 1000.0),
        _ => "FAILED".to_string(),
    });
    for i in 0..2 {
        let s = d.stations[i].stats();
        println!("  station {}: preambles {}, intact frames {}, rejected {}, sent {}, corrected bytes {}", ["A", "B"][i], s.preambles, s.frames_ok, s.frames_failed, s.frames_sent, s.corrected);
    }
    Ok(if ok { ExitCode::SUCCESS } else { ExitCode::from(1) })
}
