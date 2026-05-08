//! Validator-set commitment.
//!
//! Computes the same `keccak256` digest that
//! `ValidatorSetRegistry._computeRoot` produces on-chain. Pre-image:
//!
//! ```text
//! DOMAIN_TAG_COMMIT (32) | epoch (8 BE) | count (4 BE)
//!                       | (pubkey (48) | stake (16 BE))*
//! ```
//!
//! The Solidity side hashes the *abi.encodePacked* concatenation of the
//! same fields. `abi.encodePacked` for these primitive types is just the
//! big-endian byte representation, so the byte pre-image is identical.
//!
//! Cross-side agreement is checked by `tests/valset_agreement.rs`
//! (Rust-generated digest equals the hardcoded Solidity expectation).

use crate::constants::{domain_tag, DOMAIN_TAG_COMMIT_TEXT, MAX_VALIDATORS};
use sha3::{Digest, Keccak256};

/// One validator entry. Pubkey is BLS12-381 G1 compressed (48 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validator {
    /// BLS12-381 G1 compressed point (48 bytes).
    pub pubkey: [u8; 48],
    /// Stake weight; total across the set must fit in `u128`.
    pub stake: u128,
}

/// Errors from valset commitment.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValsetError {
    /// Validator list was empty.
    #[error("valset is empty")]
    Empty,
    /// More than [`MAX_VALIDATORS`] supplied.
    #[error("validator count {0} exceeds MAX_VALIDATORS={1}")]
    TooMany(usize, usize),
    /// Sum of stakes exceeded `u128::MAX`.
    #[error("stake sum overflowed u128")]
    StakeOverflow,
}

/// Compute the valset commitment root for `(epoch, validators)`.
///
/// Returns `(root, total_stake)`. Order of `validators` is significant
/// (must match the order the Solidity-side caller will pass).
pub fn compute_root(epoch: u64, validators: &[Validator]) -> Result<([u8; 32], u128), ValsetError> {
    if validators.is_empty() {
        return Err(ValsetError::Empty);
    }
    if validators.len() > MAX_VALIDATORS {
        return Err(ValsetError::TooMany(validators.len(), MAX_VALIDATORS));
    }

    let mut h = Keccak256::new();
    h.update(domain_tag(DOMAIN_TAG_COMMIT_TEXT));
    h.update(epoch.to_be_bytes());
    h.update((validators.len() as u32).to_be_bytes());

    let mut total: u128 = 0;
    for v in validators {
        h.update(v.pubkey);
        h.update(v.stake.to_be_bytes());
        total = total.checked_add(v.stake).ok_or(ValsetError::StakeOverflow)?;
    }

    let out = h.finalize();
    let mut root = [0u8; 32];
    root.copy_from_slice(&out);
    Ok((root, total))
}

/// Sort validators by pubkey big-endian. The producer SHOULD call this
/// before submitting on-chain to keep a canonical order, though the
/// commitment scheme accepts any consistent order.
pub fn canonicalise(validators: &mut [Validator]) {
    validators.sort_by(|a, b| a.pubkey.cmp(&b.pubkey));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(seed: u8, stake: u128) -> Validator {
        Validator {
            pubkey: [seed; 48],
            stake,
        }
    }

    #[test]
    fn empty_rejected() {
        assert_eq!(compute_root(1, &[]), Err(ValsetError::Empty));
    }

    #[test]
    fn too_many_rejected() {
        let big: Vec<Validator> = (0..(MAX_VALIDATORS + 1))
            .map(|i| v((i & 0xFF) as u8, 1))
            .collect();
        match compute_root(1, &big) {
            Err(ValsetError::TooMany(n, _)) => assert_eq!(n, MAX_VALIDATORS + 1),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn deterministic() {
        let vs = vec![v(0x11, 100), v(0x22, 100)];
        let (a, _) = compute_root(1, &vs).unwrap();
        let (b, _) = compute_root(1, &vs).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn order_matters() {
        let a = vec![v(0x11, 100), v(0x22, 100)];
        let b = vec![v(0x22, 100), v(0x11, 100)];
        let (ra, _) = compute_root(1, &a).unwrap();
        let (rb, _) = compute_root(1, &b).unwrap();
        assert_ne!(ra, rb);
    }

    #[test]
    fn epoch_changes_root() {
        let vs = vec![v(0x11, 100)];
        let (r1, _) = compute_root(1, &vs).unwrap();
        let (r2, _) = compute_root(2, &vs).unwrap();
        assert_ne!(r1, r2);
    }

    #[test]
    fn total_stake_summed() {
        let vs = vec![v(0x11, 100), v(0x22, 200), v(0x33, 300)];
        let (_, total) = compute_root(1, &vs).unwrap();
        assert_eq!(total, 600);
    }

    #[test]
    fn stake_overflow_caught() {
        let vs = vec![
            Validator {
                pubkey: [1; 48],
                stake: u128::MAX,
            },
            Validator {
                pubkey: [2; 48],
                stake: 1,
            },
        ];
        assert_eq!(compute_root(1, &vs), Err(ValsetError::StakeOverflow));
    }

    #[test]
    fn canonicalise_sorts_by_pubkey() {
        let mut vs = vec![v(0x33, 1), v(0x11, 1), v(0x22, 1)];
        canonicalise(&mut vs);
        assert_eq!(vs[0].pubkey[0], 0x11);
        assert_eq!(vs[1].pubkey[0], 0x22);
        assert_eq!(vs[2].pubkey[0], 0x33);
    }

    /// The fixed test vector. The same input + expected hex is hardcoded
    /// in the Solidity test `ValsetAgreement.t.sol`. Both sides hash
    /// independently and compare against this constant — if Solidity ever
    /// changes the pre-image schema, this test fails AND the Solidity
    /// test fails together.
    /// The fixed test vector — same input + expected hex is hardcoded
    /// in `ValsetAgreement.t.sol`. If Solidity ever changes the pre-image
    /// schema, this test fails AND the Solidity test fails.
    #[test]
    fn cross_side_test_vector() {
        let vs = vec![
            v(0x11, 100),
            v(0x22, 200),
            v(0x33, 300),
            v(0x44, 400),
            v(0x55, 500),
        ];
        let (root, total) = compute_root(7, &vs).unwrap();
        assert_eq!(total, 1500);
        assert_eq!(
            hex::encode(root),
            // EXPECTED — see ValsetAgreement.t.sol for the byte-identical Solidity fixture.
            // To regenerate: replace, run cargo test, copy left value to both sides.
            "d9772b11c3a1277e03d3e44f3bee65806a0360c27ae1b98fab1ccb1ccc4a8a2b"
        );
    }
}
