pub mod block_stm;
pub mod economics;
pub mod energy_audit;
pub mod fees;
pub mod genesis;
pub mod genesis_invariant;
pub mod lyapunov_fees;
pub mod parallel;
pub mod privacy_exec;
pub mod rewards;
pub mod sanov_slash_helpers;
pub mod temporal;
#[cfg(test)]
mod audit_tests;

use evaporchain_contracts::{ContractEngine, ContractTemplate};
use evaporchain_crypto::signatures::{HybridVerifier, Verifier};
use evaporchain_crypto::MerkleMountainRange;
use evaporchain_proving::evaporation_proof::{EvaporationClaim, EvaporationProof, EvaporationProver};
use evaporchain_script::ScriptEngine;
use evaporchain_state::db::StateDB;
use evaporchain_state::{EvaporationEngine, RefreshEngine};
use evaporchain_types::{
    Block, CallContractTx, CallScriptTx, ClaimDelegationTx, CreateObjectTx, DelegateTx,
    DelegationRecord, DeployContractTx, DeployScriptTx, Epoch, GovernanceAction,
    GovernanceProposal, GovernanceTx, MultiSigTx, ObjectState, ProposalStatus, RefreshTx,
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
}

/// Contract event emitted during block execution, tagged with origin.
#[derive(Debug, Clone)]
pub struct BlockContractEvent {
    pub contract_id: u64,
    pub tx_index: usize,
    pub event: evaporchain_script::ContractEvent,
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
}

/// Gas cost constants for transaction types.
pub(crate) const GAS_TRANSFER: u64 = 21_000;
pub(crate) const GAS_CREATE_OBJECT_BASE: u64 = 50_000;
pub(crate) const GAS_CREATE_OBJECT_PER_BYTE: u64 = 200;
pub(crate) const GAS_REFRESH: u64 = 30_000;
pub(crate) const GAS_DEPLOY_CONTRACT: u64 = 100_000;
pub(crate) const GAS_CALL_CONTRACT: u64 = 40_000;
pub(crate) const GAS_DEPLOY_SCRIPT: u64 = 150_000;
pub(crate) const GAS_CALL_SCRIPT: u64 = 50_000;
pub(crate) const GAS_VALIDATOR_STAKE: u64 = 50_000;
pub(crate) const GAS_VALIDATOR_EXIT: u64 = 30_000;
pub(crate) const GAS_VALIDATOR_CLAIM_STAKE: u64 = 30_000;
pub(crate) const GAS_GOVERNANCE: u64 = 25_000;
pub(crate) const GAS_MULTISIG: u64 = 50_000;
pub(crate) const GAS_USER_OP: u64 = 30_000;
pub(crate) const GAS_UPGRADE_CONTRACT: u64 = 100_000;
pub(crate) const GAS_DELEGATE: u64 = 40_000;
pub(crate) const GAS_UNDELEGATE: u64 = 40_000;
pub(crate) const GAS_CLAIM_DELEGATION: u64 = 30_000;
/// BLS key rotation: covers two PoP-style verifications (old + new) plus
/// the validator-set update. Higher than stake/exit because of the
/// double signature check.
pub(crate) const GAS_ROTATE_VALIDATOR_KEY: u64 = 80_000;

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
    if proposal.votes_for > proposal.votes_against.saturating_mul(PASS_THRESHOLD_MULTIPLIER) {
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
                    Err(format!("block_gas_limit out of range [1_000, 10_000_000_000]: {}", v))
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
                    Err(format!("target_gas_utilization out of range [0.0, 1.0]: {}", v))
                }
            }),
        k if k.starts_with("upgrade_contract:") => {
            if value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()) {
                Ok(())
            } else {
                Err("upgrade_contract value must be 64-char hex (blake3 of new bytecode)".into())
            }
        }
        _ => Err(format!("param_key '{}' is not on the governable allowlist", key)),
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
        self.entries.insert(key, CachedResult {
            gas_used,
            success,
            last_used_height: height,
        });
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
    txs.sort_by(|a, b| {
        SimpleExecutor::estimate_gas(b).cmp(&SimpleExecutor::estimate_gas(a))
    });
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
}

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
        }
    }

    /// Set the chain ID for signing message domain separation.
    pub fn set_chain_id(&mut self, chain_id: String) {
        self.chain_id = chain_id;
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

    /// Estimate gas for a transaction.
    fn estimate_gas(tx: &Transaction) -> u64 {
        match tx {
            Transaction::Transfer(_) => GAS_TRANSFER,
            Transaction::CreateObject(create) => {
                GAS_CREATE_OBJECT_BASE.saturating_add(GAS_CREATE_OBJECT_PER_BYTE.saturating_mul(create.data.len() as u64))
            }
            Transaction::Refresh(_) => GAS_REFRESH,
            Transaction::DeployContract(_) => GAS_DEPLOY_CONTRACT,
            Transaction::CallContract(_) => GAS_CALL_CONTRACT,
            Transaction::DeployScript(_) => GAS_DEPLOY_SCRIPT,
            Transaction::CallScript(_) => GAS_CALL_SCRIPT,
            Transaction::ValidatorStake(_) => GAS_VALIDATOR_STAKE,
            Transaction::ValidatorExit(_) => GAS_VALIDATOR_EXIT,
            Transaction::ValidatorClaimStake(_) => GAS_VALIDATOR_CLAIM_STAKE,
            Transaction::Shield(_) => privacy_exec::GAS_SHIELD,
            Transaction::Unshield(_) => privacy_exec::GAS_UNSHIELD,
            Transaction::PrivateTransfer(ptx) => {
                privacy_exec::PrivacyExecutor::estimate_private_transfer_gas(ptx)
            }
            Transaction::Deferred(dtx) => {
                temporal::GAS_DEFERRED_SUBMIT
                    .saturating_add(temporal::GAS_PER_GUARD.saturating_mul(dtx.guards.len() as u64))
            }
            Transaction::Blob(tx) => {
                GAS_CREATE_OBJECT_BASE.saturating_add(GAS_CREATE_OBJECT_PER_BYTE.saturating_mul(tx.data.len() as u64))
            }
            Transaction::Governance(_) => GAS_GOVERNANCE,
            Transaction::MultiSig(_) => GAS_MULTISIG,
            Transaction::UserOp(tx) => GAS_USER_OP.saturating_add(tx.call_data.len() as u64 * 16),
            Transaction::UpgradeContract(tx) => GAS_UPGRADE_CONTRACT.saturating_add(tx.new_bytecode.len() as u64 * 200),
            Transaction::Delegate(_) => GAS_DELEGATE,
            Transaction::Undelegate(_) => GAS_UNDELEGATE,
            Transaction::RotateValidatorKey(_) => GAS_ROTATE_VALIDATOR_KEY,
            Transaction::ClaimDelegation(_) => GAS_CLAIM_DELEGATION,
        }
    }

    /// Verify the ML-DSA signature on a transaction (if verification is enabled).
    /// Unshield and PrivateTransfer are authenticated by ZK proofs, not signatures.
    fn verify_tx_signature(&self, tx: &Transaction) -> Result<(), ExecutionError> {
        if !self.verify_signatures {
            return Ok(());
        }

        // ZK-authenticated transactions don't use signatures
        if matches!(tx, Transaction::Unshield(_) | Transaction::PrivateTransfer(_)) {
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
    ) -> Result<(), ExecutionError> {
        if tx.from == tx.to {
            return Err(ExecutionError::SelfTransfer);
        }
        if tx.amount == 0 {
            return Err(ExecutionError::ZeroAmount);
        }

        // Check sender nonce
        let sender = db.get_or_create_account(&tx.from);
        if sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        if sender.balance < tx.amount {
            return Err(ExecutionError::InsufficientBalance {
                account: hex::encode(tx.from),
                available: sender.balance,
                required: tx.amount,
            });
        }

        // Debit sender
        sender.balance -= tx.amount;
        sender.nonce += 1;

        // Credit receiver
        let receiver = db.get_or_create_account(&tx.to);
        receiver.balance = receiver.balance.saturating_add(tx.amount);

        debug!(
            from = hex::encode(tx.from),
            to = hex::encode(tx.to),
            amount = tx.amount,
            "Transfer executed"
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
            return Err(ExecutionError::ObjectAlreadyExists(hex::encode(tx.object_id)));
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
        };

        db.put_object(obj);

        debug!(
            object_id = hex::encode(tx.object_id),
            energy = tx.energy,
            half_life = tx.half_life,
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

        let id = self
            .contract_engine
            .deploy(template, init_args, rules, tx.deployer, tx.energy, tx.half_life, epoch)
            .map_err(|e| ExecutionError::ContractError(e.to_string()))?;

        debug!(contract_id = id, template = %tx.template, "Contract deployed");
        Ok(())
    }

    /// Execute a contract call transaction.
    fn execute_call_contract(
        &mut self,
        tx: &CallContractTx,
    ) -> Result<(), ExecutionError> {
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
        tx: &DeployScriptTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        let id = self
            .script_engine
            .deploy(&tx.source_code, tx.deployer, tx.energy, tx.half_life, epoch)
            .map_err(|e| ExecutionError::ScriptError(e.to_string()))?;

        debug!(script_id = id, "Script contract deployed");
        Ok(())
    }

    /// Execute a script call transaction.
    fn execute_call_script(
        &mut self,
        tx: &CallScriptTx,
    ) -> Result<(), ExecutionError> {
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
                result.structured_events.into_iter().map(|event| {
                    (tx.contract_id, event)
                })
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
        if sender.balance < tx.stake_amount {
            return Err(ExecutionError::InsufficientBalance {
                account: hex::encode(tx.validator_address),
                available: sender.balance,
                required: tx.stake_amount,
            });
        }

        // Lock stake by deducting from balance
        sender.balance -= tx.stake_amount;
        sender.nonce += 1;

        let existing_stake = db.get_stake(tx.validator_id).map(|s| s.staked_amount).unwrap_or(0);
        db.put_stake(StakeRecord {
            validator_id: tx.validator_id,
            validator_address: tx.validator_address,
            staked_amount: existing_stake + tx.stake_amount,
            staked_at_epoch: 0,
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
        if delegator.balance < tx.amount {
            return Err(ExecutionError::InsufficientBalance {
                account: hex::encode(tx.delegator),
                available: delegator.balance,
                required: tx.amount,
            });
        }
        delegator.balance -= tx.amount;
        delegator.nonce += 1;

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

        let mut record = db
            .get_delegation(&tx.delegator, tx.validator_id)
            .cloned()
            .ok_or_else(|| ExecutionError::ContractError(format!(
                "no delegation from {} to validator-id {}",
                hex::encode(tx.delegator), tx.validator_id
            )))?;
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

        let mut record = db
            .get_delegation(&tx.delegator, tx.validator_id)
            .cloned()
            .ok_or_else(|| ExecutionError::ContractError(format!(
                "no delegation from {} to validator-id {}",
                hex::encode(tx.delegator), tx.validator_id
            )))?;
        if record.unbonding_amount == 0 {
            return Err(ExecutionError::ContractError(
                "no unbonding amount to claim".into(),
            ));
        }
        let unbonding_started = record.unbonding_epoch.ok_or_else(|| {
            ExecutionError::ContractError("unbonding_epoch unset".into())
        })?;
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

        let mut stake = db.get_stake(tx.validator_id).cloned().ok_or_else(|| {
            ExecutionError::ObjectNotFound(
                format!("no stake record for validator {}", tx.validator_id),
            )
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
        let stake = stake.ok_or_else(|| ExecutionError::ObjectNotFound(
            format!("no stake record for validator {}", tx.validator_id)
        ))?;

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

        match &tx.action {
            GovernanceAction::CreateProposal { title, param_key, param_value, voting_epochs } => {
                // Gap-A #4: bound checks on proposal admission.
                if title.len() > MAX_PROPOSAL_TITLE_BYTES {
                    return Err(ExecutionError::ContractError(format!(
                        "proposal title exceeds {} bytes ({})",
                        MAX_PROPOSAL_TITLE_BYTES, title.len()
                    )));
                }
                if param_key.len() > MAX_PARAM_KEY_BYTES {
                    return Err(ExecutionError::ContractError(format!(
                        "param_key exceeds {} bytes ({})",
                        MAX_PARAM_KEY_BYTES, param_key.len()
                    )));
                }
                if param_value.len() > MAX_PARAM_VALUE_BYTES {
                    return Err(ExecutionError::ContractError(format!(
                        "param_value exceeds {} bytes ({})",
                        MAX_PARAM_VALUE_BYTES, param_value.len()
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
                        "invalid param_value: {}", e
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
                let mut proposal = proposal.ok_or_else(|| ExecutionError::ContractError(
                    format!("proposal {} not found", proposal_id)
                ))?;

                if proposal.status != ProposalStatus::Active {
                    return Err(ExecutionError::ContractError(
                        "proposal is not active".to_string()
                    ));
                }

                if current_epoch > proposal.end_epoch {
                    // Voting closed — finalize using Gap-A #4 quorum + super-majority.
                    proposal.status = decide_proposal_outcome(&proposal);
                    db.put_proposal(proposal);
                    return Err(ExecutionError::ContractError(
                        "voting period has ended".to_string()
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
    ) -> Result<(), ExecutionError> {
        if (tx.signatures.len() as u8) < tx.threshold {
            return Err(ExecutionError::ContractError(format!(
                "multi-sig requires {} signatures, got {}",
                tx.threshold, tx.signatures.len()
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
                    "signer not in authorized signers list".to_string()
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

        Ok(())
    }

    fn execute_user_op(
        &self,
        db: &mut dyn StateDB,
        tx: &evaporchain_types::UserOpTx,
    ) -> Result<(), ExecutionError> {
        let sender = db.get_or_create_account(&tx.sender);
        if sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        sender.nonce += 1;

        if let Some(ref paymaster) = tx.paymaster {
            let pm = db.get_or_create_account(paymaster);
            let total_gas_cost = tx.call_gas_limit.saturating_add(GAS_USER_OP);
            if pm.balance < total_gas_cost {
                return Err(ExecutionError::InsufficientGas {
                    account: hex::encode(paymaster),
                    required: total_gas_cost,
                    available: pm.balance,
                });
            }
            pm.balance = pm.balance.saturating_sub(total_gas_cost);
        }

        Ok(())
    }

    /// Execute UpgradeContract — closes K-10.
    ///
    /// Authorization layers (all must hold for the upgrade to apply):
    ///   1. Sender nonce matches and bumps.
    ///   2. A governance proposal with key `upgrade_contract:{contract_id}`
    ///      and status Passed exists, AND its value equals the
    ///      hex-encoded blake3 of `tx.new_bytecode`. The hash binding
    ///      prevents bait-and-switch: the proposal commits to specific
    ///      bytecode at proposal time and the tx must supply that
    ///      exact bytecode at apply time.
    ///   3. ScriptEngine::upgrade_contract enforces caller-is-creator
    ///      and schema-compatibility internally (no field removal,
    ///      no type narrowing).
    ///
    /// On success the proposal is marked Executed so a single approval
    /// can't be replayed.
    fn execute_upgrade_contract(
        &mut self,
        db: &mut dyn StateDB,
        tx: &evaporchain_types::UpgradeContractTx,
        current_epoch: u64,
    ) -> Result<(), ExecutionError> {
        // Nonce check + bump.
        let sender = db.get_or_create_account(&tx.owner);
        if sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        sender.nonce = sender.nonce.saturating_add(1);

        // Governance gate: find a Passed proposal whose key/value
        // commit to this contract's upgrade with the supplied bytecode.
        let bytecode_hash = hex::encode(blake3::hash(&tx.new_bytecode).as_bytes());
        let key = format!("upgrade_contract:{}", tx.contract_id);
        let approval = db
            .all_proposals()
            .into_iter()
            .find(|p| {
                p.status == evaporchain_types::ProposalStatus::Passed
                    && p.param_key == key
                    && p.param_value == bytecode_hash
            })
            .cloned();
        let mut approval = approval.ok_or_else(|| {
            ExecutionError::ContractError(format!(
                "UpgradeContract: no Passed governance proposal authorizing key='{}' \
                 with bytecode hash {}",
                key, bytecode_hash
            ))
        })?;

        // Apply the upgrade through ScriptEngine. UTF-8 source is the
        // shape DeployScript uses; UpgradeContractTx mirrors it.
        let new_source = std::str::from_utf8(&tx.new_bytecode).map_err(|_| {
            ExecutionError::ContractError(
                "UpgradeContract: new_bytecode is not valid UTF-8 EvaporScript source".into(),
            )
        })?;
        self.script_engine
            .upgrade_contract(tx.contract_id, new_source, tx.owner, current_epoch)
            .map_err(|e| ExecutionError::ContractError(e.to_string()))?;

        // Mark the proposal Executed so it can't be replayed.
        approval.status = evaporchain_types::ProposalStatus::Executed;
        db.put_proposal(approval);

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
                let rent = acct.storage_bytes.saturating_mul(
                    evaporchain_types::STORAGE_RENT_PER_BYTE_PER_EPOCH
                );
                (rent, acct.balance)
            };
            let acct = db.get_or_create_account(&addr);
            if acct.balance >= rent_info.0 {
                acct.balance -= rent_info.0;
            } else {
                // Account is being zeroed out by storage rent — engrave
                // the chain's small-deaths memorial via evaporchain-
                // tombstone before we wipe state. Per doctrine §A2.5
                // the eulogy trie is the deliberate exception to §2.2's
                // anti-immutability rule.
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
                        let to_acct = db.get_or_create_account(&to_addr);
                        to_acct.balance += *amount;
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
        self.apply_governance_params(db);
        self.finalize_expired_proposals(db, block.epoch);

        let mut txs_executed = 0;
        let mut txs_failed = 0;
        let mut gas_used = 0u64;
        let mut total_fees = 0u64;
        let base_fee = self
            .fee_controller
            .as_ref()
            .map_or(0, |fc| fc.base_fee);
        let mut validator_key_rotations: Vec<ValidatorKeyRotation> = Vec::new();

        // Execute transactions
        for tx in &block.transactions {
            // Signature verification (if enabled)
            if let Err(e) = self.verify_tx_signature(tx) {
                debug!(error = %e, "Signature verification failed");
                txs_failed += 1;
                continue;
            }

            let tx_gas = Self::estimate_gas(tx);

            // Enforce per-block gas limit: skip transactions that would exceed it
            if self.block_gas_limit > 0 && gas_used + tx_gas > self.block_gas_limit {
                debug!(
                    gas_used,
                    tx_gas,
                    block_gas_limit = self.block_gas_limit,
                    "Skipping transaction: block gas limit would be exceeded"
                );
                txs_failed += 1;
                continue;
            }

            // Compute and deduct fees BEFORE execution (if fee controller enabled)
            let tx_fee = if let Some(fc) = &self.fee_controller {
                let gas_fee = fc.compute_gas_fee(tx_gas, 0);
                let extra_fee = match tx {
                    Transaction::CreateObject(create) => {
                        fc.compute_creation_deposit(create.data.len())
                    }
                    Transaction::Refresh(refresh) => {
                        fc.compute_refresh_fee(refresh.energy_deposit)
                    }
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
                        continue;
                    }
                    // Deduct fees upfront (burned — deflationary model)
                    sender.balance -= total_tx_fee;
                }
                total_tx_fee
            } else {
                0
            };

            // Snapshot sender state before execution for revert-on-failure
            let sender_snapshot = tx.sender().and_then(|addr| {
                db.get_account(addr).map(|acct| (acct.balance, acct.nonce))
            });

            let result = match tx {
                Transaction::Transfer(transfer) => self.execute_transfer(db, transfer),
                Transaction::CreateObject(create) => {
                    self.execute_create_object(db, create, block.epoch)
                }
                Transaction::Refresh(refresh) => self.execute_refresh(db, refresh, block.epoch),
                Transaction::DeployContract(deploy) => self.execute_deploy_contract(deploy, block.epoch),
                Transaction::CallContract(call) => self.execute_call_contract(call),
                Transaction::DeployScript(deploy) => self.execute_deploy_script(deploy, block.epoch),
                Transaction::CallScript(call) => self.execute_call_script(call),
                Transaction::ValidatorStake(stake) => self.execute_validator_stake(db, stake),
                Transaction::ValidatorExit(exit) => self.execute_validator_exit(db, exit, block.epoch),
                Transaction::ValidatorClaimStake(claim) => self.execute_validator_claim_stake(db, claim, block.epoch),
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
                Transaction::Deferred(dtx) => {
                    self.deferred_queue
                        .submit(dtx.clone())
                        .map(|_| ())
                        .map_err(|e| ExecutionError::ContractError(e.to_string()))
                }
                Transaction::Blob(blob) => {
                    if blob.data.is_empty() {
                        Err(ExecutionError::ContractError("blob data cannot be empty".into()))
                    } else if blob.data.len() > MAX_BLOB_SIZE {
                        Err(ExecutionError::ContractError(format!(
                            "blob size {} exceeds limit {}", blob.data.len(), MAX_BLOB_SIZE
                        )))
                    } else if blob.namespace_id == 0 {
                        Err(ExecutionError::ContractError("reserved namespace_id 0".into()))
                    } else {
                        Ok(())
                    }
                }
                Transaction::Governance(gov) => self.execute_governance(db, gov, block.epoch),
                Transaction::MultiSig(msig) => self.execute_multisig(db, msig),
                Transaction::UserOp(uop) => self.execute_user_op(db, uop),
                Transaction::UpgradeContract(up) => self.execute_upgrade_contract(db, up, block.epoch),
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
                        let stake_addr = db
                            .get_stake(rot.validator_id)
                            .map(|s| s.validator_address);
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
                                    use evaporchain_crypto::signatures::{BlsPublicKey, BlsSignature, BlsVerifier};
                                    let pk = BlsPublicKey(rot.new_bls_public_key.clone());
                                    let pop = BlsSignature(rot.bls_pop_new.clone());
                                    BlsVerifier::verify_proof_of_possession(&pk, &pop)
                                } {
                                    Err(ExecutionError::ContractError(
                                        "RotateValidatorKey: bls_pop_new failed verification".into(),
                                    ))
                                } else {
                                    if let Some(acct) = db.get_account_mut(&rot.validator_address) {
                                        acct.nonce = acct.nonce.saturating_add(1);
                                    }
                                    validator_key_rotations.push(ValidatorKeyRotation {
                                        validator_id: rot.validator_id,
                                        new_bls_public_key: rot.new_bls_public_key.clone(),
                                        bls_pop_old: rot.bls_pop_old.clone(),
                                        new_bls_pop: rot.bls_pop_new.clone(),
                                        prev_key_expiry_epoch: rot.effective_epoch
                                            .saturating_add(KEY_ROTATION_GRACE_EPOCHS),
                                    });
                                    Ok(())
                                }
                            }
                        }
                    }
                }
                Transaction::ClaimDelegation(c) => self.execute_claim_delegation(db, c, block.epoch),
            };

            match result {
                Ok(()) => {
                    txs_executed += 1;
                    gas_used += tx_gas;
                    total_fees += tx_fee;
                }
                Err(e) => {
                    // Revert sender state changes from the failed execution,
                    // but KEEP the fee deduction (sender still pays for gas used).
                    // Snapshot was taken AFTER fee deduction but BEFORE execution,
                    // so restoring it reverts execution changes while keeping fees burned.
                    if let (Some(sender_addr), Some((snap_balance, snap_nonce))) = (tx.sender(), sender_snapshot) {
                        if let Some(acct) = db.get_account_mut(sender_addr) {
                            acct.balance = snap_balance;
                            acct.nonce = snap_nonce;
                        }
                    }
                    debug!(error = %e, "Transaction failed — state reverted, fee kept");
                    txs_failed += 1;
                    total_fees += tx_fee; // Fee is still burned even on failure
                }
            }
        }

        // Run evaporation at end of block (with MMR nullifier accumulation)
        let evap_result = self.evaporation_engine.process_epoch_with_mmr(db, block.epoch, &mut self.mmr);

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
            let args_val: serde_json::Value = serde_json::from_str(args)
                .unwrap_or(serde_json::Value::Null);
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
                        Transaction::Transfer(t) => self.execute_transfer(db, t),
                        Transaction::CreateObject(c) => self.execute_create_object(db, c, block.epoch),
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

            // Distribute staker rewards every 100 blocks
            if block.number.is_multiple_of(100) && ra.pending_staker_rewards > 0 {
                let stakers: Vec<([u8; 32], u64)> = db
                    .all_stakes()
                    .iter()
                    .filter(|s| s.unbonding_epoch.is_none())
                    .map(|s| (s.validator_address, s.staked_amount))
                    .collect();
                let distributed = ra.distribute_staker_rewards(db, &stakers);
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
            db.put_last_rent_epoch(block.epoch);
        }

        // Vesting timelock release tick — runs every block (per-block
        // schedules need responsive release). Idempotent within an
        // epoch because pending_release_at == 0 once released_amount
        // catches up to vested_at.
        self.tick_vesting(db, block.epoch);

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

        let contract_events: Vec<BlockContractEvent> = self.pending_events.drain(..)
            .map(|(contract_id, event)| BlockContractEvent {
                contract_id,
                tx_index: 0,
                event,
            })
            .collect();

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
        })
    }

    fn mmr_root(&self) -> [u8; 32] {
        self.mmr.root()
    }

    fn mmr_size(&self) -> usize {
        self.mmr.size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_crypto::signatures::{MlDsaKeypair, Signer};
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::Account;

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
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);

        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 100);
        assert!(db.get_account(&addr(2)).is_none());
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
            from: addr(1), to: addr(2), amount: 100, nonce: 0,
            signature: None, public_key: None,
        });
        let tx2 = Transaction::Transfer(TransferTx {
            from: addr(1), to: addr(2), amount: 100, nonce: 0,
            signature: None, public_key: None,
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
            from: addr(1), to: addr(2), amount: 100, nonce: 0,
            signature: None, public_key: None,
        });
        let tx2 = Transaction::Transfer(TransferTx {
            from: addr(1), to: addr(2), amount: 100, nonce: 1,
            signature: None, public_key: None,
        });
        let block = make_block(1, 1, vec![tx1, tx2]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 2);
        assert_eq!(result.txs_failed, 0);
        assert_eq!(db.get_account(&addr(1)).unwrap().nonce, 2);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 999_800);
    }

    // ─── Object Creation with Energy ───

    #[test]
    fn test_create_object_with_energy() {
        let mut db = InMemoryStateDB::new();
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
    }

    #[test]
    fn test_create_object_with_decay_curve() {
        let mut db = InMemoryStateDB::new();
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
                decay_curve: Some(evaporchain_types::DecayCurve::Linear { rate_per_epoch: 100 }),
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);

        let obj = db.get_object(&obj_id(77)).unwrap();
        assert_eq!(
            obj.decay_curve,
            Some(evaporchain_types::DecayCurve::Linear { rate_per_epoch: 100 })
        );
    }

    // ─── Duplicate Object Creation Fails ───

    #[test]
    fn test_duplicate_object_creation_fails() {
        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);

        let create_tx = Transaction::CreateObject(CreateObjectTx {
            creator: addr(1),
            object_id: obj_id(42),
            energy: 1000,
            half_life: 50,
            data: vec![],
            decay_curve: None,
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
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(2),
                    to: addr(3),
                    amount: 1000,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                }),
                Transaction::CreateObject(CreateObjectTx {
                    creator: addr(1),
                    object_id: obj_id(10),
                    energy: 500,
                    half_life: 50,
                    data: vec![1],
                    decay_curve: None,
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
                }),
            ],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 4);
        assert_eq!(result.txs_failed, 0);

        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 7500);
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
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(3),
                    amount: 400,
                    nonce: 1,
                    signature: None,
                    public_key: None,
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(4),
                    amount: 100,
                    nonce: 1,
                    signature: None,
                    public_key: None,
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
        let mut executor = SimpleExecutor::new_with_sig_verification_for_test(7);
        let kp = MlDsaKeypair::generate();

        let mut tx = Transaction::CreateObject(CreateObjectTx {
            creator: addr(1),
            object_id: obj_id(42),
            energy: 5000,
            half_life: 100,
            data: vec![0xDE, 0xAD],
            decay_curve: None,
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
            vec![Transaction::CallScript(
                evaporchain_types::CallScriptTx {
                    caller: addr(2),
                    contract_id: 1,
                    method: "increment".to_string(),
                    args: r#"[{"U64": 42}]"#.to_string(),
                    epoch: 2,
                    signature: None,
                    public_key: None,
                },
            )],
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
        let mut executor = SimpleExecutor::new_for_test(7);

        // Deploy a template contract
        let deploy_template = make_block(
            1,
            1,
            vec![Transaction::DeployContract(DeployContractTx {
                deployer: addr(1),
                template: "DecayingToken".to_string(),
                init_args: r#"{"name":"TestToken","symbol":"TT","total_supply":1000000,"decay_half_life":100,"owner":"alice"}"#
                    .to_string(),
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
            from: addr(1), to: addr(2), amount: 100, nonce: 0,
            signature: None, public_key: None,
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
            from: addr(1), to: addr(2), amount: 100, nonce: 0,
            signature: None, public_key: None,
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
            from: addr(1), to: addr(2), amount: 100, nonce: 0,
            signature: None, public_key: None,
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
            from: addr(1), to: addr(2), amount: 100, nonce: 0,
            signature: None, public_key: None,
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
            from: addr(1), to: addr(2), amount: 100, nonce: 0,
            signature: None, public_key: None,
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
            from: addr(1), to: addr(2), amount: 100, nonce: 0,
            signature: None, public_key: None,
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

        let block = make_block(1, 1, vec![
            Transaction::Transfer(TransferTx {
                from: addr(1), to: addr(2), amount: 100, nonce: 0,
                signature: None, public_key: None,
            }),
        ]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert!(result.total_fees > 0, "Fees should be collected");

        let sender = db.get_account(&addr(1)).unwrap();
        // Balance should be: 1_000_000 - 100 (transfer) - gas_fee
        assert!(sender.balance < 1_000_000 - 100, "Fees should have been deducted: balance={}", sender.balance);
    }

    #[test]
    fn test_insufficient_balance_for_gas_rejected() {
        let mut db = InMemoryStateDB::new();
        // Fund with only 10 — not enough for gas (transfer costs 21000 * 1 = 21000)
        fund_account(&mut db, 1, 10);

        let fc = fees::PidFeeController::testnet_config();
        let mut executor = SimpleExecutor::new_with_fees_for_test(7, fc, 500_000);

        let block = make_block(1, 1, vec![
            Transaction::Transfer(TransferTx {
                from: addr(1), to: addr(2), amount: 5, nonce: 0,
                signature: None, public_key: None,
            }),
        ]);
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

        let block = make_block(1, 1, vec![
            Transaction::Transfer(TransferTx {
                from: addr(1), to: addr(2), amount: 999_999, nonce: 0,
                signature: None, public_key: None,
            }),
        ]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);
        assert!(result.total_fees > 0, "Fee should still be burned on failure");

        // Balance should be reduced by gas fee even though transfer failed
        let sender = db.get_account(&addr(1)).unwrap();
        assert!(sender.balance < 50_000, "Gas fee should have been deducted: balance={}", sender.balance);
    }

    #[test]
    fn test_creation_deposit_deducted() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);

        let fc = fees::PidFeeController::testnet_config();
        let mut executor = SimpleExecutor::new_with_fees_for_test(7, fc, 500_000);

        let block = make_block(1, 1, vec![
            Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(42),
                energy: 5000,
                half_life: 100,
                data: vec![1, 2, 3, 4, 5],
                decay_curve: None,
                signature: None,
                public_key: None,
            }),
        ]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);

        // Fee should include gas + creation deposit
        // Gas: 50000 + 200*5 = 51000; fee = 51000 * 1 = 51000
        // Creation deposit: max(100 * 5, 1000) = 1000
        // Total: 52000
        assert!(result.total_fees >= 52_000, "Creation deposit should be included: fees={}", result.total_fees);
    }

    #[test]
    fn test_revert_on_failed_tx_keeps_fee() {
        let mut db = InMemoryStateDB::new();
        // Balance: 100_000. Transfer gas fee = 21000.
        // After fee deduction: 79_000. Transfer amount 999_999 > 79_000 → fail.
        fund_account(&mut db, 1, 100_000);

        let fc = fees::PidFeeController::testnet_config();
        let mut executor = SimpleExecutor::new_with_fees_for_test(7, fc, 500_000);

        let block = make_block(1, 1, vec![
            Transaction::Transfer(TransferTx {
                from: addr(1), to: addr(2), amount: 999_999, nonce: 0,
                signature: None, public_key: None,
            }),
        ]);
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
        let block = make_block(1, 1, vec![
            Transaction::Transfer(TransferTx {
                from: addr(1), to: addr(2), amount: 100, nonce: 0,
                signature: None, public_key: None,
            }),
        ]);
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
        assert!(fee > 0, "Fee should remain positive even at overflow boundaries");
    }

    #[test]
    fn stress_block_gas_limit_enforcement() {
        let mut db = InMemoryStateDB::new();
        let fee_controller = crate::fees::PidFeeController::new(0.5, 0.1, 0.01, 0.05, 1000, 1, 1_000_000_000);
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
        db.put_account(Account { address: owner, balance: 1_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0 });
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
        };
        db.put_object(obj);

        assert_eq!(executor.mmr_root(), [0u8; 32], "MMR should be empty at start");
        assert_eq!(executor.mmr_size(), 0);

        for epoch in 1..=20 {
            let block = make_block(epoch, epoch, vec![]);
            let result = executor.execute_block(&mut db, &block).unwrap();
            if result.objects_evaporated > 0 {
                assert_ne!(result.mmr_root, [0u8; 32], "MMR root should be non-zero after evaporation");
                assert_eq!(executor.mmr_size(), 1, "Exactly one nullifier in MMR");
                assert_eq!(executor.mmr_root(), result.mmr_root, "Trait accessor matches result");
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
        db.put_account(Account { address: owner, balance: 1_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0 });

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
                assert_ne!(result.mmr_root, prev_root, "MMR root should change on evaporation");
            } else {
                assert_eq!(result.mmr_root, prev_root, "MMR root should not change without evaporation");
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
            payload: MessagePayload::Transfer { from: from_20, amount: 500 },
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
            payload: MessagePayload::Transfer { from: from_20, amount: 500 },
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
        });

        let mut target = [0u8; 20];
        target.copy_from_slice(&oid[..20]);

        let msg = CrossShardMessage {
            id: 0,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            target_object: target,
            payload: MessagePayload::Eviction { reason: "low energy".into() },
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
            1, 1,
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

        let rec = db.get_delegation(&addr(1), 7).expect("delegation must exist");
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
            1, 1,
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
            1, 1,
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
            1, 1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1), validator_id: 7, amount: 1_000, nonce: 0,
                signature: None, public_key: None,
            })],
        );
        executor.execute_block(&mut db, &block1).unwrap();
        let block2 = make_block(
            2, 2,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1), validator_id: 7, amount: 2_500, nonce: 1,
                signature: None, public_key: None,
            })],
        );
        executor.execute_block(&mut db, &block2).unwrap();

        let rec = db.get_delegation(&addr(1), 7).unwrap();
        assert_eq!(rec.amount, 3_500, "delegations to same validator should be additive");
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
            1, 1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1), validator_id: 7, amount: 1_000, nonce: 0,
                signature: None, public_key: None,
            })],
        );
        executor.execute_block(&mut db, &b1).unwrap();
        let pre_balance = db.get_account(&addr(1)).unwrap().balance;

        let b2 = make_block(
            2, 5,
            vec![Transaction::Undelegate(UndelegateTx {
                delegator: addr(1), validator_id: 7, amount: 600, nonce: 1,
                signature: None, public_key: None,
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
            1, 1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1), validator_id: 7, amount: 1_000, nonce: 0,
                signature: None, public_key: None,
            })],
        );
        executor.execute_block(&mut db, &b1).unwrap();

        let b2 = make_block(
            2, 2,
            vec![Transaction::Undelegate(UndelegateTx {
                delegator: addr(1), validator_id: 7, amount: 5_000, nonce: 1,
                signature: None, public_key: None,
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
        executor.execute_block(&mut db, &make_block(
            1, 1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1), validator_id: 7, amount: 2_000, nonce: 0,
                signature: None, public_key: None,
            })],
        )).unwrap();
        executor.execute_block(&mut db, &make_block(
            2, 5,
            vec![Transaction::Undelegate(UndelegateTx {
                delegator: addr(1), validator_id: 7, amount: 1_500, nonce: 1,
                signature: None, public_key: None,
            })],
        )).unwrap();
        let pre_claim_balance = db.get_account(&addr(1)).unwrap().balance;

        // Claim before unbonding period elapses → must fail.
        let early = executor.execute_block(&mut db, &make_block(
            3, 50,
            vec![Transaction::ClaimDelegation(ClaimDelegationTx {
                delegator: addr(1), validator_id: 7, nonce: 2,
                signature: None, public_key: None,
            })],
        )).unwrap();
        assert_eq!(early.txs_failed, 1, "claim before unbonding period must fail");
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, pre_claim_balance);

        // Claim after unbonding period (5 + 256 = 261) → succeeds.
        let ready = executor.execute_block(&mut db, &make_block(
            4, 261,
            vec![Transaction::ClaimDelegation(ClaimDelegationTx {
                delegator: addr(1), validator_id: 7, nonce: 2,
                signature: None, public_key: None,
            })],
        )).unwrap();
        assert_eq!(ready.txs_executed, 1);
        assert_eq!(
            db.get_account(&addr(1)).unwrap().balance,
            pre_claim_balance + 1_500,
            "claimed amount credited to balance"
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
        executor.execute_block(&mut db, &make_block(
            1, 1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1), validator_id: 7, amount: 2_000, nonce: 0,
                signature: None, public_key: None,
            })],
        )).unwrap();
        executor.execute_block(&mut db, &make_block(
            2, 1,
            vec![Transaction::Undelegate(UndelegateTx {
                delegator: addr(1), validator_id: 7, amount: 2_000, nonce: 1,
                signature: None, public_key: None,
            })],
        )).unwrap();
        executor.execute_block(&mut db, &make_block(
            3, 257,
            vec![Transaction::ClaimDelegation(ClaimDelegationTx {
                delegator: addr(1), validator_id: 7, nonce: 2,
                signature: None, public_key: None,
            })],
        )).unwrap();

        assert!(
            db.get_delegation(&addr(1), 7).is_none(),
            "fully unbonded delegation should be removed"
        );
    }

    #[test]
    fn test_claim_delegation_with_no_unbonding_amount_fails() {
        let mut db = InMemoryStateDB::new();
        seed_validator(&mut db, 7, 9, 100_000);
        fund_account(&mut db, 1, 5_000);

        let mut executor = SimpleExecutor::new_for_test(7);
        executor.execute_block(&mut db, &make_block(
            1, 1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(1), validator_id: 7, amount: 1_000, nonce: 0,
                signature: None, public_key: None,
            })],
        )).unwrap();

        let r = executor.execute_block(&mut db, &make_block(
            2, 500,
            vec![Transaction::ClaimDelegation(ClaimDelegationTx {
                delegator: addr(1), validator_id: 7, nonce: 1,
                signature: None, public_key: None,
            })],
        )).unwrap();
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
}
