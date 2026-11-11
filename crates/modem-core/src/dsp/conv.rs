//! Virtaava konvoluutio pitkällä impulssivasteella (tasajakoinen osituskonvoluutio).
//!
//! Huoneen impulssivaste voi olla kymmeniä tuhansia näytteitä. Suora konvoluutio
//! maksaisi silloin kymmeniä tuhansia kertolaskuja per näyte. Tässä impulssivaste
//! jaetaan B näytteen osiin, jokainen osa muunnetaan taajuustasoon kerran, ja
//! tulosignaalin lohkojen spektreistä pidetään viivelinjaa. Jokaista B näytteen
//! lohkoa kohti tarvitaan yksi FFT, yksi käänteis-FFT ja P spektrikertolaskua.
//! Lähtöä tulee aina yhtä monta näytettä kuin tuloa, ja viive on tasan B näytettä
//! (lohkon puskurointi).

use super::fft::Fft;

pub struct Convolver {
    b: usize,
    fft: Fft,
    // Impulssivasteen osien spektrit.
    h_re: Vec<Vec<f32>>,
    h_im: Vec<Vec<f32>>,
    // Tulolohkojen spektrien viivelinja (rengaspuskuri).
    x_re: Vec<Vec<f32>>,
    x_im: Vec<Vec<f32>>,
    head: usize,
    prev: Vec<f32>,
    pending: Vec<f32>,
    // Valmis lähtö, alustettu B nollalla, jotta lähtöä riittää jokaiselle tulonäytteelle.
    ready: std::collections::VecDeque<f32>,
    // Työpuskurit.
    w_re: Vec<f32>,
    w_im: Vec<f32>,
    a_re: Vec<f32>,
    a_im: Vec<f32>,
    /// Pelkkä vahvistus, jos impulssivaste on yhden näytteen mittainen.
    trivial: Option<f32>,
}

impl Convolver {
    pub const BLOCK: usize = 1024;

    pub fn new(ir: &[f32]) -> Self {
        let b = Self::BLOCK;
        let fft = Fft::new(2 * b);
        let parts = ir.len().div_ceil(b).max(1);
        let mut h_re = Vec::with_capacity(parts);
        let mut h_im = Vec::with_capacity(parts);
        for p in 0..parts {
            let mut re = vec![0.0; 2 * b];
            let mut im = vec![0.0; 2 * b];
            let lo = p * b;
            let hi = ((p + 1) * b).min(ir.len());
            if lo < hi {
                re[..hi - lo].copy_from_slice(&ir[lo..hi]);
            }
            fft.forward(&mut re, &mut im);
            h_re.push(re);
            h_im.push(im);
        }
        Convolver {
            b,
            x_re: vec![vec![0.0; 2 * b]; parts],
            x_im: vec![vec![0.0; 2 * b]; parts],
            h_re,
            h_im,
            head: 0,
            prev: vec![0.0; b],
            pending: Vec::with_capacity(b),
            ready: std::iter::repeat(0.0).take(b).collect(),
            w_re: vec![0.0; 2 * b],
            w_im: vec![0.0; 2 * b],
            a_re: vec![0.0; 2 * b],
            a_im: vec![0.0; 2 * b],
            trivial: if ir.len() <= 1 { Some(ir.first().copied().unwrap_or(0.0)) } else { None },
            fft,
        }
    }

    /// Viive näytteinä, jonka lohkopuskurointi lisää.
    pub fn latency(&self) -> usize {
        if self.trivial.is_some() {
            0
        } else {
            self.b
        }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if let Some(g) = self.trivial {
            out.extend(input.iter().map(|&v| v * g));
            return;
        }
        let mut rest = input;
        while !rest.is_empty() {
            let take = (self.b - self.pending.len()).min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.pending.len() == self.b {
                self.block();
                self.pending.clear();
            }
        }
        out.extend(self.ready.drain(..input.len()));
    }

    fn block(&mut self) {
        let b = self.b;
        let parts = self.h_re.len();
        // Spektri kahdesta viimeisimmästä lohkosta [edellinen | nykyinen].
        self.w_re[..b].copy_from_slice(&self.prev);
        self.w_re[b..].copy_from_slice(&self.pending);
        self.w_im.iter_mut().for_each(|v| *v = 0.0);
        self.fft.forward(&mut self.w_re, &mut self.w_im);
        self.head = (self.head + parts - 1) % parts;
        self.x_re[self.head].copy_from_slice(&self.w_re);
        self.x_im[self.head].copy_from_slice(&self.w_im);
        self.prev.copy_from_slice(&self.pending);

        self.a_re.iter_mut().for_each(|v| *v = 0.0);
        self.a_im.iter_mut().for_each(|v| *v = 0.0);
        // Reaalisen signaalin spektri on konjugaattisymmetrinen, joten lasketaan vain
        // taajuudet 0..=B ja peilataan loput.
        let half = b + 1;
        for p in 0..parts {
            let xi = (self.head + p) % parts;
            let xr = &self.x_re[xi][..half];
            let xim = &self.x_im[xi][..half];
            let hr = &self.h_re[p][..half];
            let him = &self.h_im[p][..half];
            let ar = &mut self.a_re[..half];
            let ai = &mut self.a_im[..half];
            for k in 0..half {
                ar[k] += xr[k] * hr[k] - xim[k] * him[k];
                ai[k] += xr[k] * him[k] + xim[k] * hr[k];
            }
        }
        for k in 1..b {
            self.a_re[2 * b - k] = self.a_re[k];
            self.a_im[2 * b - k] = -self.a_im[k];
        }
        self.fft.inverse(&mut self.a_re, &mut self.a_im);
        // Kehäkonvoluution jälkipuolisko on laskostumaton.
        self.ready.extend(self.a_re[b..].iter().copied());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_direct_convolution() {
        let ir: Vec<f32> = (0..1700).map(|i| ((i * 31 % 17) as f32 - 8.0) / 8.0 * (-(i as f32) / 400.0).exp()).collect();
        let x: Vec<f32> = (0..6000).map(|i| ((i * 13 % 29) as f32 - 14.0) / 14.0).collect();
        let mut c = Convolver::new(&ir);
        let mut y = Vec::new();
        for chunk in x.chunks(333) {
            c.process(chunk, &mut y);
        }
        let lat = c.latency();
        // Verrataan suoraan konvoluutioon niiltä osin kuin lähtöä on valmiina.
        for n in (0..y.len() - lat).step_by(37) {
            let mut acc = 0.0f64;
            for (k, &h) in ir.iter().enumerate() {
                if n >= k {
                    acc += h as f64 * x[n - k] as f64;
                }
            }
            let got = y[n + lat] as f64;
            assert!((got - acc).abs() < 2e-3, "n={n}: {got} vs {acc}");
        }
    }
}
