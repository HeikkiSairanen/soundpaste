//! Real-audio commands: microphone test, channel measurement, acoustic loop and corpus.

use crate::args::Args;
use crate::audio;
use crate::bench;
use crate::corpus;
use crate::json::Json;
use modem_core::measure::{self, ChannelMeasurement, Sweep};
use modem_core::wav;
use std::process::ExitCode;

fn dbfs(rms: f32) -> f64 {
    20.0 * (rms.max(1e-9) as f64).log10()
}

/// Simulated "recording" for silent testing: the same path as with real
/// audio, but with a simulator profile as the channel.
fn sim_record(tx: &[f32], profile: &str, pre_s: f64, level: f32) -> Result<Vec<f32>, String> {
    let profiles = bench::synthetic_profiles();
    let p = profiles.iter().find(|p| p.name == profile).ok_or_else(|| format!("unknown profile '{profile}'"))?;
    let mut cfg = (p.make)(35.0, 1);
    cfg.lead_s = pre_s;
    cfg.tail_s = 1.0;
    cfg.rx_level = level;
    Ok(modem_core::sim::run_oneway(tx, &cfg, 1))
}

/// Record, optionally adjusting the system volume for the duration of playback.
fn capture(a: &Args, tx: &[f32], fs: u32, level: f32, pre_s: f64) -> Result<(Vec<f32>, Option<u32>), String> {
    if let Some(profile) = a.str("sim") {
        return Ok((sim_record(tx, profile, pre_s, level)?, None));
    }
    let before = audio::system_volume();
    let wanted: Option<u32> = a.str("volume").map(|v| v.parse().map_err(|_| "--volume: give a number 0–100".to_string())).transpose()?;
    if let Some(v) = wanted {
        audio::set_system_volume(v);
    }
    let result = audio::play_and_record(tx, fs, pre_s, 0.6);
    if let (Some(_), Some(b)) = (wanted, before) {
        audio::set_system_volume(b);
    }
    Ok((result?, wanted.or(before)))
}

fn common_meta(a: &Args, id: &str, kind: &str, fs: u32, volume: Option<u32>, gain: f64) -> Vec<(&'static str, Json)> {
    let holdout = if a.has("holdout") {
        true
    } else if a.has("no-holdout") {
        false
    } else {
        corpus::auto_holdout(id)
    };
    vec![
        ("id", Json::str(id)),
        ("kind", Json::str(kind)),
        ("time_utc", Json::str(&bench::utc_now())),
        ("device", Json::str(&if a.has("sim") { format!("simulation:{}", a.str_or("sim", "")) } else { a.str("device").map(String::from).unwrap_or_else(audio::device_description) })),
        ("sample_rate", Json::num(fs)),
        ("system_volume", Json::opt(volume.map(|v| v as f64))),
        ("gain", Json::num(gain)),
        ("distance_m", Json::opt(a.str("distance").and_then(|d| d.parse().ok()))),
        ("note", Json::str(a.str_or("note", ""))),
        ("holdout", Json::Bool(holdout)),
    ]
}

pub fn cmd_audio_test(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &[])?;
    let fs: u32 = a.num("rate", 48000)?;
    let secs: f64 = a.num("seconds", 2.0)?;
    println!("device: {}", audio::device_description());
    println!("system volume: {}", audio::system_volume().map_or("?".into(), |v| v.to_string()));
    println!("recording {secs} s (playing nothing)...");
    let rec = audio::record(fs, secs)?;
    let skip = (0.2 * fs as f64) as usize;
    let body = &rec[skip.min(rec.len())..];
    let peak = body.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    println!("microphone works: {} samples, level {:.1} dBFS, peak {:.1} dBFS", rec.len(), dbfs(modem_core::dsp::rms(body)), dbfs(peak));
    if let Some(out) = a.str("out") {
        std::fs::write(out, wav::encode_pcm16(fs, &rec)).map_err(|e| e.to_string())?;
    }
    Ok(ExitCode::SUCCESS)
}

pub fn print_measurement(m: &ChannelMeasurement) {
    println!(
        "  level: sweep {:.1} dBFS, background noise {:.1} dBFS (difference {:.1} dB), clipped {:.2} %",
        dbfs(m.signal_rms),
        dbfs(m.noise_rms),
        dbfs(m.signal_rms) - dbfs(m.noise_rms),
        m.clipped * 100.0
    );
    println!("  sweep start in the recording: {:.0} ms", m.delay_samples as f64 / m.fs * 1e3);
    println!("  impulse response: {:.0} ms down to the noise floor, dynamic range {:.0} dB", m.ir.len() as f64 / m.fs * 1e3, m.ir_dynamic_db);
    println!(
        "  RT60 {}, direct/reverb {:.1} dB, delay spread {:.1} ms (coherence bandwidth about {:.0} Hz)",
        m.rt60_s.map_or("not measurable".into(), |t| format!("{t:.2} s")),
        m.drr_db,
        m.delay_spread_s * 1e3,
        1.0 / (5.0 * m.delay_spread_s.max(1e-6))
    );
    if m.thd2_db.is_finite() {
        println!("  second-harmonic distortion: {:.0} dB", m.thd2_db);
    } else {
        println!("  second-harmonic distortion: below the noise floor");
    }
    // When the reverb has decayed 20/40/60 dB from the peak (power in a 10 ms window).
    let win = (0.010 * m.fs) as usize;
    let power = |i: usize| m.ir[i..(i + win).min(m.ir.len())].iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
    let p0 = power(0).max(1e-30);
    let mut line = String::from("  reverb decay from the peak:");
    for target in [20.0, 40.0, 60.0] {
        let t = (0..m.ir.len().saturating_sub(win)).step_by(win / 2).find(|&i| 10.0 * (power(i) / p0).log10() <= -target);
        line += &match t {
            Some(i) => format!("  -{target:.0} dB @ {:.0} ms", i as f64 / m.fs * 1e3),
            None => format!("  -{target:.0} dB not reached"),
        };
    }
    println!("{line}");
    // How much of the energy arrives later than t after the direct sound.
    // This is the level of inter-symbol interference if the next symbol starts at time t.
    let total: f64 = m.ir.iter().map(|v| (*v as f64).powi(2)).sum();
    let mut line = String::from("  energy after the delay:");
    for t_ms in [2.5, 5.0, 10.0, 20.0, 40.0] {
        let from = (m.ir_pre + (t_ms * 1e-3 * m.fs) as usize).min(m.ir.len());
        let late: f64 = m.ir[from..].iter().map(|v| (*v as f64).powi(2)).sum();
        line += &format!("  >{t_ms} ms: {:.0} dB", 10.0 * (late / total).max(1e-30).log10());
    }
    println!("{line}");
    println!("  {:>9} {:>10} {:>12} {:>12}", "band Hz", "gain", "noise/50 Hz", "tone SNR");
    for b in &m.bands {
        println!("  {:>9.0} {:>8.1} dB {:>9.1} dBFS {:>9.1} dB", b.center_hz, b.gain_db, b.noise_db, b.tone_snr_db);
    }
}

fn measurement_json(m: &ChannelMeasurement) -> Json {
    Json::obj(vec![
        ("signal_dbfs", Json::num(dbfs(m.signal_rms))),
        ("noise_dbfs", Json::num(dbfs(m.noise_rms))),
        ("clipped", Json::num(m.clipped)),
        ("delay_ms", Json::num(m.delay_samples as f64 / m.fs * 1e3)),
        ("ir_ms", Json::num(m.ir.len() as f64 / m.fs * 1e3)),
        ("ir_pre", Json::num(m.ir_pre as f64)),
        ("ir_dynamic_db", Json::num(m.ir_dynamic_db)),
        ("rt60_s", Json::opt(m.rt60_s)),
        ("drr_db", Json::num(m.drr_db)),
        ("delay_spread_ms", Json::num(m.delay_spread_s * 1e3)),
        ("thd2_db", Json::opt(Some(m.thd2_db))),
        (
            "bands",
            Json::Arr(
                m.bands
                    .iter()
                    .map(|b| Json::obj(vec![("hz", Json::num(b.center_hz.round())), ("gain_db", Json::num(b.gain_db)), ("noise_db", Json::num(b.noise_db)), ("tone_snr_db", Json::num(b.tone_snr_db))]))
                    .collect(),
            ),
        ),
    ])
}

fn save_channel(a: &Args, id: &str, fs: u32, rec: &[f32], sweep: &Sweep, m: &ChannelMeasurement, volume: Option<u32>) -> Result<(), String> {
    let mut meta = common_meta(a, id, "channel", fs, volume, sweep.amplitude as f64);
    meta.push(("label", Json::str(a.str_or("label", "mac"))));
    store_channel(id, fs, rec, sweep, m, meta)
}

/// Store a channel measurement in the corpus: recording, metadata, impulse response and background noise.
pub fn store_channel(id: &str, fs: u32, rec: &[f32], sweep: &Sweep, m: &ChannelMeasurement, mut meta: Vec<(&'static str, Json)>) -> Result<(), String> {
    std::fs::create_dir_all(corpus::CHANNEL_DIR).map_err(|e| e.to_string())?;
    let base = std::path::Path::new(corpus::CHANNEL_DIR).join(id);
    let w = |ext: &str, bytes: Vec<u8>| std::fs::write(base.with_extension(ext), bytes).map_err(|e| e.to_string());
    w("wav", wav::encode_pcm16(fs, rec))?;
    w("ir.wav", wav::encode_f32(fs, &m.ir))?;
    w("noise.wav", wav::encode_f32(fs, &m.noise))?;
    meta.push(("sweep", Json::obj(vec![("f1", Json::num(sweep.f1)), ("f2", Json::num(sweep.f2)), ("duration_s", Json::num(sweep.duration_s)), ("amplitude", Json::num(sweep.amplitude))])));
    meta.push(("results", measurement_json(m)));
    w("json", Json::obj(meta).pretty().into_bytes())?;
    println!("saved: {}.{{wav,json,ir.wav,noise.wav}}", base.display());
    Ok(())
}

/// Channel measurement: play a sweep, record, compute the impulse response and background noise.
///
/// Two-machine measurement via the development server (test station open in a browser):
/// - `--from-remote <station>`: the test station plays the sweep and this machine records
/// - `--to-remote <station>`: this machine plays and the test station records and sends
///   the recording to the server, which analyzes and stores it
pub fn cmd_measure(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &["holdout", "no-holdout", "no-save"])?;
    let fs: u32 = a.num("rate", 48000)?;
    let port: u16 = a.num("port", 8787)?;
    let sweep = Sweep { amplitude: a.num("gain", 0.3)?, ..Sweep::default() };
    let tx = measure::test_signal(&sweep, fs as f64);

    if let Some(station) = a.str("to-remote") {
        // The test station records 10 s from this moment; the sweep is played within it.
        let extra = format!("{}{}", if a.has("holdout") { "holdout " } else { "" }, a.str_or("note", ""));
        print!("{}", crate::serve::http(port, "POST", "/dev/command", format!("record 10 channel {} {extra}", sweep.amplitude).as_bytes())?);
        std::thread::sleep(std::time::Duration::from_millis(1500));
        println!("CHANNEL MEASUREMENT Mac -> {station}: playing the sweep, the station records and the server analyzes (see the server output)");
        audio::play_and_record(&tx, fs, 0.2, 0.2)?;
        return Ok(ExitCode::SUCCESS);
    }

    let label = match a.str("from-remote") {
        Some(station) => format!("{station}-mac"),
        None => a.str_or("label", "mac").to_string(),
    };
    let id = format!("{}-{}-g{:03.0}", bench::utc_now(), label, sweep.amplitude * 100.0);
    println!("CHANNEL MEASUREMENT {id}: sweep {:.0}–{:.0} Hz, {:.1} s, amplitude {}", sweep.f1, sweep.f2, sweep.duration_s, sweep.amplitude);
    let (rec, volume) = if let Some(station) = a.str("from-remote") {
        // The test station plays (it polls for commands once a second), this machine records.
        println!("  test station {station} plays, this machine records 9 s");
        print!("{}", crate::serve::http(port, "POST", "/dev/command", format!("sweep {}", sweep.amplitude).as_bytes())?);
        (audio::record(fs, 9.0)?, None)
    } else {
        // A long silent lead-in: it yields several seconds of room background noise.
        capture(&a, &tx, fs, 0.05, 3.0)?
    };
    let m = measure::analyze(&rec, fs as f64, &sweep)?;
    print_measurement(&m);
    if !a.has("no-save") {
        let mut meta = common_meta(&a, &id, "channel", fs, volume, sweep.amplitude as f64);
        if let Some(station) = a.str("from-remote") {
            meta.retain(|(k, _)| *k != "device");
            meta.push(("device", Json::str(&format!("speaker: {station} (browser); microphone: {}", audio::device_description()))));
        }
        meta.push(("label", Json::str(&label)));
        store_channel(&id, fs, &rec, &sweep, &m, meta)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Analyze an existing sweep recording (e.g. one recorded on another device).
pub fn cmd_analyze(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &["holdout", "no-holdout", "save"])?;
    let path = a.positional.first().ok_or("give an audio file")?;
    let w = corpus::read_wav(std::path::Path::new(path))?;
    let sweep = Sweep { amplitude: a.num("gain", 0.3)?, ..Sweep::default() };
    let m = measure::analyze(&w.samples, w.sample_rate as f64, &sweep)?;
    println!("CHANNEL ANALYSIS {path}");
    print_measurement(&m);
    if a.has("save") {
        let id = format!("{}-{}-g{:03.0}", bench::utc_now(), a.str_or("label", "imported"), sweep.amplitude * 100.0);
        save_channel(&a, &id, w.sample_rate, &w.samples, &sweep, &m, None)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Acoustic loop: play a modem transmission from the speaker at several gains,
/// record, decode and add the recordings to the corpus.
pub fn cmd_acoustic(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &["holdout", "no-holdout", "no-save"])?;
    let fs: u32 = a.num("rate", 48000)?;
    let kind = crate::phy_arg(&a)?;
    let text = a.positional.first().map(|s| s.as_str()).unwrap_or(bench::LINK60);
    let gains: Vec<f32> = a.str_or("gains", "1.0,0.3,0.1").split(',').map(|g| g.trim().parse::<f32>().map_err(|_| format!("--gains: invalid value '{g}'"))).collect::<Result<_, _>>()?;
    let label = a.str_or("label", "mac");
    let modulator = kind.modulator(fs as f64);
    let clean = modulator.modulate(text.as_bytes());
    println!("ACOUSTIC LOOP {}: {} bytes, {:.2} s per transmission", kind.name(), text.len(), clean.len() as f64 / fs as f64);
    let mut ok_count = 0;
    for &g in &gains {
        let tx: Vec<f32> = clean.iter().map(|v| v * g).collect();
        let (rec, volume) = capture(&a, &tx, fs, 0.1 * g, 0.7)?;
        let w = wav::Wav { sample_rate: fs, samples: rec };
        let r = corpus::decode_expect(kind, &w, text.as_bytes());
        ok_count += r.ok as u32;
        let rms = modem_core::dsp::rms(&w.samples);
        println!(
            "  gain {g:<5} {}  SNR {}  corrected {}  recording level {:.1} dBFS",
            if r.ok { "ok   " } else if r.detected { "ERROR" } else { "NOT DETECTED" },
            r.snr_db.map_or("-".into(), |s| format!("{s:.1} dB")),
            r.corrected,
            dbfs(rms)
        );
        if !a.has("no-save") {
            std::fs::create_dir_all(corpus::TX_DIR).map_err(|e| e.to_string())?;
            let id = format!("{}-{}-{}-g{:03.0}", bench::utc_now(), label, kind.name(), g * 100.0);
            let base = std::path::Path::new(corpus::TX_DIR).join(&id);
            std::fs::write(base.with_extension("wav"), wav::encode_pcm16(fs, &w.samples)).map_err(|e| e.to_string())?;
            let mut meta = common_meta(&a, &id, "tx", fs, volume, g as f64);
            meta.push(("phy", Json::str(kind.name())));
            meta.push(("text", Json::str(text)));
            meta.push(("ok_when_recorded", Json::Bool(r.ok)));
            std::fs::write(base.with_extension("json"), Json::obj(meta).pretty()).map_err(|e| e.to_string())?;
        }
    }
    println!("  {ok_count}/{} succeeded", gains.len());
    Ok(if ok_count as usize == gains.len() { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

/// Corpus regression run and listing.
pub fn cmd_corpus(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &["holdout"])?;
    let only = match a.str("phy") {
        Some(_) => Some(crate::phy_arg(&a)?),
        None => None,
    };
    let channels = corpus::entries(corpus::CHANNEL_DIR);
    println!("CHANNEL MEASUREMENTS ({})", channels.len());
    for (_, meta) in &channels {
        let r = meta.get("results");
        let num = |k: &str| r.and_then(|r| r.num_at(k));
        println!(
            "  {:<40}{} RT60 {}  direct/reverb {}  noise {}  {}",
            meta.str_at("id").unwrap_or("?"),
            if corpus::is_holdout(meta) { " (hold.)" } else { "        " },
            num("rt60_s").map_or("-".into(), |v| format!("{v:.2} s")),
            num("drr_db").map_or("-".into(), |v| format!("{v:.0} dB")),
            num("noise_dbfs").map_or("-".into(), |v| format!("{v:.0} dBFS")),
            meta.str_at("note").unwrap_or(""),
        );
    }
    println!("TRANSMISSION RECORDINGS");
    let s = corpus::run_regression(only, a.has("holdout"), true);
    println!("  tuning set: {}/{} ok", s.ok, s.n);
    if a.has("holdout") {
        println!("  holdout set: {}/{} ok", s.holdout_ok, s.holdout_n);
    }
    if s.stale > 0 {
        println!("  stale: {}", s.stale);
    }
    Ok(if s.ok == s.n { ExitCode::SUCCESS } else { ExitCode::from(1) })
}
