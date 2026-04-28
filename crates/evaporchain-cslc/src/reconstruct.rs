//! Single-state baseline reconstruction.
//!
//! Builds an `EpsilonMachine` with **one** causal state — i.e. the
//! minimal machine that just emits the unconditional symbol
//! distribution. This is what CSSR converges to when no statistically-
//! significant past-history dependencies are detected.
//!
//! It is the *floor* of the ε-machine model class:
//!
//! - Provably optimal for memoryless processes.
//! - A safe-and-correct stand-in for general processes until full
//!   CSSR is wired in (the ε-machine equivalence relation simply
//!   collapses everything to the single past-equivalence class
//!   `~ all`, which is sound but not minimal for non-memoryless
//!   processes).
//!
//! Future commit replaces this with the Shalizi-Klinkner CSSR
//! algorithm; downstream consumers reading `EpsilonMachine` are
//! unaffected by the swap.

use evaporchain_sanov_slashing::{Distribution, DistributionError};

use crate::machine::EpsilonMachine;

/// Build the single-state ε-machine from raw symbol counts.
/// `counts[i]` is the observed frequency of symbol `i`.
pub fn reconstruct_unconditional(counts: &[u64]) -> Result<EpsilonMachine, DistributionError> {
    let alphabet_size = counts.len() as u32;
    let dist = Distribution::from_counts(counts)?;
    let mut m = EpsilonMachine::new(alphabet_size);
    let s0 = m.add_state(dist);
    // Self-loop on every symbol — single-state machine.
    for sym in 0..alphabet_size {
        m.set_transition(s0, sym, s0);
    }
    m.start_state = s0;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_counts_rejected() {
        assert!(reconstruct_unconditional(&[]).is_err());
    }

    #[test]
    fn fair_coin_single_state_uniform_output() {
        let m = reconstruct_unconditional(&[500, 500]).unwrap();
        assert_eq!(m.state_count(), 1);
        assert_eq!(m.alphabet_size, 2);
        let out = m.output_for(0).unwrap();
        assert_eq!(out.pmf, vec![500_000, 500_000]);
        // Self-loops on every symbol.
        assert_eq!(m.next_state(0, 0), Some(0));
        assert_eq!(m.next_state(0, 1), Some(0));
    }

    #[test]
    fn skewed_counts_skewed_output() {
        let m = reconstruct_unconditional(&[800, 200]).unwrap();
        let out = m.output_for(0).unwrap();
        assert_eq!(out.pmf[0], 800_000);
        assert_eq!(out.pmf[1], 200_000);
    }
}
