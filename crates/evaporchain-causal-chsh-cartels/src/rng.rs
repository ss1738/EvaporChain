//! Validator-deterministic RNG via BLAKE3 XOF.
//!
//! Pure-integer; no `rand` crate, no system entropy. The same `seed`
//! produces the same byte stream on every validator.

use blake3::Hasher;

/// Streaming generator: BLAKE3-keyed-XOF starting from `seed`. We
/// expose a counter-driven API: `next_u8`, `next_u32`, `next_u64`,
/// and `next_in_range`.
pub struct Blake3Rng {
    hasher: Hasher,
    counter: u64,
    buf: [u8; 64],
    /// Index of the next unread byte in `buf`. When `cursor == 64`,
    /// we refill via `refill()`.
    cursor: usize,
}

impl Blake3Rng {
    /// Domain-separated constructor. Seed is hashed against the
    /// domain tag so independent callers cannot collide.
    pub fn new(domain: &[u8], seed: &[u8; 32]) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(domain);
        hasher.update(&[0u8]);
        hasher.update(seed);
        Self {
            hasher,
            counter: 0,
            buf: [0u8; 64],
            cursor: 64,
        }
    }

    fn refill(&mut self) {
        let mut h = self.hasher.clone();
        h.update(&self.counter.to_le_bytes());
        let mut xof = h.finalize_xof();
        xof.fill(&mut self.buf);
        self.counter = self.counter.wrapping_add(1);
        self.cursor = 0;
    }

    pub fn next_u8(&mut self) -> u8 {
        if self.cursor >= 64 {
            self.refill();
        }
        let b = self.buf[self.cursor];
        self.cursor += 1;
        b
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut bytes = [0u8; 4];
        for b in bytes.iter_mut() {
            *b = self.next_u8();
        }
        u32::from_le_bytes(bytes)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut bytes = [0u8; 8];
        for b in bytes.iter_mut() {
            *b = self.next_u8();
        }
        u64::from_le_bytes(bytes)
    }

    /// Uniform integer in [0, n). Uses rejection sampling against the
    /// largest multiple-of-n bound representable in u64 to avoid
    /// modulo bias.
    pub fn next_in_range(&mut self, n: u64) -> u64 {
        assert!(n > 0, "next_in_range requires n > 0");
        let bound = u64::MAX - (u64::MAX % n);
        loop {
            let v = self.next_u64();
            if v < bound {
                return v % n;
            }
        }
    }

    /// Bernoulli sample: returns true with probability `numerator/denominator`.
    /// Pure integer (no float arithmetic anywhere on the consensus path).
    pub fn bernoulli(&mut self, numerator: u64, denominator: u64) -> bool {
        assert!(denominator > 0, "bernoulli denominator must be > 0");
        assert!(
            numerator <= denominator,
            "bernoulli numerator must be <= denominator"
        );
        if numerator == 0 {
            return false;
        }
        if numerator == denominator {
            return true;
        }
        self.next_in_range(denominator) < numerator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let seed = [7u8; 32];
        let mut a = Blake3Rng::new(b"test", &seed);
        let mut b = Blake3Rng::new(b"test", &seed);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seed_different_stream() {
        let mut a = Blake3Rng::new(b"test", &[1u8; 32]);
        let mut b = Blake3Rng::new(b"test", &[2u8; 32]);
        let av: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let bv: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_ne!(av, bv);
    }

    #[test]
    fn different_domain_different_stream() {
        let seed = [3u8; 32];
        let mut a = Blake3Rng::new(b"alpha", &seed);
        let mut b = Blake3Rng::new(b"beta", &seed);
        let av: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let bv: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_ne!(av, bv);
    }

    #[test]
    fn next_in_range_respects_bound() {
        let mut r = Blake3Rng::new(b"test", &[42u8; 32]);
        for _ in 0..10_000 {
            let v = r.next_in_range(7);
            assert!(v < 7);
        }
    }

    #[test]
    fn bernoulli_zero_always_false() {
        let mut r = Blake3Rng::new(b"test", &[0u8; 32]);
        for _ in 0..100 {
            assert!(!r.bernoulli(0, 100));
        }
    }

    #[test]
    fn bernoulli_full_always_true() {
        let mut r = Blake3Rng::new(b"test", &[0u8; 32]);
        for _ in 0..100 {
            assert!(r.bernoulli(100, 100));
        }
    }

    #[test]
    fn bernoulli_half_is_balanced() {
        let mut r = Blake3Rng::new(b"test", &[123u8; 32]);
        let mut trues = 0u32;
        let n = 10_000;
        for _ in 0..n {
            if r.bernoulli(1, 2) {
                trues += 1;
            }
        }
        // Expect ~5000 ± a few hundred. Validator-deterministic so
        // we can assert a tight band.
        assert!(trues > 4_700 && trues < 5_300, "trues = {trues}");
    }
}
