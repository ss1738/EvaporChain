//! `Amendment` — the "I want to upgrade from version X to version Y"
//! bundle, with the proof artefact attached.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_epv::registry::VersionId;

use crate::proof::{AmendmentHash, LlsaProof, ProofError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Amendment {
    pub from_version: VersionId,
    pub to_version: VersionId,
    /// Opaque descriptor of the new step semantics — typically the
    /// hash of the canonical Coq spec text or the WASM module bytes.
    /// Bound into the amendment hash; the chain doesn't interpret it.
    pub step_new_descriptor: Vec<u8>,
    pub proof: LlsaProof,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AmendmentError {
    #[error("amendment proof error: {0}")]
    Proof(#[from] ProofError),
    #[error("amendment from_version {0} is not currently registered")]
    FromVersionAbsent(VersionId),
    #[error("amendment to_version {0} is already registered (collision)")]
    ToVersionExists(VersionId),
}

impl Amendment {
    /// Compute the canonical `AmendmentHash` for this amendment.
    /// Production proofs MUST bind to this hash via `bound_amendment_hash`.
    pub fn hash(&self) -> AmendmentHash {
        let mut h = blake3::Hasher::new();
        h.update(b"evaporchain-llsa-amendment");
        h.update(&self.from_version.to_le_bytes());
        h.update(&self.to_version.to_le_bytes());
        h.update(&(self.step_new_descriptor.len() as u64).to_le_bytes());
        h.update(&self.step_new_descriptor);
        *h.finalize().as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proof::LlsaProof;

    fn dummy_proof() -> LlsaProof {
        LlsaProof {
            coq_term_hash: [0u8; 32],
            target_invariant_id: [1u8; 32],
            bound_amendment_hash: [2u8; 32],
            proof_bytes: vec![],
        }
    }

    #[test]
    fn amendment_hash_is_deterministic() {
        let a = Amendment {
            from_version: 1,
            to_version: 2,
            step_new_descriptor: b"new-vm-impl-v2".to_vec(),
            proof: dummy_proof(),
        };
        assert_eq!(a.hash(), a.hash());
    }

    #[test]
    fn amendment_hash_differs_with_to_version() {
        let a1 = Amendment {
            from_version: 1,
            to_version: 2,
            step_new_descriptor: b"x".to_vec(),
            proof: dummy_proof(),
        };
        let a2 = Amendment {
            from_version: 1,
            to_version: 3,
            step_new_descriptor: b"x".to_vec(),
            proof: dummy_proof(),
        };
        assert_ne!(a1.hash(), a2.hash());
    }

    #[test]
    fn amendment_hash_differs_with_descriptor() {
        let a1 = Amendment {
            from_version: 1,
            to_version: 2,
            step_new_descriptor: b"v2-A".to_vec(),
            proof: dummy_proof(),
        };
        let a2 = Amendment {
            from_version: 1,
            to_version: 2,
            step_new_descriptor: b"v2-B".to_vec(),
            proof: dummy_proof(),
        };
        assert_ne!(a1.hash(), a2.hash());
    }
}
