//! Pieni deterministinen satunnaislukugeneraattori (xoshiro256**, siemennys SplitMix64:llä).
//!
//! Sama siemen tuottaa saman jonon kaikilla alustoilla, joten simulaatiot ja
//! mittausajot ovat toistettavia. Ei kelpaa kryptografiaan.

#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
    spare_gauss: Option<f32>,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut x = seed;
        let mut next = || {
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        Rng { s: [next(), next(), next(), next()], spare_gauss: None }
    }

    /// Johdettu, riippumaton generaattori (esim. jokaiselle simulaation osalle oma).
    pub fn fork(&mut self, tag: u64) -> Rng {
        Rng::new(self.next_u64() ^ tag.wrapping_mul(0xA24B_AED4_963E_E407))
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Tasajakauma välillä [0, 1).
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Tasajakauma välillä [lo, hi).
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.uniform()
    }

    /// Kokonaisluku välillä [0, n).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        (self.uniform() * n as f64) as u64 % n
    }

    /// Normaalijakauma N(0, 1), Box–Muller.
    pub fn gauss(&mut self) -> f32 {
        if let Some(g) = self.spare_gauss.take() {
            return g;
        }
        let u1 = (1.0 - self.uniform()).max(1e-300);
        let u2 = self.uniform();
        let r = (-2.0 * u1.ln()).sqrt();
        let th = std::f64::consts::TAU * u2;
        self.spare_gauss = Some((r * th.sin()) as f32);
        (r * th.cos()) as f32
    }

    pub fn fill_bytes(&mut self, out: &mut [u8]) {
        for b in out {
            *b = self.next_u64() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_roughly_gaussian() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        assert_eq!(a.next_u64(), b.next_u64());
        let n = 200_000;
        let (mut sum, mut sq) = (0.0f64, 0.0f64);
        for _ in 0..n {
            let g = a.gauss() as f64;
            sum += g;
            sq += g * g;
        }
        assert!((sum / n as f64).abs() < 0.02);
        assert!((sq / n as f64 - 1.0).abs() < 0.03);
    }
}
