//! Real Shamir secret-sharing over `GF(2^61 - 1)`.
//!
//! Implements `(k, n)`-threshold secret sharing per Shamir 1979:
//!
//! 1. **Deal:** Pick a random degree-`(k - 1)` polynomial `f(x)` with
//!    `f(0) = secret`. Compute `share_i = f(i)` for `i = 1..=n`.
//!    Distribute `(i, share_i)` to share-holder `i`.
//!
//! 2. **Reconstruct:** Given any `k` shares `(i_1, s_1), ..., (i_k,
//!    s_k)`, recover `f(0)` via Lagrange interpolation at `x = 0`.
//!
//! Any `k - 1` shares reveal NOTHING about the secret (information-
//! theoretic property of polynomial interpolation over a finite
//! field).
//!
//! ## Integration with energy survival
//!
//! The full HLTS protocol layers Shamir on top of the energy-decay
//! gate from [`crate::quorum`]:
//!
//! ```text
//! reconstruct = quorum_alive(shares, k, λ, current_epoch, threshold)
//!               && lagrange_at_zero(any_k_share_values)
//! ```
//!
//! The chain holds [`crate::Share`] (energy + observed_epoch + idx)
//! for the survival gate; share-holders hold [`SecretShare`] (idx +
//! field-element value) for actual reconstruction. They're linked
//! by `idx`.
//!
//! ## V1 caveats
//!
//! - **Field size 61 bits** — secrets ≤ 60 bits per share. For larger
//!   secrets, chunk + share each chunk. V2 over BLS12-381 lifts this
//!   to 254 bits per share.
//! - **Deal RNG is blake3-XOF over a seed** — deterministic for tests
//!   + reproducibility, NOT a CSPRNG. Production V2 uses
//!   `OsRng`/`getrandom`.
//! - **No share-validity proofs.** A malicious dealer can produce
//!   shares of garbage; share-holders can't tell. V2 adds Pedersen
//!   commitments to the polynomial coefficients so dealer cheating
//!   is detectable.
//! - **No ZK refresh attestations.** A share-holder cannot prove they
//!   refreshed without revealing the share. Tracked as the V2.5
//!   slice — uses Sigma protocols over the BLS12-381 field.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::field::Scalar;

/// A secret value that fits in one field element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Secret(pub Scalar);

impl Secret {
    /// Wrap a `u64` (must be < `PRIME`) as a secret.
    pub fn from_u64(v: u64) -> Self {
        Self(Scalar::from_u64(v))
    }

    pub fn to_u64(self) -> u64 {
        self.0.to_u64()
    }
}

/// One share's cryptographic payload — the `(x, y)` point on the
/// secret-defining polynomial. The chain-side energy bookkeeping
/// lives in [`crate::Share`] keyed by the same `idx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecretShare {
    /// 1-indexed share index. Index 0 is the secret itself; never
    /// distributed.
    pub idx: u32,
    /// `f(idx)` — the polynomial evaluated at this share's x-coord.
    pub value: Scalar,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HltsError {
    #[error("threshold k must be in 1..=n; got k={k}, n={n}")]
    BadThreshold { k: usize, n: usize },
    #[error("zero shares supplied — need at least k for reconstruction")]
    NoShares,
    #[error("insufficient shares: have {have}, need k={k}")]
    InsufficientShares { have: usize, k: usize },
    #[error("duplicate share index {idx} — Lagrange basis undefined")]
    DuplicateIndex { idx: u32 },
    #[error("share index 0 is reserved for the secret; got idx=0")]
    ZeroIndex,
}

/// Deterministic-RNG (blake3-XOF over a seed). NOT a CSPRNG — for
/// tests + reproducibility only. Production deal uses `OsRng`.
pub struct DealRng {
    reader: blake3::OutputReader,
}

impl DealRng {
    /// Construct a deal-RNG from a 32-byte seed. Two RNGs with the
    /// same seed produce the same coefficient stream.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(b"evaporchain-hlts-deal-rng-v1");
        h.update(&seed);
        Self {
            reader: h.finalize_xof(),
        }
    }

    /// Pull the next field element from the stream.
    pub fn next_scalar(&mut self) -> Scalar {
        // Read 8 bytes and reduce; rejection sampling not needed
        // because Mersenne-61 wraps cleanly under reduce().
        let mut buf = [0u8; 8];
        self.reader.fill(&mut buf);
        Scalar::from_u64(u64::from_le_bytes(buf))
    }
}

/// `(k, n)` Shamir deal. Returns `n` shares; any `k` of them
/// reconstruct the secret.
///
/// `k = n` requires every share for reconstruction (information-
/// theoretic security but no fault tolerance). `k = 1` is just
/// "everyone gets the secret" (no security).
pub fn deal(
    secret: Secret,
    n: usize,
    k: usize,
    rng: &mut DealRng,
) -> Result<Vec<SecretShare>, HltsError> {
    if k == 0 || k > n || n == 0 {
        return Err(HltsError::BadThreshold { k, n });
    }
    // Polynomial coefficients: a_0 = secret; a_1..a_(k-1) random.
    // f(x) = a_0 + a_1·x + a_2·x^2 + ... + a_(k-1)·x^(k-1)
    let mut coeffs = Vec::with_capacity(k);
    coeffs.push(secret.0);
    for _ in 1..k {
        coeffs.push(rng.next_scalar());
    }
    // Evaluate at x = 1..=n via Horner's method.
    let mut shares = Vec::with_capacity(n);
    for i in 1..=n {
        let x = Scalar::from_u64(i as u64);
        let mut acc = Scalar::ZERO;
        // Horner: f(x) = ((..(a_(k-1)·x + a_(k-2))·x + ...)·x + a_0
        for j in (0..k).rev() {
            acc = acc.mul(x).add(coeffs[j]);
        }
        shares.push(SecretShare {
            idx: i as u32,
            value: acc,
        });
    }
    Ok(shares)
}

/// Lagrange interpolation at `x = 0` from any `k` shares.
///
/// `f(0) = Σ_i  s_i · L_i(0)`  where  `L_i(0) = Π_{j≠i}  (-x_j) / (x_i - x_j)`.
///
/// Errors:
/// - `NoShares` — zero shares supplied.
/// - `InsufficientShares` — fewer than `k`.
/// - `DuplicateIndex` — two shares share an `idx` (Lagrange basis is
///   undefined; would divide by zero in the denominator).
/// - `ZeroIndex` — a share at `idx = 0` (reserved for the secret).
pub fn reconstruct(shares: &[SecretShare], k: usize) -> Result<Secret, HltsError> {
    if shares.is_empty() {
        return Err(HltsError::NoShares);
    }
    if shares.len() < k {
        return Err(HltsError::InsufficientShares {
            have: shares.len(),
            k,
        });
    }
    // Take the first k shares.
    let used = &shares[..k];
    // Validate: no zero index, no duplicates.
    for s in used {
        if s.idx == 0 {
            return Err(HltsError::ZeroIndex);
        }
    }
    for i in 0..used.len() {
        for j in (i + 1)..used.len() {
            if used[i].idx == used[j].idx {
                return Err(HltsError::DuplicateIndex { idx: used[i].idx });
            }
        }
    }
    // Compute the Lagrange interpolation at x = 0.
    //   Σ_i  s_i · Π_{j≠i}  (-x_j) / (x_i - x_j)
    let mut acc = Scalar::ZERO;
    for (i, share_i) in used.iter().enumerate() {
        let x_i = Scalar::from_u64(share_i.idx as u64);
        // Compute L_i(0) = Π_{j≠i} (-x_j) / (x_i - x_j).
        let mut numerator = Scalar::ONE;
        let mut denominator = Scalar::ONE;
        for (j, share_j) in used.iter().enumerate() {
            if i == j {
                continue;
            }
            let x_j = Scalar::from_u64(share_j.idx as u64);
            // numerator *= -x_j
            numerator = numerator.mul(x_j.neg());
            // denominator *= (x_i - x_j)
            denominator = denominator.mul(x_i.sub(x_j));
        }
        // L_i(0) = numerator / denominator
        let inv_denom = denominator
            .inv()
            .expect("denominator non-zero (duplicate-index check above)");
        let lagrange = numerator.mul(inv_denom);
        // Add s_i · L_i(0) to the accumulator.
        acc = acc.add(share_i.value.mul(lagrange));
    }
    Ok(Secret(acc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng_for(tag: &str) -> DealRng {
        let mut seed = [0u8; 32];
        let bytes = tag.as_bytes();
        seed[..bytes.len().min(32)].copy_from_slice(&bytes[..bytes.len().min(32)]);
        DealRng::from_seed(seed)
    }

    #[test]
    fn deal_returns_n_shares_with_correct_indices() {
        let mut rng = rng_for("test1");
        let shares = deal(Secret::from_u64(42), 5, 3, &mut rng).unwrap();
        assert_eq!(shares.len(), 5);
        for (i, s) in shares.iter().enumerate() {
            assert_eq!(s.idx as usize, i + 1);
        }
    }

    #[test]
    fn round_trip_recovers_secret_with_exactly_k() {
        let secret = Secret::from_u64(0xDEADBEEF);
        let mut rng = rng_for("rt-exact-k");
        let shares = deal(secret, 5, 3, &mut rng).unwrap();
        let recovered = reconstruct(&shares[..3], 3).unwrap();
        assert_eq!(recovered, secret);
    }

    #[test]
    fn round_trip_recovers_secret_with_more_than_k() {
        // Reconstruct uses the first k shares — with 5 supplied at k=3,
        // the first 3 are used.
        let secret = Secret::from_u64(0xCAFE_BABE);
        let mut rng = rng_for("rt-more");
        let shares = deal(secret, 5, 3, &mut rng).unwrap();
        let recovered = reconstruct(&shares, 3).unwrap();
        assert_eq!(recovered, secret);
    }

    #[test]
    fn round_trip_works_for_any_k_subset() {
        // Pick non-contiguous shares — index 1, 3, 5 — and reconstruct.
        let secret = Secret::from_u64(12345);
        let mut rng = rng_for("rt-subset");
        let all = deal(secret, 5, 3, &mut rng).unwrap();
        let subset = vec![all[0], all[2], all[4]];
        let recovered = reconstruct(&subset, 3).unwrap();
        assert_eq!(recovered, secret);
    }

    #[test]
    fn fewer_than_k_shares_rejects() {
        let mut rng = rng_for("insufficient");
        let shares = deal(Secret::from_u64(99), 5, 3, &mut rng).unwrap();
        let err = reconstruct(&shares[..2], 3).expect_err("k=3 needs 3 shares");
        match err {
            HltsError::InsufficientShares { have, k } => {
                assert_eq!(have, 2);
                assert_eq!(k, 3);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn k_minus_one_shares_reveal_nothing() {
        // Information-theoretic property: 2 shares of a 3-of-5 sharing
        // are consistent with EVERY possible secret value. So if we
        // attempt reconstruction with k=2 from a deal that used k=3,
        // we get SOME value — but it won't equal the original secret
        // (with overwhelming probability). This tests that the
        // protocol genuinely needs k shares.
        let secret = Secret::from_u64(0x1234);
        let mut rng = rng_for("k-1-leak");
        let shares = deal(secret, 5, 3, &mut rng).unwrap();
        // Reconstruct with k=2 — wrong threshold. The math runs; the
        // returned value is meaningless (it's a Lagrange interp of
        // degree 1 through 2 points on a degree-2 polynomial — they
        // don't match).
        let bogus = reconstruct(&shares[..2], 2).unwrap();
        assert_ne!(
            bogus, secret,
            "k=2 reconstruct of a k=3 sharing must NOT equal the secret"
        );
    }

    #[test]
    fn duplicate_index_rejects() {
        let dup = vec![
            SecretShare {
                idx: 1,
                value: Scalar::from_u64(10),
            },
            SecretShare {
                idx: 1,
                value: Scalar::from_u64(20),
            },
        ];
        let err = reconstruct(&dup, 2).expect_err("dup idx must reject");
        match err {
            HltsError::DuplicateIndex { idx } => assert_eq!(idx, 1),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn zero_index_rejects() {
        let bad = vec![
            SecretShare {
                idx: 0,
                value: Scalar::from_u64(10),
            },
            SecretShare {
                idx: 1,
                value: Scalar::from_u64(20),
            },
        ];
        let err = reconstruct(&bad, 2).expect_err("idx=0 reserved for secret");
        match err {
            HltsError::ZeroIndex => {}
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn deal_threshold_validation() {
        let mut rng = rng_for("validation");
        // k = 0 rejects.
        assert!(matches!(
            deal(Secret::from_u64(1), 5, 0, &mut rng),
            Err(HltsError::BadThreshold { .. })
        ));
        // k > n rejects.
        assert!(matches!(
            deal(Secret::from_u64(1), 3, 5, &mut rng),
            Err(HltsError::BadThreshold { .. })
        ));
        // n = 0 rejects.
        assert!(matches!(
            deal(Secret::from_u64(1), 0, 1, &mut rng),
            Err(HltsError::BadThreshold { .. })
        ));
    }

    #[test]
    fn deterministic_under_same_seed() {
        // Same seed → same shares. Useful for reproducibility +
        // testing.
        let s = Secret::from_u64(777);
        let mut a = rng_for("determinism");
        let mut b = rng_for("determinism");
        let sa = deal(s, 5, 3, &mut a).unwrap();
        let sb = deal(s, 5, 3, &mut b).unwrap();
        assert_eq!(sa, sb);
    }

    #[test]
    fn k_equals_n_works() {
        // n-of-n threshold — needs every share. Information-theoretic
        // security but zero fault tolerance.
        let secret = Secret::from_u64(99999);
        let mut rng = rng_for("n-of-n");
        let shares = deal(secret, 4, 4, &mut rng).unwrap();
        let recovered = reconstruct(&shares, 4).unwrap();
        assert_eq!(recovered, secret);
    }
}
