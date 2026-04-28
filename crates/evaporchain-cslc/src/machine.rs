//! `EpsilonMachine` — labeled-transition graph over causal states.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

use evaporchain_sanov_slashing::Distribution;

/// Causal-state index. The reconstruction crate maps observed-history
/// equivalence classes to these ids.
pub type StateId = u32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpsilonMachine {
    /// Per-state output distribution over the alphabet.
    pub output: BTreeMap<StateId, Distribution>,
    /// Per-(state, symbol) → next state. Missing entries indicate the
    /// machine has no observation of that transition (caller may
    /// route via `start_state` for resync).
    pub transitions: BTreeMap<(StateId, u32), StateId>,
    /// The "no-history" entry state — the reconstruction's choice for
    /// where prediction begins when no past has been observed.
    pub start_state: StateId,
    /// Alphabet size (number of distinct output symbols).
    pub alphabet_size: u32,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MachineError {
    #[error("state {0} has no output distribution")]
    UnknownState(StateId),
    #[error(
        "transition (state={state}, symbol={symbol}) not in machine"
    )]
    UnknownTransition { state: StateId, symbol: u32 },
    #[error("symbol {0} is outside the machine's alphabet")]
    OutOfAlphabet(u32),
}

impl EpsilonMachine {
    /// Empty machine with the given alphabet size.
    pub fn new(alphabet_size: u32) -> Self {
        Self {
            output: BTreeMap::new(),
            transitions: BTreeMap::new(),
            start_state: 0,
            alphabet_size,
        }
    }

    /// Number of causal states.
    pub fn state_count(&self) -> usize {
        self.output.len()
    }

    /// Add a state with its output distribution. Returns the state id.
    pub fn add_state(&mut self, output: Distribution) -> StateId {
        let id = self.output.len() as StateId;
        self.output.insert(id, output);
        id
    }

    /// Set a transition `(state, symbol) → next`.
    pub fn set_transition(&mut self, state: StateId, symbol: u32, next: StateId) {
        self.transitions.insert((state, symbol), next);
    }

    /// Look up the next state for `(state, symbol)`. Returns `None`
    /// if no transition is known.
    pub fn next_state(&self, state: StateId, symbol: u32) -> Option<StateId> {
        self.transitions.get(&(state, symbol)).copied()
    }

    /// Output distribution for `state`. Errors if state is unknown.
    pub fn output_for(&self, state: StateId) -> Result<&Distribution, MachineError> {
        self.output
            .get(&state)
            .ok_or(MachineError::UnknownState(state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_sanov_slashing::FIXED_POINT_SCALE;

    fn dist(pmf: Vec<u64>) -> Distribution {
        Distribution::new(pmf).unwrap()
    }

    #[test]
    fn empty_machine_zero_states() {
        let m = EpsilonMachine::new(2);
        assert_eq!(m.state_count(), 0);
        assert_eq!(m.alphabet_size, 2);
    }

    #[test]
    fn add_states_and_transitions() {
        let mut m = EpsilonMachine::new(2);
        let s0 = m.add_state(dist(vec![FIXED_POINT_SCALE / 2, FIXED_POINT_SCALE / 2]));
        let s1 = m.add_state(dist(vec![FIXED_POINT_SCALE - 1, 1]));
        m.set_transition(s0, 0, s0);
        m.set_transition(s0, 1, s1);
        assert_eq!(m.state_count(), 2);
        assert_eq!(m.next_state(s0, 1), Some(s1));
        assert_eq!(m.next_state(s1, 0), None);
    }

    #[test]
    fn output_for_unknown_state_errs() {
        let m = EpsilonMachine::new(2);
        assert!(matches!(
            m.output_for(99).unwrap_err(),
            MachineError::UnknownState(99)
        ));
    }
}
