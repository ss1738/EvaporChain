//! `Attribute` — one issuer-attested claim on a DID.

use serde::{Deserialize, Serialize};

pub const ATTRIBUTE_TAG: &[u8] = b"evaporchain:ra-did:attribute:v1\0";

/// 32-byte attribute key — chain-wide handle for an attribute
/// type. Validators agree on the (key → semantic-meaning) map at
/// the higher layer (e.g., key([0x01; 32]) = "age-verified-18+").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct AttributeKey(pub [u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribute {
    pub key: AttributeKey,
    /// Opaque value (e.g., "1985-04-12" for age-verification).
    /// Bytes — chain-agnostic.
    pub value: Vec<u8>,
    /// 32-byte issuer pubkey (or hash). Identifies who attested.
    /// Validator agreement at higher layer.
    pub issuer: [u8; 32],
    /// Energy at attestation time. Decays under chain's λ. The
    /// presentation gate enforces `energy > verifier_floor`.
    pub energy: u64,
    /// Epoch the attribute was issued. Bound into the hash.
    pub issued_at_epoch: u64,
}

impl Attribute {
    pub fn new(
        key: AttributeKey,
        value: Vec<u8>,
        issuer: [u8; 32],
        energy: u64,
        issued_at_epoch: u64,
    ) -> Self {
        Self {
            key,
            value,
            issuer,
            energy,
            issued_at_epoch,
        }
    }

    /// Domain-tagged BLAKE3 hash of the attribute, binding
    /// (key, value, issuer, energy, issued_at_epoch). Length-
    /// prefixed value to prevent cross-attribute hash collision.
    pub fn hash(&self) -> [u8; 32] {
        attribute_hash(
            &self.key,
            &self.value,
            &self.issuer,
            self.energy,
            self.issued_at_epoch,
        )
    }
}

pub fn attribute_hash(
    key: &AttributeKey,
    value: &[u8],
    issuer: &[u8; 32],
    energy: u64,
    issued_at_epoch: u64,
) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(ATTRIBUTE_TAG);
    h.update(&key.0);
    h.update(&(value.len() as u32).to_le_bytes()); // length-prefix
    h.update(value);
    h.update(issuer);
    h.update(&energy.to_le_bytes());
    h.update(&issued_at_epoch.to_le_bytes());
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(b: u8) -> AttributeKey {
        AttributeKey([b; 32])
    }

    #[test]
    fn hash_is_deterministic() {
        let a = Attribute::new(k(1), b"alice".to_vec(), [0xAB; 32], 1000, 100);
        assert_eq!(a.hash(), a.hash());
    }

    #[test]
    fn hash_changes_with_each_field() {
        let base = Attribute::new(k(1), b"alice".to_vec(), [0xAB; 32], 1000, 100);
        let h0 = base.hash();
        // value
        let a1 = Attribute::new(k(1), b"bob".to_vec(), [0xAB; 32], 1000, 100);
        assert_ne!(a1.hash(), h0);
        // issuer
        let a2 = Attribute::new(k(1), b"alice".to_vec(), [0xCD; 32], 1000, 100);
        assert_ne!(a2.hash(), h0);
        // key
        let a3 = Attribute::new(k(2), b"alice".to_vec(), [0xAB; 32], 1000, 100);
        assert_ne!(a3.hash(), h0);
        // energy — anti-pump check
        let a4 = Attribute::new(k(1), b"alice".to_vec(), [0xAB; 32], 9999, 100);
        assert_ne!(a4.hash(), h0);
        // issued_at
        let a5 = Attribute::new(k(1), b"alice".to_vec(), [0xAB; 32], 1000, 200);
        assert_ne!(a5.hash(), h0);
    }

    #[test]
    fn length_prefix_separates_collisions() {
        // ("ab", "cd") and ("a", "bcd") would collide without
        // length-prefix.
        let a = Attribute::new(k(1), b"abcd".to_vec(), [0xAB; 32], 1000, 100);
        let b = Attribute::new(k(1), b"abc".to_vec(), [0xAB; 32], 1000, 100);
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn hash_uses_domain_tag() {
        let a = Attribute::new(k(1), b"alice".to_vec(), [0xAB; 32], 1000, 100);
        let with = a.hash();
        let mut naive = blake3::Hasher::new();
        naive.update(&a.key.0);
        naive.update(&(a.value.len() as u32).to_le_bytes());
        naive.update(&a.value);
        naive.update(&a.issuer);
        naive.update(&a.energy.to_le_bytes());
        naive.update(&a.issued_at_epoch.to_le_bytes());
        let without: [u8; 32] = *naive.finalize().as_bytes();
        assert_ne!(with, without);
    }

    #[test]
    fn round_trip_serde() {
        let a = Attribute::new(k(1), b"alice".to_vec(), [0xAB; 32], 1000, 100);
        let s = serde_json::to_string(&a).unwrap();
        let back: Attribute = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }
}
