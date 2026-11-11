//! Oikean äänen komennot: mikrofonitesti, kanavamittaus, akustinen kehä ja korpus.

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

/// Simuloitu "äänitys" hiljaista testausta varten: sama polku kuin oikealla
/// äänellä, mutta kanavana simulaattorin profiili.
fn sim_record(tx: &[f32], profile: &str, pre_s: f64, level: f32) -> Result<Vec<f32>, String> {
    let profiles = bench::synthetic_profiles();
    let p = profiles.iter().find(|p| p.name == profile).ok_or_else(|| format!("tuntematon profiili '{profile}'"))?;
    let mut cfg = (p.make)(35.0, 1);
    cfg.lead_s = pre_s;
    cfg.tail_s = 1.0;
    cfg.rx_level = level;
    Ok(modem_core::sim::run_oneway(tx, &cfg, 1))
}

/// Äänitä, valinnaisesti säätäen järjestelmän äänenvoimakkuutta toiston ajaksi.
fn capture(a: &Args, tx: &[f32], fs: u32, level: f32, pre_s: f64) -> Result<(Vec<f32>, Option<u32>), String> {
    if let Some(profile) = a.str("sim") {
        return Ok((sim_record(tx, profile, pre_s, level)?, None));
    }
    let before = audio::system_volume();
    let wanted: Option<u32> = a.str("volume").map(|v| v.parse().map_err(|_| "--volume: anna luku 0–100".to_string())).transpose()?;
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
        ("device", Json::str(&if a.has("sim") { format!("simulaatio:{}", a.str_or("sim", "")) } else { a.str("device").map(String::from).unwrap_or_else(audio::device_description) })),
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
    println!("laite: {}", audio::device_description());
    println!("järjestelmän äänenvoimakkuus: {}", audio::system_volume().map_or("?".into(), |v| v.to_string()));
    println!("äänitetään {secs} s (ei soiteta mitään)...");
    let rec = audio::record(fs, secs)?;
    let skip = (0.2 * fs as f64) as usize;
    let body = &rec[skip.min(rec.len())..];
    let peak = body.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    println!("mikrofoni toimii: {} näytettä, taso {:.1} dBFS, huippu {:.1} dBFS", rec.len(), dbfs(modem_core::dsp::rms(body)), dbfs(peak));
    if let Some(out) = a.str("out") {
        std::fs::write(out, wav::encode_pcm16(fs, &rec)).map_err(|e| e.to_string())?;
    }
    Ok(ExitCode::SUCCESS)
}

pub fn print_measurement(m: &ChannelMeasurement) {
    println!(
        "  taso: pyyhkäisy {:.1} dBFS, taustamelu {:.1} dBFS (ero {:.1} dB), leikkautunut {:.2} %",
        dbfs(m.signal_rms),
        dbfs(m.noise_rms),
        dbfs(m.signal_rms) - dbfs(m.noise_rms),
        m.clipped * 100.0
    );
    println!("  pyyhkäisyn alku äänitteessä: {:.0} ms", m.delay_samples as f64 / m.fs * 1e3);
    println!("  impulssivaste: {:.0} ms kohinapohjaan asti, dynamiikka {:.0} dB", m.ir.len() as f64 / m.fs * 1e3, m.ir_dynamic_db);
    println!(
        "  RT60 {}, suora/kaiku {:.1} dB, viivehaje {:.1} ms (koherenssikaista noin {:.0} Hz)",
        m.rt60_s.map_or("ei mitattavissa".into(), |t| format!("{t:.2} s")),
        m.drr_db,
        m.delay_spread_s * 1e3,
        1.0 / (5.0 * m.delay_spread_s.max(1e-6))
    );
    if m.thd2_db.is_finite() {
        println!("  toinen harmoninen särö: {:.0} dB", m.thd2_db);
    } else {
        println!("  toinen harmoninen särö: kohinapohjan alla");
    }
    // Milloin oma kaiku on vaimennut 20/40/60 dB huipusta (10 ms ikkunan teho).
    let win = (0.010 * m.fs) as usize;
    let power = |i: usize| m.ir[i..(i + win).min(m.ir.len())].iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
    let p0 = power(0).max(1e-30);
    let mut line = String::from("  kaiun vaimeneminen huipusta:");
    for target in [20.0, 40.0, 60.0] {
        let t = (0..m.ir.len().saturating_sub(win)).step_by(win / 2).find(|&i| 10.0 * (power(i) / p0).log10() <= -target);
        line += &match t {
            Some(i) => format!("  -{target:.0} dB @ {:.0} ms", i as f64 / m.fs * 1e3),
            None => format!("  -{target:.0} dB ei saavutettu"),
        };
    }
    println!("{line}");
    // Kuinka suuri osa energiasta saapuu myöhemmin kuin t suoran äänen jälkeen.
    // Tämä on symbolien välisen häiriön taso, jos seuraava symboli alkaa hetkellä t.
    let total: f64 = m.ir.iter().map(|v| (*v as f64).powi(2)).sum();
    let mut line = String::from("  energia viiveen jälkeen:");
    for t_ms in [2.5, 5.0, 10.0, 20.0, 40.0] {
        let from = (m.ir_pre + (t_ms * 1e-3 * m.fs) as usize).min(m.ir.len());
        let late: f64 = m.ir[from..].iter().map(|v| (*v as f64).powi(2)).sum();
        line += &format!("  >{t_ms} ms: {:.0} dB", 10.0 * (late / total).max(1e-30).log10());
    }
    println!("{line}");
    println!("  {:>9} {:>10} {:>12} {:>12}", "kaista Hz", "vahvistus", "melu/50 Hz", "sävelen SNR");
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
    std::fs::create_dir_all(corpus::CHANNEL_DIR).map_err(|e| e.to_string())?;
    let base = std::path::Path::new(corpus::CHANNEL_DIR).join(id);
    let w = |ext: &str, bytes: Vec<u8>| std::fs::write(base.with_extension(ext), bytes).map_err(|e| e.to_string());
    w("wav", wav::encode_pcm16(fs, rec))?;
    w("ir.wav", wav::encode_f32(fs, &m.ir))?;
    w("kohina.wav", wav::encode_f32(fs, &m.noise))?;
    let mut meta = common_meta(a, id, "kanava", fs, volume, sweep.amplitude as f64);
    meta.push(("label", Json::str(a.str_or("label", "mac"))));
    meta.push(("sweep", Json::obj(vec![("f1", Json::num(sweep.f1)), ("f2", Json::num(sweep.f2)), ("duration_s", Json::num(sweep.duration_s)), ("amplitude", Json::num(sweep.amplitude))])));
    meta.push(("results", measurement_json(m)));
    w("json", Json::obj(meta).pretty().into_bytes())?;
    println!("tallennettu: {}.{{wav,json,ir.wav,kohina.wav}}", base.display());
    Ok(())
}

/// Kanavamittaus: soita pyyhkäisy, äänitä, laske impulssivaste ja taustamelu.
pub fn cmd_measure(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &["holdout", "no-holdout", "no-save"])?;
    let fs: u32 = a.num("rate", 48000)?;
    let sweep = Sweep { amplitude: a.num("gain", 0.3)?, ..Sweep::default() };
    let tx = measure::test_signal(&sweep, fs as f64);
    let label = a.str_or("label", "mac");
    let id = format!("{}-{}-g{:03.0}", bench::utc_now(), label, sweep.amplitude * 100.0);
    println!("KANAVAMITTAUS {id}: pyyhkäisy {:.0}–{:.0} Hz, {:.1} s, amplitudi {}", sweep.f1, sweep.f2, sweep.duration_s, sweep.amplitude);
    // Pitkä hiljainen alku: siitä saadaan useampi sekunti huoneen taustamelua.
    let (rec, volume) = capture(&a, &tx, fs, 0.05, 3.0)?;
    let m = measure::analyze(&rec, fs as f64, &sweep)?;
    print_measurement(&m);
    if !a.has("no-save") {
        save_channel(&a, &id, fs, &rec, &sweep, &m, volume)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Analysoi valmis pyyhkäisyäänite (esim. toisella laitteella äänitetty).
pub fn cmd_analyze(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &["holdout", "no-holdout", "save"])?;
    let path = a.positional.first().ok_or("anna äänitiedosto")?;
    let w = corpus::read_wav(std::path::Path::new(path))?;
    let sweep = Sweep { amplitude: a.num("gain", 0.3)?, ..Sweep::default() };
    let m = measure::analyze(&w.samples, w.sample_rate as f64, &sweep)?;
    println!("KANAVA-ANALYYSI {path}");
    print_measurement(&m);
    if a.has("save") {
        let id = format!("{}-{}-g{:03.0}", bench::utc_now(), a.str_or("label", "tuotu"), sweep.amplitude * 100.0);
        save_channel(&a, &id, w.sample_rate, &w.samples, &sweep, &m, None)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Akustinen kehä: soita modeemilähetys kaiuttimesta usealla voimakkuudella,
/// äänitä, pura ja lisää äänitteet korpukseen.
pub fn cmd_acoustic(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &["holdout", "no-holdout", "no-save"])?;
    let fs: u32 = a.num("rate", 48000)?;
    let kind = crate::phy_arg(&a)?;
    let text = a.positional.first().map(|s| s.as_str()).unwrap_or(bench::LINK60);
    let gains: Vec<f32> = a.str_or("gains", "1.0,0.3,0.1").split(',').map(|g| g.trim().parse::<f32>().map_err(|_| format!("--gains: virheellinen arvo '{g}'"))).collect::<Result<_, _>>()?;
    let label = a.str_or("label", "mac");
    let modulator = kind.modulator(fs as f64);
    let clean = modulator.modulate(text.as_bytes());
    println!("AKUSTINEN KEHÄ {}: {} tavua, {:.2} s per lähetys", kind.name(), text.len(), clean.len() as f64 / fs as f64);
    let mut ok_count = 0;
    for &g in &gains {
        let tx: Vec<f32> = clean.iter().map(|v| v * g).collect();
        let (rec, volume) = capture(&a, &tx, fs, 0.1 * g, 0.7)?;
        let w = wav::Wav { sample_rate: fs, samples: rec };
        let r = corpus::decode_expect(kind, &w, text.as_bytes());
        ok_count += r.ok as u32;
        let rms = modem_core::dsp::rms(&w.samples);
        println!(
            "  voimakkuus {g:<5} {}  SNR {}  korjattu {}  äänitteen taso {:.1} dBFS",
            if r.ok { "ok   " } else if r.detected { "VIRHE" } else { "EI HAVAITTU" },
            r.snr_db.map_or("-".into(), |s| format!("{s:.1} dB")),
            r.corrected,
            dbfs(rms)
        );
        if !a.has("no-save") {
            std::fs::create_dir_all(corpus::TX_DIR).map_err(|e| e.to_string())?;
            let id = format!("{}-{}-{}-g{:03.0}", bench::utc_now(), label, kind.name(), g * 100.0);
            let base = std::path::Path::new(corpus::TX_DIR).join(&id);
            std::fs::write(base.with_extension("wav"), wav::encode_pcm16(fs, &w.samples)).map_err(|e| e.to_string())?;
            let mut meta = common_meta(&a, &id, "lahetys", fs, volume, g as f64);
            meta.push(("phy", Json::str(kind.name())));
            meta.push(("text", Json::str(text)));
            meta.push(("ok_when_recorded", Json::Bool(r.ok)));
            std::fs::write(base.with_extension("json"), Json::obj(meta).pretty()).map_err(|e| e.to_string())?;
        }
    }
    println!("  {ok_count}/{} onnistui", gains.len());
    Ok(if ok_count as usize == gains.len() { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

/// Korpuksen regressioajo ja listaus.
pub fn cmd_corpus(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &["holdout"])?;
    let only = match a.str("phy") {
        Some(_) => Some(crate::phy_arg(&a)?),
        None => None,
    };
    let channels = corpus::entries(corpus::CHANNEL_DIR);
    println!("KANAVAMITTAUKSET ({})", channels.len());
    for (_, meta) in &channels {
        let r = meta.get("results");
        let num = |k: &str| r.and_then(|r| r.num_at(k));
        println!(
            "  {:<40}{} RT60 {}  suora/kaiku {}  melu {}  {}",
            meta.str_at("id").unwrap_or("?"),
            if corpus::is_holdout(meta) { " (vert.)" } else { "        " },
            num("rt60_s").map_or("-".into(), |v| format!("{v:.2} s")),
            num("drr_db").map_or("-".into(), |v| format!("{v:.0} dB")),
            num("noise_dbfs").map_or("-".into(), |v| format!("{v:.0} dBFS")),
            meta.str_at("note").unwrap_or(""),
        );
    }
    println!("LÄHETYSÄÄNITTEET");
    let s = corpus::run_regression(only, a.has("holdout"), true);
    println!("  viritysaineisto: {}/{} ok", s.ok, s.n);
    if a.has("holdout") {
        println!("  vertailuaineisto: {}/{} ok", s.holdout_ok, s.holdout_n);
    }
    if s.stale > 0 {
        println!("  vanhentuneita: {}", s.stale);
    }
    Ok(if s.ok == s.n { ExitCode::SUCCESS } else { ExitCode::from(1) })
}
