//! Recording corpus: real recordings with their metadata.
//!
//! - `corpus/channel/<id>.wav`: a channel measurement (sweep + background noise). Next to
//!   it `<id>.json` (metadata and results), `<id>.ir.wav` (impulse response) and
//!   `<id>.noise.wav` (background noise). These do not go stale when the modulation changes.
//! - `corpus/tx/<id>.wav`: a recorded modem transmission and `<id>.json` with the
//!   waveform name and the expected content. Serves as a receiver regression test
//!   for as long as the same waveform exists.
//!
//! Part of the material is marked as a holdout set (`holdout`) that is not used for
//! tuning: it is run only when explicitly requested.

use crate::json::Json;
use modem_core::phy::{PhyEvent, PhyKind};
use modem_core::wav;
use std::path::{Path, PathBuf};

pub const CHANNEL_DIR: &str = "corpus/channel";
pub const TX_DIR: &str = "corpus/tx";

/// Every fourth recording ends up in the holdout set, based on a hash of its id.
pub fn auto_holdout(id: &str) -> bool {
    modem_core::crc::crc32(id.as_bytes()) % 4 == 0
}

pub fn entries(dir: &str) -> Vec<(PathBuf, Json)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    let mut paths: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map_or(false, |x| x == "json")).collect();
    paths.sort();
    for p in paths {
        if let Ok(text) = std::fs::read_to_string(&p) {
            match Json::parse(&text) {
                Ok(j) => out.push((p, j)),
                Err(e) => eprintln!("warning: {}: {e}", p.display()),
            }
        }
    }
    out
}

pub fn read_wav(path: &Path) -> Result<wav::Wav, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    wav::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn is_holdout(meta: &Json) -> bool {
    matches!(meta.get("holdout"), Some(Json::Bool(true)))
}

#[derive(Default, Debug, Clone, Copy)]
pub struct CorpusScore {
    pub n: u32,
    pub ok: u32,
    pub holdout_n: u32,
    pub holdout_ok: u32,
    pub stale: u32,
}

pub struct DecodeResult {
    pub ok: bool,
    pub detected: bool,
    pub snr_db: Option<f32>,
    pub corrected: u32,
}

pub fn decode_expect(kind: PhyKind, w: &wav::Wav, expected: &[u8]) -> DecodeResult {
    let mut d = kind.demodulator(w.sample_rate as f64);
    let mut events = Vec::new();
    for chunk in w.samples.chunks(1024) {
        d.push(chunk, &mut events);
    }
    let mut r = DecodeResult { ok: false, detected: false, snr_db: None, corrected: 0 };
    for ev in events {
        match ev {
            PhyEvent::Detected { .. } => r.detected = true,
            PhyEvent::Frame { payload, stats, .. } if payload == expected => {
                r.ok = true;
                r.snr_db = Some(stats.snr_db);
                r.corrected = stats.corrected;
            }
            PhyEvent::Failed { stats, .. } if !r.ok => r.snr_db = Some(stats.snr_db),
            _ => {}
        }
    }
    r
}

/// Session recording (the whole exchange from one station's microphone): the recording
/// is fed to a fresh station that acts as a passive receiver. Its own replies go
/// nowhere, but it assembles the message from the frames it hears with the same
/// code as a real receiver.
pub fn decode_session(kind: PhyKind, w: &wav::Wav, expected: &[u8]) -> DecodeResult {
    use modem_core::link::{Event, LinkConfig};
    let mut station = modem_core::station::Station::new(kind, w.sample_rate as f64, LinkConfig::default(), 1);
    let mut out = vec![0.0f32; 480];
    let mut ok = false;
    for chunk in w.samples.chunks(480) {
        station.process(chunk, &mut out[..chunk.len()]);
        while let Some(e) = station.poll_event() {
            if let Event::Received { data, .. } = e {
                ok |= data == expected;
            }
        }
    }
    let st = station.stats();
    DecodeResult { ok, detected: st.preambles > 0, snr_db: (st.frames_ok > 0).then_some(st.last_snr_db), corrected: st.corrected }
}

/// Run the transmission recordings through the current receivers. `only`: restrict to one
/// waveform. `verbose`: print a line per recording.
pub fn run_regression(only: Option<PhyKind>, include_holdout: bool, verbose: bool) -> CorpusScore {
    let mut score = CorpusScore::default();
    for (path, meta) in entries(TX_DIR) {
        let id = meta.str_at("id").unwrap_or("?").to_string();
        let phy_name = meta.str_at("phy").unwrap_or("");
        let Some(kind) = PhyKind::from_name(phy_name) else {
            score.stale += 1;
            if verbose {
                println!("  {id:<50} stale (waveform '{phy_name}' no longer exists)");
            }
            continue;
        };
        if only.map_or(false, |k| k != kind) {
            continue;
        }
        let holdout = is_holdout(&meta);
        if holdout && !include_holdout {
            continue;
        }
        let Some(text) = meta.str_at("text") else { continue };
        let w = match read_wav(&path.with_extension("wav")) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("warning: {e}");
                continue;
            }
        };
        let r = if meta.str_at("kind") == Some("session") { decode_session(kind, &w, text.as_bytes()) } else { decode_expect(kind, &w, text.as_bytes()) };
        if holdout {
            score.holdout_n += 1;
            score.holdout_ok += r.ok as u32;
        } else {
            score.n += 1;
            score.ok += r.ok as u32;
        }
        if verbose {
            println!(
                "  {id:<50} {:<8} {}{}  gain {}, SNR {}",
                phy_name,
                if r.ok { "ok   " } else if r.detected { "ERROR" } else { "NOT DETECTED" },
                if holdout { " (hold.)" } else { "" },
                meta.num_at("gain").map_or("?".into(), |g| format!("{g:.3}")),
                r.snr_db.map_or("-".into(), |s| format!("{s:.1} dB")),
            );
        }
    }
    score
}
