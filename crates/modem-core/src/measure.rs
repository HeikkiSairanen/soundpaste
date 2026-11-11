//! Akustisen kanavan mittaus pyyhkäisyllä.
//!
//! Kaiuttimesta soitetaan eksponentiaalinen sinipyyhkäisy (taajuus kasvaa
//! tasaisesti oktaaveissa). Äänitteestä ratkaistaan huoneen ja laitteiden
//! yhteinen impulssivaste jakamalla äänitteen spektri pyyhkäisyn spektrillä
//! (dekonvoluutio). Pyyhkäisyä ennen äänitetty hiljaisuus on huoneen taustamelu.
//!
//! Impulssivasteesta ja melusta johdetaan luvut, joilla modulaatio valitaan:
//! taajuusvaste, jälkikaiunta-aika, viivehaje ja kaistakohtainen
//! signaali-kohinasuhde.

use crate::dsp::fft::{next_pow2, Fft};

#[derive(Clone, Copy, Debug)]
pub struct Sweep {
    pub f1: f64,
    pub f2: f64,
    pub duration_s: f64,
    pub amplitude: f32,
}

impl Default for Sweep {
    fn default() -> Self {
        Sweep { f1: 100.0, f2: 20000.0, duration_s: 3.0, amplitude: 0.3 }
    }
}

impl Sweep {
    pub fn generate(&self, fs: f64) -> Vec<f32> {
        let n = (self.duration_s * fs) as usize;
        let k = (self.f2 / self.f1).ln();
        let fade = (0.02 * fs) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / fs;
                let phase = std::f64::consts::TAU * self.f1 * self.duration_s / k * ((t * k / self.duration_s).exp() - 1.0);
                let edge = i.min(n - 1 - i);
                let g = if edge < fade { 0.5 - 0.5 * (std::f64::consts::PI * edge as f64 / fade as f64).cos() } else { 1.0 };
                (phase.sin() * g) as f32 * self.amplitude
            })
            .collect()
    }

    /// Aika, jonka verran k:nnen harmonisen särön "haamuvaste" edeltää varsinaista
    /// impulssivastetta dekonvoluution jälkeen.
    pub fn harmonic_lead_s(&self, k: u32) -> f64 {
        self.duration_s * (k as f64).ln() / (self.f2 / self.f1).ln()
    }
}

/// Koko mittaussignaali: hiljaisuus (taustamelu), pyyhkäisy, hiljaisuus (kaiun häntä).
pub fn test_signal(sweep: &Sweep, fs: f64) -> Vec<f32> {
    let mut out = vec![0.0; (1.0 * fs) as usize];
    out.extend(sweep.generate(fs));
    out.extend(vec![0.0; (1.5 * fs) as usize]);
    out
}

/// Dekonvoluutio taajuustasossa: h = IFFT( Y·conj(X) / (|X|² + ε) ).
/// Säännöllistys ε estää kohinan räjähtämisen taajuuksilla, joilla pyyhkäisyssä
/// ei ole tehoa.
pub fn deconvolve(rec: &[f32], reference: &[f32]) -> Vec<f32> {
    let n = next_pow2(rec.len() + reference.len());
    let fft = Fft::new(n);
    let mut yr = vec![0.0f32; n];
    let mut yi = vec![0.0f32; n];
    let mut xr = vec![0.0f32; n];
    let mut xi = vec![0.0f32; n];
    yr[..rec.len()].copy_from_slice(rec);
    xr[..reference.len()].copy_from_slice(reference);
    fft.forward(&mut yr, &mut yi);
    fft.forward(&mut xr, &mut xi);
    let max_p = xr.iter().zip(&xi).map(|(a, b)| a * a + b * b).fold(0.0f32, f32::max);
    let eps = max_p * 1e-4;
    for k in 0..n {
        let p = xr[k] * xr[k] + xi[k] * xi[k] + eps;
        let (a, b) = (yr[k], yi[k]);
        yr[k] = (a * xr[k] + b * xi[k]) / p;
        yi[k] = (b * xr[k] - a * xi[k]) / p;
    }
    fft.inverse(&mut yr, &mut yi);
    yr
}

#[derive(Clone, Debug)]
pub struct Band {
    pub center_hz: f64,
    /// Kanavan vahvistus (kaiutin + huone + mikrofoni), dB.
    pub gain_db: f64,
    /// Taustamelun teho 50 Hz:n kaistalla, dBFS.
    pub noise_db: f64,
    /// Yhden sävelen SNR 50 Hz:n kaistalla, jos sävel soitetaan pyyhkäisyn amplitudilla.
    pub tone_snr_db: f64,
}

#[derive(Clone, Debug)]
pub struct ChannelMeasurement {
    pub fs: f64,
    /// Pyyhkäisyn alun paikka äänitteessä (näytteinä).
    pub delay_samples: usize,
    /// Impulssivaste, suora ääni kohdassa `ir_pre`.
    pub ir: Vec<f32>,
    pub ir_pre: usize,
    /// Taustamelu ennen pyyhkäisyä.
    pub noise: Vec<f32>,
    pub noise_rms: f32,
    /// Äänitetyn pyyhkäisyn tehollisarvo.
    pub signal_rms: f32,
    pub clipped: f32,
    /// Jälkikaiunta-aika (T20:stä ekstrapoloitu), jos vaimeneminen oli mitattavissa.
    pub rt60_s: Option<f64>,
    /// Suoran äänen (±2,5 ms) ja muun energian suhde.
    pub drr_db: f64,
    /// Tehollinen viivehaje (RMS delay spread).
    pub delay_spread_s: f64,
    /// Toisen harmonisen särön taso suhteessa perusvasteeseen.
    pub thd2_db: f64,
    /// Kuinka monta desibeliä impulssivasteen huippu on dekonvoluution kohinapohjan yläpuolella.
    pub ir_dynamic_db: f64,
    pub bands: Vec<Band>,
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|&v| (v as f64) * (v as f64)).sum()
}

/// Tehospektrin tiheys Welchin menetelmällä (Hann-ikkuna, 50 % limitys).
/// Palauttaa tehon per Hz taajuuksille k·fs/n, k = 0..n/2.
pub fn welch_psd(x: &[f32], fs: f64, n: usize) -> Vec<f64> {
    let fft = Fft::new(n);
    let win: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
    let wsum: f64 = win.iter().map(|&w| (w as f64) * (w as f64)).sum();
    let mut acc = vec![0.0f64; n / 2 + 1];
    let mut count = 0;
    let mut start = 0;
    let mut re = vec![0.0f32; n];
    let mut im = vec![0.0f32; n];
    while start + n <= x.len() {
        for i in 0..n {
            re[i] = x[start + i] * win[i];
            im[i] = 0.0;
        }
        fft.forward(&mut re, &mut im);
        for k in 0..=n / 2 {
            acc[k] += (re[k] as f64).powi(2) + (im[k] as f64).powi(2);
        }
        count += 1;
        start += n / 2;
    }
    let scale = 2.0 / (fs * wsum * count.max(1) as f64);
    acc.iter().map(|&p| p * scale).collect()
}

/// Kolmannesoktaavikaistojen keskitaajuudet välillä 200 Hz – 20 kHz.
pub fn third_octave_centers() -> Vec<f64> {
    (-7..=13).map(|i| 1000.0 * 2f64.powf(i as f64 / 3.0)).collect()
}

pub fn analyze(rec: &[f32], fs: f64, sweep: &Sweep) -> Result<ChannelMeasurement, String> {
    let reference = sweep.generate(fs);
    if rec.len() < reference.len() {
        return Err("äänite on lyhyempi kuin pyyhkäisy".into());
    }
    let h = deconvolve(rec, &reference);
    // Suora ääni on dekonvoluution suurin huippu; sen paikka on pyyhkäisyn alku äänitteessä.
    let search = rec.len().min(h.len());
    let (peak, peak_val) = h[..search].iter().enumerate().fold((0, 0.0f32), |b, (i, &v)| if v.abs() > b.1 { (i, v.abs()) } else { b });
    if peak_val <= 0.0 {
        return Err("äänite on hiljainen".into());
    }
    let ms = |t: f64| (t * 1e-3 * fs) as usize;

    // Dekonvoluution kohinapohja alueelta, jossa ei ole vastetta eikä särön haamuja.
    let lead2 = (sweep.harmonic_lead_s(2) * fs) as usize;
    let floor_lo = peak.saturating_sub(lead2.saturating_sub(ms(60.0)).max(ms(100.0)));
    let floor_hi = peak.saturating_sub(ms(50.0));
    let floor_power = if floor_hi > floor_lo + ms(10.0) { energy(&h[floor_lo..floor_hi]) / (floor_hi - floor_lo) as f64 } else { 1e-20 };

    // Impulssivaste: 2 ms ennen huippua, loppu siihen kohtaan, jossa 10 ms:n
    // ikkunan teho painuu kohinapohjaan (enintään 1,2 s).
    let pre = ms(2.0).min(peak);
    let win = ms(10.0).max(1);
    let max_len = ((1.2 * fs) as usize).min(h.len() - peak);
    let mut len = max_len;
    let mut i = ms(20.0);
    while i + win <= max_len {
        if energy(&h[peak + i..peak + i + win]) / (win as f64) < 2.0 * floor_power {
            len = i;
            break;
        }
        i += win;
    }
    let ir: Vec<f32> = h[peak - pre..peak + len].to_vec();

    // Suoran äänen osuus ja viivehaje.
    let direct_hi = (pre + ms(2.5)).min(ir.len());
    let e_direct = energy(&ir[..direct_hi]);
    let e_total = energy(&ir);
    let drr_db = 10.0 * (e_direct / (e_total - e_direct).max(1e-30)).log10();
    let (mut m1, mut m2) = (0.0f64, 0.0f64);
    for (n, &v) in ir.iter().enumerate() {
        let t = (n as f64 - pre as f64) / fs;
        let p = (v as f64) * (v as f64);
        m1 += t * p;
        m2 += t * t * p;
    }
    let mean = m1 / e_total;
    let delay_spread_s = (m2 / e_total - mean * mean).max(0.0).sqrt();

    // RT60 Schroederin takaperin integroidusta energiakäyrästä: sovitetaan suora
    // välille -5..-25 dB ja ekstrapoloidaan 60 dB:iin.
    let mut edc = vec![0.0f64; ir.len() + 1];
    for n in (0..ir.len()).rev() {
        edc[n] = edc[n + 1] + (ir[n] as f64) * (ir[n] as f64);
    }
    let db = |n: usize| 10.0 * (edc[n] / edc[0]).max(1e-30).log10();
    let t5 = (0..ir.len()).find(|&n| db(n) <= -5.0);
    let t25 = (0..ir.len()).find(|&n| db(n) <= -25.0);
    let rt60_s = match (t5, t25) {
        (Some(a), Some(b)) if b > a => Some((b - a) as f64 / fs * 3.0),
        _ => None,
    };

    // Toinen harmoninen: haamuvaste lead2 näytettä ennen huippua.
    let thd2_db = if peak > lead2 + ms(10.0) {
        let c = peak - lead2;
        let e2 = energy(&h[c - ms(5.0)..c + ms(15.0)]) - floor_power * ms(20.0) as f64;
        10.0 * (e2.max(1e-30) / energy(&h[peak - pre..peak + ms(15.0)])).log10()
    } else {
        f64::NAN
    };

    // Taustamelu: äänitteen alku ennen pyyhkäisyä (0,15 s alusta ohitetaan, koska
    // äänityksen käynnistyminen voi naksahtaa).
    let noise_lo = ms(150.0).min(peak);
    let noise_hi = peak.saturating_sub(ms(50.0)).max(noise_lo);
    let noise: Vec<f32> = rec[noise_lo..noise_hi].to_vec();
    let noise_rms = crate::dsp::rms(&noise);
    let sweep_end = (peak + reference.len()).min(rec.len());
    let sig = &rec[peak..sweep_end];
    let signal_rms = crate::dsp::rms(sig);
    let clipped = sig.iter().filter(|v| v.abs() >= 0.99).count() as f32 / sig.len().max(1) as f32;

    // Kaistakohtaiset luvut.
    let nfft = next_pow2(ir.len()).max(4096);
    let fft = Fft::new(nfft);
    let mut re = vec![0.0f32; nfft];
    let mut im = vec![0.0f32; nfft];
    re[..ir.len()].copy_from_slice(&ir);
    fft.forward(&mut re, &mut im);
    let psd_n = 4096;
    let psd = if noise.len() >= psd_n { welch_psd(&noise, fs, psd_n) } else { vec![0.0; psd_n / 2 + 1] };
    let mut bands = Vec::new();
    for fc in third_octave_centers() {
        let (lo, hi) = (fc * 2f64.powf(-1.0 / 6.0), fc * 2f64.powf(1.0 / 6.0));
        if hi >= fs / 2.0 {
            break;
        }
        let avg = |spec: &dyn Fn(usize) -> f64, n: usize| -> f64 {
            let k0 = ((lo * n as f64 / fs).floor() as usize).max(1);
            let k1 = ((hi * n as f64 / fs).ceil() as usize).min(n / 2).max(k0 + 1);
            (k0..k1).map(spec).sum::<f64>() / (k1 - k0) as f64
        };
        let gain = avg(&|k| (re[k] as f64).powi(2) + (im[k] as f64).powi(2), nfft);
        let noise_density = avg(&|k| psd[k], psd_n);
        let noise_50 = noise_density * 50.0;
        let tone = (sweep.amplitude as f64).powi(2) * gain / 2.0;
        bands.push(Band {
            center_hz: fc,
            gain_db: 10.0 * gain.max(1e-30).log10(),
            noise_db: 10.0 * noise_50.max(1e-30).log10(),
            tone_snr_db: 10.0 * (tone / noise_50.max(1e-30)).max(1e-30).log10(),
        });
    }

    Ok(ChannelMeasurement {
        fs,
        delay_samples: peak,
        ir,
        ir_pre: pre,
        noise,
        noise_rms,
        signal_rms,
        clipped,
        rt60_s,
        drr_db,
        delay_spread_s,
        thd2_db,
        ir_dynamic_db: 10.0 * ((peak_val as f64).powi(2) / floor_power.max(1e-30)).log10(),
        bands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{run_oneway, synth_ir, OneWay};

    #[test]
    fn recovers_known_channel() {
        let fs = 48000.0;
        let sweep = Sweep::default();
        let tx = test_signal(&sweep, fs);
        let ir = synth_ir(fs, 0.3, 6.0, 5);
        let mut cfg = OneWay::clean(fs);
        cfg.ir = ir.clone();
        cfg.snr_db = 40.0;
        cfg.lead_s = 0.25;
        cfg.tail_s = 1.0;
        cfg.rx_level = 0.05;
        let rec = run_oneway(&tx, &cfg, 9);
        let m = analyze(&rec, fs, &sweep).unwrap();
        // Pyyhkäisy alkaa 0,25 s + 1,0 s kohdalla (+ konvoluution lohkoviive).
        let expect = (1.25 * fs) as i64 + crate::dsp::conv::Convolver::BLOCK as i64;
        assert!((m.delay_samples as i64 - expect).abs() < 30, "{} vs {expect}", m.delay_samples);
        let rt = m.rt60_s.unwrap();
        assert!((rt - 0.3).abs() < 0.08, "rt60 {rt}");
        assert!((m.drr_db - 6.0).abs() < 2.5, "drr {}", m.drr_db);
        // Impulssivasteen muoto vastaa alkuperäistä (korrelaatio alun 50 ms:lta).
        // Pyyhkäisy kattaa vain 100 Hz – 20 kHz, joten täyttä vastaavuutta ei tule.
        let n = 2400;
        let a = &m.ir[m.ir_pre..m.ir_pre + n];
        let b = &ir[..n];
        let dot: f64 = a.iter().zip(b).map(|(x, y)| (*x as f64) * (*y as f64)).sum();
        let corr = dot / (energy(a) * energy(b)).sqrt();
        assert!(corr > 0.85, "corr {corr}");
        // Vaste on tasainen, joten kaistojen vahvistukset ovat lähellä toisiaan.
        let g: Vec<f64> = m.bands.iter().filter(|b| b.center_hz > 400.0 && b.center_hz < 12000.0).map(|b| b.gain_db).collect();
        let spread = g.iter().cloned().fold(f64::MIN, f64::max) - g.iter().cloned().fold(f64::MAX, f64::min);
        assert!(spread < 8.0, "spread {spread}");
    }

    #[test]
    fn sweep_spans_requested_band() {
        let fs = 48000.0;
        let s = Sweep::default().generate(fs);
        let early = &s[4800..9600];
        let late = &s[s.len() - 9600..s.len() - 4800];
        let p = |x: &[f32], f: f64| crate::dsp::goertzel_power(x, f, fs);
        assert!(p(early, 150.0) > 10.0 * p(early, 5000.0));
        assert!(p(late, 14000.0) > 10.0 * p(late, 500.0));
    }
}
