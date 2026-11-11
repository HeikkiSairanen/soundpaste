//! Äänitekorpus: oikeat äänitteet metatietoineen.
//!
//! - `corpus/kanava/<id>.wav`: kanavamittaus (pyyhkäisy + taustamelu). Vieressä
//!   `<id>.json` (metatiedot ja tulokset), `<id>.ir.wav` (impulssivaste) ja
//!   `<id>.kohina.wav` (taustamelu). Nämä eivät vanhene, kun modulaatio muuttuu.
//! - `corpus/lahetys/<id>.wav`: äänitetty modeemilähetys ja `<id>.json`, jossa on
//!   aaltomuodon nimi ja odotettu sisältö. Toimii vastaanottimen regressiotestinä
//!   niin kauan kuin sama aaltomuoto on olemassa.
//!
//! Osa aineistosta merkitään vertailuaineistoksi (`holdout`), jota ei käytetä
//! virittämiseen: se ajetaan vain erikseen pyydettäessä.

use crate::json::Json;
use modem_core::phy::{PhyEvent, PhyKind};
use modem_core::wav;
use std::path::{Path, PathBuf};

pub const CHANNEL_DIR: &str = "corpus/kanava";
pub const TX_DIR: &str = "corpus/lahetys";

/// Joka neljäs äänite päätyy vertailuaineistoon tunnisteen tiivisteen perusteella.
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
                Err(e) => eprintln!("varoitus: {}: {e}", p.display()),
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

/// Aja lähetysäänitteet nykyisillä vastaanottimilla. `only`: rajaa yhteen aaltomuotoon.
/// `verbose`: tulosta rivi per äänite.
pub fn run_regression(only: Option<PhyKind>, include_holdout: bool, verbose: bool) -> CorpusScore {
    let mut score = CorpusScore::default();
    for (path, meta) in entries(TX_DIR) {
        let id = meta.str_at("id").unwrap_or("?").to_string();
        let phy_name = meta.str_at("phy").unwrap_or("");
        let Some(kind) = PhyKind::from_name(phy_name) else {
            score.stale += 1;
            if verbose {
                println!("  {id:<44} vanhentunut (aaltomuotoa '{phy_name}' ei enää ole)");
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
                eprintln!("varoitus: {e}");
                continue;
            }
        };
        let r = decode_expect(kind, &w, text.as_bytes());
        if holdout {
            score.holdout_n += 1;
            score.holdout_ok += r.ok as u32;
        } else {
            score.n += 1;
            score.ok += r.ok as u32;
        }
        if verbose {
            println!(
                "  {id:<44} {:<8} {}{}  voimakkuus {}, SNR {}",
                phy_name,
                if r.ok { "ok   " } else if r.detected { "VIRHE" } else { "EI HAVAITTU" },
                if holdout { " (vert.)" } else { "" },
                meta.num_at("gain").map_or("?".into(), |g| format!("{g:.3}")),
                r.snr_db.map_or("-".into(), |s| format!("{s:.1} dB")),
            );
        }
    }
    score
}
