//! `BraidWord` — sequence of generators.
//!
//! Encoding: `+i` is `σ_i` (i ≥ 1); `-i` is `σ_i^{-1}`. `0` is
//! never a valid generator (no `σ_0`).

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct BraidWord {
    pub generators: Vec<i32>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WordError {
    #[error("generator 0 is invalid (B_n indexes σ_1 .. σ_{{n−1}})")]
    ZeroGenerator,
    #[error("generator {got} exceeds n−1 = {bound}")]
    OutOfRange { got: i32, bound: i32 },
}

impl BraidWord {
    /// Construct + validate against `n` (the number of strands).
    pub fn new(generators: Vec<i32>, n: u32) -> Result<Self, WordError> {
        let bound = n as i32 - 1;
        for &g in &generators {
            if g == 0 {
                return Err(WordError::ZeroGenerator);
            }
            if g.abs() > bound {
                return Err(WordError::OutOfRange { got: g, bound });
            }
        }
        Ok(Self { generators })
    }

    pub fn identity() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.generators.len()
    }

    pub fn is_empty(&self) -> bool {
        self.generators.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_word() {
        let w = BraidWord::new(vec![1, 2, -1, 3], 4).unwrap();
        assert_eq!(w.len(), 4);
    }

    #[test]
    fn zero_generator_rejected() {
        assert!(matches!(
            BraidWord::new(vec![0], 4).unwrap_err(),
            WordError::ZeroGenerator
        ));
    }

    #[test]
    fn out_of_range_rejected() {
        // n=4 → max |g| = 3.
        assert!(matches!(
            BraidWord::new(vec![5], 4).unwrap_err(),
            WordError::OutOfRange { .. }
        ));
        assert!(matches!(
            BraidWord::new(vec![-5], 4).unwrap_err(),
            WordError::OutOfRange { .. }
        ));
    }

    #[test]
    fn identity_is_empty() {
        assert!(BraidWord::identity().is_empty());
    }
}
