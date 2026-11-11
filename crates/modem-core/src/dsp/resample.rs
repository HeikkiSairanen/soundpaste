//! Virtaava näytteenottotaajuuden muunnin (ikkunoitu sinc-interpolointi).
//!
//! Käytetään kanavasimulaattorissa mallintamaan sitä, että lähettäjän ja
//! vastaanottajan äänikortit pyörivät eri taajuudella (44,1 vs 48 kHz) ja
//! että kellot eivät ole täsmälleen nimellisarvossaan (ppm-virhe).

const TAPS: usize = 24; // interpolointisuotimen pituus näytteinä
const PHASES: usize = 512; // murto-osaviiveen tarkkuus

pub struct Resampler {
    /// Montako tulonäytettä yksi lähtönäyte vastaa.
    step: f64,
    /// Seuraavan lähtönäytteen paikka historiapuskurin koordinaateissa.
    pos: f64,
    hist: Vec<f32>,
    table: Vec<f32>, // PHASES+1 riviä, TAPS saraketta
    bypass: bool,
}

impl Resampler {
    /// `fs_in`: tulon todellinen taajuus, `fs_out`: lähdön todellinen taajuus.
    pub fn new(fs_in: f64, fs_out: f64) -> Self {
        let step = fs_in / fs_out;
        // Alaspäin muunnettaessa rajataajuutta lasketaan laskostumisen estämiseksi.
        let cutoff = (1.0 / step).min(1.0) * 0.94;
        let mut table = Vec::with_capacity((PHASES + 1) * TAPS);
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            let mut row = [0.0f64; TAPS];
            let mut sum = 0.0;
            for (k, r) in row.iter_mut().enumerate() {
                // Näytteen k etäisyys interpolointipisteestä.
                let d = k as f64 - (TAPS as f64 / 2.0 - 1.0) - frac;
                let s = if d.abs() < 1e-12 { 1.0 } else { (std::f64::consts::PI * d * cutoff).sin() / (std::f64::consts::PI * d * cutoff) };
                // Hann-ikkuna koko suotimen leveydelle.
                let w = 0.5 + 0.5 * (std::f64::consts::PI * d / (TAPS as f64 / 2.0)).cos();
                *r = s * w.max(0.0);
                sum += *r;
            }
            for r in row {
                table.push((r / sum) as f32);
            }
        }
        Resampler { step, pos: 0.0, hist: vec![0.0; TAPS], table, bypass: (step - 1.0).abs() < 1e-12 }
    }

    /// Syötä lohko ja lisää valmistuneet lähtönäytteet `out`-vektoriin.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.bypass {
            out.extend_from_slice(input);
            return;
        }
        self.hist.extend_from_slice(input);
        // Lähtönäyte kohdassa pos tarvitsee näytteet [floor(pos), floor(pos)+TAPS).
        loop {
            let i = self.pos.floor() as usize;
            if i + TAPS > self.hist.len() {
                break;
            }
            let frac = self.pos - i as f64;
            let fp = frac * PHASES as f64;
            let p0 = fp.floor() as usize;
            let a = (fp - p0 as f64) as f32;
            let r0 = &self.table[p0 * TAPS..(p0 + 1) * TAPS];
            let r1 = &self.table[(p0 + 1) * TAPS..(p0 + 2) * TAPS];
            let x = &self.hist[i..i + TAPS];
            let mut acc = 0.0f32;
            for k in 0..TAPS {
                acc += x[k] * (r0[k] + a * (r1[k] - r0[k]));
            }
            out.push(acc);
            self.pos += self.step;
        }
        // Pudotetaan käytetty historia.
        let keep_from = (self.pos.floor() as usize).min(self.hist.len());
        if keep_from > 0 {
            self.hist.drain(0..keep_from);
            self.pos -= keep_from as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::goertzel_power;

    #[test]
    fn tone_survives_44k1_to_48k() {
        let (fi, fo) = (44100.0, 48000.0);
        let x: Vec<f32> = (0..44100).map(|i| (std::f64::consts::TAU * 3000.0 * i as f64 / fi).sin() as f32).collect();
        let mut r = Resampler::new(fi, fo);
        let mut y = Vec::new();
        for chunk in x.chunks(317) {
            r.process(chunk, &mut y);
        }
        assert!((y.len() as i64 - 48000).abs() < 40, "{}", y.len());
        let mid = &y[10000..10000 + 4800];
        let p = goertzel_power(mid, 3000.0, fo);
        assert!((p - 0.25).abs() < 0.01, "{p}");
        // Viereisellä taajuudella ei saa olla merkittävää tehoa.
        assert!(goertzel_power(mid, 3500.0, fo) < 1e-4);
    }

    #[test]
    fn bypass_is_exact() {
        let x: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let mut y = Vec::new();
        Resampler::new(48000.0, 48000.0).process(&x, &mut y);
        assert_eq!(x, y);
    }
}
