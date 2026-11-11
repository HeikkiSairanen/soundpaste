//! Radix-2 FFT (Cooley–Tukey, iteratiivinen, paikallaan).
//!
//! Reaali- ja imaginaariosat ovat erillisissä taulukoissa, mikä on yksinkertaisin
//! muoto ilman kompleksilukutyyppiä. Kiertotekijät ja bittikäännöstaulu lasketaan
//! kerran, joten samaa `Fft`-oliota kannattaa käyttää uudelleen.

pub struct Fft {
    n: usize,
    cos: Vec<f32>,
    sin: Vec<f32>,
    rev: Vec<u32>,
}

impl Fft {
    /// `n` on kahden potenssi.
    pub fn new(n: usize) -> Self {
        assert!(n.is_power_of_two() && n >= 2, "FFT:n koon pitää olla kahden potenssi");
        let bits = n.trailing_zeros();
        let half = n / 2;
        let mut cos = Vec::with_capacity(half);
        let mut sin = Vec::with_capacity(half);
        for k in 0..half {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            cos.push(a.cos() as f32);
            sin.push(a.sin() as f32);
        }
        let rev = (0..n as u32).map(|i| i.reverse_bits() >> (32 - bits)).collect();
        Fft { n, cos, sin, rev }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    /// X[k] = Σ x[n]·e^{-j2πkn/N}
    pub fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        self.transform(re, im, false);
    }

    /// x[n] = (1/N)·Σ X[k]·e^{+j2πkn/N}
    pub fn inverse(&self, re: &mut [f32], im: &mut [f32]) {
        self.transform(re, im, true);
        let s = 1.0 / self.n as f32;
        for v in re.iter_mut() {
            *v *= s;
        }
        for v in im.iter_mut() {
            *v *= s;
        }
    }

    fn transform(&self, re: &mut [f32], im: &mut [f32], inverse: bool) {
        let n = self.n;
        assert!(re.len() == n && im.len() == n);
        for i in 0..n {
            let j = self.rev[i] as usize;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let wr = self.cos[k * step];
                    let wi = if inverse { self.sin[k * step] } else { -self.sin[k * step] };
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }
}

pub fn next_pow2(n: usize) -> usize {
    n.max(2).next_power_of_two()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_naive_dft() {
        let n = 64;
        let x: Vec<f32> = (0..n).map(|i| ((i * 7 % 13) as f32 - 6.0) / 6.0).collect();
        let mut re = x.clone();
        let mut im = vec![0.0; n];
        Fft::new(n).forward(&mut re, &mut im);
        for k in 0..n {
            let (mut sr, mut si) = (0.0f64, 0.0f64);
            for (i, &v) in x.iter().enumerate() {
                let a = -std::f64::consts::TAU * (k * i) as f64 / n as f64;
                sr += v as f64 * a.cos();
                si += v as f64 * a.sin();
            }
            assert!((re[k] as f64 - sr).abs() < 1e-3 && (im[k] as f64 - si).abs() < 1e-3);
        }
    }

    #[test]
    fn inverse_restores_input() {
        let n = 1024;
        let x: Vec<f32> = (0..n).map(|i| (i as f32 * 0.37).sin() + (i as f32 * 0.011).cos()).collect();
        let f = Fft::new(n);
        let mut re = x.clone();
        let mut im = vec![0.0; n];
        f.forward(&mut re, &mut im);
        f.inverse(&mut re, &mut im);
        for i in 0..n {
            assert!((re[i] - x[i]).abs() < 1e-4 && im[i].abs() < 1e-4);
        }
    }
}
