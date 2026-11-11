//! Reed–Solomon error correction over the field GF(256).
//!
//! A codeword is `data | parity`, and p parity bytes can correct
//! e erroneous bytes and f erasures (a byte whose position is known to be
//! unreliable) when 2e + f <= p. The code is shortened: the codeword length
//! can be anything up to 255.
//!
//! Why Reed–Solomon: errors on the acoustic channel come in bursts (a thump,
//! a fading tone), and RS corrects whole bytes regardless of how many
//! bits in the byte are wrong.
//!
//! The implementation step by step:
//! 1. syndromes S_j = c(α^j): all zero if the codeword is intact
//! 2. Berlekamp–Massey finds the error locator polynomial Λ(x), whose roots
//!    give the error positions (erasures are given as the initial value)
//! 3. Chien search tries every position
//! 4. Forney's formula computes the error values
//!
//! The field: elements are bytes, addition is XOR and multiplication is
//! polynomial multiplication modulo x^8 + x^4 + x^3 + x^2 + 1 (0x11D).
//! Multiplication is implemented with log tables: a·b = exp(log a + log b).

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum RsError {
    /// There are more errors than the parity can correct.
    TooManyErrors,
}

struct Gf {
    exp: [u8; 512],
    log: [u8; 256],
}

impl Gf {
    fn new() -> Self {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut x: u16 = 1;
        for i in 0..255 {
            exp[i] = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x11D;
            }
        }
        for i in 255..512 {
            exp[i] = exp[i - 255];
        }
        Gf { exp, log }
    }
    #[inline]
    fn mul(&self, a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 {
            0
        } else {
            self.exp[self.log[a as usize] as usize + self.log[b as usize] as usize]
        }
    }
    #[inline]
    fn div(&self, a: u8, b: u8) -> u8 {
        debug_assert!(b != 0);
        if a == 0 {
            0
        } else {
            self.exp[self.log[a as usize] as usize + 255 - self.log[b as usize] as usize]
        }
    }
    /// α^n
    #[inline]
    fn pow_alpha(&self, n: usize) -> u8 {
        self.exp[n % 255]
    }
    #[inline]
    fn inv(&self, a: u8) -> u8 {
        self.exp[255 - self.log[a as usize] as usize]
    }
    /// Value of a polynomial at x; coefficients from the lowest degree up.
    fn eval(&self, poly: &[u8], x: u8) -> u8 {
        poly.iter().rev().fold(0, |acc, &c| self.mul(acc, x) ^ c)
    }
}

pub struct ReedSolomon {
    gf: Gf,
    parity: usize,
    /// Generator polynomial g(x) = Π (x - α^i), i = 0..p-1; coefficients from the highest degree.
    generator: Vec<u8>,
}

impl ReedSolomon {
    pub fn new(parity: usize) -> Self {
        assert!(parity >= 1 && parity < 255);
        let gf = Gf::new();
        let mut generator = vec![1u8];
        for i in 0..parity {
            // Multiply by (x - α^i); subtraction is the same as addition (XOR).
            let root = gf.pow_alpha(i);
            let mut next = vec![0u8; generator.len() + 1];
            for (j, &c) in generator.iter().enumerate() {
                next[j] ^= c;
                next[j + 1] ^= gf.mul(c, root);
            }
            generator = next;
        }
        ReedSolomon { gf, parity, generator }
    }

    pub fn parity_len(&self) -> usize {
        self.parity
    }

    /// Compute the parity bytes: (data(x)·x^p) mod g(x).
    pub fn encode(&self, data: &[u8]) -> Vec<u8> {
        assert!(data.len() + self.parity <= 255, "a codeword can be at most 255 bytes");
        let mut rem = vec![0u8; self.parity];
        for &d in data {
            let factor = d ^ rem[0];
            rem.rotate_left(1);
            rem[self.parity - 1] = 0;
            if factor != 0 {
                for (r, &g) in rem.iter_mut().zip(&self.generator[1..]) {
                    *r ^= self.gf.mul(g, factor);
                }
            }
        }
        rem
    }

    /// Correct a codeword (data + parity) in place. `erasures` is a list of
    /// indices of bytes known to be unreliable. Returns the number of
    /// bytes changed.
    pub fn decode(&self, codeword: &mut [u8], erasures: &[usize]) -> Result<usize, RsError> {
        let gf = &self.gf;
        let n = codeword.len();
        let p = self.parity;
        assert!(n <= 255 && n > p);
        if erasures.len() > p {
            return Err(RsError::TooManyErrors);
        }
        let syndromes = self.syndromes(codeword);
        if syndromes.iter().all(|&s| s == 0) {
            return Ok(0);
        }
        // The locator of position i (byte index) is X = α^(n-1-i).
        let locator_of = |i: usize| gf.pow_alpha(n - 1 - i);

        // Erasure locator polynomial Γ(x) = Π (1 - X_k·x).
        let mut lambda = vec![1u8];
        for &e in erasures {
            if e >= n {
                return Err(RsError::TooManyErrors);
            }
            let x = locator_of(e);
            let mut next = vec![0u8; lambda.len() + 1];
            for (j, &c) in lambda.iter().enumerate() {
                next[j] ^= c;
                next[j + 1] ^= gf.mul(c, x);
            }
            lambda = next;
        }

        // Berlekamp–Massey, with Γ as the initial value.
        let rho = erasures.len();
        let mut prev = lambda.clone();
        let mut l = rho;
        let mut m = 1usize;
        let mut b = 1u8;
        for r in rho..p {
            let mut delta = 0u8;
            for i in 0..lambda.len().min(r + 1) {
                delta ^= gf.mul(lambda[i], syndromes[r - i]);
            }
            if delta == 0 {
                m += 1;
                continue;
            }
            let scale = gf.div(delta, b);
            let mut next = lambda.clone();
            if next.len() < prev.len() + m {
                next.resize(prev.len() + m, 0);
            }
            for (i, &c) in prev.iter().enumerate() {
                next[i + m] ^= gf.mul(scale, c);
            }
            if 2 * l <= r + rho {
                l = r + 1 - l + rho;
                prev = lambda;
                b = delta;
                m = 1;
            } else {
                m += 1;
            }
            lambda = next;
        }
        while lambda.last() == Some(&0) {
            lambda.pop();
        }
        let degree = lambda.len() - 1;
        if degree == 0 || degree > p || 2 * (degree - rho) + rho > p {
            return Err(RsError::TooManyErrors);
        }

        // Chien search: position i is in error if Λ(X_i^-1) = 0.
        let mut positions = Vec::with_capacity(degree);
        for i in 0..n {
            if gf.eval(&lambda, gf.inv(locator_of(i))) == 0 {
                positions.push(i);
            }
        }
        if positions.len() != degree {
            return Err(RsError::TooManyErrors);
        }

        // Forney: Ω(x) = S(x)·Λ(x) mod x^p, error value e = X·Ω(X^-1) / Λ'(X^-1).
        let mut omega = vec![0u8; p];
        for (i, &s) in syndromes.iter().enumerate() {
            for (j, &c) in lambda.iter().enumerate() {
                if i + j < p {
                    omega[i + j] ^= gf.mul(s, c);
                }
            }
        }
        // Formal derivative: in a field of characteristic 2 the even terms vanish.
        let derivative: Vec<u8> = (1..lambda.len()).map(|j| if j % 2 == 1 { lambda[j] } else { 0 }).collect();
        let mut changed = 0;
        for &i in &positions {
            let x = locator_of(i);
            let x_inv = gf.inv(x);
            let denom = gf.eval(&derivative, x_inv);
            if denom == 0 {
                return Err(RsError::TooManyErrors);
            }
            let magnitude = gf.mul(x, gf.div(gf.eval(&omega, x_inv), denom));
            if magnitude != 0 {
                codeword[i] ^= magnitude;
                changed += 1;
            }
        }
        // Check: the syndromes of the corrected word must be zero.
        if self.syndromes(codeword).iter().any(|&s| s != 0) {
            return Err(RsError::TooManyErrors);
        }
        Ok(changed)
    }

    fn syndromes(&self, codeword: &[u8]) -> Vec<u8> {
        (0..self.parity).map(|j| codeword.iter().fold(0u8, |acc, &c| self.gf.mul(acc, self.gf.pow_alpha(j)) ^ c)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn field_axioms_hold() {
        let gf = Gf::new();
        for a in 1..=255u8 {
            assert_eq!(gf.mul(a, gf.inv(a)), 1);
            assert_eq!(gf.div(gf.mul(a, 0x53), 0x53), a);
        }
        // A known example: 0x53 · 0xCA = 0x01 in the AES field, but with our
        // polynomial (0x11D) α = 2 and α^8 = 0x1D.
        assert_eq!(gf.pow_alpha(8), 0x1D);
    }

    #[test]
    fn clean_codeword_needs_no_correction() {
        let rs = ReedSolomon::new(8);
        let data = b"soundpaste".to_vec();
        let mut cw = data.clone();
        cw.extend(rs.encode(&data));
        assert_eq!(rs.decode(&mut cw, &[]), Ok(0));
        assert_eq!(&cw[..data.len()], &data[..]);
    }

    #[test]
    fn corrects_up_to_capacity_with_errors_and_erasures() {
        let mut rng = Rng::new(42);
        for trial in 0..3000 {
            let parity = 2 + rng.below(40) as usize;
            let k = 1 + rng.below((255 - parity) as u64) as usize;
            let rs = ReedSolomon::new(parity);
            let mut data = vec![0u8; k];
            rng.fill_bytes(&mut data);
            let mut cw = data.clone();
            cw.extend(rs.encode(&data));
            let n = cw.len();
            // A random number of erasures and errors within capacity.
            let erasures_n = (rng.below(parity as u64 + 1) as usize).min(n);
            let errors_n = (rng.below(((parity - erasures_n) / 2) as u64 + 1) as usize).min(n - erasures_n);
            let mut idx: Vec<usize> = (0..n).collect();
            for i in 0..(erasures_n + errors_n) {
                let j = i + rng.below((n - i) as u64) as usize;
                idx.swap(i, j);
            }
            let erasures = &idx[..erasures_n];
            let original = cw.clone();
            for &i in &idx[..erasures_n + errors_n] {
                cw[i] ^= 1 + rng.below(255) as u8;
            }
            let res = rs.decode(&mut cw, erasures);
            assert!(res.is_ok(), "trial {trial}: p={parity} k={k} e={errors_n} f={erasures_n}: {res:?}");
            assert_eq!(cw, original, "trial {trial}");
        }
    }

    #[test]
    fn too_many_errors_are_reported_or_miscorrected_to_valid_codeword() {
        // When capacity is exceeded the decoder either reports an error or
        // (rarely) ends up at another valid codeword. That is why the frame also
        // has a CRC. Here we check that nothing crashes and that reporting
        // is the most common outcome.
        let mut rng = Rng::new(7);
        let rs = ReedSolomon::new(8);
        let mut reported = 0;
        for _ in 0..500 {
            let mut data = vec![0u8; 40];
            rng.fill_bytes(&mut data);
            let mut cw = data.clone();
            cw.extend(rs.encode(&data));
            for _ in 0..9 {
                let i = rng.below(cw.len() as u64) as usize;
                cw[i] ^= 1 + rng.below(255) as u8;
            }
            match rs.decode(&mut cw, &[]) {
                Err(RsError::TooManyErrors) => reported += 1,
                Ok(_) => assert!(rs.syndromes(&cw).iter().all(|&s| s == 0)),
            }
        }
        assert!(reported > 450, "{reported}");
    }

    #[test]
    fn full_length_and_all_erasures() {
        let rs = ReedSolomon::new(32);
        let data: Vec<u8> = (0..223).map(|i| (i * 3) as u8).collect();
        let mut cw = data.clone();
        cw.extend(rs.encode(&data));
        assert_eq!(cw.len(), 255);
        let erasures: Vec<usize> = (100..132).collect();
        for &i in &erasures {
            cw[i] = 0xEE;
        }
        assert!(rs.decode(&mut cw, &erasures).is_ok());
        assert_eq!(&cw[..223], &data[..]);
    }
}
