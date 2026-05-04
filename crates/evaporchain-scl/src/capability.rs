//! Capability tuple — `(verb, object)`. The `subject` lives on the
//! Lease (because the subject changes with resale).

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type CapabilityId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CapabilityError {
    #[error("verb hash must not be all-zero")]
    EmptyVerb,
    #[error("object hash must not be all-zero")]
    EmptyObject,
}

/// Opaque (verb, object) capability. Higher layers project these onto
/// domain semantics (e.g. verb = blake3("treasury.transfer"), object
/// = treasury account hash).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capability {
    pub verb: [u8; 32],
    pub object: [u8; 32],
}

impl Capability {
    pub fn new(verb: [u8; 32], object: [u8; 32]) -> Result<Self, CapabilityError> {
        if verb == [0u8; 32] {
            return Err(CapabilityError::EmptyVerb);
        }
        if object == [0u8; 32] {
            return Err(CapabilityError::EmptyObject);
        }
        Ok(Self { verb, object })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_verb() {
        let err = Capability::new([0; 32], [1; 32]).unwrap_err();
        assert_eq!(err, CapabilityError::EmptyVerb);
    }

    #[test]
    fn rejects_empty_object() {
        let err = Capability::new([1; 32], [0; 32]).unwrap_err();
        assert_eq!(err, CapabilityError::EmptyObject);
    }

    #[test]
    fn distinct_objects_distinct_capabilities() {
        let a = Capability::new([1; 32], [0xAA; 32]).unwrap();
        let b = Capability::new([1; 32], [0xBB; 32]).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn round_trip_serde() {
        let c = Capability::new([0x77; 32], [0xCD; 32]).unwrap();
        let s = serde_json::to_string(&c).unwrap();
        let back: Capability = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }
}
