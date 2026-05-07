//! V2 vote gate — IB vote wrapped in jail + energy filters.

use evaporchain_ib_validators::{ib_vote, IbParams, IbVote, StateSignature};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::jail::{JailEntry, JailReason, JailState, ValidatorId};

/// Outcome of the V2 vote gate. Same `Commit` / `Abstain` cases as
/// V1, plus a `Jailed` rejection that names the reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoteV2 {
    Commit,
    Abstain,
    Jailed { reason: JailReason },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VoteV2Error {
    #[error("energy_floor must be > 0 — set to 1 to admit any positive energy")]
    ZeroFloor,
}

/// V2 vote gate. Returns:
///
/// - `Jailed{reason}` if the validator is in `jail_state` at
///   `current_epoch` (active jail entry).
/// - `Jailed{EnergyBelowFloor}` if `energy < energy_floor`. The
///   energy check is *not* memoised in `jail_state` — callers
///   that want persistent jailing on energy decay should call
///   `apply_energy_jail` first to write the entry.
/// - Otherwise: V1 `ib_vote(local, prior, params)`'s `Commit` or
///   `Abstain` lifted into `VoteV2::Commit` / `VoteV2::Abstain`.
pub fn ib_vote_v2(
    local_sig: &StateSignature,
    prior_sig: &StateSignature,
    params: &IbParams,
    validator_id: &ValidatorId,
    energy: u64,
    energy_floor: u64,
    jail_state: &JailState,
    current_epoch: u64,
) -> Result<VoteV2, VoteV2Error> {
    if energy_floor == 0 {
        return Err(VoteV2Error::ZeroFloor);
    }

    if let Some(entry) = jail_state.get(validator_id) {
        if current_epoch < entry.expires_at_epoch {
            return Ok(VoteV2::Jailed {
                reason: entry.reason,
            });
        }
    }

    if energy < energy_floor {
        return Ok(VoteV2::Jailed {
            reason: JailReason::EnergyBelowFloor {
                observed: energy,
                floor: energy_floor,
            },
        });
    }

    Ok(match ib_vote(local_sig, prior_sig, params) {
        IbVote::Commit => VoteV2::Commit,
        IbVote::Abstain => VoteV2::Abstain,
    })
}

/// Idempotent helper: write an `EnergyBelowFloor` jail entry for
/// `validator_id` if `energy < energy_floor`. Does nothing
/// otherwise. Useful for the chain to memoise the jail across
/// epochs once a validator has decayed below the floor.
pub fn apply_energy_jail(
    jail_state: &mut JailState,
    validator_id: ValidatorId,
    energy: u64,
    energy_floor: u64,
    expires_at_epoch: u64,
) -> bool {
    if energy < energy_floor {
        jail_state.insert(
            validator_id,
            JailEntry {
                reason: JailReason::EnergyBelowFloor {
                    observed: energy,
                    floor: energy_floor,
                },
                expires_at_epoch,
            },
        );
        true
    } else {
        false
    }
}

/// Mass-jail every validator in `participants` with a CHSH-failed
/// reason. Jail expiry = `current_epoch + jail_epochs`.
pub fn apply_chsh_failure_jail(
    jail_state: &mut JailState,
    participants: &[ValidatorId],
    window_start: u64,
    window_end: u64,
    current_epoch: u64,
    jail_epochs: u64,
) {
    let expires = current_epoch.saturating_add(jail_epochs);
    for v in participants {
        jail_state.insert(
            *v,
            JailEntry {
                reason: JailReason::ChshFailedWindow {
                    window_start,
                    window_end,
                },
                expires_at_epoch: expires,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> ValidatorId {
        [b; 32]
    }

    fn high_kl_local() -> StateSignature {
        // All accounts in the lowest energy bucket → mass concentrated
        // at bin 0, distinct from the uniform prior → high KL.
        let energies = vec![0u64; 16];
        StateSignature::from_energies(&energies, 1024)
    }

    fn prior_sig() -> StateSignature {
        // Uniform energies across the [0, scale) range → roughly even
        // bin distribution.
        let energies: Vec<u64> = (0..16).map(|i| i as u64 * 64).collect();
        StateSignature::from_energies(&energies, 1024)
    }

    fn params() -> IbParams {
        IbParams { lambda_mb: 100 } // small threshold so high-KL local commits
    }

    // ── shape errors ─────────────────────────────────────────────

    #[test]
    fn zero_floor_rejected() {
        let r = ib_vote_v2(
            &high_kl_local(),
            &prior_sig(),
            &params(),
            &id(1),
            1000,
            0,
            &JailState::new(),
            0,
        );
        assert_eq!(r.unwrap_err(), VoteV2Error::ZeroFloor);
    }

    // ── happy path: no jail, energy above floor ──────────────────

    #[test]
    fn unjailed_high_kl_commits() {
        let r = ib_vote_v2(
            &high_kl_local(),
            &prior_sig(),
            &params(),
            &id(1),
            1000,
            10,
            &JailState::new(),
            0,
        )
        .unwrap();
        assert_eq!(r, VoteV2::Commit);
    }

    #[test]
    fn unjailed_zero_kl_abstains() {
        // local == prior → KL = 0 → abstain.
        let p = prior_sig();
        let r = ib_vote_v2(&p, &p, &params(), &id(1), 1000, 10, &JailState::new(), 0).unwrap();
        assert_eq!(r, VoteV2::Abstain);
    }

    // ── jail reasons ─────────────────────────────────────────────

    #[test]
    fn jailed_validator_returns_jailed_reason() {
        let mut js = JailState::new();
        js.insert(
            id(1),
            JailEntry {
                reason: JailReason::Slashed { code: 7 },
                expires_at_epoch: 100,
            },
        );
        let r = ib_vote_v2(
            &high_kl_local(),
            &prior_sig(),
            &params(),
            &id(1),
            1000,
            10,
            &js,
            50,
        )
        .unwrap();
        assert!(matches!(
            r,
            VoteV2::Jailed {
                reason: JailReason::Slashed { code: 7 }
            }
        ));
    }

    #[test]
    fn jail_expires_at_expiry_epoch() {
        let mut js = JailState::new();
        js.insert(
            id(1),
            JailEntry {
                reason: JailReason::Slashed { code: 0 },
                expires_at_epoch: 100,
            },
        );
        // At epoch 100 (exclusive), no longer jailed → V1 vote applies.
        let r = ib_vote_v2(
            &high_kl_local(),
            &prior_sig(),
            &params(),
            &id(1),
            1000,
            10,
            &js,
            100,
        )
        .unwrap();
        assert_eq!(r, VoteV2::Commit);
    }

    #[test]
    fn energy_below_floor_returns_jailed() {
        let r = ib_vote_v2(
            &high_kl_local(),
            &prior_sig(),
            &params(),
            &id(1),
            5,
            10,
            &JailState::new(),
            0,
        )
        .unwrap();
        assert!(matches!(
            r,
            VoteV2::Jailed {
                reason: JailReason::EnergyBelowFloor {
                    observed: 5,
                    floor: 10
                }
            }
        ));
    }

    // ── jail-state mutators ──────────────────────────────────────

    #[test]
    fn apply_energy_jail_writes_when_below_floor() {
        let mut js = JailState::new();
        let did_jail = apply_energy_jail(&mut js, id(1), 5, 10, 200);
        assert!(did_jail);
        assert!(js.is_jailed(&id(1), 50));
    }

    #[test]
    fn apply_energy_jail_idempotent_when_above_floor() {
        let mut js = JailState::new();
        let did_jail = apply_energy_jail(&mut js, id(1), 100, 10, 200);
        assert!(!did_jail);
        assert_eq!(js.len(), 0);
    }

    #[test]
    fn apply_chsh_failure_jail_marks_all_participants() {
        let mut js = JailState::new();
        apply_chsh_failure_jail(&mut js, &[id(1), id(2), id(3)], 100, 200, 0, 50);
        for i in 1u8..=3u8 {
            assert!(js.is_jailed(&id(i), 25));
            assert!(js.is_jailed(&id(i), 49));
            assert!(!js.is_jailed(&id(i), 50));
        }
    }

    #[test]
    fn chsh_jail_blocks_subsequent_vote() {
        let mut js = JailState::new();
        apply_chsh_failure_jail(&mut js, &[id(1)], 100, 200, 0, 50);
        let r = ib_vote_v2(
            &high_kl_local(),
            &prior_sig(),
            &params(),
            &id(1),
            1000,
            10,
            &js,
            10,
        )
        .unwrap();
        assert!(matches!(
            r,
            VoteV2::Jailed {
                reason: JailReason::ChshFailedWindow {
                    window_start: 100,
                    window_end: 200,
                }
            }
        ));
    }

    // ── press claim ──────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "IB Validators V2 wraps the V1 IB vote gate with a
        // structural jail layer. A validator that was active during
        // a CHSH-failed window cannot vote until jail_epochs elapse;
        // a validator below energy_floor cannot vote until refresh;
        // explicit slashes block voting with a typed code. Outside
        // the jail set, the vote outcome is identical to V1.
        // JailState is BTreeMap-canonical and expiry is deterministic
        // on epoch."

        let mut js = JailState::new();

        // (1) CHSH jail blocks vote for the whole jail window.
        apply_chsh_failure_jail(&mut js, &[id(1)], 100, 200, 0, 50);
        let v_jailed = ib_vote_v2(
            &high_kl_local(),
            &prior_sig(),
            &params(),
            &id(1),
            1000,
            10,
            &js,
            10,
        )
        .unwrap();
        assert!(matches!(v_jailed, VoteV2::Jailed { .. }));

        // (2) After jail expires, V1 vote applies.
        let v_free = ib_vote_v2(
            &high_kl_local(),
            &prior_sig(),
            &params(),
            &id(1),
            1000,
            10,
            &js,
            50,
        )
        .unwrap();
        assert_eq!(v_free, VoteV2::Commit);

        // (3) Energy floor jails on the spot, no jail-state mutation
        // required.
        let v_low_energy = ib_vote_v2(
            &high_kl_local(),
            &prior_sig(),
            &params(),
            &id(2),
            5,
            10,
            &JailState::new(),
            0,
        )
        .unwrap();
        assert!(matches!(
            v_low_energy,
            VoteV2::Jailed {
                reason: JailReason::EnergyBelowFloor { .. }
            }
        ));

        // (4) Jail expiry pruning is deterministic.
        let pruned = js.prune_expired(50);
        assert!(pruned >= 1);
    }
}
