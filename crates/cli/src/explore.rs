//! Modulaatiovaihtoehtojen vertailukoe (DESIGN.md:n pohjaksi).
//!
//! Jokaisella vaihtoehdolla lähetetään satunnaisia symboleita kanavaprofiilien
//! läpi ja mitataan raaka bittivirhesuhde, kun symbolien ajoitus tunnetaan
//! täsmälleen (ihanteellinen tahdistus). Näin nähdään pelkän modulaation
//! kestävyys kaikua ja kohinaa vastaan ilman kehystystä ja virheenkorjausta.
//!
//! Kaikki vaihtoehdot rakentuvat samasta palikasta: tasavälisestä sävelristikosta,
//! jossa kullakin symbolilla osa sävelistä on päällä.
//! - M-FSK: jokaisessa alikanavassa yksi M:stä sävelestä on päällä (tieto on
//!   sävelen valinnassa, ilmaisu energian perusteella).
//! - hyppy: peräkkäiset symbolit käyttävät eri sävelryhmää (G ryhmää), jolloin
//!   edellisen symbolin kaiku ei osu nykyisen symbolin ehdokkaisiin.
//! - DQPSK: kaikki sävelet ovat päällä ja tieto on vaiheen muutoksessa
//!   edelliseen symboliin nähden (OFDM:n yksinkertaisin muoto).

use crate::args::Args;
use crate::bench;
use modem_core::rng::Rng;
use modem_core::sim::{run_oneway, OneWay};
use std::f64::consts::TAU;
use std::process::ExitCode;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layout {
    /// Sävel = alikanava·(M·G) + arvo·G + ryhmä: ehdokkaat ovat G sävelvälin päässä toisistaan.
    Interleaved,
    /// Sävel = alikanava·(M·G) + ryhmä·M + arvo: ryhmät ovat vierekkäisiä lohkoja alikanavan sisällä.
    Block,
    /// Sävel = ryhmä·(K·M) + alikanava·M + arvo: koko kaista on jaettu G lohkoon.
    Band,
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Mfsk { m: usize, k: usize, g: usize, layout: Layout },
    Dqpsk { n: usize },
}

#[derive(Clone, Debug)]
pub struct Scheme {
    pub name: &'static str,
    pub kind: Kind,
    pub f0: f64,
    /// Ilmaisuikkunan pituus; sävelväli on 1/t_win.
    pub t_win: f64,
    /// Suoja-aika ennen ikkunaa.
    pub t_guard: f64,
}

impl Scheme {
    pub fn tones(&self) -> usize {
        match self.kind {
            Kind::Mfsk { m, k, g, .. } => m * k * g,
            Kind::Dqpsk { n } => n,
        }
    }
    pub fn bits_per_symbol(&self) -> usize {
        match self.kind {
            Kind::Mfsk { m, k, .. } => k * m.trailing_zeros() as usize,
            Kind::Dqpsk { n } => 2 * n,
        }
    }
    pub fn t_sym(&self) -> f64 {
        self.t_win + self.t_guard
    }
    pub fn bit_rate(&self) -> f64 {
        self.bits_per_symbol() as f64 / self.t_sym()
    }
    pub fn bandwidth(&self) -> f64 {
        self.tones() as f64 / self.t_win
    }
    fn freq(&self, tone: usize) -> f64 {
        self.f0 + tone as f64 / self.t_win
    }
    fn tone_index(&self, sub: usize, value: usize, sym: usize) -> usize {
        match self.kind {
            Kind::Mfsk { m, k, g, layout } => {
                let grp = sym % g;
                match layout {
                    Layout::Interleaved => sub * m * g + value * g + grp,
                    Layout::Block => sub * m * g + grp * m + value,
                    Layout::Band => grp * k * m + sub * m + value,
                }
            }
            Kind::Dqpsk { .. } => sub,
        }
    }
}

/// Satunnaiset symbolit: MFSK:lle alikanavien arvot, DQPSK:lle vaihesiirrot 0..4.
fn random_symbols(s: &Scheme, count: usize, rng: &mut Rng) -> Vec<Vec<usize>> {
    let (subs, levels) = match s.kind {
        Kind::Mfsk { m, k, .. } => (k, m),
        Kind::Dqpsk { n } => (n, 4),
    };
    (0..count).map(|_| (0..subs).map(|_| rng.below(levels as u64) as usize).collect()).collect()
}

pub fn modulate(s: &Scheme, symbols: &[Vec<usize>], fs: f64, rng: &mut Rng) -> Vec<f32> {
    let n_sym = (s.t_sym() * fs).round() as usize;
    let ramp = ((s.t_guard * 0.5).min(0.003) * fs) as usize;
    let mut out = vec![0.0f32; n_sym * symbols.len()];
    // Jokaisella sävelellä on oma satunnainen alkuvaiheensa, jotta samanaikaiset
    // sävelet eivät summaudu yhdeksi piikiksi.
    let phases: Vec<f64> = (0..s.tones()).map(|_| rng.range(0.0, TAU)).collect();
    let mut dq_phase = vec![0usize; s.tones()];
    for (t, sym) in symbols.iter().enumerate() {
        let base = t * n_sym;
        for (sub, &v) in sym.iter().enumerate() {
            let (tone, extra) = match s.kind {
                Kind::Mfsk { .. } => (s.tone_index(sub, v, t), 0.0),
                Kind::Dqpsk { .. } => {
                    dq_phase[sub] = (dq_phase[sub] + v) % 4;
                    (sub, dq_phase[sub] as f64 * TAU / 4.0)
                }
            };
            let w = TAU * s.freq(tone) / fs;
            for i in 0..n_sym {
                let edge = i.min(n_sym - 1 - i);
                let g = if edge < ramp { 0.5 - 0.5 * (std::f64::consts::PI * edge as f64 / ramp as f64).cos() } else { 1.0 };
                out[base + i] += (g * (w * (base + i) as f64 + phases[tone] + extra).sin()) as f32;
            }
        }
    }
    out
}

/// Yhden sävelen kompleksinen DFT-kerroin ikkunassa.
fn bin(x: &[f32], freq: f64, fs: f64, t0: usize) -> (f64, f64) {
    let w = TAU * freq / fs;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    // Vaiheen eteneminen rekursiolla (ei sin/cos-kutsua joka näytteellä).
    let (cw, sw) = (w.cos(), w.sin());
    let (mut c, mut s) = ((w * t0 as f64).cos(), (w * t0 as f64).sin());
    for &v in x {
        re += v as f64 * c;
        im -= v as f64 * s;
        let nc = c * cw - s * sw;
        s = s * cw + c * sw;
        c = nc;
    }
    (re, im)
}

/// Palauttaa (bittivirheet, bitit).
pub fn demodulate(s: &Scheme, rx: &[f32], start: usize, fs: f64, symbols: &[Vec<usize>]) -> (u64, u64) {
    let n_sym = (s.t_sym() * fs).round() as usize;
    let n_win = (s.t_win * fs).round() as usize;
    let n_guard = n_sym - n_win;
    let (mut errors, mut bits) = (0u64, 0u64);
    let mut prev: Vec<(f64, f64)> = Vec::new();
    for (t, sym) in symbols.iter().enumerate() {
        let a = start + t * n_sym + n_guard;
        if a + n_win > rx.len() {
            break;
        }
        let win = &rx[a..a + n_win];
        match s.kind {
            Kind::Mfsk { m, .. } => {
                let bps = m.trailing_zeros() as u64;
                for (sub, &v) in sym.iter().enumerate() {
                    let best = (0..m)
                        .map(|c| {
                            let (re, im) = bin(win, s.freq(s.tone_index(sub, c, t)), fs, a);
                            (re * re + im * im, c)
                        })
                        .fold((f64::MIN, 0), |b, x| if x.0 > b.0 { x } else { b })
                        .1;
                    errors += (best ^ v).count_ones() as u64;
                    bits += bps;
                }
            }
            Kind::Dqpsk { n } => {
                let cur: Vec<(f64, f64)> = (0..n).map(|k| bin(win, s.freq(k), fs, a)).collect();
                if !prev.is_empty() {
                    for (k, &v) in sym.iter().enumerate() {
                        // cur · conj(prev): vaihe-ero edelliseen symboliin.
                        let re = cur[k].0 * prev[k].0 + cur[k].1 * prev[k].1;
                        let im = cur[k].1 * prev[k].0 - cur[k].0 * prev[k].1;
                        let q = ((im.atan2(re) / (TAU / 4.0)).round() as i64).rem_euclid(4) as usize;
                        // Vaihesiirto q vastaa bittejä Gray-koodattuna: viereinen vaihe = 1 bittivirhe.
                        let gray = |x: usize| x ^ (x >> 1);
                        errors += (gray(q) ^ gray(v)).count_ones() as u64;
                        bits += 2;
                    }
                }
                prev = cur;
            }
        }
    }
    (errors, bits)
}

/// Suoran äänen saapumishetki `run_oneway`:n tuottamassa signaalissa.
fn direct_arrival(cfg: &OneWay) -> usize {
    let lead = (cfg.lead_s * cfg.fs_tx) as usize;
    if cfg.ir.len() <= 1 {
        return lead;
    }
    let peak = cfg.ir.iter().enumerate().fold((0, 0.0f32), |b, (i, &v)| if v.abs() > b.1 { (i, v.abs()) } else { b }).0;
    lead + modem_core::dsp::conv::Convolver::BLOCK + peak
}

pub fn schemes() -> Vec<Scheme> {
    use Kind::*;
    use Layout::*;
    let s = |name, kind, f0, t_win, t_guard| Scheme { name, kind, f0, t_win, t_guard };
    vec![
        s("2-FSK 10 ms (kuten v0)", Mfsk { m: 2, k: 1, g: 1, layout: Block }, 2000.0, 0.010, 0.0),
        s("16-FSK 40+10 ms", Mfsk { m: 16, k: 1, g: 1, layout: Block }, 2000.0, 0.040, 0.010),
        s("16-FSK 40+10 ms, hyppy G=4", Mfsk { m: 16, k: 1, g: 4, layout: Interleaved }, 2000.0, 0.040, 0.010),
        s("16x 4-FSK 40+10, ei hyppyä", Mfsk { m: 4, k: 16, g: 1, layout: Block }, 1000.0, 0.040, 0.010),
        s("16x 4-FSK 40+10, G=2 lomitettu", Mfsk { m: 4, k: 16, g: 2, layout: Interleaved }, 1000.0, 0.040, 0.010),
        s("16x 4-FSK 40+10, G=4 lomitettu", Mfsk { m: 4, k: 16, g: 4, layout: Interleaved }, 1000.0, 0.040, 0.010),
        s("16x 4-FSK 40+10, G=4 lohko", Mfsk { m: 4, k: 16, g: 4, layout: Block }, 1000.0, 0.040, 0.010),
        s("16x 4-FSK 40+10, G=4 kaista", Mfsk { m: 4, k: 16, g: 4, layout: Band }, 1000.0, 0.040, 0.010),
        s("8x 4-FSK 20+5, G=4 lomitettu", Mfsk { m: 4, k: 8, g: 4, layout: Interleaved }, 1000.0, 0.020, 0.005),
        s("8x 4-FSK 20+5, G=8 lomitettu", Mfsk { m: 4, k: 8, g: 8, layout: Interleaved }, 1000.0, 0.020, 0.005),
        s("8x 16-FSK 40+10, G=2 lomit.", Mfsk { m: 16, k: 8, g: 2, layout: Interleaved }, 1000.0, 0.040, 0.010),
        s("8x 4-FSK 40+10, G=4 lomitettu", Mfsk { m: 4, k: 8, g: 4, layout: Interleaved }, 1500.0, 0.040, 0.010),
        s("32x 4-FSK 80+20, G=4 lomit.", Mfsk { m: 4, k: 32, g: 4, layout: Interleaved }, 1000.0, 0.080, 0.020),
        s("DQPSK 64 säveltä 40+10", Dqpsk { n: 64 }, 2000.0, 0.040, 0.010),
        s("DQPSK 256 säveltä 40+10", Dqpsk { n: 256 }, 1000.0, 0.040, 0.010),
        s("DQPSK 128 säveltä 20+10", Dqpsk { n: 128 }, 1000.0, 0.020, 0.010),
        s("DQPSK 512 säveltä 80+20", Dqpsk { n: 512 }, 1000.0, 0.080, 0.020),
    ]
}

/// Binäärisen symmetrisen kanavan kapasiteetti: osuus bittinopeudesta, joka
/// ihanteellisella virheenkorjauksella jäisi hyötykäyttöön.
fn bsc_capacity(ber: f64) -> f64 {
    let p = ber.clamp(1e-12, 0.5);
    1.0 + p * p.log2() + (1.0 - p) * (1.0 - p).log2()
}

pub fn cmd_explore(raw: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(raw, &["clip"])?;
    let seeds: u64 = a.num("seeds", 6)?;
    let seconds: f64 = a.num("seconds", 6.0)?;
    let fs = 48000.0;
    let mut profiles = bench::synthetic_profiles();
    profiles.extend(bench::measured_profiles(false));
    // (profiili, SNR): kaikurajoitteiset tapaukset hyvällä SNR:llä ja kohinarajoitteiset huonolla.
    let mut cases: Vec<(String, f64)> = vec![("puhdas".into(), 0.0), ("huone".into(), 20.0), ("huone".into(), 0.0), ("kaikuisa".into(), 20.0), ("kaikuisa".into(), 5.0)];
    for p in &profiles {
        if p.name.starts_with("mitattu:") && p.name.contains("g100") {
            cases.push((p.name.clone(), 20.0));
        }
    }
    println!("MODULAATIOVERTAILU: raaka bittivirhesuhde (%) ihanteellisella tahdistuksella, {seeds} x {seconds} s");
    println!("SNR koko kaistalla 0..24 kHz, sama kokonaisteho kaikilla. 'netto' = bittinopeus x BSC-kapasiteetti (ihanteellinen koodaus).");
    print!("{:<32}{:>7}{:>7}{:>6}", "vaihtoehto", "bit/s", "kaista", "PAPR");
    for (name, snr) in &cases {
        print!("{:>14}", format!("{}@{snr:.0}", bench::short_name(name).replace(":kaukana", ":k").replace("-mac", "")));
    }
    println!("{:>10}", "netto min");
    for s in schemes() {
        let mut row = format!("{:<32}{:>7.0}{:>7.0}", s.name, s.bit_rate(), s.bandwidth());
        let mut papr_db = 0.0;
        let mut worst_net = f64::MAX;
        let mut cells = String::new();
        for (ci, (name, snr)) in cases.iter().enumerate() {
            let profile = profiles.iter().find(|p| &p.name == name).ok_or("profiili puuttuu")?;
            let (mut errors, mut bits) = (0u64, 0u64);
            for seed in 0..seeds {
                let mut rng = Rng::new(seed * 7919 + 13);
                let count = (seconds / s.t_sym()) as usize;
                let symbols = random_symbols(&s, count, &mut rng);
                let mut tx = modulate(&s, &symbols, fs, &mut rng);
                let peak = tx.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                let rms = modem_core::dsp::rms(&tx);
                papr_db = 20.0 * (peak / rms).log10();
                if a.has("clip") {
                    // Kaiuttimen rajallinen huipputaso: leikataan 6 dB tehollisarvon yläpuolelta.
                    let lim = rms * 2.0;
                    tx.iter_mut().for_each(|v| *v = v.clamp(-lim, lim));
                }
                let cfg = (profile.make)(*snr, seed + 100 * ci as u64);
                let rx = run_oneway(&tx, &cfg, seed + 1);
                let (e, b) = demodulate(&s, &rx, direct_arrival(&cfg), fs, &symbols);
                errors += e;
                bits += b;
            }
            let ber = errors as f64 / bits.max(1) as f64;
            worst_net = worst_net.min(s.bit_rate() * bsc_capacity(ber));
            cells += &format!("{:>14.2}", ber * 100.0);
        }
        row += &format!("{papr_db:>6.1}");
        println!("{row}{cells}{worst_net:>10.0}");
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemes_are_error_free_on_clean_channel() {
        let fs = 48000.0;
        for s in schemes() {
            let mut rng = Rng::new(5);
            let symbols = random_symbols(&s, 30, &mut rng);
            let tx = modulate(&s, &symbols, fs, &mut rng);
            let mut cfg = OneWay::clean(fs);
            cfg.snr_db = 40.0;
            let rx = run_oneway(&tx, &cfg, 2);
            let (e, b) = demodulate(&s, &rx, direct_arrival(&cfg), fs, &symbols);
            assert!(b > 0 && e == 0, "{}: {e}/{b}", s.name);
        }
    }

    #[test]
    fn tone_maps_are_bijective() {
        for s in schemes() {
            if let Kind::Mfsk { m, k, g, .. } = s.kind {
                let mut seen = std::collections::HashSet::new();
                for grp in 0..g {
                    for sub in 0..k {
                        for v in 0..m {
                            let t = s.tone_index(sub, v, grp);
                            assert!(t < s.tones() && seen.insert(t), "{}", s.name);
                        }
                    }
                }
            }
        }
    }
}
