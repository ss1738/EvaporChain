//! Trigger conditions — when a [`crate::MementoContract`] may be revealed.

use serde::{Deserialize, Serialize};

/// The five trigger variants. Each holds the parameters the predicate
/// needs to evaluate against the chain state. The chain-state view
/// (block height, owner-last-active-epoch, owner-energy, attester
/// registry) is read from [`crate::ChainObservation`] at reveal time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MementoTrigger {
    /// Classical timelock: reveal permitted once chain block height
    /// ≥ `target`.
    ///
    /// Use case: scheduled disclosure (e.g. press embargo, vesting
    /// announcement at a specific block).
    BlockHeightReached(evaporchain_types::Epoch),

    /// The owner has not been the sender of any transaction for at
    /// least `min_idle_epochs` epochs since [`crate::MementoContract::sealed_at_epoch`].
    ///
    /// Use case: "wallet inactive 5 years → execute will". The
    /// reveal lets a beneficiary decommit the will-payload only
    /// after the owner has demonstrably stopped using the chain.
    /// A single transaction from the owner resets the idle counter.
    OwnerInactiveSince {
        /// Minimum epochs of continuous owner-inactivity required.
        min_idle_epochs: evaporchain_types::Epoch,
    },

    /// The owner's account energy has decayed below `threshold`.
    ///
    /// Use case: thermodynamically-native trigger. Only EvaporChain
    /// can offer this primitive — other chains have no canonical
    /// notion of "account energy". If the owner stops refreshing
    /// their account, their energy decays per `energy_at_epoch`;
    /// once it crosses `threshold`, the memento can be revealed.
    OwnerEnergyBelow {
        /// Energy threshold (in the same units as
        /// `AccountState.energy`).
        threshold: u64,
    },

    /// The owner has explicitly signed a reveal-permission message.
    /// The signed message hash is `BLAKE3("evaporchain-memento-reveal-v1"
    /// || commitment_bytes)`; if the chain witnesses a valid
    /// signature from `owner` over that hash, the memento may
    /// be revealed.
    ///
    /// Use case: owner short-circuits the wait by explicitly
    /// authorising a single reveal. The signature is fresh per
    /// memento (binds the commitment) so it cannot be reused for
    /// other contracts.
    OwnerSignedReveal,

    /// A pre-registered `attester_address` has approved the reveal.
    /// The chain looks up the attester's approval registry and
    /// requires a "yes" entry for this commitment hash.
    ///
    /// Use case: multi-signature-style escape hatch. The contract
    /// creator pre-designates a trusted third party (lawyer,
    /// executor service, oracle) who can approve the reveal under
    /// conditions outside the chain's direct knowledge.
    AttesterApproval {
        /// Account address of the attester whose approval is required.
        attester: evaporchain_types::AccountAddress,
    },
}

/// Structural failures returned by trigger predicates during reveal
/// evaluation. Distinct from [`crate::RevealError`] because these
/// are predicate-internal; the reveal-level error wraps them.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TriggerError {
    /// The chain observation did not include the data needed to
    /// evaluate this trigger (e.g. an `OwnerEnergyBelow` trigger
    /// fired with no `owner_energy` in the observation).
    #[error("trigger requires chain field that was not provided: {0}")]
    MissingChainData(&'static str),
}

impl MementoTrigger {
    /// Evaluate this trigger against a [`crate::ChainObservation`].
    /// Returns `Ok(true)` if the trigger is satisfied, `Ok(false)`
    /// if the trigger is unsatisfied but well-formed, and `Err` if
    /// the observation is missing required fields.
    pub(crate) fn is_satisfied(
        &self,
        sealed_at_epoch: evaporchain_types::Epoch,
        owner: &evaporchain_types::AccountAddress,
        observation: &crate::ChainObservation,
    ) -> Result<bool, TriggerError> {
        match self {
            MementoTrigger::BlockHeightReached(target) => Ok(observation.current_epoch >= *target),

            MementoTrigger::OwnerInactiveSince { min_idle_epochs } => {
                let last_active = observation
                    .owner_last_active_epoch
                    .ok_or(TriggerError::MissingChainData(
                        "owner_last_active_epoch (needed by OwnerInactiveSince)",
                    ))?;
                // Idle window starts at max(sealed_at, last_active) —
                // the contract cannot count idle epochs before it
                // even existed. If the owner sent a tx after sealing,
                // that resets the window to the later activity.
                let window_start = last_active.max(sealed_at_epoch);
                let elapsed = observation.current_epoch.saturating_sub(window_start);
                Ok(elapsed >= *min_idle_epochs)
            }

            MementoTrigger::OwnerEnergyBelow { threshold } => {
                let energy = observation
                    .owner_energy
                    .ok_or(TriggerError::MissingChainData(
                        "owner_energy (needed by OwnerEnergyBelow)",
                    ))?;
                Ok(energy < *threshold)
            }

            MementoTrigger::OwnerSignedReveal => {
                // The signature itself is verified by the chain
                // *before* invoking this trigger; the trigger just
                // checks whether the chain witnessed a valid one.
                let signed = observation
                    .owner_signed_reveal_for
                    .as_ref()
                    .ok_or(TriggerError::MissingChainData(
                        "owner_signed_reveal_for (needed by OwnerSignedReveal)",
                    ))?;
                // Caller passes the commitment they're attempting to
                // reveal. If the signature was for the same commitment
                // and from the owner, accept.
                Ok(signed.signer == *owner)
            }

            MementoTrigger::AttesterApproval { attester } => {
                let approved_by = observation
                    .attester_approvals
                    .iter()
                    .any(|a| a == attester);
                Ok(approved_by)
            }
        }
    }
}
