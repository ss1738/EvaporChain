//! `verify` — chain-side check.
//!
//! The chain remembers the *historical* `key_material` for each
//! period (in production via the public key from a Merkle commitment;
//! substrate just takes it as input). A signature for period `t` is
//! valid iff its MAC matches `blake3(SIGN_TAG || key_material_t ||
//! t || message)`.

use thiserror::Error;

use crate::sign::Signature;

const SIGN_TAG: &[u8] = b"evaporchain-eg-fss-sign";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("signature MAC mismatch")]
    MacMismatch,
    #[error("signature claims period {claimed} but chain remembers {expected} for that index")]
    PeriodMismatch { claimed: u64, expected: u64 },
}

pub fn verify(
    period_key_material: [u8; 32],
    period_index: u64,
    message: &[u8],
    sig: &Signature,
) -> Result<(), VerifyError> {
    if sig.period_index != period_index {
        return Err(VerifyError::PeriodMismatch {
            claimed: sig.period_index,
            expected: period_index,
        });
    }
    let mut h = blake3::Hasher::new();
    h.update(SIGN_TAG);
    h.update(&period_key_material);
    h.update(&period_index.to_le_bytes());
    h.update(message);
    let expected_mac: [u8; 32] = *h.finalize().as_bytes();
    if expected_mac != sig.mac {
        return Err(VerifyError::MacMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::EgFssKey;
    use crate::sign::sign;

    #[test]
    fn sign_then_verify_round_trip() {
        let k = EgFssKey::from_seed([7u8; 32]);
        let s = sign(&k, b"hello");
        verify(k.key_material, k.period_index, b"hello", &s).unwrap();
    }

    #[test]
    fn wrong_message_rejected() {
        let k = EgFssKey::from_seed([7u8; 32]);
        let s = sign(&k, b"hello");
        let err = verify(k.key_material, k.period_index, b"world", &s).unwrap_err();
        assert_eq!(err, VerifyError::MacMismatch);
    }

    #[test]
    fn evolved_key_signs_in_new_period() {
        let k = EgFssKey::from_seed([7u8; 32]);
        let evolved = k.evolve(1000, 100).unwrap();
        assert_eq!(evolved.period_index, 10);
        let s = sign(&evolved, b"x");
        verify(evolved.key_material, evolved.period_index, b"x", &s).unwrap();
    }

    #[test]
    fn signature_from_old_key_doesnt_verify_with_new_key_material() {
        let k_old = EgFssKey::from_seed([7u8; 32]);
        let s = sign(&k_old, b"x");
        let k_new = k_old.clone().evolve(1000, 100).unwrap();
        // Verifying the OLD signature with the NEW key material fails:
        let err = verify(k_new.key_material, s.period_index, b"x", &s).unwrap_err();
        assert_eq!(err, VerifyError::MacMismatch);
        // But verifying it with the original (old) material still passes
        // — this is the *forward* security: stealing k_new doesn't let
        // an attacker forge a signature claiming to be from k_old's
        // period because they don't have k_old's material.
        verify(k_old.key_material, s.period_index, b"x", &s).unwrap();
    }
}
