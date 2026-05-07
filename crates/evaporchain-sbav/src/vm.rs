//! Tiny stack-machine that interprets [`Op`].
//!
//! The state is an 8-register file plus a running `entropy_exported`
//! counter (sum of all `Decay.amount` events). The reversible subset
//! has the property:
//!
//! ```text
//!     for every reversible op O,
//!         apply(state, O.inverse()) ∘ apply(state, O) == state
//! ```
//!
//! Decay is not reversible — applying its (non-existent) inverse is a
//! type error. The VM exposes `entropy_exported()` so callers can
//! verify the asymmetry: classical ops contribute zero; only `Decay`
//! grows the counter.

use evaporchain_types::Energy;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::op::{Op, OpError};
use crate::reversible::Reversible;

const REG_COUNT: usize = 8;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VmError {
    #[error("op-level error: {0}")]
    Op(#[from] OpError),
    #[error("attempted to inverse-apply Decay (which is irreversible)")]
    InverseOfIrreversible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sbav {
    /// 8-register file; u64 each.
    pub regs: [u64; REG_COUNT],
    /// Running sum of all `Decay.amount` events. The chain's
    /// thermodynamic arrow.
    pub entropy_exported: Energy,
}

impl Default for Sbav {
    fn default() -> Self {
        Self::new()
    }
}

impl Sbav {
    pub fn new() -> Self {
        Self {
            regs: [0; REG_COUNT],
            entropy_exported: 0,
        }
    }

    pub fn with_regs(regs: [u64; REG_COUNT]) -> Self {
        Self {
            regs,
            entropy_exported: 0,
        }
    }

    /// Apply an op. Mutates `self` in place.
    pub fn apply(&mut self, op: Op) -> Result<(), VmError> {
        match op {
            Op::XorReg { reg, key } => {
                self.check_reg(reg)?;
                self.regs[reg as usize] ^= key;
            }
            Op::AddReg { reg, k } => {
                self.check_reg(reg)?;
                self.regs[reg as usize] = self.regs[reg as usize].wrapping_add(k);
            }
            Op::SubReg { reg, k } => {
                self.check_reg(reg)?;
                self.regs[reg as usize] = self.regs[reg as usize].wrapping_sub(k);
            }
            Op::Swap { a, b } => {
                self.check_reg(a)?;
                self.check_reg(b)?;
                self.regs.swap(a as usize, b as usize);
            }
            Op::Not { reg } => {
                self.check_reg(reg)?;
                self.regs[reg as usize] = !self.regs[reg as usize];
            }
            Op::Rotl { reg, bits } => {
                self.check_reg(reg)?;
                self.regs[reg as usize] = self.regs[reg as usize].rotate_left(bits);
            }
            Op::Rotr { reg, bits } => {
                self.check_reg(reg)?;
                self.regs[reg as usize] = self.regs[reg as usize].rotate_right(bits);
            }
            Op::Decay { amount } => {
                if amount == 0 {
                    return Err(OpError::ZeroDecayAmount.into());
                }
                self.entropy_exported = self.entropy_exported.saturating_add(amount);
            }
        }
        Ok(())
    }

    /// Apply the *inverse* of a reversible op. Errors if the caller
    /// passes `Decay` — the asymmetry is enforced at the API level.
    pub fn inverse_apply(&mut self, op: Op) -> Result<(), VmError> {
        if !op.is_reversible() {
            return Err(VmError::InverseOfIrreversible);
        }
        self.apply(op.inverse())
    }

    fn check_reg(&self, reg: u8) -> Result<(), OpError> {
        if (reg as usize) >= REG_COUNT {
            return Err(OpError::InvalidRegister(reg));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_round_trip_recovers_state() {
        let mut vm = Sbav::with_regs([0xAB; 8]);
        let snap = vm.clone();
        let op = Op::XorReg {
            reg: 3,
            key: 0xDEAD_BEEF,
        };
        vm.apply(op).unwrap();
        assert_ne!(vm, snap);
        vm.inverse_apply(op).unwrap();
        assert_eq!(vm, snap);
    }

    #[test]
    fn add_sub_round_trip() {
        let mut vm = Sbav::new();
        let snap = vm.clone();
        let op = Op::AddReg { reg: 0, k: 1234 };
        vm.apply(op).unwrap();
        vm.inverse_apply(op).unwrap();
        assert_eq!(vm, snap);
    }

    #[test]
    fn swap_round_trip() {
        let mut vm = Sbav::new();
        vm.regs[0] = 1;
        vm.regs[1] = 2;
        let snap = vm.clone();
        let op = Op::Swap { a: 0, b: 1 };
        vm.apply(op).unwrap();
        vm.inverse_apply(op).unwrap();
        assert_eq!(vm, snap);
    }

    #[test]
    fn rotl_rotr_round_trip() {
        let mut vm = Sbav::with_regs([0x1234_5678_9ABC_DEF0; 8]);
        let snap = vm.clone();
        let op = Op::Rotl { reg: 4, bits: 13 };
        vm.apply(op).unwrap();
        vm.inverse_apply(op).unwrap();
        assert_eq!(vm, snap);
    }

    #[test]
    fn not_is_self_inverse() {
        let mut vm = Sbav::with_regs([0xAB; 8]);
        let snap = vm.clone();
        let op = Op::Not { reg: 0 };
        vm.apply(op).unwrap();
        vm.apply(op).unwrap(); // applying Not twice == identity (its own inverse)
        assert_eq!(vm, snap);
    }

    #[test]
    fn classical_ops_export_zero_entropy() {
        let mut vm = Sbav::new();
        for op in [
            Op::XorReg { reg: 0, key: 1 },
            Op::AddReg { reg: 0, k: 5 },
            Op::Swap { a: 0, b: 1 },
            Op::Not { reg: 2 },
            Op::Rotl { reg: 0, bits: 1 },
        ] {
            vm.apply(op).unwrap();
        }
        // Zero. The thermodynamic arrow has not advanced.
        assert_eq!(vm.entropy_exported, 0);
    }

    #[test]
    fn decay_advances_entropy_exported() {
        let mut vm = Sbav::new();
        vm.apply(Op::Decay { amount: 100 }).unwrap();
        vm.apply(Op::Decay { amount: 50 }).unwrap();
        assert_eq!(vm.entropy_exported, 150);
    }

    #[test]
    fn decay_zero_amount_rejected() {
        let mut vm = Sbav::new();
        let err = vm.apply(Op::Decay { amount: 0 }).unwrap_err();
        assert!(matches!(err, VmError::Op(OpError::ZeroDecayAmount)));
    }

    #[test]
    fn inverse_apply_of_decay_errors() {
        // The asymmetry made operational: caller cannot inverse Decay.
        let mut vm = Sbav::new();
        let err = vm.inverse_apply(Op::Decay { amount: 1 }).unwrap_err();
        assert_eq!(err, VmError::InverseOfIrreversible);
    }

    #[test]
    fn invalid_register_rejected() {
        let mut vm = Sbav::new();
        let err = vm.apply(Op::AddReg { reg: 99, k: 1 }).unwrap_err();
        assert!(matches!(err, VmError::Op(OpError::InvalidRegister(99))));
    }

    #[test]
    fn round_trip_serde() {
        let mut vm = Sbav::with_regs([1, 2, 3, 4, 5, 6, 7, 8]);
        vm.apply(Op::Decay { amount: 42 }).unwrap();
        let s = serde_json::to_string(&vm).unwrap();
        let back: Sbav = serde_json::from_str(&s).unwrap();
        assert_eq!(vm, back);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// For every reversible op, applying then inverse-applying
        /// yields the identity. This is the foundational claim of the
        /// crate, expressed as a property test over the operand space.
        #[test]
        fn reversible_round_trip_is_identity(
            init in prop::array::uniform8(any::<u64>()),
            reg in 0u8..8,
            k in any::<u64>(),
        ) {
            for op in [
                Op::XorReg { reg, key: k },
                Op::AddReg { reg, k },
                Op::SubReg { reg, k },
                Op::Not { reg },
                Op::Rotl { reg, bits: (k % 64) as u32 },
                Op::Rotr { reg, bits: (k % 64) as u32 },
            ] {
                let mut vm = Sbav::with_regs(init);
                let snap = vm.clone();
                vm.apply(op).unwrap();
                vm.inverse_apply(op).unwrap();
                prop_assert_eq!(vm, snap, "op {:?} round-trip failed", op);
            }
        }

        /// Classical ops never grow `entropy_exported`. Only `Decay`
        /// does. This is the Landauer claim made operational.
        #[test]
        fn entropy_exported_grows_only_via_decay(
            init in prop::array::uniform8(any::<u64>()),
            ops in prop::collection::vec(0u8..6, 0..32),
            reg in 0u8..8,
        ) {
            let mut vm = Sbav::with_regs(init);
            for code in ops {
                let op = match code {
                    0 => Op::XorReg { reg, key: 0xCD },
                    1 => Op::AddReg { reg, k: 7 },
                    2 => Op::SubReg { reg, k: 7 },
                    3 => Op::Not { reg },
                    4 => Op::Rotl { reg, bits: 3 },
                    _ => Op::Rotr { reg, bits: 3 },
                };
                vm.apply(op).unwrap();
            }
            prop_assert_eq!(vm.entropy_exported, 0);
        }
    }
}
