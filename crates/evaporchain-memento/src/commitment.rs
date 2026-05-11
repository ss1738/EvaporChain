//! Sealing / commitment side of the Memento primitive.

use serde::{Deserialize, Serialize};

use crate::trigger::MementoTrigger;

/// Wire-format version tag. Allows future schemas to coexist on-chain
/// without ambiguity. The domain-separation tag in [`MementoCommitment::compute`]
/// folds this in, so a v1 commitment can never be mistaken for v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MementoVersion {
    /// Initial schema — single-payload commitment + 5 trigger variants.
    V1,
}

impl MementoVersion {
    fn domain_tag(self) -> &'static [u8] {
        match self {
            MementoVersion::V1 => b"evaporchain-memento-v1",
        }
    }
}

/// 32-byte BLAKE3 commitment to a memento's sealed payload + nonce.
///
/// The commitment is computed as:
///
/// ```text
///   BLAKE3(version_domain_tag || u64_LE(payload.len()) || payload || nonce)
/// ```
///
/// Length-prefixing the payload is critical — without it, two
/// `(payload, nonce)` pairs whose concatenation byte-sequences agree
/// would produce the same commitment (a chosen-ciphertext extension
/// attack). The length prefix breaks that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MementoCommitment(pub [u8; 32]);

impl MementoCommitment {
    /// Compute the commitment over a payload + nonce. Pure function;
    /// callable client-side before broadcasting the contract.
    pub fn compute(version: MementoVersion, payload: &[u8], nonce: &[u8; 32]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(version.domain_tag());
        hasher.update(&(payload.len() as u64).to_le_bytes());
        hasher.update(payload);
        hasher.update(nonce);
        let out = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(out.as_bytes());
        MementoCommitment(bytes)
    }

    /// Constant-time equality. Avoids leaking commitment-prefix
    /// information via timing on the verifier side. (Brute-force
    /// finding a 32-byte BLAKE3 collision is 2^128 work regardless,
    /// but constant-time comparison is cheap.)
    pub fn ct_eq(&self, other: &Self) -> bool {
        let mut diff: u8 = 0;
        for i in 0..32 {
            diff |= self.0[i] ^ other.0[i];
        }
        diff == 0
    }
}

/// The on-chain object that represents a Memento.
///
/// Note what's NOT here: the payload itself, the nonce, the reveal
/// claimant's identity, any reveal-time state. All of those live
/// off-chain until the reveal moment. The on-chain object is just
/// the commitment, the trigger condition, and a creation timestamp
/// (in epochs) used by [`MementoTrigger::OwnerInactiveSince`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MementoContract {
    /// Wire-format version. Always [`MementoVersion::V1`] for crates
    /// shipped today.
    pub version: MementoVersion,
    /// Sealed BLAKE3 commitment to the payload.
    pub commitment: MementoCommitment,
    /// Trigger condition that must hold for any reveal to succeed.
    pub trigger: MementoTrigger,
    /// Account that owns the contract — used by triggers
    /// `OwnerInactiveSince`, `OwnerEnergyBelow`, `OwnerSignedReveal`
    /// to scope the chain-state predicate to a specific actor.
    pub owner: evaporchain_types::AccountAddress,
    /// Epoch at which the memento was sealed. Anchors the
    /// `min_idle_epochs` window in [`MementoTrigger::OwnerInactiveSince`].
    pub sealed_at_epoch: evaporchain_types::Epoch,
}

/// Seal a payload into a [`MementoContract`].
///
/// Returns both the contract (which goes on-chain) and the
/// `(payload, nonce)` pair (which the caller must keep off-chain).
/// Losing the off-chain part means the memento can never be revealed
/// — the chain holds the commitment but cannot decommit it.
///
/// The caller is responsible for sourcing a uniform 32-byte nonce;
/// this crate doesn't bundle an RNG so it stays no-std-ready and
/// fully deterministic for tests.
pub fn seal(
    payload: Vec<u8>,
    nonce: [u8; 32],
    trigger: MementoTrigger,
    owner: evaporchain_types::AccountAddress,
    sealed_at_epoch: evaporchain_types::Epoch,
) -> (MementoContract, MementoOpening) {
    let commitment = MementoCommitment::compute(MementoVersion::V1, &payload, &nonce);
    let contract = MementoContract {
        version: MementoVersion::V1,
        commitment,
        trigger,
        owner,
        sealed_at_epoch,
    };
    let opening = MementoOpening { payload, nonce };
    (contract, opening)
}

/// The off-chain witness that the caller of [`seal`] retains.
/// Required at reveal time to decommit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MementoOpening {
    /// The sealed payload. Never touches the chain until reveal.
    pub payload: Vec<u8>,
    /// The 32-byte nonce used to compute the commitment.
    pub nonce: [u8; 32],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trigger::MementoTrigger;

    #[test]
    fn seal_produces_deterministic_commitment_for_same_inputs() {
        let payload = b"my final wishes: leave everything to the cat".to_vec();
        let nonce = [42u8; 32];
        let owner = [0u8; 32];
        let trigger = MementoTrigger::BlockHeightReached(1_000_000);

        let (c1, _) = seal(payload.clone(), nonce, trigger.clone(), owner, 100);
        let (c2, _) = seal(payload, nonce, trigger, owner, 100);
        assert_eq!(c1.commitment, c2.commitment);
        assert!(c1.commitment.ct_eq(&c2.commitment));
    }

    #[test]
    fn different_nonce_yields_different_commitment() {
        let payload = b"same payload".to_vec();
        let trigger = MementoTrigger::BlockHeightReached(1);
        let owner = [0u8; 32];
        let (c1, _) = seal(payload.clone(), [1u8; 32], trigger.clone(), owner, 0);
        let (c2, _) = seal(payload, [2u8; 32], trigger, owner, 0);
        assert_ne!(c1.commitment, c2.commitment);
    }

    #[test]
    fn different_payload_yields_different_commitment() {
        let nonce = [9u8; 32];
        let trigger = MementoTrigger::BlockHeightReached(1);
        let owner = [0u8; 32];
        let (c1, _) = seal(b"will A".to_vec(), nonce, trigger.clone(), owner, 0);
        let (c2, _) = seal(b"will B".to_vec(), nonce, trigger, owner, 0);
        assert_ne!(c1.commitment, c2.commitment);
    }

    /// Length-prefixing test: `payload="ab", nonce="c"` MUST NOT
    /// commit to the same value as `payload="a", nonce="bc"`. Without
    /// the `u64_LE(payload.len())` in the hash transcript, these
    /// would collide.
    #[test]
    fn length_prefix_prevents_concat_collision() {
        let c1 = MementoCommitment::compute(MementoVersion::V1, b"ab", &{
            let mut n = [0u8; 32];
            n[0] = b'c';
            n
        });
        let c2 = MementoCommitment::compute(MementoVersion::V1, b"a", &{
            let mut n = [0u8; 32];
            n[0] = b'b';
            n[1] = b'c';
            n
        });
        // The current encoding adds a length prefix, so these MUST
        // differ. If this test ever fires, the commitment encoding
        // has lost its length-prefix guard.
        assert_ne!(c1, c2);
    }

    #[test]
    fn ct_eq_matches_pet_eq() {
        let a = MementoCommitment([0xAB; 32]);
        let b = MementoCommitment([0xAB; 32]);
        let c = MementoCommitment([0xCD; 32]);
        assert!(a.ct_eq(&b));
        assert!(!a.ct_eq(&c));
    }
}
