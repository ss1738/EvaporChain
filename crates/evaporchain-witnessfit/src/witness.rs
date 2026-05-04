//! Device-signed witness attestation.
//!
//! The wearable signs a tuple — `(streak_id, epoch, payload_hash)` —
//! and the chain verifies the signature against the wearable's
//! registered public key. The chain doesn't see the actual workout
//! data; only the BLAKE3 commitment + epoch.
//!
//! V1 model: signature blob is opaque; verifier closure plugs in the
//! signature scheme (Apple-style App Attest / Whoop's HMAC / our own
//! BLS / ML-DSA). Same generic-over-signature pattern Singh-Posthuma
//! uses for its death certificate.

use evaporchain_types::Epoch;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::streak::StreakId;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WitnessError {
    #[error("payload_hash must not be all-zero")]
    EmptyPayloadHash,
    #[error("device signature did not verify")]
    BadSignature,
    #[error("attestation cites streak {actual:?} but expected {expected:?}")]
    WrongStreak {
        expected: StreakId,
        actual: StreakId,
    },
}

/// Device-signed attestation that the wearer did *something* at
/// `epoch`. Validators agree on the commitment; the off-chain verifier
/// agrees on the workout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessAttestation {
    pub streak_id: StreakId,
    pub epoch: Epoch,
    /// BLAKE3 hash over the device-attested workout payload (steps,
    /// heart-rate, GPS, whatever — the shape is up to the wearable).
    pub payload_hash: [u8; 32],
    /// Opaque signature blob. The verifier closure plugs in the
    /// scheme (App Attest / HMAC / BLS / ML-DSA).
    pub signature: Vec<u8>,
}

impl WitnessAttestation {
    pub fn new(
        streak_id: StreakId,
        epoch: Epoch,
        payload_hash: [u8; 32],
        signature: Vec<u8>,
    ) -> Result<Self, WitnessError> {
        if payload_hash == [0u8; 32] {
            return Err(WitnessError::EmptyPayloadHash);
        }
        Ok(Self {
            streak_id,
            epoch,
            payload_hash,
            signature,
        })
    }

    /// Canonical bytes the wearable signed. Stable across validators;
    /// includes a domain-separation tag.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32 + 8 + 32 + 24);
        bytes.extend_from_slice(&self.streak_id);
        bytes.extend_from_slice(&self.epoch.to_le_bytes());
        bytes.extend_from_slice(&self.payload_hash);
        bytes.extend_from_slice(b"witnessfit:check-in:v1");
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> StreakId {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    #[test]
    fn rejects_empty_payload_hash() {
        let err = WitnessAttestation::new(id(1), 100, [0; 32], vec![1; 8]).unwrap_err();
        assert_eq!(err, WitnessError::EmptyPayloadHash);
    }

    #[test]
    fn signing_bytes_include_domain_separation() {
        let a = WitnessAttestation::new(id(1), 100, [0xAB; 32], vec![1; 8]).unwrap();
        let s = a.signing_bytes();
        let dst = b"witnessfit:check-in:v1";
        assert!(s.windows(dst.len()).any(|w| w == dst));
    }

    #[test]
    fn signing_bytes_change_with_inputs() {
        let a = WitnessAttestation::new(id(1), 100, [0xAB; 32], vec![1; 8]).unwrap();
        let b = WitnessAttestation::new(id(2), 100, [0xAB; 32], vec![1; 8]).unwrap();
        assert_ne!(a.signing_bytes(), b.signing_bytes());
        let c = WitnessAttestation::new(id(1), 200, [0xAB; 32], vec![1; 8]).unwrap();
        assert_ne!(a.signing_bytes(), c.signing_bytes());
        let d = WitnessAttestation::new(id(1), 100, [0xCD; 32], vec![1; 8]).unwrap();
        assert_ne!(a.signing_bytes(), d.signing_bytes());
    }

    #[test]
    fn round_trip_serde() {
        let a = WitnessAttestation::new(id(7), 1234, [0x77; 32], vec![5; 16]).unwrap();
        let s = serde_json::to_string(&a).unwrap();
        let back: WitnessAttestation = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }
}
