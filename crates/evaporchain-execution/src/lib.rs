#[cfg(test)]
mod audit_tests;
pub mod block_stm;
pub mod boltzmann_stake_integration;
pub mod demurrage_integration;
pub mod economics;
pub mod emission;
pub mod energy_audit;
pub mod fees;
#[cfg(test)]
mod four_act_integration_tests;
pub mod genesis;
pub mod genesis_invariant;
pub mod lamport_integration;
pub mod lyapunov_fees;
pub mod mera_integration;
pub mod parallel;
pub mod privacy_exec;
pub mod refresh_market_integration;
pub mod rewards;
pub mod sanov_slash_helpers;
pub mod temporal;

use evaporchain_contracts::{ContractEngine, ContractTemplate};
use evaporchain_crypto::signatures::{HybridVerifier, Verifier};
use evaporchain_crypto::MerkleMountainRange;
use evaporchain_proving::evaporation_proof::{
    EvaporationClaim, EvaporationProof, EvaporationProver,
};
use evaporchain_script::ScriptEngine;
use evaporchain_state::db::StateDB;
use evaporchain_state::{EvaporationEngine, RefreshEngine};
use evaporchain_types::{
    Block, CallContractTx, CallScriptTx, ClaimDelegationTx, CreateObjectTx, DelegateTx,
    DelegationRecord, DeployContractTx, DeployScriptTx, Epoch, GovernanceAction,
    GovernanceProposal, GovernanceTx, MultiSigTx, ObjectState, ProposalStatus, RefreshTx, RefundTx,
    StakeRecord, StateObject, Transaction, TransferTx, UndelegateTx, ValidatorClaimStakeTx,
    ValidatorExitTx, ValidatorStakeTx,
};
use thiserror::Error;
use tracing::{debug, info};

/// Errors that can occur during transaction execution.
#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("insufficient balance: account {account} has {available}, needs {required}")]
    InsufficientBalance {
        account: String,
        available: u64,
        required: u64,
    },
    #[error("invalid nonce: expected {expected}, got {got}")]
    InvalidNonce { expected: u64, got: u64 },
    #[error("object already exists: {0}")]
    ObjectAlreadyExists(String),
    #[error("object not found: {0}")]
    ObjectNotFound(String),
    #[error("self-transfer not allowed")]
    SelfTransfer,
    #[error("zero amount transfer")]
    ZeroAmount,
    #[error("refresh failed: {0}")]
    RefreshFailed(String),
    #[error("invalid signature")]
    InvalidSignature,
    #[error("missing signature")]
    MissingSignature,
    #[error("insufficient balance for gas: account {account} has {available}, needs {required} for fees")]
    InsufficientGas {
        account: String,
        available: u64,
        required: u64,
    },
    #[error("contract error: {0}")]
    ContractError(String),
    #[error("script error: {0}")]
    ScriptError(String),
    #[error("block gas limit exceeded: used {used}, limit {limit}")]
    BlockGasLimitExceeded { used: u64, limit: u64 },
    #[error("call depth exceeded: max {0}")]
    CallDepthExceeded(usize),
    /// Conservation invariant violated and governance flag
    /// `conservation_enforcement = "enforce"` is on. Block is rejected.
    /// When the flag is unset or `"observe"`, audit verdicts are
    /// stored on the executor's `last_conservation_audit` instead and
    /// this error is never raised.
    #[error("conservation invariant violated: {0:?}")]
    ConservationViolation(evaporchain_energy_kernel::ConservationViolation),
}

/// Apply the `conservation_enforcement` governance gate to a kernel
/// audit verdict and return what `execute_block` should do next:
///
/// | `audit_verdict` | `must_enforce` | Result |
/// |---|---|---|
/// | `Ok(())` | any | `Ok(Ok(()))` — store success on the executor, commit |
/// | `Err(v)` | `false` (observe) | `Ok(Err(v))` — store err on the executor, commit |
/// | `Err(v)` | `true` (enforce) | `Err(ExecutionError::ConservationViolation(v))` — reject |
///
/// Centralising the branching here lets us unit-test the gate
/// without a state-poisoning shim; integration with `execute_block`
/// is just a `?` away from the call site (Layer 0 item 1 follow-up
/// per the conservation gate's negative-case test gap).
pub fn evaluate_conservation_gate(
    audit_verdict: Result<(), evaporchain_energy_kernel::ConservationViolation>,
    must_enforce: bool,
) -> Result<Result<(), evaporchain_energy_kernel::ConservationViolation>, ExecutionError> {
    match audit_verdict {
        Ok(()) => Ok(Ok(())),
        Err(violation) if must_enforce => Err(ExecutionError::ConservationViolation(violation)),
        Err(violation) => Ok(Err(violation)),
    }
}

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_energy_kernel::ConservationViolation;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "evaluate_conservation_gate centralises the
    /// observe-vs-enforce branching for the energy-conservation
    /// audit. (a) Honest audit verdict (Ok) returns Ok(Ok(())) under
    /// either policy. (b) Violation under observe-mode returns
    /// Ok(Err(v)) so the executor records the err and commits. (c)
    /// Violation under enforce-mode returns Err(ExecutionError::
    /// ConservationViolation) so the block is rejected."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // (a) Honest verdict — both modes pass.
        let ok = evaluate_conservation_gate(Ok(()), false).unwrap();
        assert!(ok.is_ok());
        let ok2 = evaluate_conservation_gate(Ok(()), true).unwrap();
        assert!(ok2.is_ok());

        // (b) Violation in observe mode — returns Ok(Err(v)).
        let observe = evaluate_conservation_gate(
            Err(ConservationViolation::RedirectChangedTotal {
                before: 100,
                after: 90,
            }),
            false,
        )
        .unwrap();
        assert!(matches!(
            observe,
            Err(ConservationViolation::RedirectChangedTotal { .. })
        ));

        // (c) Violation in enforce mode — propagates as ExecutionError.
        let enforce_err = evaluate_conservation_gate(
            Err(ConservationViolation::RedirectChangedTotal {
                before: 100,
                after: 90,
            }),
            true,
        )
        .unwrap_err();
        assert!(matches!(
            enforce_err,
            ExecutionError::ConservationViolation(_)
        ));
    }
}

/// Contract event emitted during block execution, tagged with origin.
#[derive(Debug, Clone)]
pub struct BlockContractEvent {
    pub contract_id: u64,
    pub tx_index: usize,
    pub event: evaporchain_script::ContractEvent,
}

/// Per-transaction execution outcome surfaced through `BlockExecutionResult`.
///
/// Populated by every `ExecutionEngine::execute_block` implementation so the
/// node API layer can report accurate per-tx status (`success` / `rejected`)
/// instead of unconditionally claiming success. Closes the
/// "BlockRecord.status hardcoded" reporting bug uncovered during the 3-node
/// faucet smoke run.
#[derive(Debug, Clone)]
pub struct TxOutcome {
    /// Canonical BLAKE3 transaction hash (matches `Transaction::tx_hash`).
    pub tx_hash: [u8; 32],
    /// True iff the transaction's primary effect committed (no rollback).
    pub success: bool,
    /// Error message when `success == false`. None on success.
    pub error: Option<String>,
    /// Gas accounted to this transaction (deducted whether or not the
    /// primary effect committed — fees burn on revert).
    pub gas_used: u64,
}

/// Result of executing a single block.
#[derive(Debug)]
pub struct BlockExecutionResult {
    pub state_root: [u8; 32],
    pub mmr_root: [u8; 32],
    pub txs_executed: usize,
    pub txs_failed: usize,
    pub objects_entered_grace: usize,
    pub objects_evaporated: usize,
    /// Total gas used by all executed transactions in this block.
    pub gas_used: u64,
    /// Base fee that was active during this block.
    pub base_fee: u64,
    /// Structured contract events emitted during this block.
    pub contract_events: Vec<BlockContractEvent>,
    /// Total fees collected (gas fees + creation deposits + refresh fees).
    pub total_fees: u64,
    /// Batch evaporation proof for this block (None if no evaporations).
    pub evaporation_proof: Option<EvaporationProof>,
    /// Number of cross-shard messages executed in this block.
    pub cross_shard_processed: usize,
    /// Receipts from cross-shard message execution.
    pub cross_shard_receipts: Vec<evaporchain_sharding::cross_shard::CrossShardReceipt>,
    /// Validator BLS key rotations committed in this block. The consensus
    /// layer applies these to its live `ValidatorSet` at block-commit time
    /// (after the state root is finalized) so signature verification of
    /// subsequent blocks uses the new keys with old keys honoured during
    /// the grace window. Closes punch-list 4b cross-layer wiring.
    pub validator_key_rotations: Vec<ValidatorKeyRotation>,
    /// MERA (Multi-scale Entanglement Renormalization Ansatz) state commitment.
    /// 32-byte root hash of the λ-parameterised tensor-network tree built over
    /// all account energies after this block.  None if the MERA tree could not
    /// be computed (empty state).
    pub mera_commitment: Option<[u8; 32]>,
    /// Per-transaction execution outcomes, in block.transactions order.
    /// Empty when the executor predates the outcomes wiring (legacy path —
    /// the node layer logs a warn and falls back to the prior behaviour).
    pub tx_outcomes: Vec<TxOutcome>,
}

/// Side-effect emitted by `Transaction::RotateValidatorKey` execution and
/// applied by the consensus layer post-commit.
///
/// The consensus layer is responsible for one final verification step
/// before applying: PoP-verify `bls_pop_old` against the validator's
/// *currently-recorded* `bls_public_key` (continuity-of-control proof).
/// This step lives in consensus rather than execution because the live
/// `ValidatorSet` is consensus-owned.
#[derive(Debug, Clone)]
pub struct ValidatorKeyRotation {
    pub validator_id: u64,
    pub new_bls_public_key: Vec<u8>,
    /// **Rotation-continuity** proof: signature over `new_bls_public_key`
    /// bytes by the OLD secret key, under the dedicated rotation DST
    /// (`evaporchain_crypto::signatures::BlsVerifier::verify_rotation_continuity`).
    /// Distinct from a generic PoP of the old key — committing to a
    /// specific `new_bls_public_key` prevents replay across rotation
    /// attempts. Closes punch-list 19. Consensus-side verifier MUST use
    /// `verify_rotation_continuity(old_pk, &new_bls_public_key, &bls_pop_old)`
    /// rather than `verify_proof_of_possession(old_pk, ...)`.
    pub bls_pop_old: Vec<u8>,
    /// Standard proof-of-possession of the NEW key (sig over new_pk under
    /// POP DST). Already verified in the execution layer via
    /// `BlsVerifier::verify_proof_of_possession`.
    pub new_bls_pop: Vec<u8>,
    /// Last epoch at which the prev pubkey is still accepted by
    /// `verify_commit_certificate`.
    pub prev_key_expiry_epoch: u64,
}

/// Trait for block/transaction execution engines.
pub trait ExecutionEngine: Send + Sync {
    /// Execute all transactions in a block, returning the execution result.
    fn execute_block(
        &mut self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<BlockExecutionResult, ExecutionError>;

    /// Current MMR root (evaporation nullifier accumulator).
    fn mmr_root(&self) -> [u8; 32];

    /// Number of nullifiers in the MMR.
    fn mmr_size(&self) -> usize;

    /// Generate an inclusion proof for the leaf at `leaf_index` in the
    /// evaporation-nullifier MMR. Returns `None` if `leaf_index` is past
    /// the current MMR head. Light clients use this to verify that a
    /// specific object's evaporation was actually accumulated, without
    /// downloading the full MMR.
    fn mmr_proof(&self, leaf_index: u64) -> Option<evaporchain_crypto::accumulator::MMRProof>;
}

/// Gas cost constants for transaction types.
pub const GAS_TRANSFER: u64 = 21_000;
pub const GAS_CREATE_OBJECT_BASE: u64 = 50_000;
pub const GAS_CREATE_OBJECT_PER_BYTE: u64 = 200;
pub const GAS_REFRESH: u64 = 30_000;
pub const GAS_DEPLOY_CONTRACT: u64 = 100_000;
pub const GAS_CALL_CONTRACT: u64 = 40_000;
pub const GAS_DEPLOY_SCRIPT: u64 = 150_000;
pub const GAS_CALL_SCRIPT: u64 = 50_000;
pub const GAS_VALIDATOR_STAKE: u64 = 50_000;
pub const GAS_VALIDATOR_EXIT: u64 = 30_000;
pub const GAS_VALIDATOR_CLAIM_STAKE: u64 = 30_000;
pub const GAS_GOVERNANCE: u64 = 25_000;
pub const GAS_MULTISIG: u64 = 50_000;
pub const GAS_USER_OP: u64 = 30_000;
pub const GAS_UPGRADE_CONTRACT: u64 = 100_000;
pub const GAS_DELEGATE: u64 = 40_000;
pub const GAS_UNDELEGATE: u64 = 40_000;
pub const GAS_CLAIM_DELEGATION: u64 = 30_000;
/// Crooks-MEV Phase 3.1 — protocol-issued refund tx. Two account
/// touches (debit + credit). Set low because the proposer (not a
/// user) pays — high gas would discourage proposers from settling
/// observations they're contractually obligated to settle.
pub const GAS_REFUND: u64 = 5_000;
/// BLS key rotation: covers two PoP-style verifications (old + new) plus
/// the validator-set update. Higher than stake/exit because of the
/// double signature check.
pub const GAS_ROTATE_VALIDATOR_KEY: u64 = 80_000;

/// Unbonding period: validators must wait this many epochs after exit before claiming stake.
const UNBONDING_PERIOD_EPOCHS: u64 = 256;
/// BLS key rotation grace window: the previous pubkey remains valid for
/// signature verification this many epochs after a rotation commits.
/// Sized to cover one Tendermint round-trip across a globally distributed
/// validator set plus a comfortable safety margin.
///
/// Public so the node binary can use it for `bls_key.{N}.bin` ring
/// purging and operator tooling can reference the same value.
pub const KEY_ROTATION_GRACE_EPOCHS: u64 = 8;

// ─────────────── Governance bounds (Gap-A #4) ────────────────────────────
// Closes Gap-A #4 from end_to_end_audit_2026_04_27.md: param-range bounds,
// quorum requirement, vote-weight cap, optional timelock.

/// Per-voter vote-weight cap. Caps any single voter's contribution so a
/// whale (e.g. the 35 % Foundation Treasury entry in genesis-mainnet.json)
/// cannot pass proposals solo. Tuned for the 1B total-supply mainnet.
const MAX_VOTE_WEIGHT: u64 = 10_000_000;
/// Minimum total weighted votes (for + against) for a proposal to be
/// eligible to pass. Forces a proposal to be seen by the network.
const QUORUM_MIN_TOTAL_WEIGHT: u64 = 30_000_000; // 3% of 1B mainnet supply
/// Minimum number of distinct voters for a proposal to be eligible to pass.
const QUORUM_MIN_VOTERS: usize = 3;
/// Pass threshold: votes_for must exceed this multiple of votes_against.
const PASS_THRESHOLD_MULTIPLIER: u64 = 2;
/// Minimum and maximum proposal voting window (in epochs).
const MIN_VOTING_EPOCHS: u64 = 10;
const MAX_VOTING_EPOCHS: u64 = 100_000;
/// Maximum proposal title length (bytes). DoS guard.
const MAX_PROPOSAL_TITLE_BYTES: usize = 200;
/// Maximum param_key length (bytes).
const MAX_PARAM_KEY_BYTES: usize = 64;
/// Maximum param_value length (bytes).
const MAX_PARAM_VALUE_BYTES: usize = 256;
/// Timelock between a proposal reaching `Passed` and the parameter
/// becoming effective. Gives stakeholders a window to react / exit.
pub const GOVERNANCE_TIMELOCK_EPOCHS: u64 = 5;

/// Allowlist of governable parameter keys. Anything not on this list
/// (or the `upgrade_contract:{id}` pattern) is rejected at CreateProposal
/// admission so a malicious proposer cannot stamp arbitrary keys into
/// state and trick callers downstream.
const GOVERNABLE_PARAM_KEYS: &[&str] = &[
    "block_gas_limit",
    "base_fee_floor",
    "base_fee_ceiling",
    "target_gas_utilization",
];

fn is_governable_param_key(key: &str) -> bool {
    GOVERNABLE_PARAM_KEYS.contains(&key) || key.starts_with("upgrade_contract:")
}

/// Decide a proposal's outcome under the Gap-A #4 rules:
/// - Quorum: total weighted votes >= QUORUM_MIN_TOTAL_WEIGHT and at least
///   QUORUM_MIN_VOTERS distinct voters.
/// - Super-majority: votes_for > votes_against * PASS_THRESHOLD_MULTIPLIER.
fn decide_proposal_outcome(proposal: &GovernanceProposal) -> ProposalStatus {
    let total_weight = proposal.votes_for.saturating_add(proposal.votes_against);
    if total_weight < QUORUM_MIN_TOTAL_WEIGHT || proposal.voters.len() < QUORUM_MIN_VOTERS {
        return ProposalStatus::Rejected;
    }
    if proposal.votes_for
        > proposal
            .votes_against
            .saturating_mul(PASS_THRESHOLD_MULTIPLIER)
    {
        ProposalStatus::Passed
    } else {
        ProposalStatus::Rejected
    }
}

/// Validate that `param_value` is parseable / in-range for `param_key`.
fn validate_param_value(key: &str, value: &str) -> Result<(), String> {
    match key {
        "block_gas_limit" => value
            .parse::<u64>()
            .map_err(|_| "block_gas_limit must be a non-negative integer".to_string())
            .and_then(|v| {
                if (1_000..=10_000_000_000).contains(&v) {
                    Ok(())
                } else {
                    Err(format!(
                        "block_gas_limit out of range [1_000, 10_000_000_000]: {}",
                        v
                    ))
                }
            }),
        "base_fee_floor" | "base_fee_ceiling" => value
            .parse::<u64>()
            .map_err(|_| format!("{} must be a non-negative integer", key))
            .map(|_| ()),
        "target_gas_utilization" => value
            .parse::<f64>()
            .map_err(|_| "target_gas_utilization must be a float".to_string())
            .and_then(|v| {
                if (0.0..=1.0).contains(&v) {
                    Ok(())
                } else {
                    Err(format!(
                        "target_gas_utilization out of range [0.0, 1.0]: {}",
                        v
                    ))
                }
            }),
        k if k.starts_with("upgrade_contract:") => {
            if value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()) {
                Ok(())
            } else {
                Err("upgrade_contract value must be 64-char hex (blake3 of new bytecode)".into())
            }
        }
        _ => Err(format!(
            "param_key '{}' is not on the governable allowlist",
            key
        )),
    }
}

/// Number of state snapshots to retain before pruning older ones.
const SNAPSHOT_RETAIN_BLOCKS: u64 = 256;

pub(crate) const MAX_CALL_DEPTH: usize = 64;
pub(crate) const MAX_BLOB_SIZE: usize = 128 * 1024;

/// M-19: Cross-block execution cache keyed on (tx_hash, pre_state_root).
/// Caches idempotent read-only results to skip re-execution of identical txs
/// across blocks (e.g., repeated CallScript with same inputs and state).
pub struct ExecutionCache {
    entries: std::collections::HashMap<[u8; 32], CachedResult>,
    max_size: usize,
}

struct CachedResult {
    gas_used: u64,
    success: bool,
    last_used_height: u64,
}

impl ExecutionCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            max_size,
        }
    }

    fn cache_key(tx_hash: &[u8; 32], pre_state_root: &[u8; 32]) -> [u8; 32] {
        let mut data = Vec::with_capacity(64);
        data.extend_from_slice(tx_hash);
        data.extend_from_slice(pre_state_root);
        *blake3::hash(&data).as_bytes()
    }

    pub fn get(&self, tx_hash: &[u8; 32], pre_state_root: &[u8; 32]) -> Option<(u64, bool)> {
        let key = Self::cache_key(tx_hash, pre_state_root);
        self.entries.get(&key).map(|r| (r.gas_used, r.success))
    }

    pub fn put(
        &mut self,
        tx_hash: &[u8; 32],
        pre_state_root: &[u8; 32],
        gas_used: u64,
        success: bool,
        height: u64,
    ) {
        if self.entries.len() >= self.max_size {
            // Evict oldest entry
            if let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.last_used_height)
                .map(|(k, _)| *k)
            {
                self.entries.remove(&oldest_key);
            }
        }
        let key = Self::cache_key(tx_hash, pre_state_root);
        self.entries.insert(
            key,
            CachedResult {
                gas_used,
                success,
                last_used_height: height,
            },
        );
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// M-21: Sort transactions by estimated gas cost descending.
/// Higher-gas txs (contract deploys, scripts) get priority over simple transfers,
/// maximising block value when gas limit is enforced.
pub fn sort_txs_by_gas_priority(txs: &mut [Transaction]) {
    txs.sort_by(|a, b| SimpleExecutor::estimate_gas(b).cmp(&SimpleExecutor::estimate_gas(a)));
}

/// Simple executor that processes transactions sequentially and runs
/// evaporation at the end of each block.
pub struct SimpleExecutor {
    evaporation_engine: EvaporationEngine,
    mmr: MerkleMountainRange,
    verify_signatures: bool,
    fee_controller: Option<fees::PidFeeController>,
    /// Gas limit per block (0 = unlimited). Transactions exceeding this limit are skipped.
    pub block_gas_limit: u64,
    /// Smart contract engine (template-based).
    pub contract_engine: ContractEngine,
    /// EvaporScript engine (script-based).
    pub script_engine: ScriptEngine,
    /// Zero-knowledge privacy execution engine.
    pub privacy_executor: privacy_exec::PrivacyExecutor,
    /// Deferred transaction queue (temporal execution).
    pub deferred_queue: temporal::DeferredQueue,
    /// Decay watcher engine (energy threshold triggers).
    pub decay_watchers: temporal::DecayWatcherEngine,
    /// Pending structured events from script calls (drained per block).
    pending_events: Vec<(u64, evaporchain_script::ContractEvent)>,
    /// Reward accumulator for block rewards and fee distribution.
    pub reward_accumulator: Option<rewards::RewardAccumulator>,
    /// Chain ID for signing message domain separation (cross-chain replay protection).
    pub chain_id: String,
    /// Current contract call depth (guards against unbounded re-entrancy).
    call_depth: usize,
    /// The chain's small-deaths memorial. Every account that fully
    /// evaporates (storage rent zeros it out) is recorded here.
    /// Append-only — the deliberate exception to §2.2 of the doctrine.
    pub eulogy_trie: evaporchain_tombstone::EulogyTrie,
    /// Protocol-owned refresh pool. Storage rent + slash settlement +
    /// MEV burn flow into here under the system namespace, then pay
    /// out via `RedirectKind::RefreshPayout` for namespace keep-alive.
    ///
    /// Per INVENTION_STACK.md §1.2 conservation invariant: energy is
    /// never destroyed, only redirected.
    pub refresh_pool: evaporchain_energy_kernel::RefreshPool,
    /// Mortis monitor — tracks consecutive epochs where refresh_pool
    /// is at-or-below `condition.refresh_pool_floor`. When the run
    /// hits `condition.sustained_epochs`, the chain auto-mints its
    /// death certificate (the "final death" act). Chain ticks via
    /// `tick_mortis(current_epoch)` per block.
    pub mortis_monitor: evaporchain_mortis::MortisMonitor,
    /// The death certificate, once Mortis triggers. Latched: never
    /// reset. Light clients read this field to know the chain has died.
    pub mortis_certificate: Option<evaporchain_mortis::MortisCertificate>,
    /// Singh-Lyapunov fee state. Lives alongside the existing PID
    /// fee_controller — chain governance flips between them. Advanced
    /// per-block via `tick_lyapunov_fee_state`.
    pub lyapunov_fee_state: evaporchain_fee_controller::FeeState,
    /// Singh-Lyapunov fee params (snapshot of the chain-global λ +
    /// targets). Genesis defaults from `lyapunov_fees::default_params`.
    pub lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams,
    /// Per-block conservation audit verdict — populated by
    /// `execute_block` from `energy_audit::audit_block_step`. Stored
    /// for observability; gating is governance-controlled via
    /// `conservation_enforcement` (see `ExecutionError::ConservationViolation`).
    /// `None` until the first block runs.
    pub last_conservation_audit:
        Option<Result<(), evaporchain_energy_kernel::ConservationViolation>>,
    /// Block.epoch of the previous successful audit. Drives the
    /// `epochs_elapsed` argument fed to `energy_at_epoch` so the
    /// kernel's λ-decay floor matches the actual elapsed time between
    /// audits, not a `saturating_sub(0)` proxy on storage-rent epoch.
    /// `None` until the first audit runs.
    pub last_audit_epoch: Option<u64>,
    /// Last Cμ-Gate verdict (Shalizi-Crutchfield identity Cμ ≤ E + hμ).
    /// Populated when the chain calls `record_cmu_observation`. Pure
    /// observability — governance can promote to consensus-rejection later.
    pub last_cmu_verdict: Option<evaporchain_cmu_gate::Verdict>,
    /// Last TUR Liveness verdict (Var(J)/⟨J⟩² ≥ 2/Σ).
    /// Populated when the chain calls `record_tur_observation`. Same
    /// observability pattern as `last_cmu_verdict`.
    pub last_tur_verdict: Option<evaporchain_tur_liveness::Verdict>,
    /// Native demurrage parameters.  Controls the piecewise log-rate charged
    /// on idle balances above the threshold; sink is `refresh_pool`.
    pub demurrage_params: evaporchain_demurrage::DemurrageParams,
    /// Decay-Lamport logical clock.  Ticked per block with total gas_used as
    /// the energy proxy — one quantum = GAS_TRANSFER (21,000 gas).
    pub lamport_clock: evaporchain_decay_lamport::LamportClock,
    /// Refresh Market — AMM-priced namespace rent.  Namespaces are registered
    /// on first object creation and charged per refresh cycle.
    pub refresh_market: evaporchain_refresh_market::RefreshMarket,
}

/// Namespace key for the protocol-owned refresh pool. Storage rent
/// from `collect_storage_rent` accrues under this namespace; future
/// payouts to chain-history / beacon / light-cone-proof keep-alive
/// draw from it.
pub const SYSTEM_REFRESH_NAMESPACE: &[u8] = b"evaporchain-system-refresh";

impl SimpleExecutor {
    /// Create a new executor with the given grace period for evaporation.
    /// Signature verification is OFF by default (for backward compatibility).
    pub fn new(grace_period: u64) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            mmr: MerkleMountainRange::new(),
            verify_signatures: false,
            fee_controller: None,
            block_gas_limit: 0,
            contract_engine: ContractEngine::new(),
            script_engine: ScriptEngine::new(),
            privacy_executor: privacy_exec::PrivacyExecutor::new(),
            deferred_queue: temporal::DeferredQueue::new(),
            decay_watchers: temporal::DecayWatcherEngine::new(),
            pending_events: Vec::new(),
            reward_accumulator: None,
            chain_id: String::new(),
            call_depth: 0,
            eulogy_trie: evaporchain_tombstone::EulogyTrie::new(),
            refresh_pool: evaporchain_energy_kernel::RefreshPool::new(),
            mortis_monitor: evaporchain_mortis::MortisMonitor::new(
                evaporchain_mortis::MortisCondition::default_genesis(),
            ),
            mortis_certificate: None,
            lyapunov_fee_state: evaporchain_fee_controller::FeeState::at_equilibrium(
                evaporchain_fee_controller::FeeControllerParams::default_genesis().target_energy,
            ),
            lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams::default_genesis(),
            last_conservation_audit: None,
            last_audit_epoch: None,
            last_cmu_verdict: None,
            last_tur_verdict: None,
            demurrage_params: evaporchain_demurrage::DemurrageParams::default(),
            lamport_clock: crate::lamport_integration::genesis_clock(),
            refresh_market: crate::refresh_market_integration::genesis_market(),
        }
    }

    /// Set the chain ID for signing message domain separation.
    pub fn set_chain_id(&mut self, chain_id: String) {
        self.chain_id = chain_id;
    }

    /// Test-only public wrapper around the private `collect_storage_rent`.
    /// Keeps integration tests in `four_act_integration_tests` from
    /// having to mock the full apply_block path.
    #[doc(hidden)]
    pub fn run_storage_rent_for_test(&mut self, db: &mut dyn StateDB, current_epoch: u64) {
        self.collect_storage_rent(db, current_epoch);
    }

    /// Current Decay-Lamport logical tick. Light clients and cross-block
    /// ordering systems read this for causal ordering without a wall clock.
    pub fn lamport_tick(&self) -> u64 {
        self.lamport_clock.current_tick
    }

    /// Run the demurrage sweep directly (test helper / node API).
    #[doc(hidden)]
    pub fn run_demurrage_for_test(
        &mut self,
        db: &mut dyn StateDB,
        last_epoch: u64,
        current_epoch: u64,
    ) -> u64 {
        crate::demurrage_integration::collect_demurrage(
            db,
            &mut self.refresh_pool,
            &self.demurrage_params,
            last_epoch,
            current_epoch,
        )
    }

    /// Per-block hook: advance the Singh-Lyapunov fee state against
    /// `gas_used` from the just-applied block. Returns the new base
    /// fee + the Lyapunov drift for chain-side audit. Per
    /// INVENTION_STACK.md §4.1 #4 the empty-block drift is provably
    /// monotone-non-positive (asserted by the property test in
    /// `evaporchain-fee-controller`).
    pub fn tick_lyapunov_fee_state(
        &mut self,
        gas_used: u64,
        epochs_elapsed: u64,
    ) -> Result<
        (evaporchain_types::Energy, evaporchain_fee_controller::Drift),
        evaporchain_fee_controller::controller::FeeControllerError,
    > {
        let (new_state, drift) = evaporchain_fee_controller::FeeController::step(
            &self.lyapunov_fee_params,
            &self.lyapunov_fee_state,
            gas_used,
            epochs_elapsed,
        )?;
        self.lyapunov_fee_state = new_state;
        let new_fee = evaporchain_fee_controller::base_fee(
            &self.lyapunov_fee_state,
            &self.lyapunov_fee_params,
        );
        Ok((new_fee, drift))
    }

    /// Record a Cμ-Gate observation (passive). Caller supplies the
    /// observed Cμ + bound estimates; verdict is stored in
    /// `last_cmu_verdict`. Per INVENTION_STACK.md §A1.3, an observed
    /// Cμ above the bound is a Sybil/spam signature.
    pub fn record_cmu_observation(
        &mut self,
        observed_cmu_mb: u64,
        excess_entropy_mb: u64,
        entropy_rate_mb: u64,
    ) -> evaporchain_cmu_gate::Verdict {
        let v =
            evaporchain_cmu_gate::cmu_check(observed_cmu_mb, excess_entropy_mb, entropy_rate_mb);
        self.last_cmu_verdict = Some(v);
        v
    }

    /// Record a TUR Liveness observation (passive). Caller supplies a
    /// per-block sample window of a chain current J + the entropy
    /// production Σ; verdict is stored in `last_tur_verdict`. Per
    /// INVENTION_STACK.md §A1.3, a violation flags coordinated
    /// cartel activity (current too steady for the entropy budget).
    pub fn record_tur_observation(
        &mut self,
        j_samples: &[u64],
        sigma: u64,
    ) -> evaporchain_tur_liveness::Verdict {
        let v = evaporchain_tur_liveness::tur_check(j_samples, sigma);
        self.last_tur_verdict = Some(v);
        v
    }

    /// Per-block hook: advance the Mortis monitor against the current
    /// refresh-pool total. If the death trigger fires this tick, mints
    /// the chain's singleton death certificate from `state_root` and
    /// the eulogy-trie root. Subsequent ticks after the trigger are
    /// no-ops. Returns the certificate iff JUST minted on this tick.
    ///
    /// Per INVENTION_STACK.md Amendment 2 §A2.5: 'when refresh pool
    /// falls below ε for N epochs, the final state root is auto-minted
    /// as a single unowned NFT visible to all light clients forever.'
    pub fn tick_mortis(
        &mut self,
        current_epoch: u64,
        state_root: [u8; 32],
    ) -> Option<&evaporchain_mortis::MortisCertificate> {
        let pool_total = self.refresh_pool.total_accrued();
        let outcome = self.mortis_monitor.tick(current_epoch, pool_total);
        if matches!(outcome, evaporchain_mortis::TickOutcome::JustTriggered) {
            let cert = evaporchain_mortis::mint_certificate(
                state_root,
                self.eulogy_trie.root(),
                current_epoch,
                pool_total,
            );
            self.mortis_certificate = Some(cert);
            self.mortis_certificate.as_ref()
        } else {
            None
        }
    }

    /// Create a test-friendly executor with a small privacy tree (depth 4).
    #[cfg(test)]
    pub fn new_for_test(grace_period: u64) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            mmr: MerkleMountainRange::new(),
            verify_signatures: false,
            fee_controller: None,
            block_gas_limit: 0,
            contract_engine: ContractEngine::new(),
            script_engine: ScriptEngine::new(),
            privacy_executor: privacy_exec::PrivacyExecutor::with_depth(4),
            deferred_queue: temporal::DeferredQueue::new(),
            decay_watchers: temporal::DecayWatcherEngine::new(),
            pending_events: Vec::new(),
            reward_accumulator: None,
            chain_id: String::new(),
            call_depth: 0,
            eulogy_trie: evaporchain_tombstone::EulogyTrie::new(),
            refresh_pool: evaporchain_energy_kernel::RefreshPool::new(),
            mortis_monitor: evaporchain_mortis::MortisMonitor::new(
                evaporchain_mortis::MortisCondition::default_genesis(),
            ),
            mortis_certificate: None,
            lyapunov_fee_state: evaporchain_fee_controller::FeeState::at_equilibrium(
                evaporchain_fee_controller::FeeControllerParams::default_genesis().target_energy,
            ),
            lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams::default_genesis(),
            last_conservation_audit: None,
            last_audit_epoch: None,
            last_cmu_verdict: None,
            last_tur_verdict: None,
            demurrage_params: evaporchain_demurrage::DemurrageParams::default(),
            lamport_clock: crate::lamport_integration::genesis_clock(),
            refresh_market: crate::refresh_market_integration::genesis_market(),
        }
    }

    /// Create a new executor with signature verification enabled.
    pub fn new_with_sig_verification(grace_period: u64) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            mmr: MerkleMountainRange::new(),
            verify_signatures: true,
            fee_controller: None,
            block_gas_limit: 0,
            contract_engine: ContractEngine::new(),
            script_engine: ScriptEngine::new(),
            privacy_executor: privacy_exec::PrivacyExecutor::new(),
            deferred_queue: temporal::DeferredQueue::new(),
            decay_watchers: temporal::DecayWatcherEngine::new(),
            pending_events: Vec::new(),
            reward_accumulator: None,
            chain_id: String::new(),
            call_depth: 0,
            eulogy_trie: evaporchain_tombstone::EulogyTrie::new(),
            refresh_pool: evaporchain_energy_kernel::RefreshPool::new(),
            mortis_monitor: evaporchain_mortis::MortisMonitor::new(
                evaporchain_mortis::MortisCondition::default_genesis(),
            ),
            mortis_certificate: None,
            lyapunov_fee_state: evaporchain_fee_controller::FeeState::at_equilibrium(
                evaporchain_fee_controller::FeeControllerParams::default_genesis().target_energy,
            ),
            lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams::default_genesis(),
            last_conservation_audit: None,
            last_audit_epoch: None,
            last_cmu_verdict: None,
            last_tur_verdict: None,
            demurrage_params: evaporchain_demurrage::DemurrageParams::default(),
            lamport_clock: crate::lamport_integration::genesis_clock(),
            refresh_market: crate::refresh_market_integration::genesis_market(),
        }
    }

    #[cfg(test)]
    pub fn new_with_sig_verification_for_test(grace_period: u64) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            mmr: MerkleMountainRange::new(),
            verify_signatures: true,
            fee_controller: None,
            block_gas_limit: 0,
            contract_engine: ContractEngine::new(),
            script_engine: ScriptEngine::new(),
            privacy_executor: privacy_exec::PrivacyExecutor::with_depth(4),
            deferred_queue: temporal::DeferredQueue::new(),
            decay_watchers: temporal::DecayWatcherEngine::new(),
            pending_events: Vec::new(),
            reward_accumulator: None,
            chain_id: String::new(),
            call_depth: 0,
            eulogy_trie: evaporchain_tombstone::EulogyTrie::new(),
            refresh_pool: evaporchain_energy_kernel::RefreshPool::new(),
            mortis_monitor: evaporchain_mortis::MortisMonitor::new(
                evaporchain_mortis::MortisCondition::default_genesis(),
            ),
            mortis_certificate: None,
            lyapunov_fee_state: evaporchain_fee_controller::FeeState::at_equilibrium(
                evaporchain_fee_controller::FeeControllerParams::default_genesis().target_energy,
            ),
            lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams::default_genesis(),
            last_conservation_audit: None,
            last_audit_epoch: None,
            last_cmu_verdict: None,
            last_tur_verdict: None,
            demurrage_params: evaporchain_demurrage::DemurrageParams::default(),
            lamport_clock: crate::lamport_integration::genesis_clock(),
            refresh_market: crate::refresh_market_integration::genesis_market(),
        }
    }

    /// Create a new executor with PID fee controller.
    pub fn new_with_fees(
        grace_period: u64,
        fee_controller: fees::PidFeeController,
        block_gas_limit: u64,
    ) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            mmr: MerkleMountainRange::new(),
            verify_signatures: false,
            fee_controller: Some(fee_controller),
            block_gas_limit,
            contract_engine: ContractEngine::new(),
            script_engine: ScriptEngine::new(),
            privacy_executor: privacy_exec::PrivacyExecutor::new(),
            deferred_queue: temporal::DeferredQueue::new(),
            decay_watchers: temporal::DecayWatcherEngine::new(),
            pending_events: Vec::new(),
            reward_accumulator: None,
            chain_id: String::new(),
            call_depth: 0,
            eulogy_trie: evaporchain_tombstone::EulogyTrie::new(),
            refresh_pool: evaporchain_energy_kernel::RefreshPool::new(),
            mortis_monitor: evaporchain_mortis::MortisMonitor::new(
                evaporchain_mortis::MortisCondition::default_genesis(),
            ),
            mortis_certificate: None,
            lyapunov_fee_state: evaporchain_fee_controller::FeeState::at_equilibrium(
                evaporchain_fee_controller::FeeControllerParams::default_genesis().target_energy,
            ),
            lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams::default_genesis(),
            last_conservation_audit: None,
            last_audit_epoch: None,
            last_cmu_verdict: None,
            last_tur_verdict: None,
            demurrage_params: evaporchain_demurrage::DemurrageParams::default(),
            lamport_clock: crate::lamport_integration::genesis_clock(),
            refresh_market: crate::refresh_market_integration::genesis_market(),
        }
    }

    #[cfg(test)]
    pub fn new_with_fees_for_test(
        grace_period: u64,
        fee_controller: fees::PidFeeController,
        block_gas_limit: u64,
    ) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            mmr: MerkleMountainRange::new(),
            verify_signatures: false,
            fee_controller: Some(fee_controller),
            block_gas_limit,
            contract_engine: ContractEngine::new(),
            script_engine: ScriptEngine::new(),
            privacy_executor: privacy_exec::PrivacyExecutor::with_depth(4),
            deferred_queue: temporal::DeferredQueue::new(),
            decay_watchers: temporal::DecayWatcherEngine::new(),
            pending_events: Vec::new(),
            reward_accumulator: None,
            chain_id: String::new(),
            call_depth: 0,
            eulogy_trie: evaporchain_tombstone::EulogyTrie::new(),
            refresh_pool: evaporchain_energy_kernel::RefreshPool::new(),
            mortis_monitor: evaporchain_mortis::MortisMonitor::new(
                evaporchain_mortis::MortisCondition::default_genesis(),
            ),
            mortis_certificate: None,
            lyapunov_fee_state: evaporchain_fee_controller::FeeState::at_equilibrium(
                evaporchain_fee_controller::FeeControllerParams::default_genesis().target_energy,
            ),
            lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams::default_genesis(),
            last_conservation_audit: None,
            last_audit_epoch: None,
            last_cmu_verdict: None,
            last_tur_verdict: None,
            demurrage_params: evaporchain_demurrage::DemurrageParams::default(),
            lamport_clock: crate::lamport_integration::genesis_clock(),
            refresh_market: crate::refresh_market_integration::genesis_market(),
        }
    }

    /// Create a new executor with signature verification AND fee deduction enabled.
    /// This is the production configuration.
    pub fn new_production(
        grace_period: u64,
        fee_controller: fees::PidFeeController,
        block_gas_limit: u64,
    ) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            mmr: MerkleMountainRange::new(),
            verify_signatures: true,
            fee_controller: Some(fee_controller),
            block_gas_limit,
            contract_engine: ContractEngine::new(),
            script_engine: ScriptEngine::new(),
            privacy_executor: privacy_exec::PrivacyExecutor::new(),
            deferred_queue: temporal::DeferredQueue::new(),
            decay_watchers: temporal::DecayWatcherEngine::new(),
            pending_events: Vec::new(),
            reward_accumulator: None,
            chain_id: String::new(),
            call_depth: 0,
            eulogy_trie: evaporchain_tombstone::EulogyTrie::new(),
            refresh_pool: evaporchain_energy_kernel::RefreshPool::new(),
            mortis_monitor: evaporchain_mortis::MortisMonitor::new(
                evaporchain_mortis::MortisCondition::default_genesis(),
            ),
            mortis_certificate: None,
            lyapunov_fee_state: evaporchain_fee_controller::FeeState::at_equilibrium(
                evaporchain_fee_controller::FeeControllerParams::default_genesis().target_energy,
            ),
            lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams::default_genesis(),
            last_conservation_audit: None,
            last_audit_epoch: None,
            last_cmu_verdict: None,
            last_tur_verdict: None,
            demurrage_params: evaporchain_demurrage::DemurrageParams::default(),
            lamport_clock: crate::lamport_integration::genesis_clock(),
            refresh_market: crate::refresh_market_integration::genesis_market(),
        }
    }

    /// Get a reference to the fee controller (if configured).
    pub fn fee_controller(&self) -> Option<&fees::PidFeeController> {
        self.fee_controller.as_ref()
    }

    /// Get a mutable reference to the fee controller.
    pub fn fee_controller_mut(&mut self) -> Option<&mut fees::PidFeeController> {
        self.fee_controller.as_mut()
    }

    /// Enable reward distribution with the given tokenomics.
    pub fn enable_rewards(&mut self, tokenomics: evaporchain_types::genesis::Tokenomics) {
        self.reward_accumulator = Some(rewards::RewardAccumulator::new(tokenomics));
    }

    /// Apply the proposer-priority bonus for a block. Called by the
    /// consensus layer AFTER `execute_block` returns, with the
    /// `priority_sum` produced by `Mempool::take_with_priority_and_sum`
    /// at proposal time.
    ///
    /// `scale_per_unit` is operator-tunable (0 disables); see
    /// `RewardAccumulator::apply_priority_bonus` for semantics.
    /// Returns the bonus credited (zero if rewards aren't enabled or
    /// the divisor produced a sub-integer bonus).
    ///
    /// Phase-1.5 of the energy-stamped MEV defense.
    pub fn apply_proposer_priority_bonus(
        &mut self,
        db: &mut dyn StateDB,
        producer: &evaporchain_types::AccountAddress,
        epoch: evaporchain_types::Epoch,
        priority_sum: u64,
        scale_per_unit: u64,
    ) -> u64 {
        if let Some(ref mut ra) = self.reward_accumulator {
            ra.apply_priority_bonus(db, producer, epoch, priority_sum, scale_per_unit)
        } else {
            0
        }
    }

    /// Estimate gas for a transaction.
    fn estimate_gas(tx: &Transaction) -> u64 {
        match tx {
            Transaction::Transfer(_) => GAS_TRANSFER,
            Transaction::CreateObject(create) => GAS_CREATE_OBJECT_BASE.saturating_add(
                GAS_CREATE_OBJECT_PER_BYTE.saturating_mul(create.data.len() as u64),
            ),
            Transaction::Refresh(_) => GAS_REFRESH,
            // N3 (re-audit 2026-05-02): DeployScript was a flat cost
            // regardless of source size — a 1MB malicious script paid
            // the same as 100B. Source is stored on-chain (modulo
            // evaporation) and compiled, so cost must scale with size
            // to deny cheap on-chain ballast. Same coefficient as
            // CreateObject's per-byte rate. DeployContract dispatches
            // a built-in template + init_args string of bounded size,
            // so flat-cost there is fine.
            Transaction::DeployContract(_) => GAS_DEPLOY_CONTRACT,
            Transaction::CallContract(_) => GAS_CALL_CONTRACT,
            Transaction::DeployScript(d) => GAS_DEPLOY_SCRIPT.saturating_add(
                GAS_CREATE_OBJECT_PER_BYTE.saturating_mul(d.source_code.len() as u64),
            ),
            Transaction::CallScript(_) => GAS_CALL_SCRIPT,
            Transaction::ValidatorStake(_) => GAS_VALIDATOR_STAKE,
            Transaction::ValidatorExit(_) => GAS_VALIDATOR_EXIT,
            Transaction::ValidatorClaimStake(_) => GAS_VALIDATOR_CLAIM_STAKE,
            Transaction::Shield(_) => privacy_exec::GAS_SHIELD,
            Transaction::Unshield(_) => privacy_exec::GAS_UNSHIELD,
            Transaction::PrivateTransfer(ptx) => {
                privacy_exec::PrivacyExecutor::estimate_private_transfer_gas(ptx)
            }
            Transaction::Deferred(dtx) => temporal::GAS_DEFERRED_SUBMIT
                .saturating_add(temporal::GAS_PER_GUARD.saturating_mul(dtx.guards.len() as u64)),
            Transaction::Blob(tx) => GAS_CREATE_OBJECT_BASE
                .saturating_add(GAS_CREATE_OBJECT_PER_BYTE.saturating_mul(tx.data.len() as u64)),
            Transaction::Governance(_) => GAS_GOVERNANCE,
            Transaction::MultiSig(_) => GAS_MULTISIG,
            Transaction::UserOp(tx) => GAS_USER_OP.saturating_add(tx.call_data.len() as u64 * 16),
            Transaction::UpgradeContract(tx) => {
                GAS_UPGRADE_CONTRACT.saturating_add(tx.new_bytecode.len() as u64 * 200)
            }
            Transaction::Delegate(_) => GAS_DELEGATE,
            Transaction::Undelegate(_) => GAS_UNDELEGATE,
            Transaction::RotateValidatorKey(_) => GAS_ROTATE_VALIDATOR_KEY,
            Transaction::ClaimDelegation(_) => GAS_CLAIM_DELEGATION,
            // Crooks-MEV Phase 3.5: refund is PROTOCOL-issued — the
            // proposer pays gas, not the victim or attacker. Charged
            // at the dedicated `GAS_REFUND = 5_000` (lower than
            // `GAS_TRANSFER = 21_000`) so proposers aren't
            // economically deterred from settling observations they're
            // contractually obligated to settle. Earlier code charged
            // `GAS_TRANSFER` here — that defeated the Phase 3.5
            // economic design (Phase 3.5d's missing-refund stake
            // slash assumes proposers find it cheaper to settle than
            // to skip).
            Transaction::Refund(_) => GAS_REFUND,
        }
    }

    /// Verify the ML-DSA signature on a transaction (if verification is enabled).
    /// Unshield and PrivateTransfer are authenticated by ZK proofs, not signatures.
    fn verify_tx_signature(&self, tx: &Transaction) -> Result<(), ExecutionError> {
        if !self.verify_signatures {
            return Ok(());
        }

        // ZK-authenticated transactions don't use signatures
        if matches!(
            tx,
            Transaction::Unshield(_) | Transaction::PrivateTransfer(_)
        ) {
            return Ok(());
        }

        let sig = tx.signature().ok_or(ExecutionError::MissingSignature)?;
        let pk = tx.public_key().ok_or(ExecutionError::MissingSignature)?;
        let msg = tx.signing_message(&self.chain_id);

        if !HybridVerifier::verify(&msg, sig, pk) {
            return Err(ExecutionError::InvalidSignature);
        }

        Ok(())
    }

    /// Execute a single transfer transaction.
    fn execute_transfer(
        &self,
        db: &mut dyn StateDB,
        tx: &TransferTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        if tx.from == tx.to {
            return Err(ExecutionError::SelfTransfer);
        }
        if tx.amount == 0 {
            return Err(ExecutionError::ZeroAmount);
        }

        // Mint-bypass: a transfer from the all-zeros address skips both
        // nonce check (no source account exists) and nonce increment, and
        // creates the recipient out of thin air. Used for legacy faucet
        // and genesis-time minting paths. The canonical genesis faucet at
        // FAUCET_ADDRESS = [0xFA; 32] does NOT use this bypass — it has a
        // real balance and goes through the normal Transfer path.
        let is_mint_bypass = tx.from == [0u8; 32];
        let sender = db.get_or_create_account(&tx.from);
        if !is_mint_bypass && sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        // Vesting gate (TOKENOMICS §2.6 / Q14): outflows must come from the
        // transferable portion of balance. Mint-bypass uses raw balance
        // because the zero-address sender has no real account.
        let available = if is_mint_bypass {
            sender.balance
        } else {
            sender.transferable_balance(epoch)
        };
        if available < tx.amount {
            return Err(ExecutionError::InsufficientBalance {
                account: hex::encode(tx.from),
                available,
                required: tx.amount,
            });
        }

        // Debit sender
        sender.balance -= tx.amount;
        if !is_mint_bypass {
            sender.nonce += 1;
        }
        // Stamp the demurrage anchor: balance (and possibly nonce) just
        // mutated, so per-account demurrage starts accruing from `epoch`.
        sender.last_touched_epoch = epoch;

        // Credit receiver
        let receiver = db.get_or_create_account(&tx.to);
        receiver.balance = receiver.balance.saturating_add(tx.amount);
        // Receiver's balance changed too — reset their demurrage anchor.
        receiver.last_touched_epoch = epoch;

        debug!(
            from = hex::encode(tx.from),
            to = hex::encode(tx.to),
            amount = tx.amount,
            "Transfer executed"
        );

        Ok(())
    }

    /// Phase 3.5 of CROOKS_MEV_INTEGRATION_PLAN.md (Lane Q.1):
    /// execute a protocol-issued `RefundTx`. Debits `attacker`,
    /// credits `victim`, no nonce mutation (refund is not signed by
    /// either party — it's emitted by consensus once the determinism
    /// contract is satisfied).
    ///
    /// The Phase 3.2 determinism contract (caller must match the
    /// validator's own `mev_observations` + `mev_attacker_stats` for
    /// the source `(block_height, observation_idx)`) is enforced at
    /// the consensus layer before the tx reaches execution; this
    /// function trusts the caller has been validated.
    ///
    /// Errors:
    /// - `ZeroAmount` — refund amount must be > 0.
    /// - `SelfTransfer` — attacker == victim is a no-op refund.
    /// - `InsufficientBalance` — attacker can't cover the refund.
    fn execute_refund(
        &self,
        db: &mut dyn StateDB,
        tx: &RefundTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        if tx.attacker == tx.victim {
            return Err(ExecutionError::SelfTransfer);
        }
        if tx.amount == 0 {
            return Err(ExecutionError::ZeroAmount);
        }

        let attacker = db.get_or_create_account(&tx.attacker);
        if attacker.balance < tx.amount {
            return Err(ExecutionError::InsufficientBalance {
                account: hex::encode(tx.attacker),
                available: attacker.balance,
                required: tx.amount,
            });
        }

        attacker.balance -= tx.amount;
        // No nonce increment — refund is protocol-issued, not signed.
        // Stamp the demurrage anchor: balance just mutated.
        attacker.last_touched_epoch = epoch;

        let victim = db.get_or_create_account(&tx.victim);
        victim.balance = victim.balance.saturating_add(tx.amount);
        victim.last_touched_epoch = epoch;

        debug!(
            attacker = hex::encode(tx.attacker),
            victim = hex::encode(tx.victim),
            amount = tx.amount,
            source_block_height = tx.source_block_height,
            source_observation_idx = tx.source_observation_idx,
            settle_block_height = tx.settle_block_height,
            "Refund executed (Crooks-MEV Phase 3.5)"
        );

        Ok(())
    }

    /// Execute an object creation transaction.
    fn execute_create_object(
        &self,
        db: &mut dyn StateDB,
        tx: &CreateObjectTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        // Check if object already exists
        if db.get_object(&tx.object_id).is_some() {
            return Err(ExecutionError::ObjectAlreadyExists(hex::encode(
                tx.object_id,
            )));
        }

        // Storage rent: creator must hold at least MIN_STORAGE_DEPOSIT EVAP
        // to anchor the new object. The deposit is locked in the account's
        // storage_deposit field and charged per-epoch by collect_storage_rent.
        let object_bytes = {
            const BASE_OBJECT_BYTES: u64 = 97; // id+owner+energy+half_life+timestamps+state
            BASE_OBJECT_BYTES.saturating_add(tx.data.len() as u64)
        };
        {
            let creator = db.get_or_create_account(&tx.creator);
            // Vesting gate (TOKENOMICS §2.6 / Q14): storage deposits can't
            // come from the locked portion of balance.
            let available = creator.transferable_balance(epoch);
            if available < evaporchain_types::MIN_STORAGE_DEPOSIT {
                return Err(ExecutionError::InsufficientBalance {
                    account: hex::encode(tx.creator),
                    available,
                    required: evaporchain_types::MIN_STORAGE_DEPOSIT,
                });
            }
            creator.balance -= evaporchain_types::MIN_STORAGE_DEPOSIT;
            creator.storage_deposit = creator
                .storage_deposit
                .saturating_add(evaporchain_types::MIN_STORAGE_DEPOSIT);
            // Storage-deposit lock-up debits balance — stamp the demurrage anchor.
            creator.last_touched_epoch = epoch;
            creator.storage_bytes = creator.storage_bytes.saturating_add(object_bytes);
        }

        let obj = StateObject {
            id: tx.object_id,
            owner: tx.creator,
            energy: tx.energy,
            half_life: tx.half_life,
            created_at: epoch,
            last_refreshed: epoch,
            state: ObjectState::Active,
            grace_epoch: None,
            data: tx.data.clone(),
            decay_curve: tx.decay_curve.clone(),
            // Stamp the LAD-VM substructural mode on the new object.
            // `None` (the default) produces an ordinary state object;
            // a `Some(mode)` makes it LAD-typed and is what the
            // wallet's LAD pill / LadVmPreview keys off of.
            lad_mode: tx.lad_mode,
        };

        db.put_object(obj);

        debug!(
            object_id = hex::encode(tx.object_id),
            energy = tx.energy,
            half_life = tx.half_life,
            storage_bytes = object_bytes,
            "Object created"
        );

        Ok(())
    }

    /// Execute an energy refresh transaction.
    fn execute_refresh(
        &self,
        db: &mut dyn StateDB,
        tx: &RefreshTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        // Try refresh on active/grace object first
        if db.get_object(&tx.object_id).is_some() {
            RefreshEngine::refresh(db, &tx.object_id, tx.energy_deposit, epoch)
                .map_err(|e| ExecutionError::RefreshFailed(e.to_string()))?;
            return Ok(());
        }

        // Try resurrection from ghost
        if db.get_ghost(&tx.object_id).is_some() {
            RefreshEngine::resurrect(db, &tx.object_id, tx.energy_deposit, epoch)
                .map_err(|e| ExecutionError::RefreshFailed(e.to_string()))?;
            return Ok(());
        }

        Err(ExecutionError::ObjectNotFound(hex::encode(tx.object_id)))
    }

    /// Execute a contract deployment transaction.
    fn execute_deploy_contract(
        &mut self,
        db: &mut dyn StateDB,
        tx: &DeployContractTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        let template = match tx.template.as_str() {
            "DecayingToken" => ContractTemplate::DecayingToken,
            "MortalNFT" => ContractTemplate::MortalNFT,
            "ThermodynamicEscrow" => ContractTemplate::ThermodynamicEscrow,
            "DecayingAuction" => ContractTemplate::DecayingAuction,
            "StakingPool" => ContractTemplate::StakingPool,
            "DAOVote" => ContractTemplate::DAOVote,
            "TemporalContract" => ContractTemplate::TemporalContract,
            other => {
                return Err(ExecutionError::ContractError(format!(
                    "unknown template: {other}"
                )));
            }
        };

        let init_args: serde_json::Value = serde_json::from_str(&tx.init_args)
            .map_err(|e| ExecutionError::ContractError(format!("invalid init_args JSON: {e}")))?;

        let rules = if let Some(rules_str) = &tx.rules {
            let parsed: Vec<evaporchain_contracts::Rule> = serde_json::from_str(rules_str)
                .map_err(|e| ExecutionError::ContractError(format!("invalid rules JSON: {e}")))?;
            parsed
        } else {
            vec![]
        };

        // Phase 4.2 (2026-05-03): storage-rent enforcement on contract
        // deploy. Audit `end_to_end_audit_2026_04_27.md §3` flagged
        // that contract deploy paths added persistent state to the
        // chain without ever touching the deployer's
        // `storage_deposit` / `storage_bytes` — a deployer could
        // claim arbitrary storage with no prepayment. Charge
        // MIN_STORAGE_DEPOSIT and stamp the byte count so per-epoch
        // `collect_storage_rent` charges them like any other
        // storage holder. Mirrors the `execute_create_object` charge.
        let contract_bytes = {
            const BASE_CONTRACT_BYTES: u64 = 256; // template name + id + decay fields + bookkeeping
            let init_bytes = tx.init_args.len() as u64;
            let rules_bytes = tx.rules.as_ref().map(|s| s.len() as u64).unwrap_or(0);
            BASE_CONTRACT_BYTES
                .saturating_add(init_bytes)
                .saturating_add(rules_bytes)
        };
        {
            let deployer = db.get_or_create_account(&tx.deployer);
            // Vesting gate (TOKENOMICS §2.6 / Q14): contract deploy storage
            // deposit can't come from the locked portion of balance.
            let available = deployer.transferable_balance(epoch);
            if available < evaporchain_types::MIN_STORAGE_DEPOSIT {
                return Err(ExecutionError::InsufficientBalance {
                    account: hex::encode(tx.deployer),
                    available,
                    required: evaporchain_types::MIN_STORAGE_DEPOSIT,
                });
            }
            deployer.balance -= evaporchain_types::MIN_STORAGE_DEPOSIT;
            deployer.storage_deposit = deployer
                .storage_deposit
                .saturating_add(evaporchain_types::MIN_STORAGE_DEPOSIT);
            deployer.storage_bytes = deployer.storage_bytes.saturating_add(contract_bytes);
            deployer.last_touched_epoch = epoch;
        }

        let id = self
            .contract_engine
            .deploy(
                template,
                init_args,
                rules,
                tx.deployer,
                tx.energy,
                tx.half_life,
                epoch,
            )
            .map_err(|e| ExecutionError::ContractError(e.to_string()))?;

        debug!(
            contract_id = id,
            template = %tx.template,
            storage_bytes = contract_bytes,
            "Contract deployed"
        );
        Ok(())
    }

    /// Execute a contract call transaction.
    fn execute_call_contract(&mut self, tx: &CallContractTx) -> Result<(), ExecutionError> {
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(ExecutionError::CallDepthExceeded(MAX_CALL_DEPTH));
        }
        self.call_depth += 1;

        let args: serde_json::Value = serde_json::from_str(&tx.args)
            .map_err(|e| ExecutionError::ContractError(format!("invalid args JSON: {e}")))?;

        let result = self
            .contract_engine
            .call(tx.contract_id, &tx.method, &args, &tx.caller, tx.epoch)
            .map_err(|e| {
                self.call_depth = self.call_depth.saturating_sub(1);
                ExecutionError::ContractError(e.to_string())
            })?;

        self.call_depth = self.call_depth.saturating_sub(1);

        debug!(
            contract_id = tx.contract_id,
            method = %tx.method,
            events = ?result.events,
            "Contract called"
        );
        Ok(())
    }

    /// Execute a script deployment transaction.
    fn execute_deploy_script(
        &mut self,
        db: &mut dyn StateDB,
        tx: &DeployScriptTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        // Phase 4.2 (2026-05-03): storage-rent enforcement on script
        // deploy. See execute_deploy_contract for rationale; same
        // gap applies here.
        let script_bytes = {
            const BASE_SCRIPT_BYTES: u64 = 128;
            BASE_SCRIPT_BYTES.saturating_add(tx.source_code.len() as u64)
        };
        {
            let deployer = db.get_or_create_account(&tx.deployer);
            // Vesting gate (TOKENOMICS §2.6 / Q14): script deploy storage
            // deposit can't come from the locked portion of balance.
            let available = deployer.transferable_balance(epoch);
            if available < evaporchain_types::MIN_STORAGE_DEPOSIT {
                return Err(ExecutionError::InsufficientBalance {
                    account: hex::encode(tx.deployer),
                    available,
                    required: evaporchain_types::MIN_STORAGE_DEPOSIT,
                });
            }
            deployer.balance -= evaporchain_types::MIN_STORAGE_DEPOSIT;
            deployer.storage_deposit = deployer
                .storage_deposit
                .saturating_add(evaporchain_types::MIN_STORAGE_DEPOSIT);
            deployer.storage_bytes = deployer.storage_bytes.saturating_add(script_bytes);
            deployer.last_touched_epoch = epoch;
        }

        let id = self
            .script_engine
            .deploy(&tx.source_code, tx.deployer, tx.energy, tx.half_life, epoch)
            .map_err(|e| ExecutionError::ScriptError(e.to_string()))?;

        debug!(
            script_id = id,
            storage_bytes = script_bytes,
            "Script contract deployed"
        );
        Ok(())
    }

    /// Execute a script call transaction.
    fn execute_call_script(&mut self, tx: &CallScriptTx) -> Result<(), ExecutionError> {
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(ExecutionError::CallDepthExceeded(MAX_CALL_DEPTH));
        }
        self.call_depth += 1;

        // Parse args from JSON
        let args: Vec<evaporchain_script::Value> = if tx.args.is_empty() || tx.args == "[]" {
            vec![]
        } else {
            serde_json::from_str(&tx.args)
                .map_err(|e| ExecutionError::ScriptError(format!("invalid args JSON: {e}")))?
        };

        let result = self
            .script_engine
            .call(tx.contract_id, &tx.method, args, tx.caller, tx.epoch)
            .map_err(|e| {
                self.call_depth = self.call_depth.saturating_sub(1);
                ExecutionError::ScriptError(e.to_string())
            })?;

        if !result.structured_events.is_empty() {
            self.pending_events.extend(
                result
                    .structured_events
                    .into_iter()
                    .map(|event| (tx.contract_id, event)),
            );
        }

        self.call_depth = self.call_depth.saturating_sub(1);

        debug!(
            script_id = tx.contract_id,
            method = %tx.method,
            "Script called"
        );
        Ok(())
    }

    /// Execute a validator staking transaction.
    /// Locks `stake_amount` from the validator's balance.
    fn execute_validator_stake(
        &self,
        db: &mut dyn StateDB,
        tx: &ValidatorStakeTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        if tx.stake_amount == 0 {
            return Err(ExecutionError::ZeroAmount);
        }

        let sender = db.get_or_create_account(&tx.validator_address);
        if sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        // Vesting gate (TOKENOMICS §2.6 / Q14): validator stake must come
        // from the transferable portion of balance.
        let available = sender.transferable_balance(epoch);
        if available < tx.stake_amount {
            return Err(ExecutionError::InsufficientBalance {
                account: hex::encode(tx.validator_address),
                available,
                required: tx.stake_amount,
            });
        }

        // Lock stake by deducting from balance
        sender.balance -= tx.stake_amount;
        sender.nonce += 1;
        // Stake locks balance and bumps nonce — stamp the demurrage anchor.
        sender.last_touched_epoch = epoch;

        let existing_stake = db
            .get_stake(tx.validator_id)
            .map(|s| s.staked_amount)
            .unwrap_or(0);
        db.put_stake(StakeRecord {
            validator_id: tx.validator_id,
            validator_address: tx.validator_address,
            staked_amount: existing_stake + tx.stake_amount,
            // C1 (audit 2026-05-02): was hardcoded 0, breaking
            // unbonding-period enforcement (UNBONDING_EPOCHS check uses
            // this field). Stamping the actual epoch makes
            // execute_validator_claim_stake's cooldown gate effective.
            staked_at_epoch: epoch,
            unbonding_epoch: None,
            slashed_amount: 0,
        });

        debug!(
            validator = hex::encode(tx.validator_address),
            stake = tx.stake_amount,
            validator_id = tx.validator_id,
            "Validator stake locked"
        );

        Ok(())
    }

    /// Execute a delegation transaction. Locks `tx.amount` from the
    /// delegator's balance and credits it to the
    /// (delegator, validator_id) `DelegationRecord`. Multiple delegations
    /// to the same validator are additive.
    fn execute_delegate(
        &self,
        db: &mut dyn StateDB,
        tx: &DelegateTx,
        current_epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        if tx.amount == 0 {
            return Err(ExecutionError::ZeroAmount);
        }
        // Validator must already exist as a stake record before accepting
        // delegations — prevents griefing where someone delegates to a
        // non-existent validator id and locks funds forever.
        if db.get_stake(tx.validator_id).is_none() {
            return Err(ExecutionError::ContractError(format!(
                "validator-id {} has no stake record; cannot accept delegations",
                tx.validator_id
            )));
        }

        let delegator = db.get_or_create_account(&tx.delegator);
        if delegator.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: delegator.nonce,
                got: tx.nonce,
            });
        }
        // Vesting gate (TOKENOMICS §2.6 / Q14): delegated stake must come
        // from the transferable portion of balance.
        let available = delegator.transferable_balance(current_epoch);
        if available < tx.amount {
            return Err(ExecutionError::InsufficientBalance {
                account: hex::encode(tx.delegator),
                available,
                required: tx.amount,
            });
        }
        delegator.balance -= tx.amount;
        delegator.nonce += 1;
        // Delegate locks balance + bumps nonce — stamp the demurrage anchor.
        delegator.last_touched_epoch = current_epoch;

        // Get-or-create the (delegator, validator_id) record. Adding to
        // an existing delegation refreshes `delegated_at_epoch` so reward
        // distribution can use a cleaner time-weighted share.
        let existing = db.get_delegation(&tx.delegator, tx.validator_id).cloned();
        let record = match existing {
            Some(mut r) => {
                r.amount = r.amount.saturating_add(tx.amount);
                r.delegated_at_epoch = current_epoch;
                r
            }
            None => DelegationRecord {
                delegator: tx.delegator,
                validator_id: tx.validator_id,
                amount: tx.amount,
                delegated_at_epoch: current_epoch,
                unbonding_amount: 0,
                unbonding_epoch: None,
            },
        };
        db.put_delegation(record);

        debug!(
            delegator = hex::encode(tx.delegator),
            validator_id = tx.validator_id,
            amount = tx.amount,
            "Delegation locked"
        );
        Ok(())
    }

    /// Execute an undelegation transaction. Marks `tx.amount` as
    /// unbonding on the existing `DelegationRecord`; funds are not
    /// returned to balance until a future ClaimDelegation tx runs after
    /// the unbonding period elapses (separate, future tx type).
    fn execute_undelegate(
        &self,
        db: &mut dyn StateDB,
        tx: &UndelegateTx,
        current_epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        if tx.amount == 0 {
            return Err(ExecutionError::ZeroAmount);
        }
        let delegator_acct = db.get_or_create_account(&tx.delegator);
        if delegator_acct.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: delegator_acct.nonce,
                got: tx.nonce,
            });
        }
        delegator_acct.nonce += 1;
        // Nonce mutated — stamp the demurrage anchor.
        delegator_acct.last_touched_epoch = current_epoch;

        let mut record = db
            .get_delegation(&tx.delegator, tx.validator_id)
            .cloned()
            .ok_or_else(|| {
                ExecutionError::ContractError(format!(
                    "no delegation from {} to validator-id {}",
                    hex::encode(tx.delegator),
                    tx.validator_id
                ))
            })?;
        if record.amount < tx.amount {
            return Err(ExecutionError::ContractError(format!(
                "delegation has only {} but tried to undelegate {}",
                record.amount, tx.amount
            )));
        }
        record.amount = record.amount.saturating_sub(tx.amount);
        record.unbonding_amount = record.unbonding_amount.saturating_add(tx.amount);
        record.unbonding_epoch = Some(current_epoch);
        db.put_delegation(record);

        debug!(
            delegator = hex::encode(tx.delegator),
            validator_id = tx.validator_id,
            amount = tx.amount,
            unbonding_epoch = current_epoch,
            "Delegation unbonding"
        );
        Ok(())
    }

    /// Execute a claim-delegation transaction (P0 #4 Phase 7).
    /// Releases a previously-undelegated amount back to the delegator's
    /// balance once `unbonding_epoch + UNBONDING_PERIOD_EPOCHS` has elapsed.
    fn execute_claim_delegation(
        &self,
        db: &mut dyn StateDB,
        tx: &ClaimDelegationTx,
        current_epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        let delegator_acct = db.get_or_create_account(&tx.delegator);
        if delegator_acct.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: delegator_acct.nonce,
                got: tx.nonce,
            });
        }
        delegator_acct.nonce += 1;
        // Nonce mutated — stamp the demurrage anchor here too; the actual
        // balance credit happens after we resolve the delegation record.
        delegator_acct.last_touched_epoch = current_epoch;

        let mut record = db
            .get_delegation(&tx.delegator, tx.validator_id)
            .cloned()
            .ok_or_else(|| {
                ExecutionError::ContractError(format!(
                    "no delegation from {} to validator-id {}",
                    hex::encode(tx.delegator),
                    tx.validator_id
                ))
            })?;
        if record.unbonding_amount == 0 {
            return Err(ExecutionError::ContractError(
                "no unbonding amount to claim".into(),
            ));
        }
        let unbonding_started = record
            .unbonding_epoch
            .ok_or_else(|| ExecutionError::ContractError("unbonding_epoch unset".into()))?;
        let claim_ready_at = unbonding_started.saturating_add(UNBONDING_PERIOD_EPOCHS);
        if current_epoch < claim_ready_at {
            return Err(ExecutionError::ContractError(format!(
                "unbonding period not elapsed: claimable at epoch {} (current {})",
                claim_ready_at, current_epoch
            )));
        }

        let claimed = record.unbonding_amount;
        record.unbonding_amount = 0;
        record.unbonding_epoch = None;

        if let Some(acct) = db.get_account_mut(&tx.delegator) {
            acct.balance = acct.balance.saturating_add(claimed);
            // Balance just changed — refresh the demurrage anchor.
            acct.last_touched_epoch = current_epoch;
        }

        if record.amount == 0 && record.unbonding_amount == 0 {
            db.remove_delegation(&tx.delegator, tx.validator_id);
        } else {
            db.put_delegation(record);
        }

        debug!(
            delegator = hex::encode(tx.delegator),
            validator_id = tx.validator_id,
            claimed = claimed,
            current_epoch = current_epoch,
            "Delegation claim succeeded"
        );
        Ok(())
    }

    /// Execute a validator exit transaction.
    /// Sets unbonding_epoch so the validator must wait before claiming stake.
    fn execute_validator_exit(
        &self,
        db: &mut dyn StateDB,
        tx: &ValidatorExitTx,
        current_epoch: u64,
    ) -> Result<(), ExecutionError> {
        let sender = db.get_or_create_account(&tx.validator_address);
        if sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        sender.nonce += 1;
        // Nonce mutated — stamp the demurrage anchor.
        sender.last_touched_epoch = current_epoch;

        let mut stake = db.get_stake(tx.validator_id).cloned().ok_or_else(|| {
            ExecutionError::ObjectNotFound(format!(
                "no stake record for validator {}",
                tx.validator_id
            ))
        })?;

        if stake.validator_address != tx.validator_address {
            return Err(ExecutionError::InvalidSignature);
        }

        if stake.unbonding_epoch.is_some() {
            return Err(ExecutionError::ContractError(
                "validator already exiting".to_string(),
            ));
        }

        stake.unbonding_epoch = Some(current_epoch + UNBONDING_PERIOD_EPOCHS);
        db.put_stake(stake);

        debug!(
            validator = hex::encode(tx.validator_address),
            validator_id = tx.validator_id,
            unbonding_epoch = current_epoch + UNBONDING_PERIOD_EPOCHS,
            "Validator exit: unbonding period started"
        );

        Ok(())
    }

    fn execute_validator_claim_stake(
        &self,
        db: &mut dyn StateDB,
        tx: &ValidatorClaimStakeTx,
        current_epoch: u64,
    ) -> Result<(), ExecutionError> {
        let stake = db.get_stake(tx.validator_id).cloned();
        let stake = stake.ok_or_else(|| {
            ExecutionError::ObjectNotFound(format!(
                "no stake record for validator {}",
                tx.validator_id
            ))
        })?;

        if stake.validator_address != tx.validator_address {
            return Err(ExecutionError::InvalidSignature);
        }

        let unbonding = stake.unbonding_epoch.ok_or_else(|| {
            ExecutionError::ContractError(
                "validator has not exited — cannot claim stake".to_string(),
            )
        })?;

        if current_epoch < unbonding {
            return Err(ExecutionError::ContractError(format!(
                "unbonding period not complete: current epoch {} < unbonding epoch {}",
                current_epoch, unbonding
            )));
        }

        let sender = db.get_or_create_account(&tx.validator_address);
        if sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }

        let claimable = stake.staked_amount.saturating_sub(stake.slashed_amount);
        sender.balance += claimable;
        sender.nonce += 1;
        // Balance + nonce mutated — stamp the demurrage anchor.
        sender.last_touched_epoch = current_epoch;

        db.remove_stake(tx.validator_id);

        debug!(
            validator = hex::encode(tx.validator_address),
            validator_id = tx.validator_id,
            claimable,
            "Validator stake claimed after unbonding"
        );

        Ok(())
    }

    fn execute_governance(
        &self,
        db: &mut dyn StateDB,
        tx: &GovernanceTx,
        current_epoch: u64,
    ) -> Result<(), ExecutionError> {
        let sender = db.get_or_create_account(&tx.sender);
        if sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        sender.nonce += 1;
        // Nonce mutated — stamp the demurrage anchor.
        sender.last_touched_epoch = current_epoch;

        match &tx.action {
            GovernanceAction::CreateProposal {
                title,
                param_key,
                param_value,
                voting_epochs,
            } => {
                // Gap-A #4: bound checks on proposal admission.
                if title.len() > MAX_PROPOSAL_TITLE_BYTES {
                    return Err(ExecutionError::ContractError(format!(
                        "proposal title exceeds {} bytes ({})",
                        MAX_PROPOSAL_TITLE_BYTES,
                        title.len()
                    )));
                }
                if param_key.len() > MAX_PARAM_KEY_BYTES {
                    return Err(ExecutionError::ContractError(format!(
                        "param_key exceeds {} bytes ({})",
                        MAX_PARAM_KEY_BYTES,
                        param_key.len()
                    )));
                }
                if param_value.len() > MAX_PARAM_VALUE_BYTES {
                    return Err(ExecutionError::ContractError(format!(
                        "param_value exceeds {} bytes ({})",
                        MAX_PARAM_VALUE_BYTES,
                        param_value.len()
                    )));
                }
                if !(MIN_VOTING_EPOCHS..=MAX_VOTING_EPOCHS).contains(voting_epochs) {
                    return Err(ExecutionError::ContractError(format!(
                        "voting_epochs out of range [{}, {}]: {}",
                        MIN_VOTING_EPOCHS, MAX_VOTING_EPOCHS, voting_epochs
                    )));
                }
                if !is_governable_param_key(param_key) {
                    return Err(ExecutionError::ContractError(format!(
                        "param_key '{}' is not on the governable allowlist",
                        param_key
                    )));
                }
                if let Err(e) = validate_param_value(param_key, param_value) {
                    return Err(ExecutionError::ContractError(format!(
                        "invalid param_value: {}",
                        e
                    )));
                }

                let proposal_id = db.all_proposals().len() as u64;
                let proposal = GovernanceProposal {
                    proposal_id,
                    proposer: tx.sender,
                    title: title.clone(),
                    param_key: param_key.clone(),
                    param_value: param_value.clone(),
                    start_epoch: current_epoch,
                    end_epoch: current_epoch + voting_epochs,
                    votes_for: 0,
                    votes_against: 0,
                    status: ProposalStatus::Active,
                    created_at: current_epoch,
                    voters: std::collections::HashSet::new(),
                };
                db.put_proposal(proposal);
            }
            GovernanceAction::CastVote { proposal_id, vote } => {
                let proposal = db.get_proposal(*proposal_id).cloned();
                let mut proposal = proposal.ok_or_else(|| {
                    ExecutionError::ContractError(format!("proposal {} not found", proposal_id))
                })?;

                if proposal.status != ProposalStatus::Active {
                    return Err(ExecutionError::ContractError(
                        "proposal is not active".to_string(),
                    ));
                }

                if current_epoch > proposal.end_epoch {
                    // Voting closed — finalize using Gap-A #4 quorum + super-majority.
                    proposal.status = decide_proposal_outcome(&proposal);
                    db.put_proposal(proposal);
                    return Err(ExecutionError::ContractError(
                        "voting period has ended".to_string(),
                    ));
                }

                if proposal.voters.contains(&tx.sender) {
                    return Err(ExecutionError::ContractError(
                        "duplicate vote: account has already voted on this proposal".into(),
                    ));
                }
                proposal.voters.insert(tx.sender);

                // Gap-A #4: cap per-voter weight so a whale cannot pass solo.
                let voter_balance = db.get_account(&tx.sender).map(|a| a.balance).unwrap_or(0);
                let voter_weight = voter_balance.min(MAX_VOTE_WEIGHT);
                if *vote {
                    proposal.votes_for = proposal.votes_for.saturating_add(voter_weight);
                } else {
                    proposal.votes_against = proposal.votes_against.saturating_add(voter_weight);
                }

                // The activation step is deferred to finalize_expired_proposals
                // so the GOVERNANCE_TIMELOCK_EPOCHS window is enforced.

                db.put_proposal(proposal);
            }
        }

        Ok(())
    }

    fn execute_multisig(
        &self,
        db: &mut dyn StateDB,
        tx: &MultiSigTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        if (tx.signatures.len() as u8) < tx.threshold {
            return Err(ExecutionError::ContractError(format!(
                "multi-sig requires {} signatures, got {}",
                tx.threshold,
                tx.signatures.len()
            )));
        }

        let mut seen_signers = std::collections::HashSet::new();
        for (signer_addr, _) in &tx.signatures {
            if !seen_signers.insert(signer_addr) {
                return Err(ExecutionError::ContractError(
                    "duplicate signer in multisig transaction".into(),
                ));
            }
            if !tx.signers.contains(signer_addr) {
                return Err(ExecutionError::ContractError(
                    "signer not in authorized signers list".to_string(),
                ));
            }
        }

        let sender = db.get_or_create_account(&tx.multisig_address);
        if sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        sender.nonce += 1;
        // Nonce mutated — stamp the demurrage anchor.
        sender.last_touched_epoch = epoch;

        Ok(())
    }

    /// Execute a UserOp transaction.
    ///
    /// **Atomicity / replay protection** (audit `end_to_end_audit_2026_04_27.md`
    /// §3 was concerned about Block-STM MVCC retries draining the paymaster):
    ///
    /// 1. UserOps execute **serial only** — `block_stm.rs:722` rejects
    ///    them from the parallel path with `"user-op txs execute in
    ///    serial phase"`. So MVCC-retry-drain doesn't apply.
    ///
    /// 2. Even if UserOps moved to the parallel path, the sender-nonce
    ///    check at the top of this function is a once-per-`(sender,
    ///    nonce)` gate. A second execution of the same tx (after the
    ///    nonce was bumped) sees `sender.nonce == tx.nonce + 1` and
    ///    returns `InvalidNonce`. The paymaster cannot be debited
    ///    twice for the same `(sender, nonce)` pair.
    ///
    /// 3. `tx_access_keys` declares both `tx.sender` and
    ///    `tx.paymaster` as access keys, so the partitioner schedules
    ///    UserOps sharing a paymaster onto the same lane — they
    ///    execute in tx-order with no in-block race.
    ///
    /// The audit's "no paymaster nonce" concern is addressed by
    /// (2) — the sender's nonce IS the per-tx idempotency token.
    /// A separate paymaster-side nonce would only be needed if the
    /// chain ever processed two distinct UserOps from different
    /// senders with overlapping access patterns *and* paid by the
    /// same paymaster *and* got duplicated by a scheduler bug —
    /// every layer of that conjunction is independently prevented.
    fn execute_user_op(
        &self,
        db: &mut dyn StateDB,
        tx: &evaporchain_types::UserOpTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        let sender = db.get_or_create_account(&tx.sender);
        if sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        sender.nonce += 1;
        // Nonce mutated — stamp the demurrage anchor.
        sender.last_touched_epoch = epoch;

        if let Some(ref paymaster) = tx.paymaster {
            // Phase 4.1 (2026-05-03): wire paymaster_nonce into the
            // execution path. The field has existed on UserOpTx since
            // an earlier audit pass but `execute_user_op` ignored it —
            // leaving the replay protection unwired. The paymaster's
            // own `nonce` field doubles as a sponsorship counter
            // (operationally a paymaster does not also send its own
            // txs; it's a sponsorship-only account by convention).
            let paymaster_nonce = tx.paymaster_nonce.ok_or_else(|| {
                ExecutionError::ContractError(
                    "UserOpTx with paymaster must include paymaster_nonce \
                     (replay protection — see audit §3 / 2026-05-03 closure)"
                        .into(),
                )
            })?;
            let pm = db.get_or_create_account(paymaster);
            if pm.nonce != paymaster_nonce {
                return Err(ExecutionError::InvalidNonce {
                    expected: pm.nonce,
                    got: paymaster_nonce,
                });
            }
            let total_gas_cost = tx.call_gas_limit.saturating_add(GAS_USER_OP);
            if pm.balance < total_gas_cost {
                return Err(ExecutionError::InsufficientGas {
                    account: hex::encode(paymaster),
                    required: total_gas_cost,
                    available: pm.balance,
                });
            }
            pm.balance = pm.balance.saturating_sub(total_gas_cost);
            pm.nonce = pm.nonce.saturating_add(1);
            // Paymaster's balance just changed — reset its anchor too.
            pm.last_touched_epoch = epoch;
        }

        Ok(())
    }

    /// Execute UpgradeContract — closes K-10 (and ships the mainnet
    /// bytecode-swap surface).
    ///
    /// Authorization is one of two paths, disambiguated by whether
    /// `admin_signature` is present on the tx:
    ///
    /// **Path A — admin upgrade.** `tx.admin_signature` is `Some`. The
    /// chain verifies the ML-DSA-65 signature over the canonical payload
    /// `JSON({type:"upgrade_contract",contract_id,new_bytecode_hash_hex,
    /// nonce})` (same shape as `settle_demurrage`). The supplied
    /// `admin_public_key` must equal `contract.admin` (which must not
    /// be `None` — frozen contracts cannot use the admin path).
    ///
    /// **Path B — governance amendment.** `tx.admin_signature` is `None`
    /// (or admin verification rejects the signature in a way that asks
    /// us to fall through to governance). The chain enforces by stake
    /// quorum: `tx.endorser_stakes.iter().sum() >= tx.required_stake`.
    /// Mirrors the pattern in `/api/governance/fork_choice_mode` —
    /// no body signature on the governance side, just stake totals.
    ///
    /// Either path runs after:
    ///   1. Sender nonce check + bump on `tx.owner`.
    ///   2. `BLAKE3(tx.new_bytecode) == tx.new_bytecode_hash` check —
    ///      prevents bait-and-switch between the hash that the admin
    ///      signed (or that the quorum endorsed) and the actual bytes.
    ///
    /// On success the bytecode is swapped via
    /// `ScriptEngine::upgrade_contract`, which preserves contract
    /// state, energy, half_life, last_refreshed, creator, admin, and
    /// the schema-compatibility invariant. `upgrade_count` is bumped
    /// inside the engine.
    fn execute_upgrade_contract(
        &mut self,
        db: &mut dyn StateDB,
        tx: &evaporchain_types::UpgradeContractTx,
        current_epoch: u64,
    ) -> Result<(), ExecutionError> {
        // ── 1. Nonce check + bump ────────────────────────────────────
        let sender = db.get_or_create_account(&tx.owner);
        if sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        sender.nonce = sender.nonce.saturating_add(1);
        // Nonce mutated — stamp the demurrage anchor.
        sender.last_touched_epoch = current_epoch;

        // ── 2. Bytecode hash binding ─────────────────────────────────
        let computed_hash = blake3::hash(&tx.new_bytecode);
        if computed_hash.as_bytes() != &tx.new_bytecode_hash {
            return Err(ExecutionError::ContractError(format!(
                "UpgradeContract: new_bytecode_hash mismatch — computed {} \
                 vs supplied {}",
                hex::encode(computed_hash.as_bytes()),
                hex::encode(tx.new_bytecode_hash)
            )));
        }

        // ── 3. Look up contract ──────────────────────────────────────
        let contract = self
            .script_engine
            .get_contract(tx.contract_id)
            .ok_or_else(|| {
                ExecutionError::ContractError(format!(
                    "UpgradeContract: contract {} not found",
                    tx.contract_id
                ))
            })?;
        let contract_admin = contract.admin;

        // ── 4. Pick authorization path ───────────────────────────────
        let auth = if let Some(sig_bytes) = tx.admin_signature.as_deref() {
            // Path A — admin upgrade. Require pk to be present and to
            // match contract.admin; verify ML-DSA-65 sig over the
            // canonical settle-demurrage-style payload.
            let pk_bytes = tx.admin_public_key.as_deref().ok_or_else(|| {
                ExecutionError::ContractError(
                    "UpgradeContract: admin_signature present but admin_public_key missing".into(),
                )
            })?;

            let admin_addr = contract_admin.ok_or_else(|| {
                ExecutionError::ContractError(format!(
                    "UpgradeContract: contract {} is frozen (admin = None) — \
                     admin path unavailable, use governance quorum",
                    tx.contract_id
                ))
            })?;

            // Bind admin pk to the contract.admin address. Canonical
            // derivation across the chain is `blake3(public_key_bytes)`
            // — see `generate_address_from_pubkey` in
            // crates/evaporchain-node/src/auth.rs L115-120.
            let derived_addr: [u8; 32] = *blake3::hash(pk_bytes).as_bytes();
            if derived_addr != admin_addr {
                return Err(ExecutionError::ContractError(format!(
                    "UpgradeContract: admin_public_key does not match \
                     contract.admin (derived {} vs admin {})",
                    hex::encode(derived_addr),
                    hex::encode(admin_addr)
                )));
            }

            // Canonical payload — mirrors `settle_demurrage` in
            // crates/evaporchain-node/src/api.rs L5499.
            let canonical = format!(
                "{{\"type\":\"upgrade_contract\",\"contract_id\":{},\"new_bytecode_hash_hex\":\"{}\",\"nonce\":{}}}",
                tx.contract_id,
                hex::encode(tx.new_bytecode_hash),
                tx.nonce
            );

            use evaporchain_crypto::signatures::MlDsaVerifier;
            if !MlDsaVerifier::verify(canonical.as_bytes(), sig_bytes, pk_bytes) {
                return Err(ExecutionError::ContractError(
                    "UpgradeContract: admin_signature verification failed".into(),
                ));
            }

            evaporchain_script::UpgradeAuth::Admin(admin_addr)
        } else {
            // Path B — governance quorum. No signature on the body for
            // this path; the chain enforces by stake totals (mirrors
            // ForkChoiceAmendReq in api.rs L1905).
            let total: u64 = tx
                .endorser_stakes
                .iter()
                .copied()
                .fold(0u64, |a, b| a.saturating_add(b));
            if total < tx.required_stake {
                return Err(ExecutionError::ContractError(format!(
                    "UpgradeContract: governance quorum not met — \
                     endorser_stakes sum {} < required_stake {}",
                    total, tx.required_stake
                )));
            }
            if tx.required_stake == 0 {
                // Refuse a quorum of zero — mirrors the implicit
                // "must have at least one endorsement" sanity check
                // that the fork-choice amendment relies on.
                return Err(ExecutionError::ContractError(
                    "UpgradeContract: governance path requires required_stake > 0".into(),
                ));
            }
            evaporchain_script::UpgradeAuth::Governance
        };

        // ── 5. Apply the swap ────────────────────────────────────────
        let new_source = std::str::from_utf8(&tx.new_bytecode).map_err(|_| {
            ExecutionError::ContractError(
                "UpgradeContract: new_bytecode is not valid UTF-8 EvaporScript source".into(),
            )
        })?;
        self.script_engine
            .upgrade_contract(tx.contract_id, new_source, auth, current_epoch)
            .map_err(|e| ExecutionError::ContractError(e.to_string()))?;

        // Stamp the contract owner's account demurrage anchor — we did
        // mutate state on their behalf.
        let owner_acct = db.get_or_create_account(&tx.owner);
        owner_acct.last_touched_epoch = current_epoch;

        Ok(())
    }

    /// Tick the vesting registry: walk every active VestingSchedule,
    /// compute pending release at `current_epoch`, credit the
    /// beneficiary's transparent balance, and bump the schedule's
    /// `released_amount`. Fully-vested schedules are removed from the
    /// registry to keep the active set bounded.
    ///
    /// Addresses the 35% Foundation Treasury centralization concern:
    /// large genesis allocations can be wrapped in a VestingSchedule so
    /// they release thermodynamically over time, instead of being a
    /// single account with a huge unconstrained balance from epoch 0.
    fn tick_vesting(&self, db: &mut dyn StateDB, current_epoch: u64) {
        let schedules = db.all_vesting_schedules();
        for sched in schedules {
            let pending = sched.pending_release_at(current_epoch);
            if pending == 0 && !sched.is_fully_vested() {
                continue;
            }
            if pending > 0 {
                let acct = db.get_or_create_account(&sched.beneficiary);
                acct.balance = acct.balance.saturating_add(pending);
                // Vesting release credits balance — refresh demurrage anchor.
                acct.last_touched_epoch = current_epoch;
            }
            let mut updated = sched.clone();
            updated.released_amount = updated.released_amount.saturating_add(pending);
            if updated.is_fully_vested() {
                db.remove_vesting_schedule(updated.id);
            } else {
                db.put_vesting_schedule(updated);
            }
        }
    }

    fn collect_storage_rent(&mut self, db: &mut dyn StateDB, current_epoch: u64) {
        let addresses = db.all_account_addresses();
        for addr in addresses {
            let rent_info = {
                let acct = match db.get_account(&addr) {
                    Some(a) => a,
                    None => continue,
                };
                if acct.storage_bytes == 0 {
                    continue;
                }
                let rent = acct
                    .storage_bytes
                    .saturating_mul(evaporchain_types::STORAGE_RENT_PER_BYTE_PER_EPOCH);
                (rent, acct.balance)
            };
            let acct = db.get_or_create_account(&addr);
            // Track how much actually flowed off the account so we can
            // accrue the same amount into the refresh pool — closes
            // the §1.2 conservation loop.
            let actually_debited;
            if acct.balance >= rent_info.0 {
                acct.balance -= rent_info.0;
                actually_debited = rent_info.0;
            } else {
                // Account is being zeroed out by storage rent — engrave
                // the chain's small-deaths memorial via evaporchain-
                // tombstone before we wipe state. Per doctrine §A2.5
                // the eulogy trie is the deliberate exception to §2.2's
                // anti-immutability rule.
                actually_debited = acct.balance;
                let final_balance_before_wipe = acct.balance;
                acct.balance = 0;
                acct.storage_deposit = 0;
                acct.storage_bytes = 0;
                let tombstone = evaporchain_tombstone::mint(
                    addr,
                    final_balance_before_wipe,
                    current_epoch,
                    evaporchain_tombstone::CauseOfDeath::RentExhausted,
                );
                // Re-insertion would mean an account evaporated twice
                // — impossible by construction. Silently ignore the
                // already-memorialised case so a buggy iteration
                // doesn't take down the chain.
                let _ = self.eulogy_trie.insert(addr, tombstone);
            }
            // Accrue the debited rent into the protocol-owned refresh
            // pool under SYSTEM_REFRESH_NAMESPACE. Energy is never
            // destroyed (§1.2) — only redirected.
            if actually_debited > 0 {
                self.refresh_pool.accrue(
                    SYSTEM_REFRESH_NAMESPACE.to_vec(),
                    actually_debited,
                    current_epoch,
                );
            }
        }
    }

    fn apply_governance_params(&mut self, db: &dyn StateDB) {
        if let Some(val) = db.get_governance_param("block_gas_limit") {
            if let Ok(limit) = val.parse::<u64>() {
                self.block_gas_limit = limit;
            }
        }
        if let Some(ref mut fc) = self.fee_controller {
            if let Some(val) = db.get_governance_param("base_fee_floor") {
                if let Ok(floor) = val.parse::<u64>() {
                    fc.min_base_fee = floor;
                }
            }
            if let Some(val) = db.get_governance_param("base_fee_ceiling") {
                if let Ok(ceiling) = val.parse::<u64>() {
                    fc.max_base_fee = ceiling;
                }
            }
            if let Some(val) = db.get_governance_param("target_gas_utilization") {
                if let Ok(target) = val.parse::<f64>() {
                    if (0.0..=1.0).contains(&target) {
                        // PidFeeController stores target_utilization as
                        // f64 in [0.0, 1.0]. (Parallel-session draft
                        // referenced a `_ppm: u32` shape that was never
                        // landed on the struct — fall back to the
                        // actual field.)
                        fc.target_utilization = target;
                    }
                }
            }
        }
    }

    fn finalize_expired_proposals(&self, db: &mut dyn StateDB, current_epoch: u64) {
        // (a) Decide outcome of any Active proposals whose voting window
        //     closed. Quorum + super-majority via decide_proposal_outcome.
        let expired_active: Vec<GovernanceProposal> = db
            .all_proposals()
            .iter()
            .filter(|p| p.status == ProposalStatus::Active && current_epoch > p.end_epoch)
            .cloned()
            .cloned()
            .collect();

        for mut proposal in expired_active {
            proposal.status = decide_proposal_outcome(&proposal);
            match proposal.status {
                ProposalStatus::Passed => {
                    info!(
                        proposal_id = proposal.proposal_id,
                        param = proposal.param_key,
                        value = proposal.param_value,
                        activates_at_epoch = proposal.end_epoch + GOVERNANCE_TIMELOCK_EPOCHS,
                        "Governance proposal passed — entering timelock window"
                    );
                }
                _ => {
                    debug!(
                        proposal_id = proposal.proposal_id,
                        for_weight = proposal.votes_for,
                        against_weight = proposal.votes_against,
                        voters = proposal.voters.len(),
                        "Governance proposal rejected (no quorum or super-majority)"
                    );
                }
            }
            db.put_proposal(proposal);
        }

        // (b) Activate any Passed proposals whose timelock has elapsed.
        let timelock_due: Vec<GovernanceProposal> = db
            .all_proposals()
            .iter()
            .filter(|p| {
                p.status == ProposalStatus::Passed
                    && current_epoch >= p.end_epoch.saturating_add(GOVERNANCE_TIMELOCK_EPOCHS)
            })
            .cloned()
            .cloned()
            .collect();

        for mut proposal in timelock_due {
            db.put_governance_param(proposal.param_key.clone(), proposal.param_value.clone());
            proposal.status = ProposalStatus::Executed;
            info!(
                proposal_id = proposal.proposal_id,
                param = proposal.param_key,
                value = proposal.param_value,
                "Governance: timelock elapsed — parameter activated"
            );
            db.put_proposal(proposal);
        }
    }

    pub fn execute_cross_shard_messages(
        &mut self,
        db: &mut dyn StateDB,
        messages: Vec<evaporchain_sharding::cross_shard::CrossShardMessage>,
        epoch: u64,
    ) -> Vec<evaporchain_sharding::cross_shard::CrossShardReceipt> {
        use evaporchain_sharding::cross_shard::{CrossShardReceipt, MessagePayload};

        let mut receipts = Vec::with_capacity(messages.len());

        for msg in messages {
            let (success, result_hash) = match &msg.payload {
                MessagePayload::Transfer { from, amount } => {
                    let mut from_addr = [0u8; 32];
                    from_addr[..20].copy_from_slice(from);
                    let mut to_addr = [0u8; 32];
                    to_addr[..20].copy_from_slice(&msg.target_object);

                    let from_acct = db.get_or_create_account(&from_addr);
                    if from_acct.balance >= *amount {
                        from_acct.balance -= *amount;
                        // Cross-shard transfer mutates sender balance.
                        from_acct.last_touched_epoch = epoch;
                        let to_acct = db.get_or_create_account(&to_addr);
                        to_acct.balance += *amount;
                        // ...and receiver balance.
                        to_acct.last_touched_epoch = epoch;
                        let mut h = blake3::Hasher::new();
                        h.update(&from_addr);
                        h.update(&to_addr);
                        h.update(&amount.to_le_bytes());
                        (true, *h.finalize().as_bytes())
                    } else {
                        (false, [0u8; 32])
                    }
                }
                MessagePayload::Reference { source_object } => {
                    let mut obj_id = [0u8; 32];
                    obj_id[..20].copy_from_slice(source_object);
                    let exists = db.get_object(&obj_id).is_some();
                    let mut h = blake3::Hasher::new();
                    h.update(source_object);
                    h.update(&[exists as u8]);
                    (exists, *h.finalize().as_bytes())
                }
                MessagePayload::Query { key } => {
                    let mut h = blake3::Hasher::new();
                    h.update(key.as_bytes());
                    (true, *h.finalize().as_bytes())
                }
                MessagePayload::Eviction { .. } => {
                    let mut obj_id = [0u8; 32];
                    obj_id[..20].copy_from_slice(&msg.target_object);
                    let evicted = db.get_object(&obj_id).is_some();
                    if evicted {
                        db.delete_object(&obj_id);
                    }
                    (evicted, [0u8; 32])
                }
            };

            receipts.push(CrossShardReceipt {
                message_id: msg.id,
                from_shard: msg.from_shard,
                to_shard: msg.to_shard,
                success,
                result_hash,
                processed_at: epoch,
            });
        }

        receipts
    }
}

impl ExecutionEngine for SimpleExecutor {
    fn execute_block(
        &mut self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<BlockExecutionResult, ExecutionError> {
        // Lane B.2: stamp protocol_version onto privacy executor for
        // dual-mode double-spend gating (v0=db / v1+=PNT).
        self.privacy_executor
            .set_protocol_version(block.protocol_version);
        // Pre-block §1.2 conservation snapshot (read-only over StateDB).
        let conservation_before = crate::energy_audit::compartment_snapshot_with_pool(
            db,
            self.refresh_pool.total_accrued(),
        );
        self.apply_governance_params(db);
        self.finalize_expired_proposals(db, block.epoch);

        let mut txs_executed = 0;
        let mut txs_failed = 0;
        let mut gas_used = 0u64;
        let mut total_fees = 0u64;
        let base_fee = self.fee_controller.as_ref().map_or(0, |fc| fc.base_fee);
        let mut validator_key_rotations: Vec<ValidatorKeyRotation> = Vec::new();
        let mut tx_outcomes: Vec<TxOutcome> = Vec::with_capacity(block.transactions.len());

        // Execute transactions
        for tx in &block.transactions {
            let tx_hash = tx.tx_hash();
            // Signature verification (if enabled)
            if let Err(e) = self.verify_tx_signature(tx) {
                debug!(error = %e, "Signature verification failed");
                txs_failed += 1;
                tx_outcomes.push(TxOutcome {
                    tx_hash,
                    success: false,
                    error: Some(format!("signature: {e}")),
                    gas_used: 0,
                });
                continue;
            }

            let tx_gas = Self::estimate_gas(tx);

            // Enforce per-block gas limit: skip transactions that would exceed it.
            // Re-audit (2026-05-02): use checked_add so gas_used near
            // u64::MAX with non-zero tx_gas doesn't wrap to a small
            // value and silently bypass the limit. Overflow is a
            // skip, same as exceeding the limit.
            let exceeds = match gas_used.checked_add(tx_gas) {
                Some(p) => p > self.block_gas_limit,
                None => true, // overflow → treat as exceeded
            };
            if self.block_gas_limit > 0 && exceeds {
                debug!(
                    gas_used,
                    tx_gas,
                    block_gas_limit = self.block_gas_limit,
                    "Skipping transaction: block gas limit would be exceeded"
                );
                txs_failed += 1;
                tx_outcomes.push(TxOutcome {
                    tx_hash,
                    success: false,
                    error: Some(format!(
                        "block gas limit exceeded (used={gas_used}, tx={tx_gas}, limit={})",
                        self.block_gas_limit
                    )),
                    gas_used: 0,
                });
                continue;
            }

            // Compute and deduct fees BEFORE execution (if fee controller enabled)
            let tx_fee = if let Some(fc) = &self.fee_controller {
                let gas_fee = fc.compute_gas_fee(tx_gas, 0);
                let extra_fee = match tx {
                    Transaction::CreateObject(create) => {
                        fc.compute_creation_deposit(create.data.len())
                    }
                    Transaction::Refresh(refresh) => fc.compute_refresh_fee(refresh.energy_deposit),
                    _ => 0,
                };
                let total_tx_fee = gas_fee + extra_fee;

                // Deduct fee from sender's balance before executing
                if let Some(sender_addr) = tx.sender() {
                    let sender = db.get_or_create_account(sender_addr);
                    if sender.balance < total_tx_fee {
                        debug!(
                            sender = hex::encode(sender_addr),
                            available = sender.balance,
                            required = total_tx_fee,
                            "Insufficient balance for gas fees"
                        );
                        txs_failed += 1;
                        tx_outcomes.push(TxOutcome {
                            tx_hash,
                            success: false,
                            error: Some(format!(
                                "insufficient balance for gas: {}/{}",
                                sender.balance, total_tx_fee
                            )),
                            gas_used: 0,
                        });
                        continue;
                    }
                    // Deduct fees upfront (burned — deflationary model)
                    sender.balance -= total_tx_fee;
                    // Fee deduction is a balance mutation — refresh anchor.
                    sender.last_touched_epoch = block.epoch;
                }
                total_tx_fee
            } else {
                0
            };

            // Snapshot sender state before execution for revert-on-failure
            let sender_snapshot = tx
                .sender()
                .and_then(|addr| db.get_account(addr).map(|acct| (acct.balance, acct.nonce)));

            let result = match tx {
                Transaction::Transfer(transfer) => self.execute_transfer(db, transfer, block.epoch),
                Transaction::CreateObject(create) => {
                    self.execute_create_object(db, create, block.epoch)
                }
                Transaction::Refresh(refresh) => self.execute_refresh(db, refresh, block.epoch),
                Transaction::DeployContract(deploy) => {
                    self.execute_deploy_contract(db, deploy, block.epoch)
                }
                Transaction::CallContract(call) => {
                    let res = self.execute_call_contract(call);
                    // M8 (audit 2026-05-02): on success, stamp the caller's
                    // demurrage anchor so an active contract user isn't
                    // billed demurrage on a stale anchor while their
                    // balance is being touched indirectly via contract state.
                    if res.is_ok() {
                        if let Some(acct) = db.get_account_mut(&call.caller) {
                            acct.last_touched_epoch = block.epoch;
                        }
                    }
                    res
                }
                Transaction::DeployScript(deploy) => {
                    self.execute_deploy_script(db, deploy, block.epoch)
                }
                Transaction::CallScript(call) => {
                    let res = self.execute_call_script(call);
                    if res.is_ok() {
                        if let Some(acct) = db.get_account_mut(&call.caller) {
                            acct.last_touched_epoch = block.epoch;
                        }
                    }
                    res
                }
                Transaction::ValidatorStake(stake) => {
                    self.execute_validator_stake(db, stake, block.epoch)
                }
                Transaction::ValidatorExit(exit) => {
                    self.execute_validator_exit(db, exit, block.epoch)
                }
                Transaction::ValidatorClaimStake(claim) => {
                    self.execute_validator_claim_stake(db, claim, block.epoch)
                }
                Transaction::Shield(shield) => {
                    self.privacy_executor.set_epoch(block.epoch);
                    self.privacy_executor
                        .execute_shield(db, shield)
                        .map(|_| ())
                        .map_err(|e| ExecutionError::ContractError(e.to_string()))
                }
                Transaction::Unshield(unshield) => {
                    self.privacy_executor.set_epoch(block.epoch);
                    self.privacy_executor
                        .execute_unshield(db, unshield)
                        .map(|_| ())
                        .map_err(|e| ExecutionError::ContractError(e.to_string()))
                }
                Transaction::PrivateTransfer(ptx) => {
                    self.privacy_executor.set_epoch(block.epoch);
                    self.privacy_executor
                        .execute_private_transfer(db, ptx)
                        .map(|_| ())
                        .map_err(|e| ExecutionError::ContractError(e.to_string()))
                }
                Transaction::Deferred(dtx) => self
                    .deferred_queue
                    .submit(dtx.clone())
                    .map(|_| ())
                    .map_err(|e| ExecutionError::ContractError(e.to_string())),
                Transaction::Blob(blob) => {
                    if blob.data.is_empty() {
                        Err(ExecutionError::ContractError(
                            "blob data cannot be empty".into(),
                        ))
                    } else if blob.data.len() > MAX_BLOB_SIZE {
                        Err(ExecutionError::ContractError(format!(
                            "blob size {} exceeds limit {}",
                            blob.data.len(),
                            MAX_BLOB_SIZE
                        )))
                    } else if blob.namespace_id == 0 {
                        Err(ExecutionError::ContractError(
                            "reserved namespace_id 0".into(),
                        ))
                    } else {
                        Ok(())
                    }
                }
                Transaction::Governance(gov) => self.execute_governance(db, gov, block.epoch),
                Transaction::MultiSig(msig) => self.execute_multisig(db, msig, block.epoch),
                Transaction::UserOp(uop) => self.execute_user_op(db, uop, block.epoch),
                Transaction::UpgradeContract(up) => {
                    self.execute_upgrade_contract(db, up, block.epoch)
                }
                Transaction::Delegate(d) => self.execute_delegate(db, d, block.epoch),
                Transaction::Undelegate(u) => self.execute_undelegate(db, u, block.epoch),
                Transaction::RotateValidatorKey(rot) => {
                    // Closes punch-list 4b. Validation order: cheap checks
                    // first (effective_epoch + nonce + stake-record
                    // existence + key length), then BLS PoP verification
                    // on the new key. Old-key continuity (`bls_pop_old`)
                    // is verified by the consensus layer when it applies
                    // the rotation post-commit, since SimpleExecutor does
                    // not own the live ValidatorSet.
                    if rot.effective_epoch < block.epoch {
                        Err(ExecutionError::ContractError(format!(
                            "RotateValidatorKey: effective_epoch {} is in the past (current {})",
                            rot.effective_epoch, block.epoch
                        )))
                    } else if rot.new_bls_public_key.len() != 48 {
                        Err(ExecutionError::ContractError(format!(
                            "RotateValidatorKey: new_bls_public_key must be 48 bytes (got {})",
                            rot.new_bls_public_key.len()
                        )))
                    } else {
                        let stake_addr =
                            db.get_stake(rot.validator_id).map(|s| s.validator_address);
                        match stake_addr {
                            None => Err(ExecutionError::ContractError(format!(
                                "RotateValidatorKey: validator_id {} has no stake record",
                                rot.validator_id
                            ))),
                            Some(addr) if addr != rot.validator_address => {
                                Err(ExecutionError::ContractError(format!(
                                    "RotateValidatorKey: validator_id {} address mismatch",
                                    rot.validator_id
                                )))
                            }
                            Some(_) => {
                                let expected_nonce = db
                                    .get_account(&rot.validator_address)
                                    .map_or(0, |a| a.nonce);
                                if rot.nonce != expected_nonce {
                                    Err(ExecutionError::ContractError(format!(
                                        "RotateValidatorKey: nonce mismatch (expected {}, got {})",
                                        expected_nonce, rot.nonce
                                    )))
                                } else if !{
                                    use evaporchain_crypto::signatures::{
                                        BlsPublicKey, BlsSignature, BlsVerifier,
                                    };
                                    let pk = BlsPublicKey(rot.new_bls_public_key.clone());
                                    let pop = BlsSignature(rot.bls_pop_new.clone());
                                    BlsVerifier::verify_proof_of_possession(&pk, &pop)
                                } {
                                    Err(ExecutionError::ContractError(
                                        "RotateValidatorKey: bls_pop_new failed verification"
                                            .into(),
                                    ))
                                } else {
                                    if let Some(acct) = db.get_account_mut(&rot.validator_address) {
                                        acct.nonce = acct.nonce.saturating_add(1);
                                        // Nonce mutated — stamp anchor.
                                        acct.last_touched_epoch = block.epoch;
                                    }
                                    validator_key_rotations.push(ValidatorKeyRotation {
                                        validator_id: rot.validator_id,
                                        new_bls_public_key: rot.new_bls_public_key.clone(),
                                        bls_pop_old: rot.bls_pop_old.clone(),
                                        new_bls_pop: rot.bls_pop_new.clone(),
                                        prev_key_expiry_epoch: rot
                                            .effective_epoch
                                            .saturating_add(KEY_ROTATION_GRACE_EPOCHS),
                                    });
                                    Ok(())
                                }
                            }
                        }
                    }
                }
                Transaction::ClaimDelegation(c) => {
                    self.execute_claim_delegation(db, c, block.epoch)
                }
                // Phase 3.5 (Lane Q.1): protocol-issued refund moves
                // `amount` from attacker → victim. Determinism contract
                // (Phase 3.2 — caller must match validator's
                // `mev_observations` + `mev_attacker_stats`) is
                // enforced at the consensus layer; this execution path
                // only handles balance movement.
                Transaction::Refund(refund) => self.execute_refund(db, refund, block.epoch),
            };

            match result {
                Ok(()) => {
                    txs_executed += 1;
                    gas_used += tx_gas;
                    total_fees += tx_fee;
                    tx_outcomes.push(TxOutcome {
                        tx_hash,
                        success: true,
                        error: None,
                        gas_used: tx_gas,
                    });
                }
                Err(e) => {
                    // Revert sender state changes from the failed execution,
                    // but KEEP the fee deduction (sender still pays for gas used).
                    // Snapshot was taken AFTER fee deduction but BEFORE execution,
                    // so restoring it reverts execution changes while keeping fees burned.
                    if let (Some(sender_addr), Some((snap_balance, snap_nonce))) =
                        (tx.sender(), sender_snapshot)
                    {
                        if let Some(acct) = db.get_account_mut(sender_addr) {
                            acct.balance = snap_balance;
                            acct.nonce = snap_nonce;
                        }
                    }
                    debug!(error = %e, "Transaction failed — state reverted, fee kept");
                    txs_failed += 1;
                    total_fees += tx_fee; // Fee is still burned even on failure
                    tx_outcomes.push(TxOutcome {
                        tx_hash,
                        success: false,
                        error: Some(e.to_string()),
                        gas_used: tx_gas,
                    });
                }
            }
        }

        // Run evaporation at end of block (with MMR nullifier accumulation)
        let evap_result =
            self.evaporation_engine
                .process_epoch_with_mmr(db, block.epoch, &mut self.mmr);

        // Generate batch evaporation proof for all evaporated objects.
        // Uses ghost records (created during evaporation) as proof witnesses.
        let evaporation_proof = if !evap_result.evaporated.is_empty() {
            let mut prover = EvaporationProver::new(block.number);
            for obj_id in &evap_result.evaporated {
                if let Some(ghost) = db.get_ghost(obj_id) {
                    let obj_id_20: [u8; 20] = {
                        let mut id = [0u8; 20];
                        id.copy_from_slice(&obj_id[..20]);
                        id
                    };
                    let half_life = ghost.original_half_life.unwrap_or(10);
                    let nullifier = ghost.data_hash;
                    // For the proof, we need energy=0 at evaporation_epoch.
                    // Use half_life * 64 as a conservative initial_energy upper bound
                    // that guarantees decay to 0 at the claimed epoch.
                    // creation_epoch = evaporation_epoch - (half_life * 64) ensures
                    // 64 half-lives have passed → energy < 1 → rounds to 0.
                    let creation_epoch = ghost.evaporated_at.saturating_sub(half_life * 64);
                    let initial_energy = 1000;
                    let _ = prover.add_evaporation(EvaporationClaim {
                        object_id: obj_id_20,
                        initial_energy,
                        half_life,
                        creation_epoch,
                        evaporation_epoch: ghost.evaporated_at,
                        nullifier,
                    });
                }
            }
            Some(prover.prove())
        } else {
            None
        };

        // Tick all contracts (energy decay, auto-finalize, etc.)
        self.contract_engine.tick(block.epoch);

        // Tick all script contracts (energy decay, lifecycle hooks)
        self.script_engine.tick(block.epoch);

        // Process decay watchers (fire callbacks when energy crosses thresholds).
        let watcher_result = self.decay_watchers.process(block.epoch, db);
        for (contract_id, method, args) in &watcher_result.callbacks {
            let args_val: serde_json::Value =
                serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
            let _ = self.contract_engine.call(
                *contract_id,
                method,
                &args_val,
                &[0u8; 32], // system caller
                block.epoch,
            );
        }

        // Process deferred queue (mature deferred txs when guards are satisfied).
        let deferred_result =
            self.deferred_queue
                .process_epoch(block.epoch, db, &self.contract_engine);
        // Execute matured deferred transactions.
        for (submitter, inner_bytes, _gas_limit) in &deferred_result.matured_txs {
            match serde_json::from_slice::<Transaction>(inner_bytes) {
                Ok(inner_tx) => {
                    let result = match &inner_tx {
                        Transaction::Transfer(t) => self.execute_transfer(db, t, block.epoch),
                        Transaction::CreateObject(c) => {
                            self.execute_create_object(db, c, block.epoch)
                        }
                        Transaction::CallContract(c) => self.execute_call_contract(c),
                        Transaction::CallScript(c) => self.execute_call_script(c),
                        _ => Err(ExecutionError::ContractError(
                            "unsupported deferred inner tx type".into(),
                        )),
                    };
                    match result {
                        Ok(()) => {
                            debug!(submitter = %hex::encode(&submitter[..8]), "Matured deferred tx executed");
                            txs_executed += 1;
                        }
                        Err(e) => {
                            debug!(error = %e, "Matured deferred tx execution failed");
                            txs_failed += 1;
                        }
                    }
                }
                Err(e) => {
                    debug!(error = %e, "Failed to deserialize matured deferred tx inner bytes");
                    txs_failed += 1;
                }
            }
        }

        // Refund expired deferred tx deposits.
        for (addr, refund) in &deferred_result.refunds {
            if let Some(acct) = db.get_account_mut(addr) {
                acct.balance = acct.balance.saturating_add(*refund);
                // Deferred-tx refund credits balance — refresh anchor.
                acct.last_touched_epoch = block.epoch;
            }
        }

        // Reward distribution: route fees through tokenomics instead of pure burn
        if let Some(ref mut ra) = self.reward_accumulator {
            let producer_addr = if let Some(pid) = block.producer_id {
                db.get_stake(pid)
                    .map(|s| s.validator_address)
                    .unwrap_or_else(|| {
                        let mut addr = [0u8; 32];
                        addr[..8].copy_from_slice(&pid.to_le_bytes());
                        addr
                    })
            } else {
                [0u8; 32]
            };
            ra.process_block_rewards(db, &producer_addr, block.epoch, total_fees);

            // Lane A.3: priority bonus auto-fire (mirrors parallel.rs).
            if !block.submit_epoch_hints.is_empty() {
                let priority_sum: u64 = block
                    .submit_epoch_hints
                    .iter()
                    .filter_map(|h| h.as_ref())
                    .map(|submit| {
                        let elapsed = block.number.saturating_sub(*submit);
                        evaporchain_types::energy_at_epoch(
                            evaporchain_types::BASE_INCLUSION_ENERGY,
                            evaporchain_types::MEV_INCLUSION_HALF_LIFE_BLOCKS,
                            elapsed,
                        )
                    })
                    .fold(0u64, u64::saturating_add);
                let bonus_scale = evaporchain_types::BASE_INCLUSION_ENERGY;
                ra.apply_priority_bonus(db, &producer_addr, block.epoch, priority_sum, bonus_scale);
            }

            // Distribute staker rewards every 100 blocks
            if block.number.is_multiple_of(100) && ra.pending_staker_rewards > 0 {
                let stakers: Vec<([u8; 32], u64)> = db
                    .all_stakes()
                    .iter()
                    .filter(|s| s.unbonding_epoch.is_none())
                    .map(|s| (s.validator_address, s.staked_amount))
                    .collect();
                let distributed = ra.distribute_staker_rewards(db, &stakers, block.epoch);
                if distributed > 0 {
                    info!(
                        distributed,
                        stakers = stakers.len(),
                        block = block.number,
                        "Staker rewards distributed"
                    );
                }
            }
        }

        // Punch-list 6: gate storage-rent collection on the per-epoch
        // cursor (`last_rent_epoch`) so rent fires exactly once per
        // epoch instead of once per block. Mirrors the inline gate
        // already used by ParallelExecutor (parallel.rs:1430) and
        // Block-STM (block_stm.rs:1670). Keeping the inline pattern
        // (rather than baking it into `collect_storage_rent`) preserves
        // the existing test signatures.
        //
        // Why this matters: `STORAGE_RENT_PER_BYTE_PER_EPOCH` is a
        // per-EPOCH rate; running it per-block over-charges by
        // `blocks_per_epoch` (≈50× at 2s blocks). A Foundation account
        // would be drained roughly 50× faster than the tokenomics
        // declare. Closes the SimpleExecutor half of #6.
        if block.epoch > db.get_last_rent_epoch() {
            self.collect_storage_rent(db, block.epoch);
            // Native demurrage sweep — charges idle balances above the
            // threshold and credits the refresh pool.  Fires at the same
            // per-epoch cadence as storage rent.
            let last_rent_epoch = db.get_last_rent_epoch();
            crate::demurrage_integration::collect_demurrage(
                db,
                &mut self.refresh_pool,
                &self.demurrage_params,
                last_rent_epoch,
                block.epoch,
            );
            db.put_last_rent_epoch(block.epoch);
        }

        // Decay-Lamport logical clock — tick with total gas used this block.
        self.lamport_clock = crate::lamport_integration::tick_block(self.lamport_clock, gas_used);

        // Lane F.1: Singh-Lyapunov fee state — advance per-block against
        // the just-applied gas_used. Closes the "substrate-shipped, no
        // caller" gap from DOCTRINE_PUNCH_LIST.md §6: tick_lyapunov_fee_state
        // existed but only tests + the operator-driven REST endpoint
        // called it; the production hot path was static at genesis
        // equilibrium. Fields lived on this executor since the four-act
        // wiring; this commit makes them actually advance.
        //
        // Errors here are silently dropped: FeeControllerError fires only
        // when params validation fails (e.g. target_gas == 0), which is
        // unreachable under default_genesis. Logged at debug level if
        // future params introduce a recoverable error.
        let _ = self.tick_lyapunov_fee_state(gas_used, 1);

        // Vesting timelock release tick — runs every block (per-block
        // schedules need responsive release). Idempotent within an
        // epoch because pending_release_at == 0 once released_amount
        // catches up to vested_at.
        self.tick_vesting(db, block.epoch);

        // PNT phase auto-advance (research-buildable #8 follow-up).
        // Rotates the PhasedNullifierTree window iff the configured
        // `pnt_phase_interval_epochs` has elapsed. Stage-1 shadow-only,
        // so this has no effect on consensus state-root — the
        // per-block tick is purely operational telemetry until Stage-2
        // makes PNT authoritative.
        let _pnt_advanced = self.privacy_executor.tick_pnt_phase(block.epoch);

        // Lane E.2: EnergyVerkleTrie cold-subtree compression (mirrors
        // parallel.rs). v0=off, v1+=on.
        if block.state_root_version >= 1 {
            let _compressed = db.compress_cold_subtrees();
        }

        let state_root = db.compute_state_root();
        db.commit_state_snapshot(block.number);

        if block.number > SNAPSHOT_RETAIN_BLOCKS {
            db.prune_snapshots_before(block.number - SNAPSHOT_RETAIN_BLOCKS);
        }

        info!(
            block = block.number,
            epoch = block.epoch,
            txs_executed,
            txs_failed,
            gas_used,
            base_fee,
            total_fees,
            entered_grace = evap_result.entered_grace.len(),
            evaporated = evap_result.evaporated.len(),
            state_root = hex::encode(state_root),
            "Block executed"
        );

        let contract_events: Vec<BlockContractEvent> = self
            .pending_events
            .drain(..)
            .map(|(contract_id, event)| BlockContractEvent {
                contract_id,
                tx_index: 0,
                event,
            })
            .collect();

        // Post-block §1.2 conservation snapshot + audit.
        //
        // Governance-gated by `conservation_enforcement`:
        //   * unset / "observe" — verdict stored on
        //     `self.last_conservation_audit`; block commits regardless
        //     (legacy behaviour, kept for backward compat with chains
        //     that haven't activated enforcement).
        //   * "enforce" — a `ConservationViolation` propagates as
        //     `ExecutionError::ConservationViolation` and the block is
        //     rejected. The chain refuses to commit any block whose
        //     §1.2 invariant breaks.
        let conservation_after = crate::energy_audit::compartment_snapshot_with_pool(
            db,
            self.refresh_pool.total_accrued(),
        );
        let lambda = evaporchain_energy_kernel::ChainLambda::default_genesis();
        // epochs_elapsed = block.epoch − last_audit_epoch. On the
        // first audit (None), elapsed = 0 — the kernel's λ-decay floor
        // collapses to "no decay allowed", which is the right
        // bootstrap semantics: until we've seen one block to compare
        // against, conservation is total-preserving by definition.
        let epochs_elapsed = self
            .last_audit_epoch
            .map(|prev| block.epoch.saturating_sub(prev))
            .unwrap_or(0);
        let audit_verdict = crate::energy_audit::audit_block_step(
            &conservation_before,
            &conservation_after,
            epochs_elapsed,
            lambda,
        );
        let must_enforce = matches!(
            db.get_governance_param("conservation_enforcement"),
            Some("enforce"),
        );
        // Gate the audit verdict through the centralised
        // `evaluate_conservation_gate` helper so the branching is
        // unit-testable in isolation. `?` propagates the rejection
        // case — last_conservation_audit and last_audit_epoch stay
        // at their prior values (the next attempt compares against
        // the same baseline).
        let stored = evaluate_conservation_gate(audit_verdict, must_enforce)?;
        self.last_conservation_audit = Some(stored);
        self.last_audit_epoch = Some(block.epoch);

        let mera_root = crate::mera_integration::compute_mera_commitment(db);
        let mera_commitment = if mera_root == [0u8; 32] {
            None
        } else {
            Some(mera_root)
        };

        Ok(BlockExecutionResult {
            state_root,
            mmr_root: self.mmr.root(),
            txs_executed,
            txs_failed,
            objects_entered_grace: evap_result.entered_grace.len(),
            objects_evaporated: evap_result.evaporated.len(),
            gas_used,
            base_fee,
            total_fees,
            evaporation_proof,
            contract_events,
            cross_shard_processed: 0,
            cross_shard_receipts: Vec::new(),
            validator_key_rotations,
            mera_commitment,
            tx_outcomes,
        })
    }

    fn mmr_root(&self) -> [u8; 32] {
        self.mmr.root()
    }

    fn mmr_size(&self) -> usize {
        self.mmr.size()
    }

    fn mmr_proof(&self, leaf_index: u64) -> Option<evaporchain_crypto::accumulator::MMRProof> {
        self.mmr.prove(leaf_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_crypto::signatures::{MlDsaKeypair, Signer};
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::{Account, VestingLock};

    fn addr(byte: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = byte;
        a
    }

    fn obj_id(byte: u8) -> [u8; 32] {
        let mut id = [0u8; 32];
        id[0] = byte;
        id
    }

    fn make_block(number: u64, epoch: Epoch, txs: Vec<Transaction>) -> Block {
        Block {
            number,
            epoch,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: txs,
            timestamp: 0,
            chain_id: String::new(),
            producer_id: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
            da_row_roots: vec![],
            da_col_roots: vec![],
        }
    }

    fn fund_account(db: &mut InMemoryStateDB, byte: u8, balance: u64) {
        db.put_account(Account {
            address: addr(byte),
            balance,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
    }

    /// Helper: sign a transaction with the given keypair.
    /// Uses signing_message with empty chain_id to match executor verification.
    fn sign_tx(tx: &mut Transaction, kp: &MlDsaKeypair) {
        let msg = tx.signing_message("");
        let sig = kp.sign(&msg);
        let pk = kp.public_key_bytes();
        match tx {
            Transaction::Transfer(t) => {
                t.signature = Some(sig);
                t.public_key = Some(pk);
            }
            Transaction::Refresh(r) => {
                r.signature = Some(sig);
                r.public_key = Some(pk);
            }
            Transaction::CreateObject(c) => {
                c.signature = Some(sig);
                c.public_key = Some(pk);
            }
            Transaction::DeployContract(d) => {
                d.signature = Some(sig);
                d.public_key = Some(pk);
            }
            Transaction::CallContract(c) => {
                c.signature = Some(sig);
                c.public_key = Some(pk);
            }
            Transaction::DeployScript(d) => {
                d.signature = Some(sig);
                d.public_key = Some(pk);
            }
            Transaction::CallScript(c) => {
                c.signature = Some(sig);
                c.public_key = Some(pk);
            }
            Transaction::ValidatorStake(v) => {
                v.signature = Some(sig);
                v.public_key = Some(pk);
            }
            Transaction::ValidatorExit(v) => {
                v.signature = Some(sig);
                v.public_key = Some(pk);
            }
            Transaction::ValidatorClaimStake(v) => {
                v.signature = Some(sig);
                v.public_key = Some(pk);
            }
            Transaction::Shield(s) => {
                s.signature = Some(sig);
                s.public_key = Some(pk);
            }
            Transaction::Unshield(_) | Transaction::PrivateTransfer(_) => {}
            Transaction::Deferred(d) => {
                d.signature = Some(sig);
                d.public_key = Some(pk);
            }
            Transaction::Blob(b) => {
                b.signature = Some(sig);
                b.public_key = Some(pk);
            }
            Transaction::Governance(g) => {
                g.signature = Some(sig);
                g.public_key = Some(pk);
            }
            Transaction::MultiSig(_) => {}
            Transaction::UserOp(u) => {
                u.signature = Some(sig);
                u.public_key = Some(pk);
            }
            Transaction::UpgradeContract(u) => {
                u.signature = Some(sig);
                u.public_key = Some(pk);
            }
            Transaction::Delegate(d) => {
                d.signature = Some(sig);
                d.public_key = Some(pk);
            }
            Transaction::Undelegate(u) => {
                u.signature = Some(sig);
                u.public_key = Some(pk);
            }
            Transaction::RotateValidatorKey(r) => {
                r.signature = Some(sig);
                r.public_key = Some(pk);
            }
            Transaction::ClaimDelegation(c) => {
                c.signature = Some(sig);
                c.public_key = Some(pk);
            }
            // Refund is protocol-issued; no signature attached.
            Transaction::Refund(_) => {}
        }
    }

    // ─── Basic Transfer ───

    #[test]
    fn test_basic_transfer() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 300,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(result.txs_failed, 0);

        let sender = db.get_account(&addr(1)).unwrap();
        assert_eq!(sender.balance, 700);
        assert_eq!(sender.nonce, 1);

        let receiver = db.get_account(&addr(2)).unwrap();
        assert_eq!(receiver.balance, 300);
    }

    // ─── Insufficient Balance ───

    #[test]
    fn test_insufficient_balance() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 100);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 500,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);

        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 100);
        assert!(db.get_account(&addr(2)).is_none());
    }

    // ─── Vesting / locked balance (TOKENOMICS §2.6 / Q14) ───

    fn fund_account_with_vesting(
        db: &mut InMemoryStateDB,
        byte: u8,
        balance: u64,
        vesting: VestingLock,
    ) {
        db.put_account(Account {
            address: addr(byte),
            balance,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: Some(vesting),
        });
    }

    #[test]
    fn test_vesting_locked_at_pure_function() {
        // Cliff at epoch 100, linear release over 1000 epochs, 1000 EVP locked.
        let v = VestingLock {
            cliff_epoch: 100,
            linear_release_epochs: 1000,
            total_locked: 1000,
        };
        // Pre-cliff: fully locked.
        assert_eq!(v.locked_at(0), 1000);
        assert_eq!(v.locked_at(50), 1000);
        assert_eq!(v.locked_at(100), 1000);
        // Just past cliff: nearly fully locked.
        assert_eq!(v.locked_at(101), 999);
        // Midpoint of linear window: half locked.
        assert_eq!(v.locked_at(600), 500);
        // End of window: nothing locked.
        assert_eq!(v.locked_at(1100), 0);
        // Beyond window: stays 0.
        assert_eq!(v.locked_at(10_000), 0);

        // Cliff-only schedule (linear_release_epochs = 0): instant release after cliff.
        let cliff_only = VestingLock {
            cliff_epoch: 50,
            linear_release_epochs: 0,
            total_locked: 500,
        };
        assert_eq!(cliff_only.locked_at(50), 500);
        assert_eq!(cliff_only.locked_at(51), 0);
    }

    #[test]
    fn test_transferable_balance_with_no_vesting() {
        let acct = Account {
            address: addr(1),
            balance: 1000,
            ..Account::default()
        };
        // None vesting ⇒ entire balance transferable at any epoch.
        assert_eq!(acct.transferable_balance(0), 1000);
        assert_eq!(acct.transferable_balance(1_000_000_000), 1000);
    }

    #[test]
    fn test_transfer_blocked_by_vesting_pre_cliff() {
        let mut db = InMemoryStateDB::new();
        fund_account_with_vesting(
            &mut db,
            1,
            1000,
            VestingLock {
                cliff_epoch: 100,
                linear_release_epochs: 100,
                total_locked: 1000,
            },
        );

        // At epoch 50 (pre-cliff), all 1000 is locked. A 500 transfer must fail.
        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            50,
            50,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 500,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1, "pre-cliff transfer must be rejected");
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1000);
    }

    #[test]
    fn test_transfer_partial_unlock() {
        let mut db = InMemoryStateDB::new();
        fund_account_with_vesting(
            &mut db,
            1,
            1000,
            VestingLock {
                cliff_epoch: 100,
                linear_release_epochs: 100,
                total_locked: 1000,
            },
        );

        // At epoch 150 (50% through linear window), 500 is locked → 500 transferable.
        // A 400 transfer succeeds; balance = 600.
        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            150,
            150,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 400,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1, "400/500 transferable should succeed");
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 600);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 400);
    }

    #[test]
    fn test_transfer_after_full_unlock() {
        let mut db = InMemoryStateDB::new();
        fund_account_with_vesting(
            &mut db,
            1,
            1000,
            VestingLock {
                cliff_epoch: 100,
                linear_release_epochs: 100,
                total_locked: 1000,
            },
        );

        // At epoch 250 (well past linear window end at 200), nothing locked.
        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            250,
            250,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 999,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1);
    }

    #[test]
    fn test_validator_stake_blocked_by_vesting() {
        // Locked-balance accounts cannot stake the locked portion.
        let mut db = InMemoryStateDB::new();
        fund_account_with_vesting(
            &mut db,
            1,
            500_000,
            VestingLock {
                cliff_epoch: 1_000_000,
                linear_release_epochs: 0,
                total_locked: 500_000,
            },
        );
        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            10,
            10,
            vec![Transaction::ValidatorStake(ValidatorStakeTx {
                validator_address: addr(1),
                stake_amount: 100_000,
                validator_id: 1,
                nonce: 0,
                bls_public_key: None,
                vrf_public_key: None,
                signature: None,
                public_key: None,
            })],
        );
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(
            result.txs_failed, 1,
            "staking from locked balance must be rejected"
        );
        // Balance may have a small demurrage / storage-rent drift during the
        // test block; the key assertion is that the 100_000 stake was NOT
        // debited (which would drop balance to 400_000).
        let balance_after = db.get_account(&addr(1)).unwrap().balance;
        assert!(
            balance_after > 400_000,
            "stake debit must not have happened, got balance {balance_after}"
        );
    }

    // ─── Crooks-MEV Refund (Lane Q.1 / Phase 3.5) ───

    #[test]
    fn test_refund_moves_balance_attacker_to_victim() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000); // attacker
        fund_account(&mut db, 2, 100); // victim

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Refund(RefundTx {
                source_block_height: 0,
                source_observation_idx: 0,
                attacker: addr(1),
                victim: addr(2),
                amount: 250,
                settle_block_height: 1,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(result.txs_failed, 0);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 750);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 350);
    }

    #[test]
    fn test_refund_self_refund_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Refund(RefundTx {
                source_block_height: 0,
                source_observation_idx: 0,
                attacker: addr(1),
                victim: addr(1), // self-refund
                amount: 100,
                settle_block_height: 1,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1000);
    }

    #[test]
    fn test_refund_zero_amount_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
        fund_account(&mut db, 2, 100);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Refund(RefundTx {
                source_block_height: 0,
                source_observation_idx: 0,
                attacker: addr(1),
                victim: addr(2),
                amount: 0, // zero refund
                settle_block_height: 1,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1000);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 100);
    }

    #[test]
    fn test_refund_insufficient_attacker_balance_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 50); // attacker can't cover 250
        fund_account(&mut db, 2, 100);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Refund(RefundTx {
                source_block_height: 0,
                source_observation_idx: 0,
                attacker: addr(1),
                victim: addr(2),
                amount: 250,
                settle_block_height: 1,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
        // Sender state should be reverted (snapshot restore on failure
        // path); attacker still has 50.
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 50);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 100);
    }

    // ─── Self-Transfer Rejected ───

    #[test]
    fn test_self_transfer_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(1),
                amount: 100,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1000);
    }

    // ─── Invalid Nonce ───

    #[test]
    fn test_invalid_nonce() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 100,
                nonce: 5,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
    }

    // ─── Replay Protection ───

    #[test]
    fn test_replay_protection_same_tx_twice() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        // Same transfer submitted twice in the same block
        let tx1 = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let tx2 = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let block = make_block(1, 1, vec![tx1, tx2]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        // First succeeds (nonce 0 matches), second fails (nonce now 1, but tx says 0)
        assert_eq!(result.txs_executed, 1);
        assert_eq!(result.txs_failed, 1);
    }

    #[test]
    fn test_sequential_nonces_work() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        let tx1 = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let tx2 = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 1,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let block = make_block(1, 1, vec![tx1, tx2]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 2);
        assert_eq!(result.txs_failed, 0);
        assert_eq!(db.get_account(&addr(1)).unwrap().nonce, 2);
        // 1_000_000 − 200 (two transfers) − 8 (per-block demurrage on the
        // sender's balance accrued at block.epoch=1 from last_touched_epoch=0).
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 999_792);
    }

    // ─── Object Creation with Energy ───

    #[test]
    fn test_create_object_with_energy() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);
        let mut executor = SimpleExecutor::new_for_test(7);

        let block = make_block(
            1,
            10,
            vec![Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(42),
                energy: 5000,
                half_life: 100,
                data: vec![0xDE, 0xAD],
                decay_curve: None,
                lad_mode: None,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);

        let obj = db.get_object(&obj_id(42)).unwrap();
        assert_eq!(obj.energy, 5000);
        assert_eq!(obj.half_life, 100);
        assert_eq!(obj.created_at, 10);
        assert_eq!(obj.last_refreshed, 10);
        assert_eq!(obj.state, ObjectState::Active);
        assert_eq!(obj.data, vec![0xDE, 0xAD]);
        assert_eq!(obj.owner, addr(1));
        // Default `lad_mode = None` produces a non-substructural object.
        assert!(obj.lad_mode.is_none());
    }

    #[test]
    fn test_create_object_with_decay_curve() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);
        let mut executor = SimpleExecutor::new_for_test(7);

        let block = make_block(
            1,
            10,
            vec![Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(77),
                energy: 10_000,
                half_life: 50,
                data: vec![0xCA, 0xFE],
                decay_curve: Some(evaporchain_types::DecayCurve::Linear {
                    rate_per_epoch: 100,
                }),
                lad_mode: None,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);

        let obj = db.get_object(&obj_id(77)).unwrap();
        assert_eq!(
            obj.decay_curve,
            Some(evaporchain_types::DecayCurve::Linear {
                rate_per_epoch: 100
            })
        );
    }

    // ─── Duplicate Object Creation Fails ───

    #[test]
    fn test_duplicate_object_creation_fails() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);
        let mut executor = SimpleExecutor::new_for_test(7);

        let create_tx = Transaction::CreateObject(CreateObjectTx {
            creator: addr(1),
            object_id: obj_id(42),
            energy: 1000,
            half_life: 50,
            data: vec![],
            decay_curve: None,
            lad_mode: None,
            signature: None,
            public_key: None,
        });

        let block1 = make_block(1, 1, vec![create_tx.clone()]);
        let block2 = make_block(2, 2, vec![create_tx]);

        executor.execute_block(&mut db, &block1).unwrap();
        let result = executor.execute_block(&mut db, &block2).unwrap();
        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.object_count(), 1);
    }

    // ─── Block Execution with Multiple Txs ───

    #[test]
    fn test_block_with_multiple_txs() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);
        fund_account(&mut db, 2, 5_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(2),
                    amount: 2000,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(2),
                    to: addr(3),
                    amount: 1000,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                }),
                Transaction::CreateObject(CreateObjectTx {
                    creator: addr(1),
                    object_id: obj_id(10),
                    energy: 500,
                    half_life: 50,
                    data: vec![1],
                    decay_curve: None,
                    lad_mode: None,
                    signature: None,
                    public_key: None,
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(3),
                    amount: 500,
                    nonce: 1,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                }),
            ],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 4);
        assert_eq!(result.txs_failed, 0);

        // 10_000 − 2000 (transfer) − 1000 (CreateObject MIN_STORAGE_DEPOSIT)
        // − 500 (transfer) − 98 (per-block demurrage on the sender's
        // balance accrued at block.epoch=1) = 6402.
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 6402);
        assert_eq!(db.get_account(&addr(1)).unwrap().nonce, 2);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 6000);
        assert_eq!(db.get_account(&addr(3)).unwrap().balance, 1500);
        assert_eq!(db.object_count(), 1);
        assert_ne!(result.state_root, [0u8; 32]);
    }

    // ─── Partial Block Failure ───

    #[test]
    fn test_partial_block_failure() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 500);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(2),
                    amount: 200,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(3),
                    amount: 400,
                    nonce: 1,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(4),
                    amount: 100,
                    nonce: 1,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                }),
            ],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 2);
        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 200);
    }

    // ─── Evaporation Triggered by Block Execution ───

    #[test]
    fn test_evaporation_triggered_by_block() {
        let mut db = InMemoryStateDB::new();

        db.put_object(StateObject {
            id: obj_id(1),
            owner: addr(1),
            energy: 4,
            half_life: 1,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![0xAB],
            decay_curve: None,
            lad_mode: None,
        });

        let mut executor = SimpleExecutor::new_for_test(3);

        let block1 = make_block(1, 3, vec![]);
        let r1 = executor.execute_block(&mut db, &block1).unwrap();
        assert_eq!(r1.objects_entered_grace, 1);
        assert_eq!(r1.objects_evaporated, 0);
        assert_eq!(db.object_count(), 1);
        assert_eq!(db.get_object(&obj_id(1)).unwrap().state, ObjectState::Grace);

        let block2 = make_block(2, 5, vec![]);
        let r2 = executor.execute_block(&mut db, &block2).unwrap();
        assert_eq!(r2.objects_evaporated, 0);

        let block3 = make_block(3, 6, vec![]);
        let r3 = executor.execute_block(&mut db, &block3).unwrap();
        assert_eq!(r3.objects_evaporated, 1);
        assert_eq!(db.object_count(), 0);
        assert_eq!(db.ghost_count(), 1);

        let ghost = db.get_ghost(&obj_id(1)).unwrap();
        assert_eq!(ghost.evaporated_at, 6);
        assert_eq!(ghost.original_data, Some(vec![0xAB]));
    }

    // ─── Refresh Saves Object from Evaporation ───

    #[test]
    fn test_refresh_saves_object_from_evaporation() {
        let mut db = InMemoryStateDB::new();

        db.put_object(StateObject {
            id: obj_id(1),
            owner: addr(1),
            energy: 4,
            half_life: 1,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![],
            decay_curve: None,
            lad_mode: None,
        });

        let mut executor = SimpleExecutor::new_for_test(5);

        let block1 = make_block(1, 3, vec![]);
        let r1 = executor.execute_block(&mut db, &block1).unwrap();
        assert_eq!(r1.objects_entered_grace, 1);

        let block2 = make_block(
            2,
            4,
            vec![Transaction::Refresh(RefreshTx {
                object_id: obj_id(1),
                energy_deposit: 10_000,
                signature: None,
                public_key: None,
            })],
        );
        let r2 = executor.execute_block(&mut db, &block2).unwrap();
        assert_eq!(r2.txs_executed, 1);

        let obj = db.get_object(&obj_id(1)).unwrap();
        assert_eq!(obj.state, ObjectState::Active);
        assert_eq!(obj.energy, 10_000);
        assert_eq!(obj.last_refreshed, 4);

        let block3 = make_block(3, 10, vec![]);
        let r3 = executor.execute_block(&mut db, &block3).unwrap();
        assert_eq!(r3.objects_entered_grace, 0);
        assert_eq!(r3.objects_evaporated, 0);
    }

    // ─── Resurrection via Refresh ───

    #[test]
    fn test_resurrection_via_refresh_in_block() {
        let mut db = InMemoryStateDB::new();

        db.put_ghost(evaporchain_types::GhostRecord {
            object_id: obj_id(1),
            owner: addr(1),
            evaporated_at: 50,
            data_hash: [0u8; 32],
            original_data: Some(vec![0xCA, 0xFE]),
            mmr_position: None,
            original_half_life: Some(100),
        });

        let mut executor = SimpleExecutor::new_for_test(5);
        let block = make_block(
            10,
            60,
            vec![Transaction::Refresh(RefreshTx {
                object_id: obj_id(1),
                energy_deposit: 8000,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(db.ghost_count(), 0);
        assert_eq!(db.object_count(), 1);

        let obj = db.get_object(&obj_id(1)).unwrap();
        assert_eq!(obj.state, ObjectState::Resurrected);
        assert_eq!(obj.energy, 8000);
        assert_eq!(obj.data, vec![0xCA, 0xFE]);
    }

    // ─── State Root Changes Between Blocks ───

    #[test]
    fn test_state_root_changes_between_blocks() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);

        let mut executor = SimpleExecutor::new_for_test(7);

        let block1 = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 500,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );
        let r1 = executor.execute_block(&mut db, &block1).unwrap();

        let block2 = make_block(
            2,
            2,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(3),
                amount: 300,
                nonce: 1,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );
        let r2 = executor.execute_block(&mut db, &block2).unwrap();

        assert_ne!(r1.state_root, r2.state_root);
        assert_ne!(r1.state_root, [0u8; 32]);
        assert_ne!(r2.state_root, [0u8; 32]);
    }

    // ─── Zero Amount Transfer Rejected ───

    #[test]
    fn test_zero_amount_transfer_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 0,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
    }

    // ─── Refresh Nonexistent Object Fails ───

    #[test]
    fn test_refresh_nonexistent_object_fails() {
        let mut db = InMemoryStateDB::new();

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Refresh(RefreshTx {
                object_id: obj_id(99),
                energy_deposit: 1000,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
    }

    // ═══════════════════ Signature Verification Tests ═══════════════════

    #[test]
    fn test_signed_transfer_succeeds() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let mut executor = SimpleExecutor::new_with_sig_verification_for_test(7);
        let kp = MlDsaKeypair::generate();

        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 200,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        sign_tx(&mut tx, &kp);

        let block = make_block(1, 1, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(result.txs_failed, 0);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 800);
    }

    #[test]
    fn test_unsigned_tx_rejected_when_verification_on() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let mut executor = SimpleExecutor::new_with_sig_verification_for_test(7);

        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 200,
                nonce: 0,
                signature: None, // no signature
                public_key: None,
                mev_refund_eligible: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);
        // Balance unchanged
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1000);
    }

    #[test]
    fn test_invalid_signature_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let mut executor = SimpleExecutor::new_with_sig_verification_for_test(7);
        let kp = MlDsaKeypair::generate();

        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 200,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        sign_tx(&mut tx, &kp);

        // Tamper with the signature
        if let Transaction::Transfer(ref mut t) = tx {
            if let Some(ref mut sig) = t.signature {
                sig[0] ^= 0xFF;
            }
        }

        let block = make_block(1, 1, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1000);
    }

    #[test]
    fn test_wrong_key_signature_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let mut executor = SimpleExecutor::new_with_sig_verification_for_test(7);
        let kp1 = MlDsaKeypair::generate();
        let kp2 = MlDsaKeypair::generate();

        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 200,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        // Sign with kp1 but replace public key with kp2's
        sign_tx(&mut tx, &kp1);
        if let Transaction::Transfer(ref mut t) = tx {
            t.public_key = Some(kp2.public_key_bytes());
        }

        let block = make_block(1, 1, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
    }

    #[test]
    fn test_signed_create_object_succeeds() {
        let mut db = InMemoryStateDB::new();
        // CreateObject debits the creator for MIN_STORAGE_DEPOSIT — fund first.
        fund_account(&mut db, 1, 10_000);
        let mut executor = SimpleExecutor::new_with_sig_verification_for_test(7);
        let kp = MlDsaKeypair::generate();

        let mut tx = Transaction::CreateObject(CreateObjectTx {
            creator: addr(1),
            object_id: obj_id(42),
            energy: 5000,
            half_life: 100,
            data: vec![0xDE, 0xAD],
            decay_curve: None,
            lad_mode: None,
            signature: None,
            public_key: None,
        });
        sign_tx(&mut tx, &kp);

        let block = make_block(1, 10, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(db.object_count(), 1);
    }

    #[test]
    fn test_signed_refresh_succeeds() {
        let mut db = InMemoryStateDB::new();
        db.put_object(StateObject {
            id: obj_id(1),
            owner: addr(1),
            energy: 100,
            half_life: 10,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![],
            decay_curve: None,
            lad_mode: None,
        });

        let mut executor = SimpleExecutor::new_with_sig_verification_for_test(7);
        let kp = MlDsaKeypair::generate();

        let mut tx = Transaction::Refresh(RefreshTx {
            object_id: obj_id(1),
            energy_deposit: 500,
            signature: None,
            public_key: None,
        });
        sign_tx(&mut tx, &kp);

        let block = make_block(1, 5, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
    }

    // ═══════════════════ EvaporScript Integration Tests ═══════════════════

    const COUNTER_SCRIPT: &str = r#"
contract Counter {
    state {
        count: u64 = 0
    }
    fn increment(n: u64) {
        self.count += n
    }
    fn get() -> u64 {
        return self.count
    }
    on_evaporate() {
        emit("counter expired")
    }
    on_grace() {
        emit("counter entering grace")
    }
}
"#;

    #[test]
    fn test_deploy_script_via_transaction() {
        let mut db = InMemoryStateDB::new();
        // Phase 4.2: deploy now charges MIN_STORAGE_DEPOSIT.
        fund_account(&mut db, 1, 10_000);
        let mut executor = SimpleExecutor::new_for_test(7);

        let block = make_block(
            1,
            1,
            vec![Transaction::DeployScript(
                evaporchain_types::DeployScriptTx {
                    deployer: addr(1),
                    source_code: COUNTER_SCRIPT.to_string(),
                    energy: 10_000,
                    half_life: 100,
                    signature: None,
                    public_key: None,
                },
            )],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(result.txs_failed, 0);

        // Verify the contract was deployed
        let contract = executor.script_engine.get(1).unwrap();
        assert_eq!(contract.name, "Counter");
        assert_eq!(contract.energy, 10_000);
    }

    #[test]
    fn test_call_script_via_transaction() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);
        let mut executor = SimpleExecutor::new_for_test(7);

        // Deploy
        let deploy_block = make_block(
            1,
            1,
            vec![Transaction::DeployScript(
                evaporchain_types::DeployScriptTx {
                    deployer: addr(1),
                    source_code: COUNTER_SCRIPT.to_string(),
                    energy: 10_000,
                    half_life: 100,
                    signature: None,
                    public_key: None,
                },
            )],
        );
        executor.execute_block(&mut db, &deploy_block).unwrap();

        // Call increment
        let call_block = make_block(
            2,
            2,
            vec![Transaction::CallScript(evaporchain_types::CallScriptTx {
                caller: addr(2),
                contract_id: 1,
                method: "increment".to_string(),
                args: r#"[{"U64": 42}]"#.to_string(),
                epoch: 2,
                signature: None,
                public_key: None,
            })],
        );
        let result = executor.execute_block(&mut db, &call_block).unwrap();
        assert_eq!(result.txs_executed, 1);

        // Verify state was updated
        let contract = executor.script_engine.get(1).unwrap();
        match contract.state.get("count") {
            Some(evaporchain_script::Value::U64(n)) => assert_eq!(*n, 42),
            other => panic!("expected count=42, got {other:?}"),
        }
    }

    #[test]
    fn test_script_contract_lifecycle_with_decay() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);
        // Very short half-life: 1 epoch, energy: 4 → dies at epoch ~3
        let mut executor = SimpleExecutor::new_for_test(3);

        // Deploy with minimal energy
        let deploy_block = make_block(
            1,
            0,
            vec![Transaction::DeployScript(
                evaporchain_types::DeployScriptTx {
                    deployer: addr(1),
                    source_code: COUNTER_SCRIPT.to_string(),
                    energy: 4,
                    half_life: 1,
                    signature: None,
                    public_key: None,
                },
            )],
        );
        executor.execute_block(&mut db, &deploy_block).unwrap();

        // Tick at epoch 3 — energy should be 0, triggering on_evaporate
        let block2 = make_block(2, 3, vec![]);
        executor.execute_block(&mut db, &block2).unwrap();

        // Contract should be evaporated
        let contract = executor.script_engine.get(1).unwrap();
        assert!(contract.evaporated);
    }

    #[test]
    fn test_script_and_template_contracts_coexist() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);
        fund_account(&mut db, 2, 10_000);
        let mut executor = SimpleExecutor::new_for_test(7);

        // Deploy a template contract.
        // owner needs to be canonical 64-hex (Phase 3.1 sweep);
        // build it inline via hex::encode(addr(1)).
        let owner_hex = hex::encode(addr(1));
        let init_args = format!(
            r#"{{"name":"TestToken","symbol":"TT","total_supply":1000000,"decay_half_life":100,"owner":"{}"}}"#,
            owner_hex
        );
        let deploy_template = make_block(
            1,
            1,
            vec![Transaction::DeployContract(DeployContractTx {
                deployer: addr(1),
                template: "DecayingToken".to_string(),
                init_args,
                energy: 10_000,
                half_life: 100,
                rules: None,
                signature: None,
                public_key: None,
            })],
        );
        let r1 = executor.execute_block(&mut db, &deploy_template).unwrap();
        assert_eq!(r1.txs_executed, 1, "template deploy failed");

        // Deploy a script contract
        let deploy_script = make_block(
            2,
            2,
            vec![Transaction::DeployScript(
                evaporchain_types::DeployScriptTx {
                    deployer: addr(2),
                    source_code: COUNTER_SCRIPT.to_string(),
                    energy: 10_000,
                    half_life: 100,
                    signature: None,
                    public_key: None,
                },
            )],
        );
        let r2 = executor.execute_block(&mut db, &deploy_script).unwrap();
        assert_eq!(r2.txs_executed, 1, "script deploy failed");

        // Both should exist
        assert!(executor.contract_engine.get(1).is_some());
        assert!(executor.script_engine.get(1).is_some());
    }

    #[test]
    fn test_deploy_invalid_script_fails() {
        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);

        let block = make_block(
            1,
            1,
            vec![Transaction::DeployScript(
                evaporchain_types::DeployScriptTx {
                    deployer: addr(1),
                    source_code: "this is not valid evaporscript!!!".to_string(),
                    energy: 10_000,
                    half_life: 100,
                    signature: None,
                    public_key: None,
                },
            )],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);
    }

    // ─── Signature Verification Tests ───

    #[test]
    fn test_valid_signature_passes() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
        let kp = MlDsaKeypair::generate();

        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        sign_tx(&mut tx, &kp);

        let mut executor = SimpleExecutor::new_with_sig_verification_for_test(7);
        let block = make_block(1, 1, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(result.txs_failed, 0);
    }

    #[test]
    fn test_missing_signature_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });

        let mut executor = SimpleExecutor::new_with_sig_verification_for_test(7);
        let block = make_block(1, 1, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);
    }

    #[test]
    fn test_corrupted_signature_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
        let kp = MlDsaKeypair::generate();

        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        sign_tx(&mut tx, &kp);

        // Corrupt the signature
        if let Transaction::Transfer(ref mut t) = tx {
            if let Some(ref mut sig) = t.signature {
                sig[0] ^= 0xFF;
            }
        }

        let mut executor = SimpleExecutor::new_with_sig_verification_for_test(7);
        let block = make_block(1, 1, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);
    }

    #[test]
    fn test_wrong_key_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
        let kp1 = MlDsaKeypair::generate();
        let kp2 = MlDsaKeypair::generate();

        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        // Sign with kp1 but attach kp2's public key
        let msg = tx.signing_message("");
        let sig = kp1.sign(&msg);
        let pk = kp2.public_key_bytes(); // wrong key
        if let Transaction::Transfer(ref mut t) = tx {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }

        let mut executor = SimpleExecutor::new_with_sig_verification_for_test(7);
        let block = make_block(1, 1, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);
    }

    #[test]
    fn test_tampered_tx_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
        let kp = MlDsaKeypair::generate();

        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        sign_tx(&mut tx, &kp);

        // Tamper with the amount after signing
        if let Transaction::Transfer(ref mut t) = tx {
            t.amount = 999;
        }

        let mut executor = SimpleExecutor::new_with_sig_verification_for_test(7);
        let block = make_block(1, 1, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);
    }

    #[test]
    fn test_sig_disabled_allows_unsigned() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });

        // SimpleExecutor::new() has verify_signatures: false
        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(1, 1, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(result.txs_failed, 0);
    }

    // ─── Fee Deduction Tests ───

    #[test]
    fn test_fees_deducted_from_sender() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);

        let fc = fees::PidFeeController::testnet_config();
        let mut executor = SimpleExecutor::new_with_fees_for_test(7, fc, 500_000);

        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 100,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert!(result.total_fees > 0, "Fees should be collected");

        let sender = db.get_account(&addr(1)).unwrap();
        // Balance should be: 1_000_000 - 100 (transfer) - gas_fee
        assert!(
            sender.balance < 1_000_000 - 100,
            "Fees should have been deducted: balance={}",
            sender.balance
        );
    }

    #[test]
    fn test_insufficient_balance_for_gas_rejected() {
        let mut db = InMemoryStateDB::new();
        // Fund with only 10 — not enough for gas (transfer costs 21000 * 1 = 21000)
        fund_account(&mut db, 1, 10);

        let fc = fees::PidFeeController::testnet_config();
        let mut executor = SimpleExecutor::new_with_fees_for_test(7, fc, 500_000);

        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 5,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);

        // Balance should be unchanged (couldn't afford gas)
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 10);
    }

    #[test]
    fn test_fee_burned_even_on_tx_failure() {
        let mut db = InMemoryStateDB::new();
        // Enough for gas but not for the transfer amount
        fund_account(&mut db, 1, 50_000);

        let fc = fees::PidFeeController::testnet_config();
        let mut executor = SimpleExecutor::new_with_fees_for_test(7, fc, 500_000);

        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 999_999,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);
        assert!(
            result.total_fees > 0,
            "Fee should still be burned on failure"
        );

        // Balance should be reduced by gas fee even though transfer failed
        let sender = db.get_account(&addr(1)).unwrap();
        assert!(
            sender.balance < 50_000,
            "Gas fee should have been deducted: balance={}",
            sender.balance
        );
    }

    #[test]
    fn test_creation_deposit_deducted() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);

        let fc = fees::PidFeeController::testnet_config();
        let mut executor = SimpleExecutor::new_with_fees_for_test(7, fc, 500_000);

        let block = make_block(
            1,
            1,
            vec![Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(42),
                energy: 5000,
                half_life: 100,
                data: vec![1, 2, 3, 4, 5],
                decay_curve: None,
                lad_mode: None,
                signature: None,
                public_key: None,
            })],
        );
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);

        // Fee should include gas + creation deposit
        // Gas: 50000 + 200*5 = 51000; fee = 51000 * 1 = 51000
        // Creation deposit: max(100 * 5, 1000) = 1000
        // Total: 52000
        assert!(
            result.total_fees >= 52_000,
            "Creation deposit should be included: fees={}",
            result.total_fees
        );
    }

    #[test]
    fn test_revert_on_failed_tx_keeps_fee() {
        let mut db = InMemoryStateDB::new();
        // Balance: 100_000. Transfer gas fee = 21000.
        // After fee deduction: 79_000. Transfer amount 999_999 > 79_000 → fail.
        fund_account(&mut db, 1, 100_000);

        let fc = fees::PidFeeController::testnet_config();
        let mut executor = SimpleExecutor::new_with_fees_for_test(7, fc, 500_000);

        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 999_999,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);

        let sender = db.get_account(&addr(1)).unwrap();
        // Fee deducted (21000), but nonce NOT incremented (reverted)
        assert_eq!(sender.balance, 100_000 - 21_000);
        assert_eq!(sender.nonce, 0, "Nonce should be reverted on failed tx");

        // Receiver should NOT have been credited
        assert!(db.get_account(&addr(2)).is_none());
    }

    #[test]
    fn test_no_fees_when_controller_disabled() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let mut executor = SimpleExecutor::new_for_test(7); // No fee controller
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 100,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(result.total_fees, 0);

        // Balance should only reflect the transfer, not gas
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 900);
    }

    // ═══════════════════════════════════════════════════════════════
    // Phase 5: Stress Tests
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn stress_1000_transfers_single_block() {
        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);

        // Fund 100 accounts with 1M each
        for i in 1..=100u8 {
            fund_account(&mut db, i, 1_000_000);
        }

        // Track nonces locally so we don't mutate the DB before execution
        let mut nonces = std::collections::HashMap::<u8, u64>::new();

        // Create 1000 transfers between random pairs
        let mut txs = Vec::with_capacity(1000);
        for i in 0..1000u32 {
            let from_idx = (i % 100) as u8 + 1;
            let to_idx = ((i + 37) % 100) as u8 + 1;
            if from_idx == to_idx {
                continue;
            }
            let nonce = *nonces.entry(from_idx).or_insert(0);
            txs.push(Transaction::Transfer(TransferTx {
                from: addr(from_idx),
                to: addr(to_idx),
                amount: 10,
                nonce,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }));
            *nonces.entry(from_idx).or_insert(0) += 1;
        }

        let block = make_block(1, 1, txs);
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert!(
            result.txs_executed > 900,
            "Expected >900 txs executed, got {}",
            result.txs_executed
        );
        println!(
            "Stress 1000 transfers: {} executed, {} failed",
            result.txs_executed, result.txs_failed
        );
    }

    #[test]
    fn stress_rapid_epoch_decay() {
        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);

        fund_account(&mut db, 1, 10_000_000);
        // Energy 1000, half_life 3 → needs ~30 epochs to reach 0, +7 grace = 37 to evaporate
        for i in 0..50u8 {
            db.put_object(StateObject {
                id: obj_id(i + 1),
                owner: addr(1),
                data: vec![0xAB; 64],
                energy: 1000,
                half_life: 3,
                created_at: 0,
                last_refreshed: 0,
                state: ObjectState::Active,
                grace_epoch: None,
                decay_curve: None,
                lad_mode: None,
            });
        }

        for epoch in 1..=50u64 {
            let block = make_block(epoch, epoch, vec![]);
            executor.execute_block(&mut db, &block).unwrap();
        }

        let active_count = (1..=50u8)
            .filter(|i| {
                db.get_object(&obj_id(*i))
                    .map(|o| o.energy > 0)
                    .unwrap_or(false)
            })
            .count();
        let ghost_count = db.all_ghost_ids().len();

        println!(
            "Stress decay: {} still active, {} evaporated after 50 epochs",
            active_count, ghost_count
        );
        assert!(
            ghost_count > 30,
            "Expected most objects evaporated, got only {} ghosts",
            ghost_count
        );
    }

    #[test]
    fn stress_fee_controller_rapid_utilization_swings() {
        use crate::fees::PidFeeController;

        let mut controller = PidFeeController::new(0.5, 0.1, 0.01, 0.05, 1000, 1, 1_000_000_000);

        let mut fees = Vec::with_capacity(100);
        for i in 0..100u64 {
            let used = if i % 2 == 0 { 1_000_000 } else { 0 };
            let new_fee = controller.update(used, 1_000_000);
            fees.push(new_fee);
        }

        for (i, fee) in fees.iter().enumerate() {
            assert!(*fee > 0, "Fee should never be zero (block {i})");
            assert!(
                *fee < 1_000_000_000,
                "Fee should not explode: {} at block {i}",
                fee
            );
        }

        let last_10_range = fees[90..].iter().max().unwrap() - fees[90..].iter().min().unwrap();
        println!(
            "Fee controller: final fee={}, last-10 range={}, min={}, max={}",
            fees.last().unwrap(),
            last_10_range,
            fees.iter().min().unwrap(),
            fees.iter().max().unwrap()
        );
    }

    #[test]
    fn stress_fee_controller_sustained_full_utilization() {
        use crate::fees::PidFeeController;

        let mut controller = PidFeeController::new(0.5, 0.1, 0.01, 0.05, 100, 1, 1_000_000_000);

        let mut fee = 100;
        for _ in 0..200 {
            fee = controller.update(1_000_000, 1_000_000);
        }
        assert!(
            fee > 100,
            "Fee should rise under sustained full utilization, got {fee}"
        );

        let peak = fee;
        for _ in 0..200 {
            fee = controller.update(0, 1_000_000);
        }
        assert!(
            fee < peak,
            "Fee should fall under sustained zero utilization: peak={peak}, now={fee}"
        );
        println!("Fee controller: peak={peak}, after cooldown={fee}");
    }

    #[test]
    fn stress_fee_controller_zero_gas_limit() {
        use crate::fees::PidFeeController;

        let mut controller = PidFeeController::new(0.5, 0.1, 0.01, 0.05, 1000, 1, 1_000_000_000);
        let fee = controller.update(0, 0);
        assert_eq!(fee, 1000, "Zero gas limit should return base_fee unchanged");
    }

    #[test]
    fn stress_fee_controller_overflow_boundaries() {
        use crate::fees::PidFeeController;

        let mut controller = PidFeeController::new(0.5, 0.1, 0.01, 0.05, u64::MAX / 2, 1, u64::MAX);
        let fee = controller.update(u64::MAX, u64::MAX);
        assert!(
            fee > 0,
            "Fee should remain positive even at overflow boundaries"
        );
    }

    #[test]
    fn stress_block_gas_limit_enforcement() {
        let mut db = InMemoryStateDB::new();
        let fee_controller =
            crate::fees::PidFeeController::new(0.5, 0.1, 0.01, 0.05, 1000, 1, 1_000_000_000);
        let mut executor = SimpleExecutor::new_with_fees_for_test(7, fee_controller, 100_000);

        fund_account(&mut db, 1, 10_000_000);

        let txs: Vec<Transaction> = (0..100u64)
            .map(|i| {
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(2),
                    amount: 1,
                    nonce: i,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                })
            })
            .collect();

        let block = make_block(1, 1, txs);
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert!(
            result.txs_executed < 100,
            "Gas limit should cap execution, but all {} executed",
            result.txs_executed
        );
        println!(
            "Gas limit stress: {}/{} executed within limit",
            result.txs_executed, 100
        );
    }

    #[test]
    fn stress_concurrent_object_lifecycle() {
        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);
        fund_account(&mut db, 1, 100_000_000);

        // Epoch 1: Create 100 objects (energy 500, half_life 5)
        // Energy reaches 0 after ~45 epochs from creation, +7 grace = ~53 to evaporate
        let create_txs: Vec<Transaction> = (0..100u8)
            .map(|i| {
                let mut id = [0u8; 32];
                id[0] = i + 1;
                Transaction::CreateObject(CreateObjectTx {
                    creator: addr(1),
                    object_id: id,
                    data: vec![0xAB; 32],
                    energy: 500,
                    half_life: 5,
                    decay_curve: None,
                    lad_mode: None,
                    signature: None,
                    public_key: None,
                })
            })
            .collect();
        let block1 = make_block(1, 1, create_txs);
        executor.execute_block(&mut db, &block1).unwrap();

        // Epochs 2-70: Decay + periodic refreshes for every 3rd object
        for epoch in 2..=70u64 {
            let mut txs = vec![];
            // Refresh every 3rd object periodically to keep some alive
            if epoch % 10 == 0 {
                for i in (0..100u8).step_by(3) {
                    txs.push(Transaction::Refresh(RefreshTx {
                        object_id: obj_id(i + 1),
                        energy_deposit: 500,
                        signature: None,
                        public_key: None,
                    }));
                }
            }
            let block = make_block(epoch, epoch, txs);
            executor.execute_block(&mut db, &block).unwrap();
        }

        let active = (1..=100u8)
            .filter(|i| db.get_object(&obj_id(*i)).is_some())
            .count();
        let ghosts = db.all_ghost_ids().len();
        println!(
            "Lifecycle stress: {} active, {} evaporated after 70 epochs",
            active, ghosts
        );

        assert!(ghosts > 0, "Some objects should have evaporated");
        assert!(active > 0, "Some refreshed objects should survive");
    }

    #[test]
    fn test_mmr_accumulates_on_evaporation() {
        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);

        let owner = addr(1);
        db.put_account(Account {
            address: owner,
            balance: 1_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        let obj = evaporchain_types::StateObject {
            id: obj_id(1),
            owner,
            energy: 1,
            half_life: 1,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: b"short-lived".to_vec(),
            decay_curve: None,
            lad_mode: None,
        };
        db.put_object(obj);

        assert_eq!(
            executor.mmr_root(),
            [0u8; 32],
            "MMR should be empty at start"
        );
        assert_eq!(executor.mmr_size(), 0);

        for epoch in 1..=20 {
            let block = make_block(epoch, epoch, vec![]);
            let result = executor.execute_block(&mut db, &block).unwrap();
            if result.objects_evaporated > 0 {
                assert_ne!(
                    result.mmr_root, [0u8; 32],
                    "MMR root should be non-zero after evaporation"
                );
                assert_eq!(executor.mmr_size(), 1, "Exactly one nullifier in MMR");
                assert_eq!(
                    executor.mmr_root(),
                    result.mmr_root,
                    "Trait accessor matches result"
                );
                return;
            }
        }
        panic!("Object should have evaporated within 20 epochs");
    }

    #[test]
    fn test_mmr_root_persists_across_blocks() {
        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);

        let owner = addr(1);
        db.put_account(Account {
            address: owner,
            balance: 1_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        for i in 1..=3u8 {
            let obj = evaporchain_types::StateObject {
                id: obj_id(i),
                owner,
                energy: i as u64,
                half_life: 1,
                created_at: 0,
                last_refreshed: 0,
                state: ObjectState::Active,
                grace_epoch: None,
                data: vec![i],
                decay_curve: None,
                lad_mode: None,
            };
            db.put_object(obj);
        }

        let mut prev_root = [0u8; 32];
        let mut total_evaporated = 0usize;

        for epoch in 1..=30 {
            let block = make_block(epoch, epoch, vec![]);
            let result = executor.execute_block(&mut db, &block).unwrap();
            total_evaporated += result.objects_evaporated;

            if result.objects_evaporated > 0 {
                assert_ne!(
                    result.mmr_root, prev_root,
                    "MMR root should change on evaporation"
                );
            } else {
                assert_eq!(
                    result.mmr_root, prev_root,
                    "MMR root should not change without evaporation"
                );
            }
            prev_root = result.mmr_root;
        }

        assert_eq!(total_evaporated, 3, "All 3 objects should have evaporated");
        assert_eq!(executor.mmr_size(), 3, "MMR should have 3 nullifiers");
    }

    #[test]
    fn test_cross_shard_transfer() {
        use evaporchain_sharding::cross_shard::{CrossShardMessage, MessagePayload};
        use evaporchain_sharding::shard_assignment::ShardId;

        let mut executor = SimpleExecutor::new_for_test(100);
        let mut db = InMemoryStateDB::new();

        let from_addr = addr(1);
        let from_acct = db.get_or_create_account(&from_addr);
        from_acct.balance = 1_000_000;

        let mut from_20 = [0u8; 20];
        from_20.copy_from_slice(&from_addr[..20]);
        let mut to_20 = [0u8; 20];
        to_20.copy_from_slice(&addr(2)[..20]);

        let msg = CrossShardMessage {
            id: 0,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            target_object: to_20,
            payload: MessagePayload::Transfer {
                from: from_20,
                amount: 500,
            },
            target_energy: 100,
            timestamp: 1,
        };

        let receipts = executor.execute_cross_shard_messages(&mut db, vec![msg], 10);
        assert_eq!(receipts.len(), 1);
        assert!(receipts[0].success);
        assert_eq!(db.get_account(&from_addr).unwrap().balance, 999_500);
    }

    #[test]
    fn test_cross_shard_transfer_insufficient_balance() {
        use evaporchain_sharding::cross_shard::{CrossShardMessage, MessagePayload};
        use evaporchain_sharding::shard_assignment::ShardId;

        let mut executor = SimpleExecutor::new_for_test(100);
        let mut db = InMemoryStateDB::new();

        let from_addr = addr(1);
        let from_acct = db.get_or_create_account(&from_addr);
        from_acct.balance = 100;

        let mut from_20 = [0u8; 20];
        from_20.copy_from_slice(&from_addr[..20]);
        let mut to_20 = [0u8; 20];
        to_20.copy_from_slice(&addr(2)[..20]);

        let msg = CrossShardMessage {
            id: 0,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            target_object: to_20,
            payload: MessagePayload::Transfer {
                from: from_20,
                amount: 500,
            },
            target_energy: 100,
            timestamp: 1,
        };

        let receipts = executor.execute_cross_shard_messages(&mut db, vec![msg], 10);
        assert_eq!(receipts.len(), 1);
        assert!(!receipts[0].success);
        assert_eq!(db.get_account(&from_addr).unwrap().balance, 100);
    }

    #[test]
    fn test_cross_shard_eviction() {
        use evaporchain_sharding::cross_shard::{CrossShardMessage, MessagePayload};
        use evaporchain_sharding::shard_assignment::ShardId;

        let mut executor = SimpleExecutor::new_for_test(100);
        let mut db = InMemoryStateDB::new();

        let oid = obj_id(1);
        db.put_object(StateObject {
            id: oid,
            owner: addr(1),
            energy: 100,
            half_life: 10,
            created_at: 1,
            last_refreshed: 1,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![],
            decay_curve: None,
            lad_mode: None,
        });

        let mut target = [0u8; 20];
        target.copy_from_slice(&oid[..20]);

        let msg = CrossShardMessage {
            id: 0,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            target_object: target,
            payload: MessagePayload::Eviction {
                reason: "low energy".into(),
            },
            target_energy: 0,
            timestamp: 1,
        };

        let receipts = executor.execute_cross_shard_messages(&mut db, vec![msg], 10);
        assert!(receipts[0].success);
        assert!(db.get_object(&oid).is_none());
    }

    // ─── Delegation (P0 #4) ───────────────────────────────────────────

    fn seed_validator(db: &mut InMemoryStateDB, vid: u64, validator_byte: u8, stake: u64) {
        // Stake records are required before any delegation can target the
        // validator (anti-griefing in execute_delegate).
        fund_account(db, validator_byte, 1_000_000);
        db.put_stake(StakeRecord {
            validator_id: vid,
            validator_address: addr(validator_byte),
            staked_amount: stake,
            staked_at_epoch: 0,
            unbonding_epoch: None,
            slashed_amount: 0,
        });
    }

    #[test]
    fn test_delegate_happy_path() {
        let mut db = InMemoryStateDB::new();
        seed_validator(&mut db, 7, 9, 100_000);
        fund_account(&mut db, 1, 5_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1),
                validator_id: 7,
                amount: 1_000,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let r = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed, 1);
        assert_eq!(r.txs_failed, 0);

        let acct = db.get_account(&addr(1)).unwrap();
        assert_eq!(acct.balance, 4_000, "1000 should be debited from delegator");
        assert_eq!(acct.nonce, 1);

        let rec = db
            .get_delegation(&addr(1), 7)
            .expect("delegation must exist");
        assert_eq!(rec.amount, 1_000);
        assert_eq!(rec.unbonding_amount, 0);
        assert!(rec.unbonding_epoch.is_none());
    }

    #[test]
    fn test_delegate_to_unknown_validator_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 5_000);
        // No stake record for validator 99 — anti-griefing guard should fire.

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1),
                validator_id: 99,
                amount: 1_000,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let r = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1, "delegate to unknown validator must fail");
        // Balance should be unchanged (state reverted on failure).
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 5_000);
        assert!(db.get_delegation(&addr(1), 99).is_none());
    }

    #[test]
    fn test_delegate_insufficient_balance_rejected() {
        let mut db = InMemoryStateDB::new();
        seed_validator(&mut db, 7, 9, 100_000);
        fund_account(&mut db, 1, 500);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1),
                validator_id: 7,
                amount: 1_000,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let r = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
        assert!(db.get_delegation(&addr(1), 7).is_none());
    }

    #[test]
    fn test_delegate_additive() {
        let mut db = InMemoryStateDB::new();
        seed_validator(&mut db, 7, 9, 100_000);
        fund_account(&mut db, 1, 10_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        let block1 = make_block(
            1,
            1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1),
                validator_id: 7,
                amount: 1_000,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        executor.execute_block(&mut db, &block1).unwrap();
        let block2 = make_block(
            2,
            2,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1),
                validator_id: 7,
                amount: 2_500,
                nonce: 1,
                signature: None,
                public_key: None,
            })],
        );
        executor.execute_block(&mut db, &block2).unwrap();

        let rec = db.get_delegation(&addr(1), 7).unwrap();
        assert_eq!(
            rec.amount, 3_500,
            "delegations to same validator should be additive"
        );
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 6_500);
    }

    #[test]
    fn test_undelegate_marks_unbonding() {
        let mut db = InMemoryStateDB::new();
        seed_validator(&mut db, 7, 9, 100_000);
        fund_account(&mut db, 1, 5_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        // First delegate so there's something to undelegate.
        let b1 = make_block(
            1,
            1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1),
                validator_id: 7,
                amount: 1_000,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        executor.execute_block(&mut db, &b1).unwrap();
        let pre_balance = db.get_account(&addr(1)).unwrap().balance;

        let b2 = make_block(
            2,
            5,
            vec![Transaction::Undelegate(UndelegateTx {
                delegator: addr(1),
                validator_id: 7,
                amount: 600,
                nonce: 1,
                signature: None,
                public_key: None,
            })],
        );
        executor.execute_block(&mut db, &b2).unwrap();

        let rec = db.get_delegation(&addr(1), 7).unwrap();
        assert_eq!(rec.amount, 400, "active amount reduced by 600");
        assert_eq!(rec.unbonding_amount, 600, "600 marked unbonding");
        assert_eq!(rec.unbonding_epoch, Some(5), "epoch recorded");
        // Funds NOT yet returned to balance — that's a future ClaimDelegation tx.
        assert_eq!(
            db.get_account(&addr(1)).unwrap().balance,
            pre_balance,
            "undelegate must not credit balance immediately"
        );
    }

    #[test]
    fn test_undelegate_more_than_delegated_rejected() {
        let mut db = InMemoryStateDB::new();
        seed_validator(&mut db, 7, 9, 100_000);
        fund_account(&mut db, 1, 5_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        let b1 = make_block(
            1,
            1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1),
                validator_id: 7,
                amount: 1_000,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        executor.execute_block(&mut db, &b1).unwrap();

        let b2 = make_block(
            2,
            2,
            vec![Transaction::Undelegate(UndelegateTx {
                delegator: addr(1),
                validator_id: 7,
                amount: 5_000,
                nonce: 1,
                signature: None,
                public_key: None,
            })],
        );
        let r = executor.execute_block(&mut db, &b2).unwrap();
        assert_eq!(r.txs_failed, 1);

        // Original delegation untouched.
        let rec = db.get_delegation(&addr(1), 7).unwrap();
        assert_eq!(rec.amount, 1_000);
        assert_eq!(rec.unbonding_amount, 0);
    }

    // ─── Phase 7: ClaimDelegationTx ──────────────────────────────────

    #[test]
    fn test_claim_delegation_after_unbonding_period() {
        let mut db = InMemoryStateDB::new();
        seed_validator(&mut db, 7, 9, 100_000);
        fund_account(&mut db, 1, 5_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        // Delegate then undelegate at epoch 5.
        executor
            .execute_block(
                &mut db,
                &make_block(
                    1,
                    1,
                    vec![Transaction::Delegate(DelegateTx {
                        delegator: addr(1),
                        validator_id: 7,
                        amount: 2_000,
                        nonce: 0,
                        signature: None,
                        public_key: None,
                    })],
                ),
            )
            .unwrap();
        executor
            .execute_block(
                &mut db,
                &make_block(
                    2,
                    5,
                    vec![Transaction::Undelegate(UndelegateTx {
                        delegator: addr(1),
                        validator_id: 7,
                        amount: 1_500,
                        nonce: 1,
                        signature: None,
                        public_key: None,
                    })],
                ),
            )
            .unwrap();
        let pre_claim_balance = db.get_account(&addr(1)).unwrap().balance;

        // Claim before unbonding period elapses → must fail.
        let early = executor
            .execute_block(
                &mut db,
                &make_block(
                    3,
                    50,
                    vec![Transaction::ClaimDelegation(ClaimDelegationTx {
                        delegator: addr(1),
                        validator_id: 7,
                        nonce: 2,
                        signature: None,
                        public_key: None,
                    })],
                ),
            )
            .unwrap();
        assert_eq!(
            early.txs_failed, 1,
            "claim before unbonding period must fail"
        );
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, pre_claim_balance);

        // Claim after unbonding period (5 + 256 = 261) → succeeds.
        let ready = executor
            .execute_block(
                &mut db,
                &make_block(
                    4,
                    261,
                    vec![Transaction::ClaimDelegation(ClaimDelegationTx {
                        delegator: addr(1),
                        validator_id: 7,
                        nonce: 2,
                        signature: None,
                        public_key: None,
                    })],
                ),
            )
            .unwrap();
        assert_eq!(ready.txs_executed, 1);
        assert_eq!(
            db.get_account(&addr(1)).unwrap().balance,
            // -1 for the per-block demurrage tick at the claim epoch.
            pre_claim_balance + 1_500 - 1,
            "claimed amount credited to balance (less 1-unit per-block demurrage)"
        );
        let rec = db.get_delegation(&addr(1), 7).unwrap();
        assert_eq!(rec.unbonding_amount, 0);
        assert!(rec.unbonding_epoch.is_none());
        assert_eq!(rec.amount, 500, "active amount untouched");
    }

    #[test]
    fn test_claim_delegation_removes_record_when_fully_unbonded() {
        let mut db = InMemoryStateDB::new();
        seed_validator(&mut db, 7, 9, 100_000);
        fund_account(&mut db, 1, 5_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        executor
            .execute_block(
                &mut db,
                &make_block(
                    1,
                    1,
                    vec![Transaction::Delegate(DelegateTx {
                        delegator: addr(1),
                        validator_id: 7,
                        amount: 2_000,
                        nonce: 0,
                        signature: None,
                        public_key: None,
                    })],
                ),
            )
            .unwrap();
        executor
            .execute_block(
                &mut db,
                &make_block(
                    2,
                    1,
                    vec![Transaction::Undelegate(UndelegateTx {
                        delegator: addr(1),
                        validator_id: 7,
                        amount: 2_000,
                        nonce: 1,
                        signature: None,
                        public_key: None,
                    })],
                ),
            )
            .unwrap();
        executor
            .execute_block(
                &mut db,
                &make_block(
                    3,
                    257,
                    vec![Transaction::ClaimDelegation(ClaimDelegationTx {
                        delegator: addr(1),
                        validator_id: 7,
                        nonce: 2,
                        signature: None,
                        public_key: None,
                    })],
                ),
            )
            .unwrap();

        assert!(
            db.get_delegation(&addr(1), 7).is_none(),
            "fully unbonded delegation should be removed"
        );
    }

    #[test]
    fn test_delegations_for_validator_lists_all_delegators() {
        // Two delegators bonding to the same validator, plus one bonding
        // to a different validator. delegations_for_validator(7) must
        // surface only the first two — that's what the wallet's
        // GET /api/validator/:id/delegations endpoint returns.
        let mut db = InMemoryStateDB::new();
        seed_validator(&mut db, 7, 9, 100_000);
        seed_validator(&mut db, 8, 10, 100_000);
        fund_account(&mut db, 1, 5_000);
        fund_account(&mut db, 2, 5_000);
        fund_account(&mut db, 3, 5_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        executor
            .execute_block(
                &mut db,
                &make_block(
                    1,
                    1,
                    vec![
                        Transaction::Delegate(DelegateTx {
                            delegator: addr(1),
                            validator_id: 7,
                            amount: 1_000,
                            nonce: 0,
                            signature: None,
                            public_key: None,
                        }),
                        Transaction::Delegate(DelegateTx {
                            delegator: addr(2),
                            validator_id: 7,
                            amount: 2_000,
                            nonce: 0,
                            signature: None,
                            public_key: None,
                        }),
                        Transaction::Delegate(DelegateTx {
                            delegator: addr(3),
                            validator_id: 8,
                            amount: 500,
                            nonce: 0,
                            signature: None,
                            public_key: None,
                        }),
                    ],
                ),
            )
            .unwrap();

        let v7 = db.delegations_for_validator(7);
        assert_eq!(v7.len(), 2, "validator 7 has two delegators");
        let total: u64 = v7.iter().map(|d| d.amount).sum();
        assert_eq!(total, 3_000, "Σ amount feeds delegated_stake");

        let v8 = db.delegations_for_validator(8);
        assert_eq!(v8.len(), 1);
        assert_eq!(v8[0].amount, 500);
    }

    #[test]
    fn test_full_delegation_lifecycle_roundtrip() {
        // Round-trip: delegate → undelegate → wait for unbonding period
        // → claim. Confirms that after a complete cycle the delegation
        // record is cleaned up and the delegator's balance has been
        // restored (less per-block demurrage). Wallet-facing endpoint
        // /api/tx/{delegate,undelegate,claim_delegation} relies on this.
        let mut db = InMemoryStateDB::new();
        seed_validator(&mut db, 7, 9, 100_000);
        fund_account(&mut db, 1, 10_000);
        let starting_balance = db.get_account(&addr(1)).unwrap().balance;

        let mut executor = SimpleExecutor::new_for_test(7);
        // Bond 3000.
        executor
            .execute_block(
                &mut db,
                &make_block(
                    1,
                    1,
                    vec![Transaction::Delegate(DelegateTx {
                        delegator: addr(1),
                        validator_id: 7,
                        amount: 3_000,
                        nonce: 0,
                        signature: None,
                        public_key: None,
                    })],
                ),
            )
            .unwrap();
        // Unbond fully at epoch 5.
        executor
            .execute_block(
                &mut db,
                &make_block(
                    2,
                    5,
                    vec![Transaction::Undelegate(UndelegateTx {
                        delegator: addr(1),
                        validator_id: 7,
                        amount: 3_000,
                        nonce: 1,
                        signature: None,
                        public_key: None,
                    })],
                ),
            )
            .unwrap();
        // Claim AFTER unbonding period (5 + 256 = 261).
        let r = executor
            .execute_block(
                &mut db,
                &make_block(
                    3,
                    261,
                    vec![Transaction::ClaimDelegation(ClaimDelegationTx {
                        delegator: addr(1),
                        validator_id: 7,
                        nonce: 2,
                        signature: None,
                        public_key: None,
                    })],
                ),
            )
            .unwrap();
        assert_eq!(r.txs_executed, 1);

        // Balance restored (demurrage is small per-block; just check the
        // funds came back in the right order of magnitude).
        let final_balance = db.get_account(&addr(1)).unwrap().balance;
        assert!(
            final_balance >= starting_balance.saturating_sub(10),
            "claim must restore the bonded amount: started {}, ended {}",
            starting_balance,
            final_balance
        );
        // Delegation record fully reaped.
        assert!(
            db.get_delegation(&addr(1), 7).is_none(),
            "fully unbonded + claimed delegation should be removed"
        );
    }

    #[test]
    fn test_claim_delegation_with_no_unbonding_amount_fails() {
        let mut db = InMemoryStateDB::new();
        seed_validator(&mut db, 7, 9, 100_000);
        fund_account(&mut db, 1, 5_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        executor
            .execute_block(
                &mut db,
                &make_block(
                    1,
                    1,
                    vec![Transaction::Delegate(DelegateTx {
                        delegator: addr(1),
                        validator_id: 7,
                        amount: 1_000,
                        nonce: 0,
                        signature: None,
                        public_key: None,
                    })],
                ),
            )
            .unwrap();

        let r = executor
            .execute_block(
                &mut db,
                &make_block(
                    2,
                    500,
                    vec![Transaction::ClaimDelegation(ClaimDelegationTx {
                        delegator: addr(1),
                        validator_id: 7,
                        nonce: 1,
                        signature: None,
                        public_key: None,
                    })],
                ),
            )
            .unwrap();
        assert_eq!(r.txs_failed, 1, "claim without unbonding must fail");
    }

    // ─── Gap-A #4 governance bounds ───────────────────────────────────

    fn proposal_with(votes_for: u64, votes_against: u64, voter_count: usize) -> GovernanceProposal {
        let mut voters = std::collections::HashSet::new();
        for i in 0..voter_count {
            let mut a = [0u8; 32];
            a[0] = i as u8;
            voters.insert(a);
        }
        GovernanceProposal {
            proposal_id: 0,
            proposer: [0u8; 32],
            title: "t".into(),
            param_key: "block_gas_limit".into(),
            param_value: "1000000".into(),
            start_epoch: 0,
            end_epoch: 100,
            votes_for,
            votes_against,
            status: ProposalStatus::Active,
            created_at: 0,
            voters,
        }
    }

    #[test]
    fn test_decide_proposal_outcome_passes_super_majority_with_quorum() {
        let p = proposal_with(QUORUM_MIN_TOTAL_WEIGHT, 0, QUORUM_MIN_VOTERS);
        assert_eq!(decide_proposal_outcome(&p), ProposalStatus::Passed);
    }

    #[test]
    fn test_decide_proposal_outcome_rejects_below_quorum_weight() {
        let p = proposal_with(QUORUM_MIN_TOTAL_WEIGHT - 1, 0, QUORUM_MIN_VOTERS);
        assert_eq!(decide_proposal_outcome(&p), ProposalStatus::Rejected);
    }

    #[test]
    fn test_decide_proposal_outcome_rejects_below_min_voters() {
        let p = proposal_with(QUORUM_MIN_TOTAL_WEIGHT, 0, QUORUM_MIN_VOTERS - 1);
        assert_eq!(decide_proposal_outcome(&p), ProposalStatus::Rejected);
    }

    #[test]
    fn test_decide_proposal_outcome_rejects_without_super_majority() {
        // for == 2*against → not strictly greater → rejected.
        let p = proposal_with(20_000_000, 10_000_000, QUORUM_MIN_VOTERS);
        assert_eq!(decide_proposal_outcome(&p), ProposalStatus::Rejected);
    }

    #[test]
    fn test_validate_param_value_governable_keys() {
        assert!(validate_param_value("block_gas_limit", "500000").is_ok());
        assert!(validate_param_value("block_gas_limit", "999").is_err());
        assert!(validate_param_value("block_gas_limit", "abc").is_err());
        assert!(validate_param_value("base_fee_floor", "100").is_ok());
        assert!(validate_param_value("base_fee_ceiling", "1000").is_ok());
        assert!(validate_param_value("target_gas_utilization", "0.5").is_ok());
        assert!(validate_param_value("target_gas_utilization", "1.5").is_err());
        assert!(validate_param_value("target_gas_utilization", "-0.1").is_err());
    }

    #[test]
    fn test_validate_param_value_upgrade_contract_pattern() {
        let good = "a".repeat(64);
        let bad_len = "a".repeat(63);
        let bad_hex = "g".repeat(64);
        assert!(validate_param_value("upgrade_contract:42", &good).is_ok());
        assert!(validate_param_value("upgrade_contract:42", &bad_len).is_err());
        assert!(validate_param_value("upgrade_contract:42", &bad_hex).is_err());
    }

    #[test]
    fn test_validate_param_value_unknown_key_rejected() {
        assert!(validate_param_value("chain_id", "evaporchain-evil").is_err());
        assert!(validate_param_value("total_supply", "999999").is_err());
    }

    #[test]
    fn test_is_governable_param_key() {
        assert!(is_governable_param_key("block_gas_limit"));
        assert!(is_governable_param_key("base_fee_floor"));
        assert!(is_governable_param_key("upgrade_contract:0"));
        assert!(is_governable_param_key("upgrade_contract:99999"));
        assert!(!is_governable_param_key("chain_id"));
        assert!(!is_governable_param_key(""));
        assert!(!is_governable_param_key("upgrade_contract")); // missing colon
    }

    // ═══════════════════ UpgradeContract — admin / governance ═══════════════════
    //
    // Mainnet P0: governance-gated bytecode swap. See
    // `execute_upgrade_contract` for the two-path dispatch.

    /// Schema-compatible V2 of COUNTER_SCRIPT — adds a `version` field
    /// with a default. The schema check inside ScriptEngine::upgrade_contract
    /// permits this (no field removed; existing types unchanged).
    const COUNTER_SCRIPT_V2: &str = r#"
contract Counter {
    state {
        count: u64 = 0
        version: u64 = 2
    }
    fn increment(n: u64) {
        self.count += n
    }
    fn get() -> u64 {
        return self.count
    }
    fn get_version() -> u64 {
        return self.version
    }
    on_evaporate() {
        emit("counter expired")
    }
    on_grace() {
        emit("counter entering grace")
    }
}
"#;

    /// Schema-incompatible source — drops `count`. Rejected by the
    /// engine's upgrade check on either auth path.
    const COUNTER_SCRIPT_INCOMPATIBLE: &str = r#"
contract Counter {
    state {
        total: u64 = 0
    }
    fn increment(n: u64) {
        self.total += n
    }
    fn get() -> u64 {
        return self.total
    }
}
"#;

    /// Build an UpgradeContractTx carrying valid bytecode-hash binding,
    /// optionally signed for the admin path.
    fn build_upgrade_tx(
        owner: [u8; 32],
        contract_id: u64,
        new_source: &str,
        nonce: u64,
        admin_kp: Option<&MlDsaKeypair>,
        endorser_stakes: Vec<u64>,
        required_stake: u64,
    ) -> evaporchain_types::UpgradeContractTx {
        let new_bytecode = new_source.as_bytes().to_vec();
        let new_bytecode_hash: [u8; 32] = *blake3::hash(&new_bytecode).as_bytes();
        let (admin_signature, admin_public_key) = match admin_kp {
            Some(kp) => {
                let canonical = format!(
                    "{{\"type\":\"upgrade_contract\",\"contract_id\":{},\"new_bytecode_hash_hex\":\"{}\",\"nonce\":{}}}",
                    contract_id,
                    hex::encode(new_bytecode_hash),
                    nonce
                );
                let sig = kp.sign(canonical.as_bytes());
                (Some(sig), Some(kp.public_key_bytes()))
            }
            None => (None, None),
        };
        evaporchain_types::UpgradeContractTx {
            owner,
            contract_id,
            new_bytecode,
            new_bytecode_hash,
            nonce,
            admin_signature,
            admin_public_key,
            endorser_stakes,
            required_stake,
            governance_approved: false,
            signature: None,
            public_key: None,
        }
    }

    /// Deploy COUNTER_SCRIPT directly through the script engine with
    /// `creator = admin_addr` so the admin slot is set to a key we
    /// control. Bypasses fees/nonces — the test is about the upgrade
    /// path only.
    fn deploy_counter_with_admin(
        executor: &mut SimpleExecutor,
        admin_addr: [u8; 32],
        epoch: u64,
    ) -> u64 {
        executor
            .script_engine
            .deploy(COUNTER_SCRIPT, admin_addr, 10_000, 100, epoch)
            .expect("deploy COUNTER_SCRIPT")
    }

    #[test]
    fn upgrade_contract_admin_path_ok() {
        let kp = MlDsaKeypair::generate();
        let admin_addr: [u8; 32] = *blake3::hash(&kp.public_key_bytes()).as_bytes();

        let mut db = InMemoryStateDB::new();
        // Owner needs an account so nonce check works.
        db.put_account(Account {
            address: admin_addr,
            balance: 1_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        let mut executor = SimpleExecutor::new_for_test(7);
        let id = deploy_counter_with_admin(&mut executor, admin_addr, 1);

        let tx = build_upgrade_tx(admin_addr, id, COUNTER_SCRIPT_V2, 0, Some(&kp), vec![], 0);
        executor
            .execute_upgrade_contract(&mut db, &tx, 2)
            .expect("admin-path upgrade should succeed");

        let contract = executor.script_engine.get(id).unwrap();
        assert_eq!(contract.upgrade_count, 1, "upgrade_count must bump");
        assert_eq!(contract.admin, Some(admin_addr), "admin preserved");
        // V2 schema injects `version` — confirm it's now in state.
        assert!(contract.state.contains_key("version"));
    }

    #[test]
    fn upgrade_contract_admin_signature_invalid_rejects() {
        let admin_kp = MlDsaKeypair::generate();
        let admin_addr: [u8; 32] = *blake3::hash(&admin_kp.public_key_bytes()).as_bytes();
        // Different keypair — the wrong admin.
        let attacker_kp = MlDsaKeypair::generate();

        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: admin_addr,
            balance: 1_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        let mut executor = SimpleExecutor::new_for_test(7);
        let id = deploy_counter_with_admin(&mut executor, admin_addr, 1);

        // Sign with attacker key but claim to be the admin — pk derives
        // to a different address so the chain rejects pre-verify.
        let tx = build_upgrade_tx(
            admin_addr,
            id,
            COUNTER_SCRIPT_V2,
            0,
            Some(&attacker_kp),
            vec![],
            0,
        );

        let err = executor
            .execute_upgrade_contract(&mut db, &tx, 2)
            .expect_err("upgrade with wrong admin key must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("admin_public_key does not match")
                || msg.contains("admin_signature verification failed"),
            "unexpected error: {msg}"
        );

        // Bytecode untouched.
        let contract = executor.script_engine.get(id).unwrap();
        assert_eq!(contract.upgrade_count, 0, "upgrade_count unchanged");
    }

    #[test]
    fn upgrade_contract_governance_path_ok_with_quorum() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        // Contract is admin-owned by addr(1), but we'll exercise the
        // governance path (quorum) — admin path bypassed entirely.
        let id = deploy_counter_with_admin(&mut executor, addr(1), 1);

        // Quorum: stakes sum 1500 ≥ required 1500.
        let tx = build_upgrade_tx(
            addr(1),
            id,
            COUNTER_SCRIPT_V2,
            0,
            None,
            vec![1000, 500],
            1500,
        );
        executor
            .execute_upgrade_contract(&mut db, &tx, 2)
            .expect("governance-path upgrade must succeed when quorum met");

        let contract = executor.script_engine.get(id).unwrap();
        assert_eq!(contract.upgrade_count, 1);
    }

    #[test]
    fn upgrade_contract_governance_path_below_quorum_rejects() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        let id = deploy_counter_with_admin(&mut executor, addr(1), 1);

        // 1499 < 1500.
        let tx = build_upgrade_tx(
            addr(1),
            id,
            COUNTER_SCRIPT_V2,
            0,
            None,
            vec![1000, 499],
            1500,
        );
        let err = executor
            .execute_upgrade_contract(&mut db, &tx, 2)
            .expect_err("below-quorum governance upgrade must be rejected");
        assert!(err.to_string().contains("quorum not met"));

        let contract = executor.script_engine.get(id).unwrap();
        assert_eq!(contract.upgrade_count, 0);
    }

    #[test]
    fn upgrade_contract_bytecode_hash_mismatch_rejects() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        let id = deploy_counter_with_admin(&mut executor, addr(1), 1);

        // Build a valid governance-path tx, then corrupt the hash.
        let mut tx = build_upgrade_tx(addr(1), id, COUNTER_SCRIPT_V2, 0, None, vec![2000], 1000);
        tx.new_bytecode_hash[0] ^= 0xFF;

        let err = executor
            .execute_upgrade_contract(&mut db, &tx, 2)
            .expect_err("bytecode hash mismatch must be rejected");
        assert!(err.to_string().contains("new_bytecode_hash mismatch"));
    }

    #[test]
    fn upgrade_contract_state_preserved() {
        let kp = MlDsaKeypair::generate();
        let admin_addr: [u8; 32] = *blake3::hash(&kp.public_key_bytes()).as_bytes();

        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: admin_addr,
            balance: 1_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        let mut executor = SimpleExecutor::new_for_test(7);
        let id = deploy_counter_with_admin(&mut executor, admin_addr, 1);

        // Mutate state pre-upgrade — count := 7.
        executor
            .script_engine
            .call(
                id,
                "increment",
                vec![evaporchain_script::Value::U64(7)],
                admin_addr,
                2,
            )
            .expect("increment");
        let pre = executor.script_engine.get(id).unwrap().state.clone();
        assert_eq!(pre.get("count"), Some(&evaporchain_script::Value::U64(7)));

        // Upgrade.
        let tx = build_upgrade_tx(admin_addr, id, COUNTER_SCRIPT_V2, 0, Some(&kp), vec![], 0);
        executor
            .execute_upgrade_contract(&mut db, &tx, 3)
            .expect("upgrade ok");

        // count survives, version is the V2 default.
        let post = &executor.script_engine.get(id).unwrap().state;
        assert_eq!(
            post.get("count"),
            Some(&evaporchain_script::Value::U64(7)),
            "pre-upgrade state must survive bytecode swap"
        );
        assert_eq!(
            post.get("version"),
            Some(&evaporchain_script::Value::U64(2)),
            "new field initialised to its declared default"
        );
        assert_eq!(executor.script_engine.get(id).unwrap().upgrade_count, 1);
    }

    #[test]
    fn upgrade_contract_immutable_when_admin_none_rejects() {
        let kp = MlDsaKeypair::generate();
        let admin_addr: [u8; 32] = *blake3::hash(&kp.public_key_bytes()).as_bytes();

        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: admin_addr,
            balance: 1_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        let mut executor = SimpleExecutor::new_for_test(7);
        let id = deploy_counter_with_admin(&mut executor, admin_addr, 1);

        // Freeze: clear admin slot. Real chains would have a separate
        // tx; we patch directly because the test is about the upgrade
        // path's reaction to `admin = None`.
        if let Some(c) = executor.script_engine.contract_mut_for_test(id) {
            c.admin = None;
        }

        // Admin path should be refused — even with a valid signature
        // over the canonical payload, contract.admin is None.
        let tx = build_upgrade_tx(admin_addr, id, COUNTER_SCRIPT_V2, 0, Some(&kp), vec![], 0);
        let err = executor
            .execute_upgrade_contract(&mut db, &tx, 2)
            .expect_err("frozen contracts must reject the admin path");
        assert!(
            err.to_string().contains("frozen"),
            "expected 'frozen' in error, got: {}",
            err
        );

        // Schema-incompatible bytecode: even the governance path rejects
        // it (the engine's schema check guards both paths).
        let tx2 = build_upgrade_tx(
            admin_addr,
            id,
            COUNTER_SCRIPT_INCOMPATIBLE,
            1,
            None,
            vec![10_000],
            1,
        );
        let err2 = executor
            .execute_upgrade_contract(&mut db, &tx2, 3)
            .expect_err("schema-incompatible upgrade must be rejected");
        assert!(err2.to_string().to_lowercase().contains("upgrade"));

        // Upgrade count never bumped.
        assert_eq!(executor.script_engine.get(id).unwrap().upgrade_count, 0);
    }

    // ─── Phase 4.1 (2026-05-03): UserOp paymaster nonce ────────────

    fn make_user_op(
        sender_byte: u8,
        nonce: u64,
        paymaster_byte: Option<u8>,
        paymaster_nonce: Option<u64>,
        gas_limit: u64,
    ) -> evaporchain_types::UserOpTx {
        evaporchain_types::UserOpTx {
            sender: addr(sender_byte),
            nonce,
            call_data: vec![0u8; 16],
            call_gas_limit: gas_limit,
            paymaster: paymaster_byte.map(addr),
            paymaster_nonce,
            paymaster_data: None,
            signature: None,
            public_key: None,
        }
    }

    #[test]
    fn test_user_op_paymaster_nonce_required_when_paymaster_set() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 0);
        fund_account(&mut db, 2, 1_000_000);
        let executor = SimpleExecutor::new_for_test(7);
        // paymaster_nonce = None must fail.
        let tx = make_user_op(1, 0, Some(2), None, 1000);
        let r = executor.execute_user_op(&mut db, &tx, 0);
        assert!(matches!(r, Err(ExecutionError::ContractError(_))));
    }

    #[test]
    fn test_user_op_paymaster_nonce_correct_succeeds_and_bumps() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 0);
        fund_account(&mut db, 2, 1_000_000);
        let executor = SimpleExecutor::new_for_test(7);
        let tx = make_user_op(1, 0, Some(2), Some(0), 1000);
        executor
            .execute_user_op(&mut db, &tx, 0)
            .expect("first exec");

        let pm = db.get_account(&addr(2)).expect("paymaster exists").clone();
        assert_eq!(
            pm.nonce, 1,
            "paymaster nonce should bump from 0 to 1 after successful sponsorship"
        );
        assert!(
            pm.balance < 1_000_000,
            "paymaster balance should be debited"
        );
    }

    #[test]
    fn test_user_op_paymaster_replay_blocked() {
        // Replaying the SAME UserOp (same sender_nonce, same
        // paymaster_nonce) must fail. Without the paymaster nonce
        // check the paymaster could be drained twice.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 0);
        fund_account(&mut db, 2, 1_000_000);
        let executor = SimpleExecutor::new_for_test(7);
        let tx = make_user_op(1, 0, Some(2), Some(0), 1000);

        executor
            .execute_user_op(&mut db, &tx, 0)
            .expect("first exec");

        // Replay the same tx — sender nonce check catches it first.
        let r = executor.execute_user_op(&mut db, &tx, 0);
        assert!(matches!(r, Err(ExecutionError::InvalidNonce { .. })));

        // Even if a malicious actor somehow bumps the sender's nonce
        // independently and then tries to replay only the paymaster
        // sponsorship, the paymaster_nonce check catches it on the
        // paymaster side.
        // Simulate: bump sender nonce manually as if a new tx
        // legitimately moved it forward.
        let s = db.get_or_create_account(&addr(1));
        s.nonce = 1;
        // Same tx with sender nonce=1 but paymaster_nonce STILL 0
        // (replay of the original sponsorship).
        let replay = make_user_op(1, 1, Some(2), Some(0), 1000);
        let r = executor.execute_user_op(&mut db, &replay, 0);
        assert!(
            matches!(r, Err(ExecutionError::InvalidNonce { .. })),
            "paymaster nonce check should reject replayed sponsorship"
        );
    }

    /// Crooks-MEV Phase 3.5 economic-design regression test.
    ///
    /// `Transaction::Refund` is protocol-issued — the proposer pays
    /// gas, not the victim or attacker. The dedicated `GAS_REFUND`
    /// constant (5_000) is intentionally lower than `GAS_TRANSFER`
    /// (21_000) so proposers aren't economically deterred from
    /// settling refunds they're contractually obligated to settle
    /// under Phase 3.5d's missing-refund stake-slash rule.
    ///
    /// **Fix history (2026-05-05):** Earlier code charged
    /// `GAS_TRANSFER` for Refund txs in both `SimpleExecutor` (lib.rs
    /// `estimate_gas`) and `ParallelExecutor` (parallel.rs
    /// `gas_for_local`), which defeated the Phase 3.5 economic design.
    /// `GAS_REFUND` was declared with a clear docstring but never
    /// actually wired into the gas-charging path — surfaced as a
    /// `dead_code` warning and fixed under both executors.
    ///
    /// This test locks the contract: the per-tx gas cost for a
    /// Refund must equal `GAS_REFUND`, not `GAS_TRANSFER`.
    #[test]
    fn refund_tx_charges_gas_refund_not_gas_transfer() {
        let refund = Transaction::Refund(RefundTx {
            source_block_height: 1,
            source_observation_idx: 0,
            attacker: addr(1),
            victim: addr(2),
            amount: 100,
            settle_block_height: 2,
        });
        let cost = SimpleExecutor::estimate_gas(&refund);
        assert_eq!(
            cost, GAS_REFUND,
            "refund tx must charge GAS_REFUND ({}), not GAS_TRANSFER ({})",
            GAS_REFUND, GAS_TRANSFER
        );
        assert!(
            cost < GAS_TRANSFER,
            "GAS_REFUND must be lower than GAS_TRANSFER to keep proposer settlement economically attractive"
        );
    }
}
