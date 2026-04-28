//! DecayingDAO — governance contract that owns parameter-change proposals
//! with on-chain bounds, vote-weight cap, quorum, and timelock.
//!
//! This template extends the existing `DAOVote` (which is a poll-only contract)
//! into a real protocol-parameter governance mechanism. It addresses the
//! governance unbounded-params + whale-pass + no-quorum gap flagged at
//! `crates/evaporchain-execution/src/lib.rs:893-958` and documented in
//! `audit/end_to_end_audit_2026_04_27.md` §5.
//!
//! Design notes:
//!
//! * The contract layer cannot directly mutate execution-layer governance
//!   state — the contracts crate is sandboxed. So this template enforces all
//!   guards (bounds, vote-weight cap, quorum, timelock) and exposes a
//!   `list_ready_to_apply` / `mark_applied` lifecycle the execution layer
//!   reads to actually apply approved param changes. The execution-layer
//!   bridge is a separate change.
//! * The decay-native angle: the contract instance itself has thermodynamic
//!   energy (per the `ContractInstance` envelope), so the DAO evaporates if
//!   nobody refreshes it. Active proposals also auto-reject after their
//!   voting window plus a 100-epoch grace if nobody finalizes them, so the
//!   contract self-cleans even if its operators go absent. No other DAO
//!   primitive on any production chain has this property.
//! * Vote weight is capped at `min(balance, stake)`. This is the protocol-
//!   level fix for the whale-pass attack: a token-rich, stake-poor account
//!   cannot dominate governance.
//! * Quorum is computed against `total_stake` (snapshot at init), not against
//!   the number of voters. A quorum of 50% means 50% of stake must have
//!   voted, not 50% of validator count.
//! * Param bounds are set at init and are immutable for the contract's
//!   lifetime. Changing the bounds requires deploying a new DAO instance.
//!   This is intentional — recursive DAO-governs-DAO bounds-changes invite
//!   complexity that v0.1 avoids.
//! * Voting period and timelock are also init-fixed, for the same reason.

use crate::ContractError;
use evaporchain_types::{AccountAddress, Epoch};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ─── State types ────────────────────────────────────────────────────────────

/// Status of a parameter-change proposal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DaoProposalStatus {
    /// Voting in progress.
    Active,
    /// Voting ended; quorum + supermajority both met.
    Passed,
    /// Voting ended; either quorum or supermajority not met.
    Rejected,
    /// Passed and timelock elapsed; the execution layer may consume it.
    ReadyToApply,
    /// Execution layer has consumed and applied this proposal. Terminal.
    Applied,
}

/// A single parameter-change proposal in a DecayingDAO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaoProposal {
    pub id: u64,
    /// Hex-encoded proposer address.
    pub proposer: String,
    pub param_key: String,
    /// All proposed parameter values are u64 in v0.1 — string params and
    /// floats are out of scope for the initial governance surface.
    pub param_value_u64: u64,
    pub start_epoch: u64,
    /// Voting closes (exclusive) at this epoch.
    pub end_epoch: u64,
    /// Sum of capped weights `min(balance, stake)` across all yes-voters.
    pub votes_for: u64,
    /// Sum of capped weights across all no-voters.
    pub votes_against: u64,
    /// Hex addresses of voters; used for one-vote-per-address dedup.
    pub voters: Vec<String>,
    pub status: DaoProposalStatus,
    /// Set when status transitions to Passed; used as the timelock anchor.
    pub passed_at_epoch: Option<u64>,
}

/// State of a DecayingDAO contract instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecayingDaoState {
    pub title: String,
    /// Per-parameter bounds: param_key -> (min, max), inclusive. u64 only.
    pub param_bounds: HashMap<String, (u64, u64)>,
    /// Voting period in epochs.
    pub voting_period_epochs: u64,
    /// Quorum: minimum percent of `total_stake` that must vote (0-100).
    pub quorum_pct: u64,
    /// Epochs between proposal pass and ready-to-apply.
    pub timelock_epochs: u64,
    /// Snapshot of total stake (for quorum). Set at init; immutable in v0.1.
    pub total_stake: u64,
    /// Minimum stake required to submit a proposal (anti-spam).
    pub min_stake_to_propose: u64,
    /// All proposals, in order of creation. Index = proposal_id.
    pub proposals: Vec<DaoProposal>,
    pub last_tick_epoch: u64,
}

// ─── Local JSON helpers (mirror lib.rs's get_str / get_u64) ─────────────────
// Duplicated locally rather than crossing the module boundary; keeps the
// public surface of this module compact.

fn get_str(v: &Value, key: &str) -> Result<String, ContractError> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ContractError::InvalidParams(format!("missing string '{}'", key)))
}

fn get_u64(v: &Value, key: &str) -> Result<u64, ContractError> {
    v.get(key)
        .and_then(|x| x.as_u64())
        .ok_or_else(|| ContractError::InvalidParams(format!("missing u64 '{}'", key)))
}

fn get_bool(v: &Value, key: &str) -> Result<bool, ContractError> {
    v.get(key)
        .and_then(|x| x.as_bool())
        .ok_or_else(|| ContractError::InvalidParams(format!("missing bool '{}'", key)))
}

// ─── Init ────────────────────────────────────────────────────────────────────

/// Build a fresh `DecayingDaoState` from init params.
///
/// Required params (all u64 except `title` and `param_bounds`):
/// * `title` — string
/// * `param_bounds` — object, each entry `{key: [min, max]}`
/// * `voting_period_epochs`
/// * `quorum_pct` — 0..=100
/// * `timelock_epochs`
/// * `total_stake` — > 0
/// * `min_stake_to_propose`
pub fn init(params: &Value, current_epoch: Epoch) -> Result<Value, ContractError> {
    let title = get_str(params, "title")?;
    let voting_period_epochs = get_u64(params, "voting_period_epochs")?;
    let quorum_pct = get_u64(params, "quorum_pct")?;
    let timelock_epochs = get_u64(params, "timelock_epochs")?;
    let total_stake = get_u64(params, "total_stake")?;
    let min_stake_to_propose = get_u64(params, "min_stake_to_propose")?;

    if quorum_pct > 100 {
        return Err(ContractError::InvalidParams(
            "quorum_pct must be 0..=100".into(),
        ));
    }
    if total_stake == 0 {
        return Err(ContractError::InvalidParams(
            "total_stake must be > 0".into(),
        ));
    }
    if voting_period_epochs == 0 {
        return Err(ContractError::InvalidParams(
            "voting_period_epochs must be > 0".into(),
        ));
    }

    let bounds_v = params.get("param_bounds").ok_or_else(|| {
        ContractError::InvalidParams("missing param_bounds".into())
    })?;
    let bounds_obj = bounds_v.as_object().ok_or_else(|| {
        ContractError::InvalidParams("param_bounds must be an object".into())
    })?;
    if bounds_obj.is_empty() {
        return Err(ContractError::InvalidParams(
            "param_bounds must contain at least one entry".into(),
        ));
    }

    let mut param_bounds: HashMap<String, (u64, u64)> = HashMap::new();
    for (key, val) in bounds_obj {
        let pair = val.as_array().ok_or_else(|| {
            ContractError::InvalidParams(format!(
                "param_bounds.{} must be [min, max]", key
            ))
        })?;
        if pair.len() != 2 {
            return Err(ContractError::InvalidParams(format!(
                "param_bounds.{} must be [min, max] (length 2)", key
            )));
        }
        let min = pair[0].as_u64().ok_or_else(|| {
            ContractError::InvalidParams(format!(
                "param_bounds.{}: min must be u64", key
            ))
        })?;
        let max = pair[1].as_u64().ok_or_else(|| {
            ContractError::InvalidParams(format!(
                "param_bounds.{}: max must be u64", key
            ))
        })?;
        if min > max {
            return Err(ContractError::InvalidParams(format!(
                "param_bounds.{}: min ({}) > max ({})", key, min, max
            )));
        }
        param_bounds.insert(key.clone(), (min, max));
    }

    let state = DecayingDaoState {
        title,
        param_bounds,
        voting_period_epochs,
        quorum_pct,
        timelock_epochs,
        total_stake,
        min_stake_to_propose,
        proposals: Vec::new(),
        last_tick_epoch: current_epoch,
    };
    Ok(serde_json::to_value(state).expect("DecayingDaoState serializable"))
}

// ─── Exec ────────────────────────────────────────────────────────────────────

/// Execute a method on a DecayingDAO contract.
///
/// Methods:
/// * `propose(proposer_stake, param_key, param_value_u64)`
/// * `vote(proposal_id, support, balance, stake)`
/// * `finalize(proposal_id)`
/// * `mark_ready_to_apply(proposal_id)`
/// * `mark_applied(proposal_id)`
/// * `get_proposal(proposal_id)`
/// * `list_ready_to_apply()`
/// * `param_bounds()`
pub fn exec(
    state: &mut Value,
    method: &str,
    args: &Value,
    caller: &AccountAddress,
    current_epoch: Epoch,
) -> Result<Value, ContractError> {
    let caller_hex = hex::encode(caller);
    let mut ds: DecayingDaoState = serde_json::from_value(state.clone())
        .map_err(|e| ContractError::StateError(e.to_string()))?;

    let result = match method {
        "propose" => {
            let proposer_stake = get_u64(args, "proposer_stake")?;
            if proposer_stake < ds.min_stake_to_propose {
                return Err(ContractError::PermissionDenied(format!(
                    "proposer stake {} below required {}",
                    proposer_stake, ds.min_stake_to_propose
                )));
            }
            let param_key = get_str(args, "param_key")?;
            let param_value = get_u64(args, "param_value_u64")?;

            let bounds = ds
                .param_bounds
                .get(&param_key)
                .ok_or_else(|| {
                    ContractError::InvalidParams(format!(
                        "param_key '{}' is not bounded by this DAO", param_key
                    ))
                })?;
            if param_value < bounds.0 || param_value > bounds.1 {
                return Err(ContractError::InvalidParams(format!(
                    "param_value {} for '{}' outside bounds [{}, {}]",
                    param_value, param_key, bounds.0, bounds.1
                )));
            }

            let proposal_id = ds.proposals.len() as u64;
            ds.proposals.push(DaoProposal {
                id: proposal_id,
                proposer: caller_hex.clone(),
                param_key,
                param_value_u64: param_value,
                start_epoch: current_epoch,
                end_epoch: current_epoch.saturating_add(ds.voting_period_epochs),
                votes_for: 0,
                votes_against: 0,
                voters: Vec::new(),
                status: DaoProposalStatus::Active,
                passed_at_epoch: None,
            });
            serde_json::json!({ "proposal_id": proposal_id })
        }

        "vote" => {
            let proposal_id = get_u64(args, "proposal_id")? as usize;
            let support = get_bool(args, "support")?;
            let balance = get_u64(args, "balance")?;
            let stake = get_u64(args, "stake")?;
            // Whale-pass mitigation: vote weight is min(balance, stake).
            // A token-rich, stake-poor account cannot dominate governance.
            let weight = balance.min(stake);

            let p = ds.proposals.get_mut(proposal_id).ok_or_else(|| {
                ContractError::StateError(format!("proposal {} not found", proposal_id))
            })?;

            if p.status != DaoProposalStatus::Active {
                return Err(ContractError::StateError(
                    "proposal is not in Active state".into(),
                ));
            }
            if current_epoch >= p.end_epoch {
                return Err(ContractError::StateError(
                    "voting period ended; call finalize".into(),
                ));
            }
            if p.voters.contains(&caller_hex) {
                return Err(ContractError::PermissionDenied(
                    "caller already voted on this proposal".into(),
                ));
            }
            if support {
                p.votes_for = p.votes_for.saturating_add(weight);
            } else {
                p.votes_against = p.votes_against.saturating_add(weight);
            }
            p.voters.push(caller_hex.clone());
            serde_json::json!({
                "proposal_id": proposal_id,
                "weight_counted": weight,
                "support": support,
            })
        }

        "finalize" => {
            let proposal_id = get_u64(args, "proposal_id")? as usize;
            let total_stake = ds.total_stake;
            let quorum_pct = ds.quorum_pct;
            let p = ds.proposals.get_mut(proposal_id).ok_or_else(|| {
                ContractError::StateError(format!("proposal {} not found", proposal_id))
            })?;

            if p.status != DaoProposalStatus::Active {
                return Err(ContractError::StateError(
                    "proposal is not in Active state".into(),
                ));
            }
            if current_epoch < p.end_epoch {
                return Err(ContractError::StateError(format!(
                    "voting period not yet ended (now {}, ends at {})",
                    current_epoch, p.end_epoch
                )));
            }

            let total_weighted = p.votes_for.saturating_add(p.votes_against);
            // Quorum: total_weighted >= total_stake * quorum_pct / 100.
            // total_stake.saturating_mul(quorum_pct) won't overflow for
            // realistic stakes (u64 max / 100 ~= 1.8e17), but we use
            // saturating just in case.
            let quorum_threshold =
                total_stake.saturating_mul(quorum_pct) / 100;
            let quorum_met = total_weighted >= quorum_threshold;

            // Supermajority: votes_for > 2 * votes_against.
            // Equivalent to votes_for / total_weighted > 2/3.
            // saturating_mul guards against absurd input; in practice
            // votes_against <= total_stake, so 2 * votes_against fits u64.
            let supermajority =
                p.votes_for > p.votes_against.saturating_mul(2);

            if quorum_met && supermajority {
                p.status = DaoProposalStatus::Passed;
                p.passed_at_epoch = Some(current_epoch);
                serde_json::json!({
                    "status": "Passed",
                    "votes_for": p.votes_for,
                    "votes_against": p.votes_against,
                    "quorum_threshold": quorum_threshold,
                })
            } else {
                p.status = DaoProposalStatus::Rejected;
                serde_json::json!({
                    "status": "Rejected",
                    "votes_for": p.votes_for,
                    "votes_against": p.votes_against,
                    "quorum_met": quorum_met,
                    "supermajority": supermajority,
                    "quorum_threshold": quorum_threshold,
                })
            }
        }

        "mark_ready_to_apply" => {
            let proposal_id = get_u64(args, "proposal_id")? as usize;
            let timelock_epochs = ds.timelock_epochs;
            let p = ds.proposals.get_mut(proposal_id).ok_or_else(|| {
                ContractError::StateError(format!("proposal {} not found", proposal_id))
            })?;

            if p.status != DaoProposalStatus::Passed {
                return Err(ContractError::StateError(
                    "proposal is not in Passed state".into(),
                ));
            }
            let passed_at = p.passed_at_epoch.ok_or_else(|| {
                ContractError::StateError(
                    "Passed proposal missing passed_at_epoch (invariant violated)".into(),
                )
            })?;
            let ready_at = passed_at.saturating_add(timelock_epochs);
            if current_epoch < ready_at {
                return Err(ContractError::StateError(format!(
                    "timelock not elapsed: now {}, ready at {}",
                    current_epoch, ready_at
                )));
            }
            p.status = DaoProposalStatus::ReadyToApply;
            serde_json::json!({ "status": "ReadyToApply", "proposal_id": proposal_id })
        }

        "mark_applied" => {
            let proposal_id = get_u64(args, "proposal_id")? as usize;
            let p = ds.proposals.get_mut(proposal_id).ok_or_else(|| {
                ContractError::StateError(format!("proposal {} not found", proposal_id))
            })?;
            if p.status != DaoProposalStatus::ReadyToApply {
                return Err(ContractError::StateError(
                    "proposal is not in ReadyToApply state".into(),
                ));
            }
            p.status = DaoProposalStatus::Applied;
            serde_json::json!({
                "status": "Applied",
                "proposal_id": proposal_id,
                "param_key": p.param_key.clone(),
                "param_value_u64": p.param_value_u64,
            })
        }

        "get_proposal" => {
            let proposal_id = get_u64(args, "proposal_id")? as usize;
            let p = ds.proposals.get(proposal_id).ok_or_else(|| {
                ContractError::StateError(format!("proposal {} not found", proposal_id))
            })?;
            serde_json::to_value(p).expect("DaoProposal serializable")
        }

        "list_ready_to_apply" => {
            let ready: Vec<_> = ds
                .proposals
                .iter()
                .filter(|p| p.status == DaoProposalStatus::ReadyToApply)
                .map(|p| {
                    serde_json::json!({
                        "id": p.id,
                        "param_key": p.param_key,
                        "param_value_u64": p.param_value_u64,
                    })
                })
                .collect();
            serde_json::json!({ "ready": ready })
        }

        "param_bounds" => {
            serde_json::to_value(&ds.param_bounds)
                .expect("param_bounds serializable")
        }

        other => return Err(ContractError::UnknownMethod(other.into())),
    };

    *state = serde_json::to_value(ds).expect("DecayingDaoState serializable");
    Ok(result)
}

// ─── Tick ────────────────────────────────────────────────────────────────────

/// Per-epoch tick. Auto-rejects Active proposals whose voting window has
/// elapsed plus a 100-epoch grace, so the DAO self-cleans even if no one
/// calls `finalize`.
pub fn tick(state: &mut Value, current_epoch: Epoch) -> Vec<String> {
    let mut ds: DecayingDaoState = match serde_json::from_value(state.clone()) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut events = Vec::new();
    const FINALIZE_GRACE_EPOCHS: u64 = 100;

    for p in ds.proposals.iter_mut() {
        if p.status == DaoProposalStatus::Active
            && current_epoch
                >= p.end_epoch.saturating_add(FINALIZE_GRACE_EPOCHS)
        {
            p.status = DaoProposalStatus::Rejected;
            events.push(format!(
                "DecayingDAO proposal {} auto-rejected (no finalize after grace)",
                p.id
            ));
        }
    }

    ds.last_tick_epoch = current_epoch;
    *state = serde_json::to_value(ds).expect("DecayingDaoState serializable");
    events
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_init_params() -> Value {
        serde_json::json!({
            "title": "Param Governance",
            "param_bounds": {
                "block_gas_limit": [10_000u64, 100_000_000u64],
                "block_reward":     [1u64,      1_000u64],
            },
            "voting_period_epochs": 100u64,
            "quorum_pct": 50u64,
            "timelock_epochs": 24u64,
            "total_stake": 1_000_000u64,
            "min_stake_to_propose": 10_000u64,
        })
    }

    #[test]
    fn init_succeeds_with_valid_params() {
        let s = init(&default_init_params(), 0).expect("init ok");
        let ds: DecayingDaoState = serde_json::from_value(s).unwrap();
        assert_eq!(ds.quorum_pct, 50);
        assert_eq!(ds.total_stake, 1_000_000);
        assert!(ds.param_bounds.contains_key("block_gas_limit"));
        assert!(ds.proposals.is_empty());
    }

    #[test]
    fn init_rejects_quorum_above_100() {
        let mut p = default_init_params();
        p["quorum_pct"] = serde_json::json!(101u64);
        let r = init(&p, 0);
        assert!(r.is_err(), "expected error for quorum_pct > 100");
    }

    #[test]
    fn init_rejects_zero_total_stake() {
        let mut p = default_init_params();
        p["total_stake"] = serde_json::json!(0u64);
        assert!(init(&p, 0).is_err());
    }

    #[test]
    fn init_rejects_zero_voting_period() {
        let mut p = default_init_params();
        p["voting_period_epochs"] = serde_json::json!(0u64);
        assert!(init(&p, 0).is_err());
    }

    #[test]
    fn init_rejects_empty_param_bounds() {
        let mut p = default_init_params();
        p["param_bounds"] = serde_json::json!({});
        assert!(init(&p, 0).is_err());
    }

    #[test]
    fn init_rejects_inverted_bounds() {
        let mut p = default_init_params();
        p["param_bounds"] = serde_json::json!({ "x": [10u64, 5u64] });
        assert!(init(&p, 0).is_err());
    }

    fn fresh_state() -> Value {
        init(&default_init_params(), 0).unwrap()
    }

    #[test]
    fn propose_succeeds_within_bounds() {
        let mut s = fresh_state();
        let r = exec(
            &mut s,
            "propose",
            &serde_json::json!({
                "proposer_stake": 50_000u64,
                "param_key": "block_gas_limit",
                "param_value_u64": 50_000_000u64,
            }),
            &[1u8; 32],
            10,
        )
        .expect("propose ok");
        assert_eq!(r["proposal_id"].as_u64(), Some(0));
    }

    #[test]
    fn propose_rejects_value_above_max() {
        let mut s = fresh_state();
        let r = exec(
            &mut s,
            "propose",
            &serde_json::json!({
                "proposer_stake": 50_000u64,
                "param_key": "block_gas_limit",
                "param_value_u64": 1_000_000_000u64, // > 100M max
            }),
            &[1u8; 32],
            10,
        );
        assert!(r.is_err(), "expected bounds-violation error");
    }

    #[test]
    fn propose_rejects_unknown_param_key() {
        let mut s = fresh_state();
        let r = exec(
            &mut s,
            "propose",
            &serde_json::json!({
                "proposer_stake": 50_000u64,
                "param_key": "not_governed_by_this_dao",
                "param_value_u64": 42u64,
            }),
            &[1u8; 32],
            10,
        );
        assert!(r.is_err());
    }

    #[test]
    fn propose_rejects_below_min_stake() {
        let mut s = fresh_state();
        let r = exec(
            &mut s,
            "propose",
            &serde_json::json!({
                "proposer_stake": 100u64, // far below 10k min
                "param_key": "block_gas_limit",
                "param_value_u64": 50_000_000u64,
            }),
            &[1u8; 32],
            10,
        );
        assert!(r.is_err());
    }

    fn make_proposal(s: &mut Value, proposer: &AccountAddress, value: u64, epoch: Epoch) {
        exec(
            s,
            "propose",
            &serde_json::json!({
                "proposer_stake": 50_000u64,
                "param_key": "block_gas_limit",
                "param_value_u64": value,
            }),
            proposer,
            epoch,
        )
        .unwrap();
    }

    #[test]
    fn vote_weight_capped_at_min_balance_stake() {
        // Whale-pass mitigation: an account with balance 10^9 but stake 100
        // should only contribute 100 weight, not 10^9.
        let mut s = fresh_state();
        make_proposal(&mut s, &[1u8; 32], 50_000_000, 5);

        let r = exec(
            &mut s,
            "vote",
            &serde_json::json!({
                "proposal_id": 0u64,
                "support": true,
                "balance": 1_000_000_000u64, // huge
                "stake":   100u64,            // small
            }),
            &[2u8; 32],
            10,
        )
        .unwrap();
        assert_eq!(r["weight_counted"].as_u64(), Some(100));

        let ds: DecayingDaoState = serde_json::from_value(s).unwrap();
        assert_eq!(ds.proposals[0].votes_for, 100);
    }

    #[test]
    fn vote_dedups_by_caller() {
        let mut s = fresh_state();
        make_proposal(&mut s, &[1u8; 32], 50_000_000, 5);

        let voter = [2u8; 32];
        let args = serde_json::json!({
            "proposal_id": 0u64,
            "support": true,
            "balance": 1_000u64,
            "stake":   1_000u64,
        });
        exec(&mut s, "vote", &args, &voter, 10).unwrap();
        let r = exec(&mut s, "vote", &args, &voter, 11);
        assert!(r.is_err(), "second vote by same caller must be rejected");
    }

    #[test]
    fn finalize_rejects_when_quorum_not_met() {
        let mut s = fresh_state();
        // total_stake = 1_000_000, quorum_pct = 50 → threshold = 500_000.
        make_proposal(&mut s, &[1u8; 32], 50_000_000, 5);

        // Only 100 weight votes — far below the 500_000 threshold.
        exec(
            &mut s,
            "vote",
            &serde_json::json!({
                "proposal_id": 0u64, "support": true,
                "balance": 100u64, "stake": 100u64,
            }),
            &[2u8; 32],
            10,
        )
        .unwrap();

        // Voting period: 5 + 100 = 105.
        let r = exec(
            &mut s,
            "finalize",
            &serde_json::json!({ "proposal_id": 0u64 }),
            &[1u8; 32],
            110,
        )
        .unwrap();
        assert_eq!(r["status"], "Rejected");
        assert_eq!(r["quorum_met"], false);
    }

    #[test]
    fn finalize_passes_when_quorum_and_supermajority_met() {
        let mut s = fresh_state();
        make_proposal(&mut s, &[1u8; 32], 50_000_000, 5);

        // Three voters each contributing 200_000 yes → 600_000 for, 0 against.
        // total_weighted = 600_000 >= threshold 500_000. Supermajority trivial.
        for i in 0u8..3 {
            let mut a = [0u8; 32];
            a[0] = 0x10 + i;
            exec(
                &mut s,
                "vote",
                &serde_json::json!({
                    "proposal_id": 0u64, "support": true,
                    "balance": 200_000u64, "stake": 200_000u64,
                }),
                &a,
                10 + i as Epoch,
            )
            .unwrap();
        }

        let r = exec(
            &mut s,
            "finalize",
            &serde_json::json!({ "proposal_id": 0u64 }),
            &[1u8; 32],
            110,
        )
        .unwrap();
        assert_eq!(r["status"], "Passed");
    }

    #[test]
    fn finalize_rejects_without_supermajority() {
        let mut s = fresh_state();
        make_proposal(&mut s, &[1u8; 32], 50_000_000, 5);

        // Quorum met but not 2/3 supermajority:
        // 300_000 yes + 300_000 no (50/50). votes_for > votes_against * 2 is false.
        let mut a1 = [0u8; 32]; a1[0] = 0x20;
        let mut a2 = [0u8; 32]; a2[0] = 0x21;
        exec(
            &mut s,
            "vote",
            &serde_json::json!({
                "proposal_id": 0u64, "support": true,
                "balance": 300_000u64, "stake": 300_000u64,
            }),
            &a1,
            10,
        ).unwrap();
        exec(
            &mut s,
            "vote",
            &serde_json::json!({
                "proposal_id": 0u64, "support": false,
                "balance": 300_000u64, "stake": 300_000u64,
            }),
            &a2,
            11,
        ).unwrap();

        let r = exec(
            &mut s,
            "finalize",
            &serde_json::json!({ "proposal_id": 0u64 }),
            &[1u8; 32],
            110,
        ).unwrap();
        assert_eq!(r["status"], "Rejected");
        assert_eq!(r["supermajority"], false);
    }

    #[test]
    fn finalize_rejects_before_voting_ends() {
        let mut s = fresh_state();
        make_proposal(&mut s, &[1u8; 32], 50_000_000, 5);
        let r = exec(
            &mut s,
            "finalize",
            &serde_json::json!({ "proposal_id": 0u64 }),
            &[1u8; 32],
            50, // voting ends at 5 + 100 = 105
        );
        assert!(r.is_err());
    }

    #[test]
    fn timelock_blocks_apply_until_elapsed() {
        let mut s = fresh_state();
        make_proposal(&mut s, &[1u8; 32], 50_000_000, 5);
        // pass it
        for i in 0u8..3 {
            let mut a = [0u8; 32];
            a[0] = 0x30 + i;
            exec(
                &mut s,
                "vote",
                &serde_json::json!({
                    "proposal_id": 0u64, "support": true,
                    "balance": 200_000u64, "stake": 200_000u64,
                }),
                &a,
                10 + i as Epoch,
            )
            .unwrap();
        }
        exec(
            &mut s,
            "finalize",
            &serde_json::json!({ "proposal_id": 0u64 }),
            &[1u8; 32],
            110,
        )
        .unwrap();

        // Try mark_ready_to_apply BEFORE timelock (timelock_epochs=24, passed at 110)
        let r = exec(
            &mut s,
            "mark_ready_to_apply",
            &serde_json::json!({ "proposal_id": 0u64 }),
            &[1u8; 32],
            115, // < 110 + 24
        );
        assert!(r.is_err(), "timelock should block early apply");

        // Now try after timelock
        let r = exec(
            &mut s,
            "mark_ready_to_apply",
            &serde_json::json!({ "proposal_id": 0u64 }),
            &[1u8; 32],
            135, // >= 110 + 24
        )
        .unwrap();
        assert_eq!(r["status"], "ReadyToApply");
    }

    #[test]
    fn lifecycle_full_pass_and_apply() {
        let mut s = fresh_state();
        make_proposal(&mut s, &[1u8; 32], 50_000_000, 5);
        for i in 0u8..3 {
            let mut a = [0u8; 32];
            a[0] = 0x40 + i;
            exec(
                &mut s,
                "vote",
                &serde_json::json!({
                    "proposal_id": 0u64, "support": true,
                    "balance": 200_000u64, "stake": 200_000u64,
                }),
                &a,
                10 + i as Epoch,
            )
            .unwrap();
        }
        exec(
            &mut s,
            "finalize",
            &serde_json::json!({ "proposal_id": 0u64 }),
            &[1u8; 32],
            110,
        )
        .unwrap();
        exec(
            &mut s,
            "mark_ready_to_apply",
            &serde_json::json!({ "proposal_id": 0u64 }),
            &[1u8; 32],
            140,
        )
        .unwrap();

        // list_ready_to_apply should show this one
        let listed = exec(
            &mut s,
            "list_ready_to_apply",
            &serde_json::json!({}),
            &[1u8; 32],
            141,
        )
        .unwrap();
        let arr = listed["ready"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["param_key"], "block_gas_limit");
        assert_eq!(arr[0]["param_value_u64"], 50_000_000u64);

        // mark_applied transitions to Applied
        let applied = exec(
            &mut s,
            "mark_applied",
            &serde_json::json!({ "proposal_id": 0u64 }),
            &[1u8; 32],
            142,
        )
        .unwrap();
        assert_eq!(applied["status"], "Applied");

        // list_ready_to_apply should now be empty
        let listed = exec(
            &mut s,
            "list_ready_to_apply",
            &serde_json::json!({}),
            &[1u8; 32],
            143,
        )
        .unwrap();
        assert_eq!(listed["ready"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn tick_auto_rejects_stale_active_proposals() {
        let mut s = fresh_state();
        make_proposal(&mut s, &[1u8; 32], 50_000_000, 5);
        // Voting ends at 105. Grace is 100 epochs → cutoff at 205.
        let _events = tick(&mut s, 100); // mid-voting: no change
        let ds: DecayingDaoState = serde_json::from_value(s.clone()).unwrap();
        assert_eq!(ds.proposals[0].status, DaoProposalStatus::Active);

        let events = tick(&mut s, 250);
        assert!(!events.is_empty());
        let ds: DecayingDaoState = serde_json::from_value(s).unwrap();
        assert_eq!(ds.proposals[0].status, DaoProposalStatus::Rejected);
    }

    #[test]
    fn unknown_method_rejected() {
        let mut s = fresh_state();
        let r = exec(
            &mut s,
            "frobnicate",
            &serde_json::json!({}),
            &[1u8; 32],
            10,
        );
        match r {
            Err(ContractError::UnknownMethod(m)) => assert_eq!(m, "frobnicate"),
            _ => panic!("expected UnknownMethod"),
        }
    }
}
