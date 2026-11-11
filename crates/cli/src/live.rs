//! Asema oikealla äänellä: sama tilakone kuin selaimessa, ääni SoX:n kautta.
//!
//! `rec` syöttää mikrofonin näytteitä putkeen ja `play` soittaa putkesta.
//! Silmukka lukee 10 ms mikrofonia, antaa sen asemalle ja kirjoittaa aseman
//! tuottamat 10 ms kaiuttimelle. Hiljaisuuskin kirjoitetaan, jotta äänilaite
//! pysyy auki ja viive vakiona.

use crate::args::Args;
use crate::audio;
use crate::bench;
use crate::corpus;
use crate::json::Json;
use modem_core::link::{Event, LinkConfig};
use modem_core::station::Station;
use modem_core::wav;
use std::io::{Read, Write};
use std::process::{Command, ExitCode, Stdio};

const FS: u32 = 48000;
const BLOCK: usize = 480;

fn describe(e: &Event) -> Option<String> {
    Some(match e {
        Event::Log(s) => s.clone(),
        Event::Received { data, .. } => format!("VASTAANOTETTU ({} tavua): {}", data.len(), String::from_utf8_lossy(data)),
        Event::Delivered { elapsed, .. } => format!("PERILLÄ {:.2} s", *elapsed as f64 / 1000.0),
        other => format!("{other:?}"),
    })
}

/// `soundpaste live [--send "teksti"] [--seconds N] [--echo] [--once]`
pub fn cmd_live(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &["handshake", "quiet", "echo", "once", "no-save", "holdout", "no-holdout"])?;
    audio::check_tools()?;
    let kind = crate::phy_arg(&a)?;
    let send: Option<Vec<u8>> = a.str("send").map(|s| s.as_bytes().to_vec());
    let seconds: f64 = a.num("seconds", if send.is_some() { 40.0 } else { 60.0 })?;
    let expect: Option<String> = a.str("expect").map(String::from);
    let cfg = LinkConfig { inline_start: !a.has("handshake"), burst: a.num("burst", 0)?, ..LinkConfig::default() };
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    let mut station = Station::new(kind, FS as f64, cfg, seed);

    let raw_args = ["-q", "-t", "raw", "-e", "floating-point", "-b", "32", "-c", "1", "-r", "48000", "-"];
    let mut rec = Command::new("rec").args(["--buffer", "1920"]).args(raw_args).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("rec: {e}"))?;
    let mut play = Command::new("play").args(["--buffer", "1920"]).args(raw_args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| format!("play: {e}"))?;
    let mut mic = rec.stdout.take().unwrap();
    let mut speaker = play.stdin.take().unwrap();

    println!("ASEMA {} oikealla äänellä ({} Hz). {}", kind.name(), FS, match &send { Some(s) => format!("Lähetetään {} tavua.", s.len()), None => "Kuunnellaan.".into() });
    let mut recording: Vec<f32> = Vec::new();
    let mut bytes = vec![0u8; BLOCK * 4];
    let mut input = vec![0.0f32; BLOCK];
    let mut output = vec![0.0f32; BLOCK];
    let mut out_bytes = vec![0u8; BLOCK * 4];
    let mut sent = false;
    let mut received: Vec<Vec<u8>> = Vec::new();
    let mut delivered = None;
    let mut failed = false;
    let mut stop_at: Option<f64> = None;
    let mut echo_queue: Option<(f64, Vec<u8>)> = None;
    loop {
        if mic.read_exact(&mut bytes).is_err() {
            return Err("mikrofonin lukeminen katkesi".into());
        }
        for (i, v) in input.iter_mut().enumerate() {
            *v = f32::from_le_bytes([bytes[4 * i], bytes[4 * i + 1], bytes[4 * i + 2], bytes[4 * i + 3]]);
        }
        recording.extend_from_slice(&input);
        let t = recording.len() as f64 / FS as f64;
        // Annetaan äänilaitteen käynnistyä ennen lähetystä.
        if !sent && t >= 1.0 {
            if let Some(data) = &send {
                station.send(data).map_err(|e| format!("{e:?}"))?;
            }
            sent = true;
        }
        if let Some((at, data)) = &echo_queue {
            if t >= *at && !station.is_sending() {
                let _ = station.send(data);
                echo_queue = None;
            }
        }
        station.process(&input, &mut output);
        for (i, v) in output.iter().enumerate() {
            out_bytes[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        if speaker.write_all(&out_bytes).is_err() {
            return Err("kaiuttimeen kirjoittaminen katkesi".into());
        }
        while let Some(e) = station.poll_event() {
            match &e {
                Event::Received { data, .. } => {
                    received.push(data.clone());
                    if a.has("echo") {
                        echo_queue = Some((t + 1.5, data.clone()));
                    }
                    if a.has("once") {
                        stop_at = Some(t + 1.5);
                    }
                }
                Event::Delivered { elapsed, .. } => {
                    delivered = Some(*elapsed);
                    if send.is_some() && !a.has("echo") {
                        stop_at = Some(t + 1.0);
                    }
                }
                Event::Failed { .. } => {
                    failed = true;
                    if send.is_some() {
                        stop_at = Some(t + 1.0);
                    }
                }
                _ => {}
            }
            if !(a.has("quiet") && matches!(e, Event::Log(_))) {
                if let Some(line) = describe(&e) {
                    println!("{t:7.2} s  {line}");
                }
            }
        }
        if t >= seconds || stop_at.map_or(false, |s| t >= s) {
            break;
        }
    }
    drop(speaker);
    let _ = rec.kill();
    let _ = play.wait();
    let _ = rec.wait();
    let st = station.stats();
    println!("alkumerkkejä {}, ehjiä kehyksiä {}, hylättyjä {}, lähetettyjä {}, korjattuja tavuja {}", st.preambles, st.frames_ok, st.frames_failed, st.frames_sent, st.corrected);

    // --record tiedosto.wav: koko mikrofoniäänite talteen analyysia varten.
    if let Some(path) = a.str("record") {
        std::fs::write(path, wav::encode_pcm16(FS, &recording)).map_err(|e| format!("{path}: {e}"))?;
        println!("äänite: {path}");
    }
    // Äänite korpukseen. Odotettu sisältö on se, mitä tämän aseman piti vastaanottaa.
    let expected: Option<Vec<u8>> = expect.as_ref().map(|s| s.as_bytes().to_vec()).or_else(|| received.first().cloned());
    if !a.has("no-save") {
        if let Some(text) = &expected {
            let id = format!("{}-{}-sessio-{}", bench::utc_now(), a.str_or("label", "mac"), kind.name());
            let holdout = if a.has("holdout") { true } else if a.has("no-holdout") { false } else { corpus::auto_holdout(&id) };
            std::fs::create_dir_all(corpus::TX_DIR).map_err(|e| e.to_string())?;
            let base = std::path::Path::new(corpus::TX_DIR).join(&id);
            std::fs::write(base.with_extension("wav"), wav::encode_pcm16(FS, &recording)).map_err(|e| e.to_string())?;
            let meta = Json::obj(vec![
                ("id", Json::str(&id)),
                ("kind", Json::str("sessio")),
                ("time_utc", Json::str(&bench::utc_now())),
                ("device", Json::str(&a.str("device").map(String::from).unwrap_or_else(audio::device_description))),
                ("sample_rate", Json::num(FS)),
                ("system_volume", Json::opt(audio::system_volume().map(|v| v as f64))),
                ("distance_m", Json::opt(a.str("distance").and_then(|d| d.parse().ok()))),
                ("note", Json::str(a.str_or("note", ""))),
                ("holdout", Json::Bool(holdout)),
                ("phy", Json::str(kind.name())),
                ("text", Json::str(&String::from_utf8_lossy(text))),
                ("ok_when_recorded", Json::Bool(received.iter().any(|r| r == text))),
            ]);
            std::fs::write(base.with_extension("json"), meta.pretty()).map_err(|e| e.to_string())?;
            println!("äänite: {}.wav", base.display());
        }
    }
    let ok = match (&send, &expect) {
        (Some(_), _) => delivered.is_some() && !failed,
        (None, Some(text)) => received.iter().any(|r| r == text.as_bytes()),
        (None, None) => !received.is_empty(),
    };
    Ok(if ok { ExitCode::SUCCESS } else { ExitCode::from(1) })
}
