//! Bench run through a simulated channel, and the scoreboard.
//!
//! Every run produces the same scoreboard so that changes can be compared:
//! - transfer time: transferring a 60-character link (with acks when the link layer is in use)
//! - success rate: share of transfers in nominal conditions (all profiles, SNR 12–30 dB)
//! - min SNR: the lowest signal-to-noise ratio at and above which at least 90 % succeed
//! - BER before error correction and the number of corrected symbols: margin
//! - net rate: payload bytes per second
//!
//! Seeds are fixed, so the same code always produces the same result.

use crate::corpus::{self, CorpusScore};
use crate::json::Json;
use modem_core::phy::{bit_errors, PhyEvent, PhyKind};
use modem_core::rng::Rng;
use modem_core::sim::{run_oneway, synth_ir, Interference, InterferenceKind, NoiseKind, OneWay};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Bench message: a 60-character link.
pub const LINK60: &str = "https://sairanen.org/soundpaste/?v=Zk3mQ9xT7bLw2RfN8cYh5VpJd";

pub const SNR_GRID: &[f64] = &[-15.0, -12.0, -9.0, -6.0, -3.0, 0.0, 3.0, 6.0, 9.0, 12.0, 15.0, 18.0, 21.0, 24.0, 27.0, 30.0];
const NOMINAL_MIN_SNR: f64 = 12.0;
const SUCCESS_LIMIT: f64 = 0.9;

pub struct Profile {
    pub name: String,
    pub desc: String,
    /// Is the profile kept out of tuning (holdout set)?
    pub holdout: bool,
    pub make: Box<dyn Fn(f64, u64) -> OneWay + Send + Sync>,
}

fn room(fs_rx: f64, rt60: f64, drr: f64, seed: u64) -> Vec<f32> {
    synth_ir(fs_rx, rt60, drr, seed)
}

/// Synthetic channel profiles. Each seed has its own room (the random part of the
/// impulse response), so that the modem is not tuned to a single echo.
pub fn synthetic_profiles() -> Vec<Profile> {
    fn p(name: &str, desc: &str, make: impl Fn(f64, u64) -> OneWay + Send + Sync + 'static) -> Profile {
        Profile { name: name.into(), desc: desc.into(), holdout: false, make: Box::new(make) }
    }
    let base = |snr: f64, seed: u64| {
        let mut rng = Rng::new(seed ^ 0xC0FFEE);
        let mut c = OneWay::clean(48000.0);
        c.snr_db = snr;
        c.lead_s = rng.range(0.15, 0.7);
        c
    };
    vec![
        p("clean", "no reverb, white noise", move |snr, seed| base(snr, seed)),
        p("room", "RT60 0.35 s, DRR 8 dB, white noise", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 0.35, 8.0, seed);
            c
        }),
        p("reverberant", "RT60 0.8 s, DRR 0 dB", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 0.8, 0.0, seed);
            c
        }),
        p("hall", "RT60 1.2 s, DRR -5 dB: reverb clearly louder than the direct sound", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 1.2, -5.0, seed);
            c
        }),
        p("room-noise", "room + pink (low-frequency weighted) noise", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 0.35, 8.0, seed);
            c.noise = NoiseKind::Pink;
            c
        }),
        p("clock-44-48", "transmitter 44.1 kHz, receiver 48 kHz +120 ppm, room", move |snr, seed| {
            let mut c = base(snr, seed);
            c.fs_tx = 44100.0;
            c.clock_ppm = 120.0;
            c.ir = room(48000.0, 0.35, 8.0, seed);
            c
        }),
        p("clock-48-44", "transmitter 48 kHz, receiver 44.1 kHz -120 ppm, room", move |snr, seed| {
            let mut c = base(snr, seed);
            c.fs_rx = 44100.0;
            c.clock_ppm = -120.0;
            c.ir = room(44100.0, 0.35, 8.0, seed);
            c
        }),
        p("clipping", "microphone clips heavily (signal RMS 0.9), room", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 0.35, 8.0, seed);
            c.rx_level = 0.9;
            c
        }),
        p("faint", "very quiet signal (RMS 0.0005), room", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 0.35, 8.0, seed);
            c.rx_level = 0.0005;
            c
        }),
        p("thump", "room + three thumps during the frame (peak 14 dB above the signal)", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 0.35, 8.0, seed);
            let mut rng = Rng::new(seed ^ 0xC11C);
            c.interference = (0..3)
                .map(|_| Interference { kind: InterferenceKind::Click, start_s: c.lead_s + rng.range(0.15, 1.2), duration_s: 0.08, rms: 0.5 })
                .collect();
            c
        }),
        p("speech", "room + 0.5 s of speech-like interference during the frame (6 dB above the signal)", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 0.35, 8.0, seed);
            let mut rng = Rng::new(seed ^ 0x5BEE);
            c.interference = vec![Interference { kind: InterferenceKind::Babble, start_s: c.lead_s + rng.range(0.1, 0.7), duration_s: 0.5, rms: 0.2 }];
            c
        }),
    ]
}

/// Measured channels from the corpus. Each measurement yields two profiles:
/// - `measured:<id>`: the impulse response and background noise as they are
/// - `measured:<id>:far`: the same, but with the direct sound attenuated by 14 dB.
///   When the speaker and microphone are in the same machine, the direct sound is
///   much louder relative to the reverb than it would be for another machine a metre
///   away. The reverberant field is roughly the same everywhere in a room, so
///   attenuating the direct sound approximates a receiver further away. This is a
///   derived estimate, not a measurement.
pub fn measured_profiles(include_holdout: bool) -> Vec<Profile> {
    let mut out = Vec::new();
    for (path, meta) in corpus::entries(corpus::CHANNEL_DIR) {
        let holdout = corpus::is_holdout(&meta);
        if holdout && !include_holdout {
            continue;
        }
        let id = meta.str_at("id").unwrap_or("?").to_string();
        let base = path.with_extension("");
        let (Ok(ir), Ok(noise)) = (corpus::read_wav(&base.with_extension("ir.wav")), corpus::read_wav(&base.with_extension("noise.wav"))) else {
            eprintln!("warning: channel {id} is missing ir.wav or noise.wav");
            continue;
        };
        let fs = ir.sample_rate as f64;
        let pre = meta.get("results").and_then(|r| r.num_at("ir_pre")).unwrap_or(0.0) as usize;
        let noise = std::sync::Arc::new(noise.samples);
        let note = meta.str_at("note").unwrap_or("");
        for far in [false, true] {
            let mut h = ir.samples.clone();
            if far {
                let direct = (pre + (0.0025 * fs) as usize).min(h.len());
                let g = 10f32.powf(-14.0 / 20.0);
                h[..direct].iter_mut().for_each(|v| *v *= g);
            }
            let noise = noise.clone();
            out.push(Profile {
                name: format!("measured:{id}{}", if far { ":far" } else { "" }),
                desc: format!("measured channel and background noise{} {note}", if far { ", direct sound -14 dB (derived)" } else { "" }),
                holdout,
                make: Box::new(move |snr, seed| {
                    let mut rng = Rng::new(seed ^ 0xC0FFEE);
                    let mut c = OneWay::clean(48000.0);
                    c.fs_rx = fs;
                    c.snr_db = snr;
                    c.lead_s = rng.range(0.15, 0.7);
                    c.ir = h.clone();
                    c.noise = NoiseKind::Recorded(noise.clone());
                    c
                }),
            });
        }
    }
    out
}

#[derive(Default, Clone, Debug)]
pub struct Point {
    pub snr_db: f64,
    pub n: u32,
    pub ok: u32,
    pub detected: u32,
    /// A frame that passed as intact but had the wrong content (must not happen).
    pub false_frames: u32,
    pub bit_errors: u64,
    pub bits: u64,
    pub corrected: u64,
    /// Durations of successful sessions (ms), session mode only.
    pub times: Vec<u32>,
    /// Data frames and POLL frames sent (a measure of retransmissions), session mode only.
    pub frames_sent: u64,
}

impl Point {
    pub fn success(&self) -> f64 {
        self.ok as f64 / self.n.max(1) as f64
    }
    pub fn ber(&self) -> Option<f64> {
        (self.bits > 0).then(|| self.bit_errors as f64 / self.bits as f64)
    }
    fn add(&mut self, o: &Point) {
        self.n += o.n;
        self.ok += o.ok;
        self.detected += o.detected;
        self.false_frames += o.false_frames;
        self.bit_errors += o.bit_errors;
        self.bits += o.bits;
        self.corrected += o.corrected;
        self.times.extend_from_slice(&o.times);
        self.frames_sent += o.frames_sent;
    }
}

pub struct ProfileResult {
    pub name: String,
    pub desc: String,
    pub holdout: bool,
    pub points: Vec<Point>,
}

impl ProfileResult {
    /// The lowest SNR at and above which the success rate is at least the limit.
    pub fn min_snr(&self) -> Option<f64> {
        let mut best = None;
        for p in self.points.iter().rev() {
            if p.success() >= SUCCESS_LIMIT {
                best = Some(p.snr_db);
            } else {
                break;
            }
        }
        best
    }
}

/// One one-way transfer: modulate, pass through the channel, demodulate.
pub fn run_once(kind: PhyKind, cfg: &OneWay, payload: &[u8], seed: u64) -> Point {
    let modulator = kind.modulator(cfg.fs_tx);
    let tx = modulator.modulate(payload);
    let coded = modulator.coded(payload);
    let rx = run_oneway(&tx, cfg, seed);
    let mut demod = kind.demodulator(cfg.fs_rx);
    let mut events = Vec::new();
    // Feed in blocks of varying size, as a real audio device would.
    let mut rng = Rng::new(seed ^ 0xB10C);
    let mut pos = 0;
    while pos < rx.len() {
        let n = (64 + rng.below(4000) as usize).min(rx.len() - pos);
        demod.push(&rx[pos..pos + n], &mut events);
        pos += n;
    }
    let mut pt = Point { n: 1, ..Default::default() };
    let mut best_raw: Option<(u64, u64)> = None;
    for ev in &events {
        match ev {
            PhyEvent::Detected { .. } => pt.detected = 1,
            PhyEvent::Frame { payload: got, raw, stats } => {
                if got == payload {
                    pt.ok = 1;
                    pt.corrected = stats.corrected as u64;
                    best_raw = Some(bit_errors(&coded, raw));
                } else {
                    pt.false_frames += 1;
                }
            }
            PhyEvent::Failed { raw, .. } => {
                let e = bit_errors(&coded, raw);
                if best_raw.map_or(true, |b| e.0 < b.0) && pt.ok == 0 {
                    best_raw = Some(e);
                }
            }
        }
    }
    if let Some((e, b)) = best_raw {
        pt.bit_errors = e;
        pt.bits = b;
    }
    pt
}

pub struct BenchOptions {
    pub kind: PhyKind,
    pub seeds: u32,
    pub threads: usize,
}

pub fn run_profiles(opts: &BenchOptions, profiles: &[Profile]) -> Vec<ProfileResult> {
    let payload = LINK60.as_bytes();
    let kind = opts.kind;
    run_grid(opts, profiles, SNR_GRID, &|cfg, seed| run_once(kind, cfg, payload, seed))
}

/// Run `job` for every (profile, SNR, repetition) combination in parallel.
pub fn run_grid(opts: &BenchOptions, profiles: &[Profile], grid: &[f64], job: &(dyn Fn(&OneWay, u64) -> Point + Sync)) -> Vec<ProfileResult> {
    let jobs: Vec<(usize, usize, u32)> = (0..profiles.len())
        .flat_map(|p| (0..grid.len()).flat_map(move |s| (0..opts.seeds).map(move |i| (p, s, i))))
        .collect();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Vec<Point>>> = Mutex::new(
        profiles.iter().map(|_| grid.iter().map(|&snr_db| Point { snr_db, ..Default::default() }).collect()).collect(),
    );
    std::thread::scope(|scope| {
        for _ in 0..opts.threads.max(1) {
            scope.spawn(|| loop {
                let j = next.fetch_add(1, Ordering::Relaxed);
                let Some(&(p, s, i)) = jobs.get(j) else { break };
                // The seed depends only on the profile name, the SNR value and the repetition,
                // so the same channel recurs across modem versions.
                let name_hash = profiles[p].name.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
                let snr_index = SNR_GRID.iter().position(|&x| x == grid[s]).unwrap_or(s + 100);
                let seed = name_hash ^ ((snr_index as u64) << 32) ^ (i as u64 + 1).wrapping_mul(0x9E37_79B9);
                let cfg = (profiles[p].make)(grid[s], seed);
                let pt = job(&cfg, seed);
                results.lock().unwrap()[p][s].add(&pt);
            });
        }
    });
    let points = results.into_inner().unwrap();
    profiles
        .iter()
        .zip(points)
        .map(|(p, points)| ProfileResult { name: p.name.clone(), desc: p.desc.clone(), holdout: p.holdout, points })
        .collect()
}

pub struct Scoreboard {
    pub phy: String,
    pub mode: String,
    pub commit: String,
    pub time_utc: String,
    pub seeds: u32,
    pub transfer_time_s: f64,
    pub transfer_note: String,
    pub success_rate: f64,
    pub detect_rate: f64,
    pub false_frames: u32,
    pub ber: Option<f64>,
    pub corrected_per_frame: f64,
    pub net_rate: f64,
    pub corpus: CorpusScore,
    /// Data frames and ack requests per session in nominal conditions (1.0 = no retransmissions).
    pub frames_per_session: Option<f64>,
    pub profiles: Vec<ProfileResult>,
}

impl Scoreboard {
    pub fn oneway(kind: PhyKind, seeds: u32, profiles: Vec<ProfileResult>, corpus: CorpusScore) -> Scoreboard {
        let mut nominal = Point::default();
        let mut all_false = 0;
        for pr in &profiles {
            for pt in &pr.points {
                all_false += pt.false_frames;
                // The holdout set does not affect the headline figures.
                if pt.snr_db >= NOMINAL_MIN_SNR && !pr.holdout {
                    nominal.add(pt);
                }
            }
        }
        let t = kind.modulator(48000.0).duration(LINK60.len());
        Scoreboard {
            phy: kind.name().into(),
            mode: "one-way".into(),
            commit: git_commit(),
            time_utc: utc_now(),
            seeds,
            transfer_time_s: t,
            transfer_note: "one frame, no acks".into(),
            success_rate: nominal.success(),
            detect_rate: nominal.detected as f64 / nominal.n.max(1) as f64,
            false_frames: all_false,
            ber: nominal.ber(),
            corrected_per_frame: nominal.corrected as f64 / nominal.ok.max(1) as f64,
            net_rate: LINK60.len() as f64 / t,
            corpus,
            frames_per_session: None,
            profiles,
        }
    }

    /// Session-mode scoreboard: transfer time and success with the whole protocol
    /// (two stations, acks), margin figures from the one-way run.
    pub fn session(kind: PhyKind, seeds: u32, link_desc: &str, profiles: Vec<ProfileResult>, phy_only: &Scoreboard, corpus: CorpusScore) -> Scoreboard {
        let mut nominal = Point::default();
        for pr in profiles.iter().filter(|p| !p.holdout) {
            for pt in pr.points.iter().filter(|pt| pt.snr_db >= NOMINAL_MIN_SNR) {
                nominal.add(pt);
            }
        }
        nominal.times.sort_unstable();
        let median = nominal.times.get(nominal.times.len() / 2).map_or(f64::NAN, |&ms| ms as f64 / 1000.0);
        Scoreboard {
            phy: kind.name().into(),
            mode: format!("session: {link_desc}"),
            commit: git_commit(),
            time_utc: utc_now(),
            seeds,
            transfer_time_s: median,
            transfer_note: "from pressing Send to the Delivered notice, median in nominal conditions".into(),
            success_rate: nominal.success(),
            detect_rate: phy_only.detect_rate,
            false_frames: phy_only.false_frames,
            ber: phy_only.ber,
            corrected_per_frame: phy_only.corrected_per_frame,
            net_rate: LINK60.len() as f64 / median,
            corpus,
            frames_per_session: Some(nominal.frames_sent as f64 / nominal.n.max(1) as f64),
            profiles,
        }
    }

    pub fn min_snr(&self, name: &str) -> Option<f64> {
        self.profiles.iter().find(|p| p.name == name).and_then(|p| p.min_snr())
    }

    /// Worst profile: the highest min SNR (or a profile that never reaches the limit).
    pub fn worst(&self) -> Option<(&str, Option<f64>)> {
        self.profiles
            .iter()
            .filter(|p| !p.holdout)
            .map(|p| (p.name.as_str(), p.min_snr()))
            .max_by(|a, b| a.1.unwrap_or(f64::INFINITY).total_cmp(&b.1.unwrap_or(f64::INFINITY)))
    }

    pub fn to_json(&self) -> Json {
        Json::obj(vec![
            ("schema", Json::num(1)),
            ("commit", Json::str(&self.commit)),
            ("time_utc", Json::str(&self.time_utc)),
            ("phy", Json::str(&self.phy)),
            ("mode", Json::str(&self.mode)),
            ("seeds", Json::num(self.seeds)),
            (
                "summary",
                Json::obj(vec![
                    ("transfer_time_s", Json::num(self.transfer_time_s)),
                    ("transfer_note", Json::str(&self.transfer_note)),
                    ("success_rate", Json::num(self.success_rate)),
                    ("detect_rate", Json::num(self.detect_rate)),
                    ("false_frames", Json::num(self.false_frames)),
                    ("min_snr_db_room", Json::opt(self.min_snr("room"))),
                    ("ber_pre_fec", Json::opt(self.ber)),
                    ("corrected_per_frame", Json::num(self.corrected_per_frame)),
                    ("net_bytes_per_s", Json::num(self.net_rate)),
                    ("frames_per_session", Json::opt(self.frames_per_session)),
                    ("corpus_n", Json::num(self.corpus.n)),
                    ("corpus_ok", Json::num(self.corpus.ok)),
                    ("corpus_rate", Json::opt((self.corpus.n > 0).then(|| self.corpus.ok as f64 / self.corpus.n as f64))),
                    ("holdout_n", Json::num(self.corpus.holdout_n)),
                    ("holdout_ok", Json::num(self.corpus.holdout_ok)),
                ]),
            ),
            (
                "profiles",
                Json::Arr(
                    self.profiles
                        .iter()
                        .map(|p| {
                            Json::obj(vec![
                                ("name", Json::str(&p.name)),
                                ("desc", Json::str(&p.desc)),
                                ("holdout", Json::Bool(p.holdout)),
                                ("min_snr_db", Json::opt(p.min_snr())),
                                (
                                    "points",
                                    Json::Arr(
                                        p.points
                                            .iter()
                                            .map(|pt| {
                                                Json::obj(vec![
                                                    ("snr_db", Json::num(pt.snr_db)),
                                                    ("n", Json::num(pt.n)),
                                                    ("ok", Json::num(pt.ok)),
                                                    ("detected", Json::num(pt.detected)),
                                                    ("ber", Json::opt(pt.ber())),
                                                    ("corrected", Json::num(pt.corrected as f64)),
                                                    ("median_ms", Json::opt({
                                                        let mut t = pt.times.clone();
                                                        t.sort_unstable();
                                                        t.get(t.len() / 2).map(|&v| v as f64)
                                                    })),
                                                ])
                                            })
                                            .collect(),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    pub fn print(&self) {
        let snr = |v: Option<f64>| v.map_or("not reached".to_string(), |x| format!("{x:+.0} dB"));
        println!("SCOREBOARD  {}  ({})  commit {}  {} runs/point", self.phy, self.mode, self.commit, self.seeds);
        println!("  Transfer time, 60-char link       {:>8.2} s   ({})", self.transfer_time_s, self.transfer_note);
        println!("  Success rate, nominal conditions  {:>8.1} %   (all profiles, SNR >= {NOMINAL_MIN_SNR:.0} dB)", self.success_rate * 100.0);
        println!("  Min SNR (>= 90 %), room           {:>10}", snr(self.min_snr("room")));
        if let Some((name, v)) = self.worst() {
            println!("  Min SNR, worst profile            {:>10}   ({name})", snr(v));
        }
        println!("  BER before error correction       {:>10}   (nominal conditions, detected frames)", self.ber.map_or("-".into(), |b| format!("{b:.1e}")));
        println!("  Corrected symbols / frame         {:>8.2}", self.corrected_per_frame);
        println!("  Net rate                          {:>8.1} bytes/s", self.net_rate);
        println!("  Preamble detected                 {:>8.1} %", self.detect_rate * 100.0);
        if let Some(f) = self.frames_per_session {
            println!("  Data frames + polls / session     {:>8.2}     (1.00 = no retransmissions)", f);
        }
        if self.corpus.n + self.corpus.stale > 0 {
            println!("  Recording corpus (real audio)     {:>5}/{:<3}    recordings decoded intact", self.corpus.ok, self.corpus.n);
        } else {
            println!("  Recording corpus (real audio)            -   (no recordings for this waveform)");
        }
        if self.corpus.holdout_n > 0 {
            println!("  Holdout set                       {:>5}/{:<3}", self.corpus.holdout_ok, self.corpus.holdout_n);
        }
        if self.false_frames > 0 {
            println!("  FALSE FRAMES PASSED               {:>8}   (the checksum failed us!)", self.false_frames);
        }
        println!();
        print!("  success %           SNR dB:");
        for pt in self.profiles.first().map(|p| p.points.as_slice()).unwrap_or(&[]) {
            print!("{:>4.0}", pt.snr_db);
        }
        println!("   min SNR");
        for p in &self.profiles {
            let label = format!("{}{}", short_name(&p.name), if p.holdout { " (hold.)" } else { "" });
            print!("  {label:<26}");
            for pt in &p.points {
                print!("{:>4.0}", pt.success() * 100.0);
            }
            println!("   {}", snr(p.min_snr()));
        }
    }
}

/// Short name of a measured channel for the table: `measured:20261111T080523Z-mac-g100:far`
/// -> `m:0805-mac-g100:far` (the time of day is enough to tell apart same-day measurements).
pub fn short_name(name: &str) -> String {
    match name.strip_prefix("measured:") {
        Some(rest) if rest.len() > 17 && rest.as_bytes()[8] == b'T' && rest.as_bytes()[15] == b'Z' => format!("m:{}{}", &rest[9..13], &rest[16..]),
        Some(rest) => format!("m:{rest}"),
        None => name.to_string(),
    }
}

pub fn git_commit() -> String {
    let run = |args: &[&str]| {
        std::process::Command::new("git").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let head = run(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "no-commit".into());
    let dirty = run(&["status", "--porcelain", "--untracked-files=no"]).map_or(false, |s| !s.is_empty());
    if dirty {
        format!("{head}+dirty")
    } else {
        head
    }
}

pub fn utc_now() -> String {
    std::process::Command::new("date")
        .args(["-u", "+%Y%m%dT%H%M%SZ"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_is_60_chars() {
        assert_eq!(LINK60.len(), 60);
    }

    #[test]
    fn clean_channel_decodes() {
        let cfg = (synthetic_profiles()[0].make)(30.0, 1);
        let pt = run_once(PhyKind::DEFAULT, &cfg, LINK60.as_bytes(), 1);
        assert_eq!((pt.ok, pt.false_frames), (1, 0));
    }

    #[test]
    fn min_snr_requires_all_higher_points() {
        let mk = |snr: f64, ok: u32| Point { snr_db: snr, n: 10, ok, ..Default::default() };
        let pr = ProfileResult { name: "x".into(), desc: String::new(), holdout: false, points: vec![mk(0.0, 10), mk(3.0, 5), mk(6.0, 9), mk(9.0, 10)] };
        assert_eq!(pr.min_snr(), Some(6.0));
    }
}
