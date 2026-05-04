//! `InstanceId` — the deterministic on-chain handle for a
//! materialised template.
//!
//! Three structural decisions:
//!
//! 1. **Domain-separated from the deploy commitment.** Tag is
//!    `b"evaporchain:instance:v1\0"`. The deploy commitment hashes
//!    the *request* (what the user asked for); the instance id
//!    hashes the *placement* (which on-chain instance this becomes).
//!    Same inputs go in, but different domains so the two hashes
//!    can never collide and be confused.
//!
//! 2. **Inputs: `(template_class, deployer, nonce)` only.** Not the
//!    params. Reason: a user submitting two requests with the same
//!    `(class, deployer, nonce)` but different params is a replay
//!    attack at the deploy layer, not the instance layer. The
//!    transaction layer's nonce-bookkeeping rejects the replay
//!    before this code runs. By NOT including params we get the
//!    nice property that an instance id is computable BEFORE the
//!    full request is on the wire — useful for prefetch / mempool
//!    indexing.
//!
//! 3. **No epoch.** The submit epoch is metadata about *when* the
//!    request was made, not *what* was deployed. A relayer
//!    bouncing the same signed request across multiple epochs must
//!    not produce different instance ids — that's still the same
//!    deploy attempt.

use serde::{Deserialize, Serialize};

use evaporchain_app_templates::TemplateClass;

/// Domain-separation tag for the instance-id derivation.
pub const INSTANCE_DOMAIN_TAG: &[u8] = b"evaporchain:instance:v1\0";

/// 32-byte deterministic on-chain handle for a materialised template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct InstanceId(pub [u8; 32]);

impl InstanceId {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Derive the instance id for a deploy. Pure function: validators
/// independently derive the same handle.
///
/// Layout fed into BLAKE3:
/// ```text
/// b"evaporchain:instance:v1\0"  (24 B, fixed)
/// template_class                (u32 LE, 4 B)
/// deployer                      (32 B)
/// nonce                         (u64 LE, 8 B)
/// ```
pub fn derive_instance_id(
    template_class: TemplateClass,
    deployer: &[u8; 32],
    nonce: u64,
) -> InstanceId {
    let mut h = blake3::Hasher::new();
    h.update(INSTANCE_DOMAIN_TAG);
    h.update(&template_class.0.to_le_bytes());
    h.update(deployer);
    h.update(&nonce.to_le_bytes());
    InstanceId(*h.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_app_templates::class::{MAYFLY, SINGH_SABI};

    #[test]
    fn derivation_is_deterministic() {
        let a = derive_instance_id(SINGH_SABI, &[0xAB; 32], 42);
        let b = derive_instance_id(SINGH_SABI, &[0xAB; 32], 42);
        assert_eq!(a, b);
    }

    #[test]
    fn different_class_produces_different_id() {
        let a = derive_instance_id(SINGH_SABI, &[0xAB; 32], 42);
        let b = derive_instance_id(MAYFLY, &[0xAB; 32], 42);
        assert_ne!(a, b);
    }

    #[test]
    fn different_deployer_produces_different_id() {
        let a = derive_instance_id(SINGH_SABI, &[0xAA; 32], 42);
        let b = derive_instance_id(SINGH_SABI, &[0xBB; 32], 42);
        assert_ne!(a, b);
    }

    #[test]
    fn different_nonce_produces_different_id() {
        let a = derive_instance_id(SINGH_SABI, &[0; 32], 1);
        let b = derive_instance_id(SINGH_SABI, &[0; 32], 2);
        assert_ne!(a, b);
    }

    #[test]
    fn epoch_does_not_affect_id() {
        // Anti-regression: even though a relayer might submit at
        // different epochs, the id is fixed by the deployer's intent
        // alone.
        // (The function doesn't take epoch — this test exists to
        // document that decision.)
        let a = derive_instance_id(SINGH_SABI, &[0; 32], 7);
        let b = derive_instance_id(SINGH_SABI, &[0; 32], 7);
        assert_eq!(a, b);
    }

    #[test]
    fn id_is_domain_separated() {
        // The instance-id hash MUST differ from a naive BLAKE3 of
        // the same fields without the domain tag — otherwise a
        // future hash with identical fields could collide.
        let class = SINGH_SABI;
        let deployer = [0xAB; 32];
        let nonce: u64 = 42;

        let mut without_tag = blake3::Hasher::new();
        without_tag.update(&class.0.to_le_bytes());
        without_tag.update(&deployer);
        without_tag.update(&nonce.to_le_bytes());
        let naive: [u8; 32] = *without_tag.finalize().as_bytes();

        let id = derive_instance_id(class, &deployer, nonce);
        assert_ne!(id.0, naive);
    }

    #[test]
    fn round_trip_serde() {
        let id = derive_instance_id(SINGH_SABI, &[0xAB; 32], 42);
        let s = serde_json::to_string(&id).unwrap();
        let back: InstanceId = serde_json::from_str(&s).unwrap();
        assert_eq!(id, back);
    }
}
