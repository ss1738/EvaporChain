//! Skill class registry entry.
//!
//! A `SkillClass` is the taxonomy node — a named skill (Python, COBOL,
//! prompt-engineering) with a published decay rate (λ). The decay rate
//! is realised as a `HalfLife` (epochs) so it composes with the standard
//! `evaporchain_types::energy_at_epoch` decay function used everywhere
//! else on the chain.
//!
//! Skill classes are **registered once and frozen**. The registration's
//! half-life is the chain's published claim about how fast the skill
//! decays *in the world*. Disputing it is a governance question, not a
//! protocol question — but every credential issued under the class
//! follows that rate mechanically.

use evaporchain_types::{Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 32-byte opaque skill-class handle. Caller picks (often a hash of
/// the canonical skill name).
pub type SkillClassId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SkillClassError {
    #[error("half_life must be > 0")]
    ZeroHalfLife,
    #[error("name must be non-empty")]
    EmptyName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillClass {
    pub id: SkillClassId,
    /// Human-readable canonical name. Stored on-chain so light clients
    /// don't need an off-chain registry to render credentials.
    pub name: String,
    /// Decay rate as a half-life in epochs. The whole point of SHLM —
    /// Python's half-life is short (~18mo); COBOL's is long (~8yr).
    pub half_life: HalfLife,
    /// Epoch the class was registered. Used as `attested_at_epoch`
    /// reference in audit trails (which credentials predate which
    /// taxonomy version, etc.).
    pub registered_at: Epoch,
}

impl SkillClass {
    pub fn register(
        id: SkillClassId,
        name: impl Into<String>,
        half_life: HalfLife,
        registered_at: Epoch,
    ) -> Result<Self, SkillClassError> {
        let name = name.into();
        if name.is_empty() {
            return Err(SkillClassError::EmptyName);
        }
        if half_life == 0 {
            return Err(SkillClassError::ZeroHalfLife);
        }
        Ok(Self {
            id,
            name,
            half_life,
            registered_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> SkillClassId {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    #[test]
    fn rejects_empty_name() {
        assert_eq!(
            SkillClass::register(id(1), "", 100, 0).unwrap_err(),
            SkillClassError::EmptyName
        );
    }

    #[test]
    fn rejects_zero_half_life() {
        assert_eq!(
            SkillClass::register(id(1), "Python", 0, 0).unwrap_err(),
            SkillClassError::ZeroHalfLife
        );
    }

    #[test]
    fn registers_with_doctrine_examples() {
        // Doctrine §A5.2 examples — half-lives expressed in epochs
        // (assuming a 1-day epoch for illustration; actual chain epoch
        // length is set by genesis).
        let python = SkillClass::register(id(1), "Python", 540, 0).unwrap(); // ~18mo
        let cobol = SkillClass::register(id(2), "COBOL", 2920, 0).unwrap(); // ~8yr
        let prompt = SkillClass::register(id(3), "prompt-engineering", 120, 0).unwrap(); // ~4mo
                                                                                         // Doctrine ordering invariant: prompt < Python < COBOL.
        assert!(prompt.half_life < python.half_life);
        assert!(python.half_life < cobol.half_life);
    }

    #[test]
    fn round_trip_serde() {
        let s = SkillClass::register(id(7), "Rust", 720, 100).unwrap();
        let j = serde_json::to_string(&s).unwrap();
        let back: SkillClass = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
