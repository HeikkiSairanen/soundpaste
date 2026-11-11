//! Audio playback and recording on a Mac with command-line tools (SoX: `play`, `rec`).
//!
//! No audio libraries: the tools run as subprocesses and audio passes
//! through the temp directory as WAV files.

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
            return Err(format!("command '{t}' not found (SoX is missing)"));
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
    let bytes = std::fs::read(path).map_err(|e| format!("no recording was produced: {e}"))?;
    let _ = std::fs::remove_file(path);
    let w = wav::decode(&bytes).map_err(|e| format!("recording: {e}"))?;
    if w.samples.is_empty() {
        return Err("the recording is empty".into());
    }
    if w.samples.iter().all(|&v| v == 0.0) {
        return Err("the recording is all zeros: the terminal probably lacks microphone permission \
                    (System Settings → Privacy & Security → Microphone)"
            .into());
    }
    Ok(w.samples)
}

/// Record from the microphone. Plays nothing.
pub fn record(fs: u32, duration_s: f64) -> Result<Vec<f32>, String> {
    check_tools()?;
    let path = tmp("rec.wav");
    let out = rec_command(&path, fs, duration_s).output().map_err(|e| format!("rec: {e}"))?;
    if !out.status.success() {
        return Err(format!("rec failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    read_recording(&path)
}

/// Play `tx` from the speaker while recording from the microphone.
/// Recording starts `pre_s` before playback and continues `post_s` after it.
pub fn play_and_record(tx: &[f32], fs: u32, pre_s: f64, post_s: f64) -> Result<Vec<f32>, String> {
    check_tools()?;
    let tx_path = tmp("tx.wav");
    let rec_path = tmp("rec.wav");
    // Silence at both ends: the playback tool may cut off the very end of the audio
    // (the last buffer is never played) and the audio device waking up may eat the start.
    let pad = vec![0.0f32; (0.3 * fs as f64) as usize];
    let padded: Vec<f32> = pad.iter().chain(tx).chain(&pad).copied().collect();
    std::fs::write(&tx_path, wav::encode_pcm16(fs, &padded)).map_err(|e| e.to_string())?;
    let tx_dur = padded.len() as f64 / fs as f64;
    // Leave generous time for playback to start; the recording is trimmed during analysis.
    let total = pre_s + tx_dur + post_s + 1.0;
    let rec = rec_command(&rec_path, fs, total).spawn().map_err(|e| format!("rec: {e}"))?;
    std::thread::sleep(Duration::from_secs_f64(pre_s));
    let play = Command::new("play").arg("-q").arg(&tx_path).stdout(Stdio::null()).stderr(Stdio::piped()).output();
    let _ = std::fs::remove_file(&tx_path);
    let play = play.map_err(|e| format!("play: {e}"))?;
    let rec_out = rec.wait_with_output().map_err(|e| format!("rec: {e}"))?;
    if !play.status.success() {
        return Err(format!("play failed: {}", String::from_utf8_lossy(&play.stderr).trim()));
    }
    if !rec_out.status.success() {
        return Err(format!("rec failed: {}", String::from_utf8_lossy(&rec_out.stderr).trim()));
    }
    read_recording(&rec_path)
}

fn osascript(script: &str) -> Option<String> {
    let out = Command::new("osascript").args(["-e", script]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// System output volume, 0–100.
pub fn system_volume() -> Option<u32> {
    osascript("output volume of (get volume settings)")?.parse().ok()
}

pub fn set_system_volume(v: u32) -> bool {
    osascript(&format!("set volume output volume {}", v.min(100))).is_some()
}

/// Device description for metadata: machine model and default audio devices. Not the
/// computer name, because it often contains the owner's name.
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
    format!("{model}; microphone: {input}; speaker: {output}")
}
