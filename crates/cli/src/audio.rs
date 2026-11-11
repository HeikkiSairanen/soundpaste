//! Äänen toisto ja äänitys Macilla järjestelmän komentorivityökaluilla (SoX: `play`, `rec`).
//!
//! Ei äänikirjastoja: työkalut ajetaan aliprosesseina ja ääni kulkee
//! WAV-tiedostoina väliaikaishakemiston kautta.

use modem_core::wav;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("soundpaste-{}-{name}", std::process::id()))
}

fn have(tool: &str) -> bool {
    Command::new("which").arg(tool).stdout(Stdio::null()).stderr(Stdio::null()).status().map_or(false, |s| s.success())
}

pub fn check_tools() -> Result<(), String> {
    for t in ["rec", "play"] {
        if !have(t) {
            return Err(format!("komentoa '{t}' ei löydy (SoX puuttuu)"));
        }
    }
    Ok(())
}

fn rec_command(path: &PathBuf, fs: u32, duration_s: f64) -> Command {
    let mut c = Command::new("rec");
    c.args(["-q", "-c", "1", "-b", "16", "-r", &fs.to_string()]).arg(path).args(["trim", "0", &format!("{duration_s:.3}")]);
    c.stdout(Stdio::null()).stderr(Stdio::piped());
    c
}

fn read_recording(path: &PathBuf) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("äänitettä ei syntynyt: {e}"))?;
    let _ = std::fs::remove_file(path);
    let w = wav::decode(&bytes).map_err(|e| format!("äänite: {e}"))?;
    if w.samples.is_empty() {
        return Err("äänite on tyhjä".into());
    }
    if w.samples.iter().all(|&v| v == 0.0) {
        return Err("äänite on pelkkää nollaa: päätteeltä puuttuu todennäköisesti mikrofonilupa \
                    (Järjestelmäasetukset → Tietosuoja ja suojaus → Mikrofoni)"
            .into());
    }
    Ok(w.samples)
}

/// Äänitä mikrofonista. Ei soita mitään.
pub fn record(fs: u32, duration_s: f64) -> Result<Vec<f32>, String> {
    check_tools()?;
    let path = tmp("rec.wav");
    let out = rec_command(&path, fs, duration_s).output().map_err(|e| format!("rec: {e}"))?;
    if !out.status.success() {
        return Err(format!("rec epäonnistui: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    read_recording(&path)
}

/// Soita `tx` kaiuttimesta ja äänitä samalla mikrofonista.
/// Äänitys alkaa `pre_s` ennen toistoa ja jatkuu `post_s` sen jälkeen.
pub fn play_and_record(tx: &[f32], fs: u32, pre_s: f64, post_s: f64) -> Result<Vec<f32>, String> {
    check_tools()?;
    let tx_path = tmp("tx.wav");
    let rec_path = tmp("rec.wav");
    std::fs::write(&tx_path, wav::encode_pcm16(fs, tx)).map_err(|e| e.to_string())?;
    let tx_dur = tx.len() as f64 / fs as f64;
    // Varataan toiston käynnistymiselle reilusti aikaa; äänite leikataan analyysissä.
    let total = pre_s + tx_dur + post_s + 1.0;
    let rec = rec_command(&rec_path, fs, total).spawn().map_err(|e| format!("rec: {e}"))?;
    std::thread::sleep(Duration::from_secs_f64(pre_s));
    let play = Command::new("play").arg("-q").arg(&tx_path).stdout(Stdio::null()).stderr(Stdio::piped()).output();
    let _ = std::fs::remove_file(&tx_path);
    let play = play.map_err(|e| format!("play: {e}"))?;
    let rec_out = rec.wait_with_output().map_err(|e| format!("rec: {e}"))?;
    if !play.status.success() {
        return Err(format!("play epäonnistui: {}", String::from_utf8_lossy(&play.stderr).trim()));
    }
    if !rec_out.status.success() {
        return Err(format!("rec epäonnistui: {}", String::from_utf8_lossy(&rec_out.stderr).trim()));
    }
    read_recording(&rec_path)
}

fn osascript(script: &str) -> Option<String> {
    let out = Command::new("osascript").args(["-e", script]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Järjestelmän äänenvoimakkuus 0–100.
pub fn system_volume() -> Option<u32> {
    osascript("output volume of (get volume settings)")?.parse().ok()
}

pub fn set_system_volume(v: u32) -> bool {
    osascript(&format!("set volume output volume {}", v.min(100))).is_some()
}

/// Laitteen kuvaus metatietoihin: konemalli ja oletusäänilaitteet. Ei koneen
/// nimeä, koska siinä on usein omistajan nimi.
pub fn device_description() -> String {
    let model = Command::new("sysctl").args(["-n", "hw.model"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let mut input = String::new();
    let mut output = String::new();
    if let Ok(o) = Command::new("system_profiler").arg("SPAudioDataType").output() {
        let text = String::from_utf8_lossy(&o.stdout);
        let mut current = String::new();
        for line in text.lines() {
            let t = line.trim();
            if line.starts_with("        ") && !line.starts_with("          ") && t.ends_with(':') {
                current = t.trim_end_matches(':').to_string();
            } else if t == "Default Input Device: Yes" {
                input = current.clone();
            } else if t == "Default Output Device: Yes" {
                output = current.clone();
            }
        }
    }
    format!("{model}; mikrofoni: {input}; kaiutin: {output}")
}
