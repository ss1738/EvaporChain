//! Op alphabet — six reversible classical ops plus one irreversible
//! `Decay`. Reversible ops are bit-for-bit invertible; their gas cost
//! is `0` (in the Bennett-1973 limit). `Decay(λ)` exports entropy
//! and is the *only* op with a nonzero gas floor.
//!
//! V1 ships a minimal-but-Turing-incomplete instruction set sufficient
//! to demonstrate the asymmetry. Production work would extend this
//! with reversible Janus-style control flow (`if-fi`, `from-loop`,
//! procedure call/return) — out of scope for V1, all extensible
//! through the same `Reversible` trait.

use evaporchain_types::Energy;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::reversible::Reversible;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OpError {
    #[error("operand index {0} is outside the register file (V1 has 8 registers)")]
    InvalidRegister(u8),
    #[error("decay amount must be > 0 — zero-decay is a no-op, not a primitive")]
    ZeroDecayAmount,
}

/// Opcode set. Each variant either implements `Reversible` (the
/// classical ones) or is `Decay`, the unique irreversible primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    /// `r := r XOR k` — XOR is its own inverse (involution).
    XorReg { reg: u8, key: u64 },
    /// `r := r + k` — inverse is `SubReg(r, k)`.
    AddReg { reg: u8, k: u64 },
    /// `r := r - k` — inverse is `AddReg(r, k)`.
    SubReg { reg: u8, k: u64 },
    /// Swap two registers — its own inverse.
    Swap { a: u8, b: u8 },
    /// Bitwise NOT — its own inverse (involution).
    Not { reg: u8 },
    /// Cyclic left rotation by `bits`. Inverse rotates right by `bits`.
    Rotl { reg: u8, bits: u32 },
    /// Cyclic right rotation by `bits`. Inverse rotates left by `bits`.
    Rotr { reg: u8, bits: u32 },
    /// **The unique irreversible op.** Releases `amount` energy from
    /// the chain's state into the void; the trace cannot be
    /// reconstructed past a `Decay`. Costs gas equal to `amount`
    /// (Landauer-literal: this is the only place entropy is exported).
    Decay { amount: Energy },
}

impl Op {
    /// Gas cost. The asymmetry: reversible ops cost 0; Decay costs
    /// `amount`. *This is the Landauer claim, made operational.*
    pub fn gas_cost(&self) -> Energy {
        match self {
            Op::Decay { amount } => *amount,
            _ => 0,
        }
    }

    /// True iff this op has a `Reversible` implementation.
    pub fn is_reversible(&self) -> bool {
        !matches!(self, Op::Decay { .. })
    }
}

impl Reversible for Op {
    /// Inverse of any reversible op. Panics on `Decay` — the type
    /// check and `is_reversible()` should prevent that path; the
    /// trace recorder also refuses to record `Decay` as part of its
    /// reversible-prefix invariant.
    fn inverse(&self) -> Self {
        match *self {
            Op::XorReg { reg, key } => Op::XorReg { reg, key },
            Op::AddReg { reg, k } => Op::SubReg { reg, k },
            Op::SubReg { reg, k } => Op::AddReg { reg, k },
            Op::Swap { a, b } => Op::Swap { a, b },
            Op::Not { reg } => Op::Not { reg },
            Op::Rotl { reg, bits } => Op::Rotr { reg, bits },
            Op::Rotr { reg, bits } => Op::Rotl { reg, bits },
            Op::Decay { .. } => {
                unreachable!("Decay has no inverse — caller must check is_reversible()")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classical_ops_are_zero_gas() {
        for op in [
            Op::XorReg { reg: 0, key: 0xAB },
            Op::AddReg { reg: 1, k: 7 },
            Op::SubReg { reg: 1, k: 7 },
            Op::Swap { a: 0, b: 1 },
            Op::Not { reg: 2 },
            Op::Rotl { reg: 0, bits: 3 },
            Op::Rotr { reg: 0, bits: 3 },
        ] {
            assert_eq!(op.gas_cost(), 0, "{op:?} must be zero-gas");
            assert!(op.is_reversible());
        }
    }

    #[test]
    fn decay_is_only_op_with_nonzero_gas() {
        let d = Op::Decay { amount: 100 };
        assert_eq!(d.gas_cost(), 100);
        assert!(!d.is_reversible());
    }

    #[test]
    fn xor_is_its_own_inverse() {
        let op = Op::XorReg { reg: 0, key: 0xAB };
        assert_eq!(op.inverse(), op);
    }

    #[test]
    fn not_is_its_own_inverse() {
        let op = Op::Not { reg: 1 };
        assert_eq!(op.inverse(), op);
    }

    #[test]
    fn swap_is_its_own_inverse() {
        let op = Op::Swap { a: 0, b: 1 };
        assert_eq!(op.inverse(), op);
    }

    #[test]
    fn add_and_sub_are_inverses() {
        let a = Op::AddReg { reg: 0, k: 7 };
        let s = Op::SubReg { reg: 0, k: 7 };
        assert_eq!(a.inverse(), s);
        assert_eq!(s.inverse(), a);
    }

    #[test]
    fn rotl_and_rotr_are_inverses() {
        let l = Op::Rotl { reg: 0, bits: 5 };
        let r = Op::Rotr { reg: 0, bits: 5 };
        assert_eq!(l.inverse(), r);
        assert_eq!(r.inverse(), l);
    }

    #[test]
    fn round_trip_serde() {
        let ops = vec![
            Op::XorReg { reg: 0, key: 0xAB },
            Op::AddReg { reg: 1, k: 5 },
            Op::Decay { amount: 250 },
        ];
        let s = serde_json::to_string(&ops).unwrap();
        let back: Vec<Op> = serde_json::from_str(&s).unwrap();
        assert_eq!(ops, back);
    }

    #[test]
    #[should_panic(expected = "Decay has no inverse")]
    fn inverse_of_decay_panics() {
        let _ = Op::Decay { amount: 1 }.inverse();
    }
}
