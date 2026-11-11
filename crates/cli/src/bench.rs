//! Mittausajo simuloidun kanavan läpi ja pistetaulu.
//!
//! Sama pistetaulu tuotetaan jokaisesta ajosta, jotta muutoksia voi verrata:
//! - siirtoaika: 60 merkin linkin siirto (kuittauksineen, kun linkkikerros on käytössä)
//! - onnistuminen: osuus siirroista nimellisoloissa (kaikki profiilit, SNR 12–30 dB)
//! - alin SNR: pienin signaali-kohinasuhde, jolla ja jonka yläpuolella vähintään 90 % onnistuu
//! - BER ennen virheenkorjausta ja korjattujen symbolien määrä: marginaali
//! - nettonopeus: hyötytavua sekunnissa
//!
//! Siemenluvut ovat kiinteät, joten sama koodi tuottaa aina saman tuloksen.

use crate::json::Json;
use modem_core::phy::{bit_errors, PhyEvent, PhyKind};
use modem_core::rng::Rng;
use modem_core::sim::{run_oneway, synth_ir, NoiseKind, OneWay};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Mittausviesti: 60 merkin linkki.
pub const LINK60: &str = "https://sairanen.org/soundpaste/?v=Zk3mQ9xT7bLw2RfN8cYh5VpJd";

pub const SNR_GRID: &[f64] = &[-15.0, -12.0, -9.0, -6.0, -3.0, 0.0, 3.0, 6.0, 9.0, 12.0, 15.0, 18.0, 21.0, 24.0, 27.0, 30.0];
const NOMINAL_MIN_SNR: f64 = 12.0;
const SUCCESS_LIMIT: f64 = 0.9;

pub struct Profile {
    pub name: String,
    pub desc: String,
    /// Onko profiili pidetty sivussa virittämisestä (vertailuaineisto)?
    pub holdout: bool,
    pub make: Box<dyn Fn(f64, u64) -> OneWay + Send + Sync>,
}

fn room(fs_rx: f64, rt60: f64, drr: f64, seed: u64) -> Vec<f32> {
    synth_ir(fs_rx, rt60, drr, seed)
}

/// Synteettiset kanavaprofiilit. Jokaisella siemenellä on oma huoneensa
/// (impulssivasteen satunnaisosa), jotta modeemia ei viritetä yhteen kaikuun.
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
        p("puhdas", "ei kaikua, valkoinen kohina", move |snr, seed| base(snr, seed)),
        p("huone", "RT60 0,35 s, DRR 8 dB, valkoinen kohina", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 0.35, 8.0, seed);
            c
        }),
        p("kaikuisa", "RT60 0,8 s, DRR 0 dB", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 0.8, 0.0, seed);
            c
        }),
        p("huonemelu", "huone + vaaleanpunainen (matalapainotteinen) kohina", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 0.35, 8.0, seed);
            c.noise = NoiseKind::Pink;
            c
        }),
        p("kello-44-48", "lähetin 44,1 kHz, vastaanotin 48 kHz +120 ppm, huone", move |snr, seed| {
            let mut c = base(snr, seed);
            c.fs_tx = 44100.0;
            c.clock_ppm = 120.0;
            c.ir = room(48000.0, 0.35, 8.0, seed);
            c
        }),
        p("kello-48-44", "lähetin 48 kHz, vastaanotin 44,1 kHz -120 ppm, huone", move |snr, seed| {
            let mut c = base(snr, seed);
            c.fs_rx = 44100.0;
            c.clock_ppm = -120.0;
            c.ir = room(44100.0, 0.35, 8.0, seed);
            c
        }),
        p("leikkaus", "mikrofoni leikkaa rajusti (signaalin RMS 0,9), huone", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 0.35, 8.0, seed);
            c.rx_level = 0.9;
            c
        }),
        p("vaimea", "hyvin hiljainen signaali (RMS 0,0005), huone", move |snr, seed| {
            let mut c = base(snr, seed);
            c.ir = room(48000.0, 0.35, 8.0, seed);
            c.rx_level = 0.0005;
            c
        }),
    ]
}

#[derive(Default, Clone, Debug)]
pub struct Point {
    pub snr_db: f64,
    pub n: u32,
    pub ok: u32,
    pub detected: u32,
    /// Ehjänä läpi mennyt kehys, jonka sisältö oli väärä (ei saisi tapahtua).
    pub false_frames: u32,
    pub bit_errors: u64,
    pub bits: u64,
    pub corrected: u64,
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
    }
}

pub struct ProfileResult {
    pub name: String,
    pub desc: String,
    pub holdout: bool,
    pub points: Vec<Point>,
}

impl ProfileResult {
    /// Pienin SNR, jolla ja jonka yläpuolella onnistumisosuus on vähintään raja.
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

/// Yksi yksisuuntainen siirto: moduloi, vie kanavan läpi, pura.
pub fn run_once(kind: PhyKind, cfg: &OneWay, payload: &[u8], seed: u64) -> Point {
    let modulator = kind.modulator(cfg.fs_tx);
    let tx = modulator.modulate(payload);
    let coded = modulator.coded(payload);
    let rx = run_oneway(&tx, cfg, seed);
    let mut demod = kind.demodulator(cfg.fs_rx);
    let mut events = Vec::new();
    // Syötetään vaihtelevan kokoisina lohkoina, kuten oikea äänilaite tekisi.
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
    let jobs: Vec<(usize, usize, u32)> = (0..profiles.len())
        .flat_map(|p| (0..SNR_GRID.len()).flat_map(move |s| (0..opts.seeds).map(move |i| (p, s, i))))
        .collect();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Vec<Point>>> = Mutex::new(
        profiles.iter().map(|_| SNR_GRID.iter().map(|&snr_db| Point { snr_db, ..Default::default() }).collect()).collect(),
    );
    std::thread::scope(|scope| {
        for _ in 0..opts.threads.max(1) {
            scope.spawn(|| loop {
                let j = next.fetch_add(1, Ordering::Relaxed);
                let Some(&(p, s, i)) = jobs.get(j) else { break };
                // Siemen riippuu vain profiilin nimestä, SNR-pisteestä ja toistosta,
                // joten sama kanava toistuu eri modeemiversioilla.
                let name_hash = profiles[p].name.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
                let seed = name_hash ^ ((s as u64) << 32) ^ (i as u64 + 1).wrapping_mul(0x9E37_79B9);
                let cfg = (profiles[p].make)(SNR_GRID[s], seed);
                let pt = run_once(opts.kind, &cfg, payload, seed);
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
    pub profiles: Vec<ProfileResult>,
}

impl Scoreboard {
    pub fn oneway(kind: PhyKind, seeds: u32, profiles: Vec<ProfileResult>) -> Scoreboard {
        let mut nominal = Point::default();
        let mut all_false = 0;
        for pr in &profiles {
            for pt in &pr.points {
                all_false += pt.false_frames;
                if pt.snr_db >= NOMINAL_MIN_SNR {
                    nominal.add(pt);
                }
            }
        }
        let t = kind.modulator(48000.0).duration(LINK60.len());
        Scoreboard {
            phy: kind.name().into(),
            mode: "yksisuuntainen".into(),
            commit: git_commit(),
            time_utc: utc_now(),
            seeds,
            transfer_time_s: t,
            transfer_note: "yksi kehys, ei kuittauksia".into(),
            success_rate: nominal.success(),
            detect_rate: nominal.detected as f64 / nominal.n.max(1) as f64,
            false_frames: all_false,
            ber: nominal.ber(),
            corrected_per_frame: nominal.corrected as f64 / nominal.ok.max(1) as f64,
            net_rate: LINK60.len() as f64 / t,
            profiles,
        }
    }

    pub fn min_snr(&self, name: &str) -> Option<f64> {
        self.profiles.iter().find(|p| p.name == name).and_then(|p| p.min_snr())
    }

    /// Heikoin profiili: suurin alin SNR (tai profiili, joka ei yllä rajaan lainkaan).
    pub fn worst(&self) -> Option<(&str, Option<f64>)> {
        self.profiles
            .iter()
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
                    ("min_snr_db_room", Json::opt(self.min_snr("huone"))),
                    ("ber_pre_fec", Json::opt(self.ber)),
                    ("corrected_per_frame", Json::num(self.corrected_per_frame)),
                    ("net_bytes_per_s", Json::num(self.net_rate)),
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
        let snr = |v: Option<f64>| v.map_or("ei yllä".to_string(), |x| format!("{x:+.0} dB"));
        println!("PISTETAULU  {}  ({})  commit {}  {} toistoa/piste", self.phy, self.mode, self.commit, self.seeds);
        println!("  Siirtoaika, 60 merkin linkki      {:>8.2} s   ({})", self.transfer_time_s, self.transfer_note);
        println!("  Onnistuminen, nimellisolot        {:>8.1} %   (kaikki profiilit, SNR >= {NOMINAL_MIN_SNR:.0} dB)", self.success_rate * 100.0);
        println!("  Alin SNR (>= 90 %), huone         {:>10}", snr(self.min_snr("huone")));
        if let Some((name, v)) = self.worst() {
            println!("  Alin SNR, heikoin profiili        {:>10}   ({name})", snr(v));
        }
        println!("  BER ennen virheenkorjausta        {:>10}   (nimellisolot, havaitut kehykset)", self.ber.map_or("-".into(), |b| format!("{b:.1e}")));
        println!("  Korjatut symbolit / kehys         {:>8.2}", self.corrected_per_frame);
        println!("  Nettonopeus                       {:>8.1} tavua/s", self.net_rate);
        println!("  Alkumerkki havaittu               {:>8.1} %", self.detect_rate * 100.0);
        if self.false_frames > 0 {
            println!("  VÄÄRIÄ KEHYKSIÄ LÄPI              {:>8}   (tarkistussumma petti!)", self.false_frames);
        }
        println!();
        print!("  onnistuminen %  SNR dB:");
        for s in SNR_GRID {
            print!("{s:>4.0}");
        }
        println!("   alin SNR");
        for p in &self.profiles {
            print!("  {:<22}", format!("{}{}", p.name, if p.holdout { " (vert.)" } else { "" }));
            for pt in &p.points {
                print!("{:>4.0}", pt.success() * 100.0);
            }
            println!("   {}", snr(p.min_snr()));
        }
    }
}

pub fn git_commit() -> String {
    let run = |args: &[&str]| {
        std::process::Command::new("git").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let head = run(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "ei-commitia".into());
    let dirty = run(&["status", "--porcelain", "--untracked-files=no"]).map_or(false, |s| !s.is_empty());
    if dirty {
        format!("{head}+muutoksia")
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
        .unwrap_or_else(|| "tuntematon".into())
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
