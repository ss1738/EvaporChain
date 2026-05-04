//! `Did` — the DID document + selective-disclosure presentation.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::attr::{Attribute, AttributeKey};

/// 32-byte DID handle. The DID's controlling key is at the higher
/// layer (this crate doesn't model signature crypto).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct DidId(pub [u8; 32]);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DidError {
    #[error("attribute key {0:?} already attested — must update existing instead of adding")]
    AttributeAlreadyExists(AttributeKey),
    #[error("attribute key {0:?} not present on this DID")]
    AttributeNotFound(AttributeKey),
}

/// One DID document — handle + per-key attributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Did {
    pub id: DidId,
    /// Attributes keyed by their attribute key. One attribute per
    /// (DID, key) pair; re-attestation replaces.
    attributes: HashMap<AttributeKey, Attribute>,
}

impl Did {
    pub fn new(id: DidId) -> Self {
        Self {
            id,
            attributes: HashMap::new(),
        }
    }

    pub fn attribute_count(&self) -> usize {
        self.attributes.len()
    }

    pub fn get(&self, key: &AttributeKey) -> Option<&Attribute> {
        self.attributes.get(key)
    }

    /// Add a new attribute. Errors if one already exists for the
    /// key (use `re_attest` to replace).
    pub fn add_attribute(&mut self, attribute: Attribute) -> Result<(), DidError> {
        if self.attributes.contains_key(&attribute.key) {
            return Err(DidError::AttributeAlreadyExists(attribute.key));
        }
        self.attributes.insert(attribute.key, attribute);
        Ok(())
    }

    /// Replace an existing attribute (re-attestation refreshes
    /// energy, possibly issuer).
    pub fn re_attest(&mut self, attribute: Attribute) -> Result<(), DidError> {
        if !self.attributes.contains_key(&attribute.key) {
            return Err(DidError::AttributeNotFound(attribute.key));
        }
        self.attributes.insert(attribute.key, attribute);
        Ok(())
    }

    /// Mutate the energy of one attribute — used by the chain's
    /// decay tick.
    pub fn decay_attribute(
        &mut self,
        key: AttributeKey,
        new_energy: u64,
    ) -> Result<(), DidError> {
        let a = self
            .attributes
            .get_mut(&key)
            .ok_or(DidError::AttributeNotFound(key))?;
        a.energy = new_energy;
        Ok(())
    }

    /// Build a selective-disclosure presentation. Includes ONLY
    /// the attributes named in `disclose`. Caller chooses; the
    /// gate (`verify_presentation`) enforces verifier floor per
    /// attribute.
    pub fn present(&self, disclose: &[AttributeKey]) -> Result<Presentation, DidError> {
        let mut out = Vec::with_capacity(disclose.len());
        for k in disclose {
            let a = self
                .attributes
                .get(k)
                .ok_or(DidError::AttributeNotFound(*k))?;
            out.push(a.clone());
        }
        Ok(Presentation {
            did: self.id,
            attributes: out,
        })
    }
}

/// A selective-disclosure presentation — what the holder sends
/// to a verifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presentation {
    pub did: DidId,
    pub attributes: Vec<Attribute>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn did(b: u8) -> DidId { DidId([b; 32]) }
    fn k(b: u8) -> AttributeKey { AttributeKey([b; 32]) }

    fn attr(key: AttributeKey, energy: u64) -> Attribute {
        Attribute::new(key, b"value".to_vec(), [0xAB; 32], energy, 100)
    }

    #[test]
    fn fresh_did_has_no_attributes() {
        let d = Did::new(did(1));
        assert_eq!(d.attribute_count(), 0);
    }

    #[test]
    fn add_attribute_succeeds_once() {
        let mut d = Did::new(did(1));
        d.add_attribute(attr(k(1), 1000)).unwrap();
        let err = d.add_attribute(attr(k(1), 2000)).unwrap_err();
        assert_eq!(err, DidError::AttributeAlreadyExists(k(1)));
    }

    #[test]
    fn re_attest_replaces() {
        let mut d = Did::new(did(1));
        d.add_attribute(attr(k(1), 1000)).unwrap();
        d.re_attest(attr(k(1), 5000)).unwrap();
        assert_eq!(d.get(&k(1)).unwrap().energy, 5000);
    }

    #[test]
    fn re_attest_fails_for_missing() {
        let mut d = Did::new(did(1));
        let err = d.re_attest(attr(k(1), 1000)).unwrap_err();
        assert_eq!(err, DidError::AttributeNotFound(k(1)));
    }

    #[test]
    fn decay_attribute_updates_energy() {
        let mut d = Did::new(did(1));
        d.add_attribute(attr(k(1), 1000)).unwrap();
        d.decay_attribute(k(1), 50).unwrap();
        assert_eq!(d.get(&k(1)).unwrap().energy, 50);
    }

    #[test]
    fn decay_unknown_attribute_rejected() {
        let mut d = Did::new(did(1));
        let err = d.decay_attribute(k(99), 0).unwrap_err();
        assert_eq!(err, DidError::AttributeNotFound(k(99)));
    }

    #[test]
    fn present_includes_disclosed_attributes_only() {
        let mut d = Did::new(did(1));
        d.add_attribute(attr(k(1), 1000)).unwrap();
        d.add_attribute(attr(k(2), 1000)).unwrap();
        d.add_attribute(attr(k(3), 1000)).unwrap();
        let p = d.present(&[k(1), k(3)]).unwrap();
        assert_eq!(p.attributes.len(), 2);
        let keys: Vec<AttributeKey> = p.attributes.iter().map(|a| a.key).collect();
        assert!(keys.contains(&k(1)));
        assert!(keys.contains(&k(3)));
        assert!(!keys.contains(&k(2)));
    }

    #[test]
    fn present_fails_for_undisclosed_missing_attribute() {
        let d = Did::new(did(1));
        let err = d.present(&[k(99)]).unwrap_err();
        assert_eq!(err, DidError::AttributeNotFound(k(99)));
    }

    #[test]
    fn presentation_round_trips_serde() {
        // Did contains HashMap<AttributeKey, Attribute> where
        // AttributeKey is [u8;32]; JSON requires string-typed
        // map keys, so the full Did doesn't round-trip through
        // JSON. Chain wraps Did with binary durability.
        // Presentation is Vec<Attribute> (no map keys) — and
        // it's the wire-shaped value — so it round-trips fine.
        let mut d = Did::new(did(1));
        d.add_attribute(attr(k(1), 1000)).unwrap();
        d.add_attribute(attr(k(2), 1000)).unwrap();
        let p = d.present(&[k(1), k(2)]).unwrap();
        let s = serde_json::to_string(&p).unwrap();
        let back: Presentation = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}
