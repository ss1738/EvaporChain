//! Reveal-side logic. The opposite of [`crate::seal`]: given a
//! sealed [`crate::MementoContract`] + the off-chain
//! [`crate::MementoOpening`] (payload + nonce) + a chain-state
//! observation, decide whether the reveal is permitted.

use serde::{Deserialize, Serialize};

use crate::commitment::{MementoCommitment, MementoContract, MementoVersion};
use crate::trigger::TriggerError;

/// Witness that the reveal claimant submits. Bundles the decommitment
/// inputs the chain needs to verify the seal AND to evaluate the
/// trigger predicate (some triggers need an explicit signed-by-owner
/// witness — see [`crate::MementoTrigger::OwnerSignedReveal`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MementoReveal {
    /// The original payload the contract was sealed over.
    pub payload: Vec<u8>,
    /// The 32-byte nonce the contract was sealed with.
    pub nonce: [u8; 32],
}

/// A snapshot of the chain-state fields a reveal evaluation needs.
///
/// The crate intentionally takes this as a struct rather than a
/// `&dyn ChainView` trait so it stays standalone-testable and doesn't
/// pull a dependency on `evaporchain-state`. The chain-side caller
/// fills the relevant fields from its own observability layer before
/// invoking [`try_reveal`].
///
/// Fields are `Option<_>` where a trigger may or may not require them;
/// only the trigger variants that need them will read them. A missing
/// field for a needed trigger surfaces as [`TriggerError::MissingChainData`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChainObservation {
    /// Current epoch / block height — required by every trigger.
    pub current_epoch: evaporchain_types::Epoch,
    /// Last epoch the contract's owner was the sender of any tx,
    /// needed by [`crate::MementoTrigger::OwnerInactiveSince`].
    pub owner_last_active_epoch: Option<evaporchain_types::Epoch>,
    /// Current energy of the contract's owner account,
    /// needed by [`crate::MementoTrigger::OwnerEnergyBelow`].
    pub owner_energy: Option<u64>,
    /// If the owner signed a reveal-permission message for THIS
    /// memento, this carries the (signer, commitment) witness the
    /// chain verified. Needed by [`crate::MementoTrigger::OwnerSignedReveal`].
    pub owner_signed_reveal_for: Option<OwnerSignedRevealWitness>,
    /// List of attester addresses that have approved the reveal,
    /// needed by [`crate::MementoTrigger::AttesterApproval`].
    pub attester_approvals: Vec<evaporchain_types::AccountAddress>,
}

/// Witness for [`crate::MementoTrigger::OwnerSignedReveal`].
/// Contains the signer's address; the chain has already verified
/// the signature against the commitment's domain-separated hash
/// `BLAKE3("evaporchain-memento-reveal-v1" || commitment_bytes)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerSignedRevealWitness {
    /// The address that signed the reveal-permission message.
    /// Must match [`crate::MementoContract::owner`].
    pub signer: evaporchain_types::AccountAddress,
}

/// Errors the reveal pipeline surfaces. Distinguishes "structurally
/// well-formed but not permitted" (NotYet, CommitmentMismatch) from
/// "missing chain data" (TriggerData).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RevealError {
    /// The witness `(payload, nonce)` does not decommit to the
    /// contract's commitment. Either the wrong payload or the wrong
    /// nonce, or both.
    #[error("commitment mismatch — payload/nonce does not match the sealed commitment")]
    CommitmentMismatch,

    /// The contract's trigger predicate is well-formed but not yet
    /// satisfied. Callers should retry after the relevant chain
    /// state changes (more epochs, owner inactivity, energy decay,
    /// signature received, attester approval received).
    #[error("trigger not yet satisfied — reveal must wait")]
    TriggerNotSatisfied,

    /// The trigger predicate needs chain data that the observation
    /// didn't include. This is a caller bug — the chain-side wrapper
    /// should populate the relevant fields before invoking
    /// [`try_reveal`].
    #[error(transparent)]
    TriggerData(#[from] TriggerError),

    /// Unsupported wire-format version (e.g. v2 contract presented
    /// to a v1 verifier).
    #[error("unsupported memento version: {0:?}")]
    UnsupportedVersion(MementoVersion),
}

/// Attempt to reveal a [`MementoContract`].
///
/// Two checks, in order:
///
/// 1. **Commitment check**: `BLAKE3(version || len(payload) ||
///    payload || nonce)` must equal `contract.commitment`. If not,
///    fail with [`RevealError::CommitmentMismatch`] — this rejects
///    forged decommitments without leaking which prefix matched
///    (constant-time via [`MementoCommitment::ct_eq`]).
///
/// 2. **Trigger check**: the contract's trigger predicate must
///    evaluate to `true` against the supplied [`ChainObservation`].
///    If the predicate is well-formed but unsatisfied, fail with
///    [`RevealError::TriggerNotSatisfied`]; if the predicate needs
///    chain data the observation didn't include, fail with
///    [`RevealError::TriggerData`].
///
/// On success returns the (now-decommitted) payload bytes for the
/// caller to consume.
pub fn try_reveal(
    contract: &MementoContract,
    reveal: &MementoReveal,
    observation: &ChainObservation,
) -> Result<Vec<u8>, RevealError> {
    // Step 0 — version compatibility.
    if contract.version != MementoVersion::V1 {
        return Err(RevealError::UnsupportedVersion(contract.version));
    }

    // Step 1 — commitment binding.
    let computed = MementoCommitment::compute(contract.version, &reveal.payload, &reveal.nonce);
    if !computed.ct_eq(&contract.commitment) {
        return Err(RevealError::CommitmentMismatch);
    }

    // Step 2 — trigger predicate.
    let satisfied = contract
        .trigger
        .is_satisfied(contract.sealed_at_epoch, &contract.owner, observation)?;
    if !satisfied {
        return Err(RevealError::TriggerNotSatisfied);
    }

    Ok(reveal.payload.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commitment::seal;
    use crate::trigger::MementoTrigger;

    const OWNER: evaporchain_types::AccountAddress = [0xAB; 32];

    /// Happy path: BlockHeightReached fires at the right epoch.
    #[test]
    fn block_height_trigger_fires_after_target() {
        let (contract, opening) = seal(
            b"sealed payload".to_vec(),
            [7u8; 32],
            MementoTrigger::BlockHeightReached(100),
            OWNER,
            50,
        );
        let reveal = MementoReveal { payload: opening.payload.clone(), nonce: opening.nonce };

        // Before target: trigger unsatisfied.
        let before = ChainObservation { current_epoch: 99, ..Default::default() };
        assert_eq!(
            try_reveal(&contract, &reveal, &before),
            Err(RevealError::TriggerNotSatisfied)
        );

        // At target: trigger fires.
        let at = ChainObservation { current_epoch: 100, ..Default::default() };
        assert_eq!(try_reveal(&contract, &reveal, &at), Ok(opening.payload));
    }

    /// Tampered payload at reveal time fails with CommitmentMismatch,
    /// NOT TriggerNotSatisfied. This pins the check ordering: forgery
    /// detection happens BEFORE trigger evaluation, so an attacker
    /// can't probe the trigger predicate by trying many fake payloads.
    #[test]
    fn commitment_check_runs_before_trigger() {
        let (contract, opening) = seal(
            b"real payload".to_vec(),
            [1u8; 32],
            // Trigger that's permanently satisfied (block 0 reached).
            MementoTrigger::BlockHeightReached(0),
            OWNER,
            0,
        );
        let tampered = MementoReveal {
            payload: b"forged payload".to_vec(),
            nonce: opening.nonce,
        };
        let obs = ChainObservation { current_epoch: 1_000, ..Default::default() };
        assert_eq!(
            try_reveal(&contract, &tampered, &obs),
            Err(RevealError::CommitmentMismatch)
        );
    }

    /// OwnerInactiveSince: idle window starts at max(sealed_at,
    /// last_active). Owner activity AFTER sealing resets the window.
    #[test]
    fn inactive_since_resets_on_owner_activity_after_sealing() {
        let (contract, opening) = seal(
            b"will".to_vec(),
            [9u8; 32],
            MementoTrigger::OwnerInactiveSince { min_idle_epochs: 50 },
            OWNER,
            /* sealed_at_epoch */ 100,
        );
        let reveal = MementoReveal { payload: opening.payload, nonce: opening.nonce };

        // Owner last active at epoch 130 (AFTER sealing at 100).
        // current_epoch=170 → elapsed since last active = 40 → < 50.
        let obs_too_soon = ChainObservation {
            current_epoch: 170,
            owner_last_active_epoch: Some(130),
            ..Default::default()
        };
        assert_eq!(
            try_reveal(&contract, &reveal, &obs_too_soon),
            Err(RevealError::TriggerNotSatisfied)
        );

        // Owner last active at 130, current_epoch=180 → elapsed=50.
        // Now the trigger fires.
        let obs_ok = ChainObservation {
            current_epoch: 180,
            owner_last_active_epoch: Some(130),
            ..Default::default()
        };
        assert!(try_reveal(&contract, &reveal, &obs_ok).is_ok());
    }

    /// OwnerInactiveSince: if last_active is BEFORE sealing, the
    /// window starts at sealed_at — the owner can't claim idle time
    /// from before the contract existed.
    #[test]
    fn inactive_since_window_starts_at_sealing_not_earlier_activity() {
        let (contract, opening) = seal(
            b"will".to_vec(),
            [3u8; 32],
            MementoTrigger::OwnerInactiveSince { min_idle_epochs: 50 },
            OWNER,
            100, // sealed at epoch 100
        );
        let reveal = MementoReveal { payload: opening.payload, nonce: opening.nonce };

        // Owner was last active at epoch 50 (pre-sealing).
        // current_epoch=149 → window started at sealed_at=100, elapsed=49 → < 50.
        let obs = ChainObservation {
            current_epoch: 149,
            owner_last_active_epoch: Some(50),
            ..Default::default()
        };
        assert_eq!(
            try_reveal(&contract, &reveal, &obs),
            Err(RevealError::TriggerNotSatisfied)
        );

        // current_epoch=150 → elapsed=50 → fires.
        let obs_ok = ChainObservation {
            current_epoch: 150,
            owner_last_active_epoch: Some(50),
            ..Default::default()
        };
        assert!(try_reveal(&contract, &reveal, &obs_ok).is_ok());
    }

    /// OwnerEnergyBelow: thermodynamically-native trigger.
    #[test]
    fn energy_below_trigger_fires_when_owner_energy_decays() {
        let (contract, opening) = seal(
            b"thermo-locked secret".to_vec(),
            [11u8; 32],
            MementoTrigger::OwnerEnergyBelow { threshold: 1000 },
            OWNER,
            0,
        );
        let reveal = MementoReveal { payload: opening.payload, nonce: opening.nonce };

        // Above threshold → unsatisfied.
        let obs_above = ChainObservation {
            current_epoch: 100,
            owner_energy: Some(1500),
            ..Default::default()
        };
        assert_eq!(
            try_reveal(&contract, &reveal, &obs_above),
            Err(RevealError::TriggerNotSatisfied)
        );

        // At threshold → still unsatisfied (strict <).
        let obs_at = ChainObservation {
            current_epoch: 100,
            owner_energy: Some(1000),
            ..Default::default()
        };
        assert_eq!(
            try_reveal(&contract, &reveal, &obs_at),
            Err(RevealError::TriggerNotSatisfied)
        );

        // Below → fires.
        let obs_below = ChainObservation {
            current_epoch: 100,
            owner_energy: Some(999),
            ..Default::default()
        };
        assert!(try_reveal(&contract, &reveal, &obs_below).is_ok());
    }

    /// Missing chain data for OwnerEnergyBelow surfaces as
    /// TriggerData(MissingChainData), not TriggerNotSatisfied.
    /// Different error semantics: this is a caller bug (forgot to
    /// populate the observation), not a not-yet-permitted reveal.
    #[test]
    fn missing_owner_energy_surfaces_as_trigger_data_error() {
        let (contract, opening) = seal(
            b"x".to_vec(),
            [0u8; 32],
            MementoTrigger::OwnerEnergyBelow { threshold: 100 },
            OWNER,
            0,
        );
        let reveal = MementoReveal { payload: opening.payload, nonce: opening.nonce };
        let obs = ChainObservation {
            current_epoch: 1,
            owner_energy: None, // ← caller forgot to populate
            ..Default::default()
        };
        match try_reveal(&contract, &reveal, &obs) {
            Err(RevealError::TriggerData(TriggerError::MissingChainData(_))) => (),
            other => panic!("expected MissingChainData, got {:?}", other),
        }
    }

    /// OwnerSignedReveal: the chain witnesses a valid signature from
    /// the owner over this commitment's reveal-permission hash.
    #[test]
    fn owner_signed_reveal_fires_when_chain_witnesses_signature() {
        let (contract, opening) = seal(
            b"x".to_vec(),
            [0u8; 32],
            MementoTrigger::OwnerSignedReveal,
            OWNER,
            0,
        );
        let reveal = MementoReveal { payload: opening.payload, nonce: opening.nonce };

        // No signature witnessed → trigger needs the field → caller
        // bug surfaces as TriggerData.
        let obs_none = ChainObservation::default();
        assert!(matches!(
            try_reveal(&contract, &reveal, &obs_none),
            Err(RevealError::TriggerData(_))
        ));

        // Wrong signer → not yet satisfied.
        let obs_wrong = ChainObservation {
            owner_signed_reveal_for: Some(OwnerSignedRevealWitness {
                signer: [0xFF; 32], // not OWNER
            }),
            ..Default::default()
        };
        assert_eq!(
            try_reveal(&contract, &reveal, &obs_wrong),
            Err(RevealError::TriggerNotSatisfied)
        );

        // Correct signer → fires.
        let obs_ok = ChainObservation {
            owner_signed_reveal_for: Some(OwnerSignedRevealWitness { signer: OWNER }),
            ..Default::default()
        };
        assert!(try_reveal(&contract, &reveal, &obs_ok).is_ok());
    }

    /// AttesterApproval: pre-registered attester must appear in the
    /// approvals list.
    #[test]
    fn attester_approval_fires_when_listed() {
        let attester: evaporchain_types::AccountAddress = [0xC0; 32];
        let other: evaporchain_types::AccountAddress = [0xD0; 32];
        let (contract, opening) = seal(
            b"escape hatch".to_vec(),
            [4u8; 32],
            MementoTrigger::AttesterApproval { attester },
            OWNER,
            0,
        );
        let reveal = MementoReveal { payload: opening.payload, nonce: opening.nonce };

        // Empty approvals → unsatisfied.
        let obs_empty = ChainObservation::default();
        assert_eq!(
            try_reveal(&contract, &reveal, &obs_empty),
            Err(RevealError::TriggerNotSatisfied)
        );

        // Wrong attester only → unsatisfied.
        let obs_wrong = ChainObservation {
            attester_approvals: vec![other],
            ..Default::default()
        };
        assert_eq!(
            try_reveal(&contract, &reveal, &obs_wrong),
            Err(RevealError::TriggerNotSatisfied)
        );

        // Correct attester listed (along with others) → fires.
        let obs_ok = ChainObservation {
            attester_approvals: vec![other, attester],
            ..Default::default()
        };
        assert!(try_reveal(&contract, &reveal, &obs_ok).is_ok());
    }

    /// Forgery resistance via BLAKE3: an attacker who knows the
    /// commitment but not the nonce cannot find a payload that
    /// decommits. The check returns CommitmentMismatch for every
    /// guess.
    #[test]
    fn commitment_is_binding_under_blake3() {
        let (contract, _opening) = seal(
            b"real".to_vec(),
            [42u8; 32],
            MementoTrigger::BlockHeightReached(0),
            OWNER,
            0,
        );
        let obs = ChainObservation { current_epoch: 10, ..Default::default() };

        // Attacker tries 10 random payloads + nonces. None should
        // decommit (probability 10/2^256 ≈ 0).
        for i in 0..10u8 {
            let attempt = MementoReveal {
                payload: vec![i; 4],
                nonce: [i; 32],
            };
            assert_eq!(
                try_reveal(&contract, &attempt, &obs),
                Err(RevealError::CommitmentMismatch),
                "attempt {i} should not decommit"
            );
        }
    }
}
