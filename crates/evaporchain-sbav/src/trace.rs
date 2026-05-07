//! Execution trace + the bit-for-bit recoverability claim.
//!
//! A `Trace` records the sequence of *reversible* ops the VM
//! executed. It refuses to record `Decay` (the trace is the
//! reversible *prefix*; once decay happens, the trace is bisected
//! and the post-decay segment must be a fresh trace).
//!
//! The headline property: given a final state and a trace, applying
//! the inverse trace in reverse order yields the original pre-trace
//! state — for the reversible subset.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::op::Op;
use crate::vm::{Sbav, VmError};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TraceError {
    #[error("cannot record Decay in a reversible trace — split the trace at the decay event")]
    DecayInTrace,
    #[error("vm error during trace replay: {0}")]
    Vm(#[from] VmError),
}

/// A reversible execution trace. Append-only; never contains `Decay`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trace {
    ops: Vec<Op>,
}

impl Trace {
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }

    /// Record a reversible op. Errors on `Decay`.
    pub fn record(&mut self, op: Op) -> Result<(), TraceError> {
        if !op.is_reversible() {
            return Err(TraceError::DecayInTrace);
        }
        self.ops.push(op);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Replay the trace forward against `start`, mutating in place.
    /// Returns the final state. Panic-free; only fails on VM errors
    /// (none possible for reversible ops with valid registers).
    pub fn replay_forward(&self, start: Sbav) -> Result<Sbav, TraceError> {
        let mut vm = start;
        for &op in &self.ops {
            vm.apply(op)?;
        }
        Ok(vm)
    }

    /// Apply the *inverse* trace to `final_state`, in reverse order.
    /// Recovers the pre-trace state bit-for-bit. The headline claim of
    /// the crate, expressed as a method.
    pub fn invert(&self, final_state: Sbav) -> Result<Sbav, TraceError> {
        let mut vm = final_state;
        for &op in self.ops.iter().rev() {
            vm.inverse_apply(op)?;
        }
        Ok(vm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_trace_is_identity() {
        let trace = Trace::new();
        let start = Sbav::with_regs([0xAB; 8]);
        let after = trace.replay_forward(start.clone()).unwrap();
        assert_eq!(after, start);
        assert_eq!(trace.invert(start.clone()).unwrap(), start);
    }

    #[test]
    fn record_rejects_decay() {
        let mut trace = Trace::new();
        let err = trace.record(Op::Decay { amount: 1 }).unwrap_err();
        assert_eq!(err, TraceError::DecayInTrace);
        assert!(trace.is_empty());
    }

    #[test]
    fn record_accepts_reversible_ops() {
        let mut trace = Trace::new();
        trace.record(Op::XorReg { reg: 0, key: 0xAB }).unwrap();
        trace.record(Op::AddReg { reg: 1, k: 5 }).unwrap();
        trace.record(Op::Swap { a: 0, b: 1 }).unwrap();
        assert_eq!(trace.len(), 3);
    }

    #[test]
    fn invert_recovers_initial_state_bit_for_bit() {
        // The headline doctrine claim: state at block t is bit-for-bit
        // recoverable from t+k *for the reversible subset*. Here a
        // trace of 5 reversible ops, then we invert and confirm we get
        // the initial state back exactly.
        let initial = Sbav::with_regs([
            0x1111_1111_1111_1111,
            0x2222_2222_2222_2222,
            0x3333_3333_3333_3333,
            0x4444_4444_4444_4444,
            0x5555_5555_5555_5555,
            0x6666_6666_6666_6666,
            0x7777_7777_7777_7777,
            0x8888_8888_8888_8888,
        ]);
        let mut trace = Trace::new();
        for op in [
            Op::XorReg {
                reg: 0,
                key: 0xDEAD_BEEF,
            },
            Op::AddReg { reg: 1, k: 12_345 },
            Op::Rotl { reg: 2, bits: 17 },
            Op::Swap { a: 3, b: 4 },
            Op::Not { reg: 5 },
        ] {
            trace.record(op).unwrap();
        }
        let after = trace.replay_forward(initial.clone()).unwrap();
        assert_ne!(after, initial, "trace should mutate state");
        let recovered = trace.invert(after).unwrap();
        assert_eq!(recovered, initial);
    }

    #[test]
    fn decay_breaks_recoverability() {
        // The other half of the claim: state past a decay event is
        // NOT recoverable. We don't record the decay in the trace, so
        // inverting only restores the pre-decay state. The post-decay
        // entropy_exported counter cannot be undone.
        let initial = Sbav::with_regs([1; 8]);
        let mut trace = Trace::new();
        trace.record(Op::AddReg { reg: 0, k: 5 }).unwrap();
        let mut after = trace.replay_forward(initial.clone()).unwrap();
        // Now decay — NOT recordable in the trace.
        after.apply(Op::Decay { amount: 100 }).unwrap();
        let recovered = trace.invert(after.clone()).unwrap();
        // Reversible portion recovered perfectly.
        assert_eq!(recovered.regs, initial.regs);
        // But entropy_exported survived the inversion — the
        // thermodynamic arrow does not run backward.
        assert_eq!(recovered.entropy_exported, 100);
        assert_eq!(after.entropy_exported, 100);
    }

    #[test]
    fn round_trip_serde() {
        let mut trace = Trace::new();
        trace.record(Op::XorReg { reg: 0, key: 0xAB }).unwrap();
        trace.record(Op::Not { reg: 1 }).unwrap();
        let s = serde_json::to_string(&trace).unwrap();
        let back: Trace = serde_json::from_str(&s).unwrap();
        assert_eq!(trace, back);
    }
}
