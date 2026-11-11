//! soundpaste-komentorivityökalu: sama modeemiydin kuin selaimessa.

mod args;
mod bench;
mod compare;
mod json;

use args::Args;
use json::Json;
use modem_core::phy::{PhyEvent, PhyKind};
use modem_core::wav;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const HELP: &str = "\
soundpaste: äänimodeemin kehitystyökalu

Käyttö: soundpaste <komento> [valinnat]

Tiedostot:
  encode [--phy P] [--rate 48000] -o ulos.wav \"teksti\"   Koodaa teksti äänitiedostoksi
  decode [--phy P] sisään.wav                             Pura äänitiedosto
  sim [--phy P] [--profile huone] [--snr 20] [--seed 1] [-o rx.wav] \"teksti\"
                                                          Yksi siirto simuloidun kanavan läpi

Mittaus:
  bench [--phy P] [--seeds 12] [--quick] [--no-save]      Mittausajo ja pistetaulu
  compare [uusi.json] [--baseline bench/baseline.json]    Vertaa ajoa perustasoon
  baseline [ajo.json]                                     Aseta ajo (oletus: uusin) perustasoksi
  profiles                                                Listaa kanavaprofiilit
";

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = raw.first().cloned() else {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    };
    let rest = &raw[1..];
    let result = match cmd.as_str() {
        "encode" => cmd_encode(rest),
        "decode" => cmd_decode(rest),
        "sim" => cmd_sim(rest),
        "bench" => cmd_bench(rest),
        "compare" => cmd_compare(rest),
        "baseline" => cmd_baseline(rest),
        "profiles" => {
            for p in bench::synthetic_profiles() {
                println!("{:<14} {}", p.name, p.desc);
            }
            Ok(ExitCode::SUCCESS)
        }
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("tuntematon komento '{other}'\n\n{HELP}")),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("virhe: {e}");
            ExitCode::from(2)
        }
    }
}

fn phy_arg(a: &Args) -> Result<PhyKind, String> {
    match a.str("phy") {
        None => Ok(PhyKind::DEFAULT),
        Some(s) => PhyKind::from_name(s).ok_or_else(|| {
            format!("tuntematon aaltomuoto '{s}', vaihtoehdot: {}", PhyKind::ALL.iter().map(|k| k.name()).collect::<Vec<_>>().join(", "))
        }),
    }
}

fn cmd_encode(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &[])?;
    let kind = phy_arg(&a)?;
    let rate: u32 = a.num("rate", 48000)?;
    let text = a.positional.first().ok_or("anna koodattava teksti")?;
    let out = a.str("out").ok_or("anna tulostiedosto: -o ulos.wav")?;
    let m = kind.modulator(rate as f64);
    if text.len() > m.max_payload() {
        return Err(format!("teksti on {} tavua, enimmäispituus on {}", text.len(), m.max_payload()));
    }
    let mut samples = vec![0.0; rate as usize / 4];
    samples.extend(m.modulate(text.as_bytes()));
    samples.extend(vec![0.0; rate as usize / 4]);
    std::fs::write(out, wav::encode_pcm16(rate, &samples)).map_err(|e| format!("{out}: {e}"))?;
    println!("{out}: {} tavua, {:.2} s, {} Hz, {}", text.len(), samples.len() as f64 / rate as f64, rate, kind.name());
    Ok(ExitCode::SUCCESS)
}

fn print_events(events: &[PhyEvent], fs: f64) -> usize {
    let mut frames = 0;
    for ev in events {
        match ev {
            PhyEvent::Detected { at } => println!("[{:8.3} s] alkumerkki", *at as f64 / fs),
            PhyEvent::Frame { payload, stats, .. } => {
                frames += 1;
                println!(
                    "[{:8.3} s] kehys {} tavua, SNR {:.1} dB, taso {:.4}, leikkautunut {:.1} %, korjattu {}",
                    stats.end_sample as f64 / fs,
                    payload.len(),
                    stats.snr_db,
                    stats.rms,
                    stats.clipped * 100.0,
                    stats.corrected
                );
                println!("{}", String::from_utf8_lossy(payload));
            }
            PhyEvent::Failed { reason, stats, .. } => {
                println!("[{:8.3} s] kehys hylätty ({reason:?}), SNR {:.1} dB, taso {:.4}", stats.end_sample as f64 / fs, stats.snr_db, stats.rms);
            }
        }
    }
    frames
}

fn cmd_decode(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &[])?;
    let kind = phy_arg(&a)?;
    let path = a.positional.first().ok_or("anna äänitiedosto")?;
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let w = wav::decode(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let mut d = kind.demodulator(w.sample_rate as f64);
    let mut events = Vec::new();
    for chunk in w.samples.chunks(1024) {
        d.push(chunk, &mut events);
    }
    println!("{path}: {:.2} s, {} Hz, {}", w.samples.len() as f64 / w.sample_rate as f64, w.sample_rate, kind.name());
    let frames = print_events(&events, w.sample_rate as f64);
    if frames == 0 {
        println!("ei ehjiä kehyksiä");
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_sim(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &[])?;
    let kind = phy_arg(&a)?;
    let snr: f64 = a.num("snr", 20.0)?;
    let seed: u64 = a.num("seed", 1)?;
    let name = a.str_or("profile", "huone");
    let text = a.positional.first().map(|s| s.as_str()).unwrap_or(bench::LINK60);
    let profiles = bench::synthetic_profiles();
    let profile = profiles.iter().find(|p| p.name == name).ok_or_else(|| format!("tuntematon profiili '{name}' (ks. soundpaste profiles)"))?;
    let cfg = (profile.make)(snr, seed);
    let tx = kind.modulator(cfg.fs_tx).modulate(text.as_bytes());
    let rx = modem_core::sim::run_oneway(&tx, &cfg, seed);
    if let Some(out) = a.str("out") {
        std::fs::write(out, wav::encode_pcm16(cfg.fs_rx as u32, &rx)).map_err(|e| format!("{out}: {e}"))?;
    }
    let mut d = kind.demodulator(cfg.fs_rx);
    let mut events = Vec::new();
    for chunk in rx.chunks(1024) {
        d.push(chunk, &mut events);
    }
    println!("profiili {name} ({}), SNR {snr} dB, siemen {seed}", profile.desc);
    let frames = print_events(&events, cfg.fs_rx);
    Ok(if frames > 0 { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

fn results_dir() -> PathBuf {
    PathBuf::from("bench/results")
}

fn latest_result() -> Result<PathBuf, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(results_dir())
        .map_err(|e| format!("bench/results: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map_or(false, |x| x == "json"))
        .collect();
    files.sort();
    files.pop().ok_or_else(|| "bench/results on tyhjä, aja ensin soundpaste bench".into())
}

fn load_json(path: &Path) -> Result<Json, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Json::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn cmd_bench(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &["quick", "no-save"])?;
    let kind = phy_arg(&a)?;
    let seeds: u32 = a.num("seeds", if a.has("quick") { 4 } else { 12 })?;
    let threads: usize = a.num("threads", std::thread::available_parallelism().map_or(4, |n| n.get()))?;
    let started = std::time::Instant::now();
    let profiles = bench::synthetic_profiles();
    let results = bench::run_profiles(&bench::BenchOptions { kind, seeds, threads }, &profiles);
    let board = bench::Scoreboard::oneway(kind, seeds, results);
    board.print();
    println!("\n(ajo kesti {:.1} s)", started.elapsed().as_secs_f64());
    if !a.has("no-save") {
        std::fs::create_dir_all(results_dir()).map_err(|e| e.to_string())?;
        let commit = board.commit.replace("+muutoksia", "-muutoksia");
        let path = results_dir().join(format!("{}-{}-{}.json", board.time_utc, commit, board.phy));
        std::fs::write(&path, board.to_json().pretty()).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("tallennettu: {}", path.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_compare(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &[])?;
    let base_path = PathBuf::from(a.str_or("baseline", "bench/baseline.json"));
    let new_path = match a.positional.first() {
        Some(p) => PathBuf::from(p),
        None => latest_result()?,
    };
    if !base_path.exists() {
        return Err(format!("{} puuttuu: aseta perustaso komennolla soundpaste baseline", base_path.display()));
    }
    let outcome = compare::compare(&load_json(&base_path)?, &load_json(&new_path)?);
    Ok(if outcome.regressions > 0 { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

fn cmd_baseline(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &[])?;
    let src = match a.positional.first() {
        Some(p) => PathBuf::from(p),
        None => latest_result()?,
    };
    load_json(&src)?;
    std::fs::copy(&src, "bench/baseline.json").map_err(|e| e.to_string())?;
    println!("perustaso: bench/baseline.json <- {}", src.display());
    Ok(ExitCode::SUCCESS)
}
