//! Comparison of two bench runs: highlights regressions.

use crate::json::Json;

pub struct Outcome {
    pub regressions: u32,
    pub improvements: u32,
}

fn fmt(v: Option<f64>, unit: &str, digits: usize) -> String {
    match v {
        Some(x) => format!("{x:.digits$}{unit}"),
        None => "-".into(),
    }
}

struct Metric {
    label: &'static str,
    key: &'static str,
    unit: &'static str,
    digits: usize,
    /// +1: larger is better, -1: smaller is better.
    better: f64,
    /// Smallest difference that counts as a change: absolute, or as a relative fraction.
    tol_abs: f64,
    tol_rel: f64,
    /// A missing value means "not reached" (worst possible).
    missing_is_worst: bool,
}

const METRICS: &[Metric] = &[
    Metric { label: "Transfer time (s)", key: "transfer_time_s", unit: "", digits: 2, better: -1.0, tol_abs: 0.0, tol_rel: 0.05, missing_is_worst: false },
    Metric { label: "Success rate", key: "success_rate", unit: "", digits: 3, better: 1.0, tol_abs: 0.02, tol_rel: 0.0, missing_is_worst: false },
    Metric { label: "Min SNR, room (dB)", key: "min_snr_db_room", unit: "", digits: 0, better: -1.0, tol_abs: 1.0, tol_rel: 0.0, missing_is_worst: true },
    Metric { label: "BER before FEC", key: "ber_pre_fec", unit: "", digits: 5, better: -1.0, tol_abs: 1e-4, tol_rel: 1.0, missing_is_worst: false },
    Metric { label: "Net rate (bytes/s)", key: "net_bytes_per_s", unit: "", digits: 1, better: 1.0, tol_abs: 0.0, tol_rel: 0.05, missing_is_worst: false },
    Metric { label: "Corpus, decoded", key: "corpus_rate", unit: "", digits: 2, better: 1.0, tol_abs: 0.001, tol_rel: 0.0, missing_is_worst: false },
    Metric { label: "False frames", key: "false_frames", unit: "", digits: 0, better: -1.0, tol_abs: 0.5, tol_rel: 0.0, missing_is_worst: false },
];

/// Returns -1 (regression), 0 (no change) or +1 (improvement).
fn judge(old: Option<f64>, new: Option<f64>, better: f64, tol_abs: f64, tol_rel: f64, missing_is_worst: bool) -> i32 {
    match (old, new) {
        (Some(a), Some(b)) => {
            let tol = tol_abs.max(tol_rel * a.abs());
            let d = (b - a) * better;
            if d > tol {
                1
            } else if d < -tol {
                -1
            } else {
                0
            }
        }
        (Some(_), None) if missing_is_worst => -1,
        (None, Some(_)) if missing_is_worst => 1,
        _ => 0,
    }
}

fn mark(j: i32) -> &'static str {
    match j {
        1 => "improved",
        -1 => "REGRESSED",
        _ => "",
    }
}

pub fn compare(base: &Json, new: &Json) -> Outcome {
    let mut out = Outcome { regressions: 0, improvements: 0 };
    let mut tally = |j: i32| match j {
        1 => out.improvements += 1,
        -1 => out.regressions += 1,
        _ => {}
    };
    println!(
        "COMPARISON  baseline: {} ({}, {})   new: {} ({}, {})",
        base.str_at("commit").unwrap_or("?"),
        base.str_at("phy").unwrap_or("?"),
        base.str_at("mode").unwrap_or("?"),
        new.str_at("commit").unwrap_or("?"),
        new.str_at("phy").unwrap_or("?"),
        new.str_at("mode").unwrap_or("?"),
    );
    if base.str_at("mode") != new.str_at("mode") {
        println!("  NOTE: the runs were measured differently (mode differs); transfer time and rate are not comparable.");
    }
    println!("  {:<26}{:>12}{:>12}{:>12}", "metric", "baseline", "new", "change");
    let (bs, ns) = (base.get("summary"), new.get("summary"));
    for m in METRICS {
        let a = bs.and_then(|s| s.num_at(m.key));
        let b = ns.and_then(|s| s.num_at(m.key));
        let j = judge(a, b, m.better, m.tol_abs, m.tol_rel, m.missing_is_worst);
        tally(j);
        let delta = match (a, b) {
            (Some(a), Some(b)) => format!("{:+.*}", m.digits, b - a),
            _ => String::new(),
        };
        println!("  {:<26}{:>12}{:>12}{:>12}  {}", m.label, fmt(a, m.unit, m.digits), fmt(b, m.unit, m.digits), delta, mark(j));
    }
    println!("  min SNR per profile (dB):");
    let empty = Json::Arr(vec![]);
    let bp = base.get("profiles").unwrap_or(&empty).arr();
    for p in new.get("profiles").unwrap_or(&empty).arr() {
        let name = p.str_at("name").unwrap_or("?");
        let b = p.num_at("min_snr_db");
        let old = bp.iter().find(|q| q.str_at("name") == Some(name));
        let a = old.and_then(|q| q.num_at("min_snr_db"));
        let j = if old.is_some() { judge(a, b, -1.0, 1.0, 0.0, true) } else { 0 };
        tally(j);
        let delta = match (a, b) {
            (Some(a), Some(b)) => format!("{:+.0}", b - a),
            _ => String::new(),
        };
        let note = if old.is_none() { "(new profile)" } else { mark(j) };
        println!("  {:<26}{:>12}{:>12}{:>12}  {}", name, fmt(a, "", 0), fmt(b, "", 0), delta, note);
    }
    for q in bp {
        let name = q.str_at("name").unwrap_or("?");
        if !new.get("profiles").unwrap_or(&empty).arr().iter().any(|p| p.str_at("name") == Some(name)) {
            println!("  {name:<26} missing from the new run");
        }
    }
    println!("  total: {} regressions, {} improvements", out.regressions, out.improvements);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judging() {
        assert_eq!(judge(Some(0.95), Some(0.90), 1.0, 0.02, 0.0, false), -1);
        assert_eq!(judge(Some(0.95), Some(0.94), 1.0, 0.02, 0.0, false), 0);
        assert_eq!(judge(Some(5.0), Some(4.0), -1.0, 0.0, 0.05, false), 1);
        assert_eq!(judge(Some(3.0), None, -1.0, 1.0, 0.0, true), -1);
        assert_eq!(judge(None, Some(3.0), -1.0, 1.0, 0.0, true), 1);
    }
}
