//! `UsageProfile` — how many times each variable was used. The linear
//! checker compares this against the variable's type's exponential
//! flavour to decide legality.
//!
//! This is deliberately a thin record; production-grade type
//! inference would live in a separate compiler crate (analogous to
//! how `evaporchain-script/parser.rs` produces an AST that the VM
//! consumes). Here we expose the shape so other crates can build
//! checkers, fuzzers, and proof obligations.

use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UsageProfileError {
    #[error("variable already recorded — call `add_use` instead of `record`")]
    AlreadyRecorded,
}

/// Per-variable usage count. Variables are identified by `String`
/// names (V1 simplification — production systems use de-Bruijn or
/// hash-cons'd handles). The protocol-level usage is integer-only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageProfile {
    counts: HashMap<String, u64>,
}

impl UsageProfile {
    pub fn new() -> Self {
        Self {
            counts: HashMap::new(),
        }
    }

    /// Initialise a variable at zero uses. Errors if the variable
    /// was already recorded — keeps the API explicit about which
    /// variables exist in scope.
    pub fn record(&mut self, name: impl Into<String>) -> Result<(), UsageProfileError> {
        let name = name.into();
        if self.counts.contains_key(&name) {
            return Err(UsageProfileError::AlreadyRecorded);
        }
        self.counts.insert(name, 0);
        Ok(())
    }

    /// Count one use of `name`. If the variable wasn't pre-recorded,
    /// it's added at count = 1 (forgiving — a parser that emits
    /// uses without tracking declarations still produces a usable
    /// profile).
    pub fn add_use(&mut self, name: impl Into<String>) {
        *self.counts.entry(name.into()).or_insert(0) += 1;
    }

    /// Look up a variable's usage count. Returns 0 for unknown names.
    pub fn count(&self, name: &str) -> u64 {
        *self.counts.get(name).unwrap_or(&0)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &u64)> {
        self.counts.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_then_add_use_counts() {
        let mut p = UsageProfile::new();
        p.record("x").unwrap();
        p.add_use("x");
        p.add_use("x");
        assert_eq!(p.count("x"), 2);
    }

    #[test]
    fn record_twice_errors() {
        let mut p = UsageProfile::new();
        p.record("x").unwrap();
        let err = p.record("x").unwrap_err();
        assert_eq!(err, UsageProfileError::AlreadyRecorded);
    }

    #[test]
    fn unknown_name_returns_zero() {
        let p = UsageProfile::new();
        assert_eq!(p.count("nope"), 0);
    }

    #[test]
    fn add_use_without_record_works() {
        let mut p = UsageProfile::new();
        p.add_use("y");
        p.add_use("y");
        assert_eq!(p.count("y"), 2);
    }
}
