//! `predict_next` — given a causal state on an `EpsilonMachine`,
//! return the predicted next-symbol distribution.

use evaporchain_sanov_slashing::Distribution;

use crate::machine::{EpsilonMachine, MachineError, StateId};

/// Predicted next-symbol distribution at `state`. Forwards
/// [`MachineError::UnknownState`] if `state` is not in the machine.
pub fn predict_next<'a>(
    machine: &'a EpsilonMachine,
    state: StateId,
) -> Result<&'a Distribution, MachineError> {
    machine.output_for(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconstruct::reconstruct_unconditional;

    #[test]
    fn predict_at_start_state_matches_unconditional() {
        let m = reconstruct_unconditional(&[300, 700]).unwrap();
        let d = predict_next(&m, m.start_state).unwrap();
        assert_eq!(d.pmf, vec![300_000, 700_000]);
    }

    #[test]
    fn predict_unknown_state_errs() {
        let m = reconstruct_unconditional(&[1, 1]).unwrap();
        assert!(matches!(
            predict_next(&m, 99).unwrap_err(),
            MachineError::UnknownState(99)
        ));
    }
}
