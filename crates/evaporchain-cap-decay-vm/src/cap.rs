//! `Capability` — a typed authorization token + decay state.

use serde::{Deserialize, Serialize};

/// Energy floor below which a capability is non-invocable.
/// `0` is the strict floor; the registry's invoke gate checks
/// `energy > ENERGY_FLOOR`. The chain's decay model writes the
/// energy down over time toward 0; the moment it crosses the
/// floor, the capability is structurally revoked.
pub const ENERGY_FLOOR: u64 = 0;

/// 32-byte deterministic id for a capability. Computed as
/// BLAKE3 of (issuer || nonce || authority canonical bytes), so
/// validators agree on the id and an attenuated child has a
/// different id from its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct CapabilityId(pub [u8; 32]);

/// What a capability authorizes — `verb` on `object` with
/// optional bound on max invocations per epoch.
///
/// Authority forms a partial order:
///
/// `A ⊑ B` iff `A.verb == B.verb`, `A.object == B.object`, and
/// `A.max_invokes_per_epoch ≤ B.max_invokes_per_epoch`.
///
/// Attenuation requires `child ⊑ parent` (strict subset OR equal —
/// equal is a *no-op attenuation* the registry rejects to keep the
/// graph minimal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Authority {
    /// Verb (e.g., "read", "write", "transfer"). The chain
    /// validates the verb domain at the higher layer; this layer
    /// is verb-agnostic.
    pub verb: String,
    /// 32-byte object handle the verb operates on (e.g., a
    /// resource id, contract id).
    pub object: [u8; 32],
    /// Max invocations per epoch. `u64::MAX` = unbounded.
    pub max_invokes_per_epoch: u64,
}

impl Authority {
    /// True iff `self` is a strict subset of `parent` (a valid
    /// attenuation target).
    pub fn is_strict_subset_of(&self, parent: &Authority) -> bool {
        self.verb == parent.verb
            && self.object == parent.object
            && self.max_invokes_per_epoch < parent.max_invokes_per_epoch
    }

    /// True iff `self` is at-most-equal to `parent` (subset or
    /// equal). Useful for invariant checks.
    pub fn is_at_most(&self, parent: &Authority) -> bool {
        self.verb == parent.verb
            && self.object == parent.object
            && self.max_invokes_per_epoch <= parent.max_invokes_per_epoch
    }

    /// Canonical bytes of the authority — what goes into the
    /// CapabilityId hash. Layout: u32-LE verb-len || verb bytes
    /// || object || u64-LE max_invokes. Length-prefixed so two
    /// authorities with different verbs but the same byte
    /// sequence can never collide.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let verb_bytes = self.verb.as_bytes();
        let mut out = Vec::with_capacity(4 + verb_bytes.len() + 32 + 8);
        out.extend_from_slice(&(verb_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(verb_bytes);
        out.extend_from_slice(&self.object);
        out.extend_from_slice(&self.max_invokes_per_epoch.to_le_bytes());
        out
    }
}

/// One capability — handle + decay state + authority + provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub id: CapabilityId,
    /// Account that minted this capability. Only the issuer can
    /// revoke it.
    pub issuer: [u8; 32],
    /// Account currently holding the capability. Transfers update
    /// this; attenuation creates a new cap with a new holder.
    pub holder: [u8; 32],
    pub authority: Authority,
    /// Current energy. Decays over time per the chain's λ. The
    /// invoke gate requires `energy > ENERGY_FLOOR`.
    pub energy: u64,
    /// `Some(parent)` if attenuated from another cap; `None` if
    /// minted as a root. Structural revocation walks back up
    /// this chain at invoke time.
    pub parent: Option<CapabilityId>,
    /// Epoch at which the cap was minted. The chain uses this to
    /// compute decay externally; this layer just stores it.
    pub minted_at_epoch: u64,
}

impl Capability {
    /// Compute a fresh capability id.
    /// `nonce` is a 32-byte unique nonce the chain provides
    /// (typically the deploy-tx commitment or similar).
    pub fn derive_id(issuer: &[u8; 32], nonce: &[u8; 32], authority: &Authority) -> CapabilityId {
        let mut h = blake3::Hasher::new();
        h.update(b"evaporchain:capability:v1\0");
        h.update(issuer);
        h.update(nonce);
        h.update(&authority.canonical_bytes());
        CapabilityId(*h.finalize().as_bytes())
    }

    /// True iff the cap's energy is above the floor — i.e., it
    /// is *currently* invocable considered in isolation. The
    /// registry's full check also walks the parent chain.
    pub fn is_live(&self) -> bool {
        self.energy > ENERGY_FLOOR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth(verb: &str, object: u8, max: u64) -> Authority {
        Authority {
            verb: verb.into(),
            object: [object; 32],
            max_invokes_per_epoch: max,
        }
    }

    #[test]
    fn strict_subset_requires_lower_max() {
        let parent = auth("read", 1, 100);
        let child_strict = auth("read", 1, 50);
        let child_equal = auth("read", 1, 100);
        let child_more = auth("read", 1, 200);
        assert!(child_strict.is_strict_subset_of(&parent));
        assert!(!child_equal.is_strict_subset_of(&parent));
        assert!(!child_more.is_strict_subset_of(&parent));
    }

    #[test]
    fn strict_subset_requires_same_verb() {
        let parent = auth("read", 1, 100);
        let different_verb = auth("write", 1, 50);
        assert!(!different_verb.is_strict_subset_of(&parent));
    }

    #[test]
    fn strict_subset_requires_same_object() {
        let parent = auth("read", 1, 100);
        let different_obj = auth("read", 2, 50);
        assert!(!different_obj.is_strict_subset_of(&parent));
    }

    #[test]
    fn at_most_includes_equal() {
        let parent = auth("read", 1, 100);
        let equal = auth("read", 1, 100);
        let subset = auth("read", 1, 50);
        assert!(equal.is_at_most(&parent));
        assert!(subset.is_at_most(&parent));
    }

    #[test]
    fn canonical_bytes_length_prefix_separates() {
        // Two authorities where the byte sequence might collide
        // without the length prefix produce different canonical
        // bytes.
        let a = auth("readwrite", 1, 100);
        let b = auth("read", 1, 100); // shorter verb
        assert_ne!(a.canonical_bytes(), b.canonical_bytes());
    }

    #[test]
    fn derive_id_is_deterministic() {
        let issuer = [1u8; 32];
        let nonce = [2u8; 32];
        let a = auth("read", 1, 100);
        let id1 = Capability::derive_id(&issuer, &nonce, &a);
        let id2 = Capability::derive_id(&issuer, &nonce, &a);
        assert_eq!(id1, id2);
    }

    #[test]
    fn derive_id_changes_with_authority() {
        let issuer = [1u8; 32];
        let nonce = [2u8; 32];
        let a = auth("read", 1, 100);
        let b = auth("write", 1, 100);
        let id_a = Capability::derive_id(&issuer, &nonce, &a);
        let id_b = Capability::derive_id(&issuer, &nonce, &b);
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn is_live_threshold() {
        let cap = Capability {
            id: CapabilityId([0; 32]),
            issuer: [0; 32],
            holder: [0; 32],
            authority: auth("read", 1, 100),
            energy: 1,
            parent: None,
            minted_at_epoch: 0,
        };
        assert!(cap.is_live());
        let dead = Capability { energy: 0, ..cap };
        assert!(!dead.is_live());
    }

    #[test]
    fn round_trip_serde() {
        let cap = Capability {
            id: CapabilityId([0xAB; 32]),
            issuer: [1; 32],
            holder: [2; 32],
            authority: auth("read", 1, 100),
            energy: 1000,
            parent: None,
            minted_at_epoch: 42,
        };
        let s = serde_json::to_string(&cap).unwrap();
        let back: Capability = serde_json::from_str(&s).unwrap();
        assert_eq!(cap, back);
    }

    /// T1.20 — `derive_id` is sensitive to the issuer and nonce
    /// inputs, not just the authority. The existing
    /// `derive_id_changes_with_authority` only varied authority;
    /// without this, a refactor that dropped issuer or nonce from
    /// the hash would silently collapse caps from different issuers
    /// or replay nonces.
    #[test]
    fn t1_20_derive_id_sensitive_to_issuer_and_nonce() {
        let a = auth("read", 1, 100);
        let issuer_a = [1u8; 32];
        let issuer_b = [9u8; 32];
        let nonce_a = [2u8; 32];
        let nonce_b = [8u8; 32];

        // Same authority + nonce, different issuer → different id.
        assert_ne!(
            Capability::derive_id(&issuer_a, &nonce_a, &a),
            Capability::derive_id(&issuer_b, &nonce_a, &a),
            "issuer must be in the digest"
        );
        // Same authority + issuer, different nonce → different id.
        assert_ne!(
            Capability::derive_id(&issuer_a, &nonce_a, &a),
            Capability::derive_id(&issuer_a, &nonce_b, &a),
            "nonce must be in the digest"
        );
    }

    /// T1.20 — `derive_id` includes the `evaporchain:capability:v1\0`
    /// domain tag (line 105). A bare BLAKE3 of the same fields
    /// without the tag must produce a different id. This pins the
    /// cross-protocol domain separation.
    #[test]
    fn t1_20_derive_id_uses_domain_tag() {
        let issuer = [1u8; 32];
        let nonce = [2u8; 32];
        let a = auth("read", 1, 100);
        let with_tag = Capability::derive_id(&issuer, &nonce, &a);
        let without_tag = {
            let mut h = blake3::Hasher::new();
            h.update(&issuer);
            h.update(&nonce);
            h.update(&a.canonical_bytes());
            CapabilityId(*h.finalize().as_bytes())
        };
        assert_ne!(
            with_tag, without_tag,
            "derive_id must include the v1 domain tag"
        );
    }

    /// T1.20 — `is_at_most` rejects mismatched verb or object even
    /// when `max_invokes_per_epoch` would satisfy `<=`. Existing
    /// `at_most_includes_equal` only covered the happy path. Pins
    /// the same-verb / same-object preconditions on the partial-order
    /// gate.
    #[test]
    fn t1_20_is_at_most_rejects_verb_and_object_mismatch() {
        let parent = auth("read", 1, 100);
        // Different verb, equal max.
        let other_verb = auth("write", 1, 100);
        assert!(!other_verb.is_at_most(&parent));
        // Different verb, smaller max (would otherwise satisfy <=).
        let other_verb_smaller = auth("write", 1, 50);
        assert!(!other_verb_smaller.is_at_most(&parent));
        // Different object.
        let other_obj = auth("read", 9, 50);
        assert!(!other_obj.is_at_most(&parent));
    }

    /// T1.20 — `canonical_bytes` length prefix actually prevents
    /// collision. The existing test asserts inequality; this
    /// constructs the adversarial pair where a naive
    /// `verb_bytes || object || max` concatenation WOULD collide,
    /// and confirms the length prefix breaks it.
    #[test]
    fn t1_20_canonical_bytes_length_prefix_breaks_collision() {
        // verb "ab" + object [0xCD; 32] vs verb "abCD..." (where
        // bytes of verb-tail look like the start of an object).
        // The length prefix puts a distinct u32-LE in front, so
        // canonical_bytes differ. Specifically: verb "ab" has
        // prefix [0x02,0,0,0]; verb "abXY" has prefix [0x04,0,0,0].
        let a = auth("ab", 0xCD, 100);
        let b = auth("abCD", 0xCD, 100);
        let ca = a.canonical_bytes();
        let cb = b.canonical_bytes();
        assert_ne!(ca, cb);
        // First 4 bytes encode the verb length explicitly.
        assert_eq!(&ca[..4], &2u32.to_le_bytes());
        assert_eq!(&cb[..4], &4u32.to_le_bytes());
    }
}
