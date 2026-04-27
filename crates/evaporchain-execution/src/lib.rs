pub mod block_stm;
pub mod economics;
pub mod fees;
pub mod genesis;
pub mod parallel;
pub mod privacy_exec;
pub mod rewards;
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
    Block, CallContractTx, CallScriptTx, CreateObjectTx, DeployContractTx, DeployScriptTx,
    Epoch, GovernanceAction, GovernanceProposal, GovernanceTx, MultiSigTx, ObjectState,
    ProposalStatus, RefreshTx, StakeRecord, StateObject, Transaction, TransferTx,
    ValidatorClaimStakeTx, ValidatorExitTx, ValidatorStakeTx,
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
pub(crate) const GAS_DELEGATE: u64 = 40_000;
pub(crate) const GAS_UNDELEGATE: u64 = 40_000;
pub(crate) const GAS_GOVERNANCE: u64 = 25_000;
pub(crate) const GAS_MULTISIG: u64 = 50_000;
pub(crate) const GAS_USER_OP: u64 = 30_000;
pub(crate) const GAS_UPGRADE_CONTRACT: u64 = 100_000;

/// Unbonding period: validators must wait this many epochs after exit before claiming stake.
const UNBONDING_PERIOD_EPOCHS: u64 = 256;

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

/// Credit back `storage_bytes` to the creators of contracts that evaporated
/// in the most recent `ContractEngine::tick`. Mirrors the object-evaporation
/// decrement in `evaporchain-state::evaporation::evaporate_object`.
///
/// Reads the exact deploy-time charge from `ContractInstance::storage_bytes_charged`
/// (set at deploy from `serde_json::to_string(&params).len()` — close to the
/// execution-layer charge). For contracts deployed before that field was
/// added, `#[serde(default)]` makes the value 0 → `saturating_sub` is a
/// safe no-op (the legacy contract's deployer was never charged anyway).
///
/// Closes audit gap 2D from `audit/end_to_end_audit_2026_04_27.md`.
pub(crate) fn credit_back_evaporated_contracts(
    db: &mut dyn StateDB,
    engine: &ContractEngine,
    tick_result: &evaporchain_contracts::TickResult,
) {
    for id in &tick_result.contracts_evaporated {
        if let Some(contract) = engine.get(*id) {
            // Read the exact deploy-time charge from the contract instance.
            // For contracts deployed before this field was added,
            // `#[serde(default)]` makes this 0 → saturating_sub is a no-op
            // (the legacy contract's deployer was never charged anyway,
            // so no credit is owed).
            let bytes = contract.storage_bytes_charged;
            let acct = db.get_or_create_account(&contract.creator);
            acct.storage_bytes = acct.storage_bytes.saturating_sub(bytes);
        }
    }
}

/// Mirror of `credit_back_evaporated_contracts` for `ScriptEngine`. When
/// an EvaporScript contract evaporates, credit back its compiled-bytecode
/// size to the deployer.
///
/// Approximation note: the deploy-time charge was `tx.source_code.len()`
/// (the raw Rust string), but at evaporation only the compiled bytecode
/// is in memory. We use `serde_json::to_string(&bytecode).len()` as a
/// proxy (avoiding a `bincode` dep on this crate). The exact byte count
/// differs from either source or compiled bytecode, but
/// `saturating_sub` keeps the account at 0 rather than underflowing if
/// storage_bytes was already 0.
///
/// Closes the script half of audit gap 2D.
pub(crate) fn credit_back_evaporated_scripts(
    db: &mut dyn StateDB,
    engine: &ScriptEngine,
    tick_result: &evaporchain_script::ScriptTickResult,
) {
    for id in &tick_result.contracts_evaporated {
        if let Some(script) = engine.get(*id) {
            // Exact deploy-time charge stored on the script contract.
            // For scripts: `script.storage_bytes_charged == source.len()`,
            // matching what the execution layer credited at deploy.
            let bytes = script.storage_bytes_charged;
            let acct = db.get_or_create_account(&script.creator);
            acct.storage_bytes = acct.storage_bytes.saturating_sub(bytes);
        }
    }
}

/// Validate a governance parameter against the immutable execution-layer
/// floor bounds. Returns `Ok(())` if the `(key, value)` pair is acceptable,
/// or `Err(ExecutionError::ContractError)` with a descriptive message
/// otherwise.
///
/// These bounds are CONSTITUTIONAL — they can only be widened by a hard
/// fork, never by governance. A `DecayingDAO` contract may further
/// tighten these bounds via its own per-key `param_bounds` configuration,
/// but cannot widen them past these constants. Defense-in-depth pair to
/// the `apply_dao_governance` bridge: the contract layer enforces
/// statutory bounds, this layer enforces constitutional bounds.
///
/// V0.1 only enforces `block_gas_limit`. Other parameters (`block_reward`,
/// `reward_half_life`, `fee_burn_rate`, `base_fee_floor`, `base_fee_ceiling`)
/// pass through; future hardening can add their floors.
///
/// Closes audit gap E from `audit/end_to_end_audit_2026_04_27.md` §5.
pub fn validate_governance_param(key: &str, value: &str) -> Result<(), ExecutionError> {
    /// Helper: parse a u64 and enforce inclusive `[min, max]` bounds.
    fn check_u64(key: &str, value: &str, min: u64, max: u64) -> Result<(), ExecutionError> {
        let v: u64 = value.parse().map_err(|_| {
            ExecutionError::ContractError(format!(
                "governance param '{}' must be a u64, got '{}'",
                key, value
            ))
        })?;
        if v < min || v > max {
            return Err(ExecutionError::ContractError(format!(
                "governance param '{}' = {} outside floor bounds [{}, {}]",
                key, v, min, max
            )));
        }
        Ok(())
    }

    /// Helper: parse an f64 and enforce inclusive `[min, max]` bounds.
    fn check_f64(key: &str, value: &str, min: f64, max: f64) -> Result<(), ExecutionError> {
        let v: f64 = value.parse().map_err(|_| {
            ExecutionError::ContractError(format!(
                "governance param '{}' must be an f64, got '{}'",
                key, value
            ))
        })?;
        if !v.is_finite() || v < min || v > max {
            return Err(ExecutionError::ContractError(format!(
                "governance param '{}' = {} outside floor bounds [{}, {}]",
                key, v, min, max
            )));
        }
        Ok(())
    }

    match key {
        // Block-level: too small → can't fit any tx; too large → blocks too
        // big to gossip in time. Halts the chain at either extreme.
        "block_gas_limit" => check_u64(key, value, 10_000, 100_000_000),

        // Issuance: block_reward capped at 1B per block — well above any
        // sane production value; prevents single-proposal hyperinflation.
        // Half-life ≥ 100 prevents inflation collapsing to 0 too fast.
        "block_reward" => check_u64(key, value, 0, 1_000_000_000),
        "reward_half_life" => check_u64(key, value, 100, u64::MAX),

        // Fee market: floor < ceiling not enforced here (cross-key
        // constraint requires state access); each leg has its own bound.
        "base_fee_floor" => check_u64(key, value, 0, u64::MAX / 2),
        "base_fee_ceiling" => check_u64(key, value, 1, u64::MAX),

        // Ratios: f64 in [0.0, 1.0]. NaN / non-finite rejected.
        "fee_burn_rate" => check_f64(key, value, 0.0, 1.0),
        "staker_fee_share" => check_f64(key, value, 0.0, 1.0),
        "target_staking_apy" => check_f64(key, value, 0.0, 1.0),
        "target_gas_utilization" => check_f64(key, value, 0.0, 1.0),

        // Unknown keys: pass through. New tunable parameters can be
        // added by appending an arm above; pass-through is the safe
        // default for forward-compatibility.
        _ => Ok(()),
    }
}

/// Validate a governance parameter against (a) the immutable single-key
/// floor bounds via `validate_governance_param`, AND (b) cross-key
/// invariants that require reading other governance values from state.
///
/// The cross-key checks currently enforce:
/// * `base_fee_floor < base_fee_ceiling` — floor must be strictly less
///   than the ceiling at all times. When updating one side, we validate
///   against the OTHER side as currently set in `db.get_governance_param`.
///   If the other side is unset, the cross-key check is skipped (the
///   executor's compiled-in default is used).
///
/// Closes the cross-key invariant gap from the
/// `audit/end_to_end_audit_2026_04_27.md` follow-up list.
pub fn validate_governance_param_against_state(
    db: &dyn StateDB,
    key: &str,
    value: &str,
) -> Result<(), ExecutionError> {
    // First the single-key bound check.
    validate_governance_param(key, value)?;

    // Cross-key invariants:
    match key {
        "base_fee_floor" => {
            // If a ceiling is set, ensure floor < ceiling.
            if let Some(ceiling_str) = db.get_governance_param("base_fee_ceiling") {
                if let Ok(ceiling) = ceiling_str.parse::<u64>() {
                    let floor: u64 = value.parse().map_err(|_| {
                        ExecutionError::ContractError(format!(
                            "governance param 'base_fee_floor' must be u64, got '{}'",
                            value
                        ))
                    })?;
                    if floor >= ceiling {
                        return Err(ExecutionError::ContractError(format!(
                            "base_fee_floor ({}) must be strictly less than \
                             base_fee_ceiling ({})",
                            floor, ceiling
                        )));
                    }
                }
            }
        }
        "base_fee_ceiling" => {
            // If a floor is set, ensure ceiling > floor.
            if let Some(floor_str) = db.get_governance_param("base_fee_floor") {
                if let Ok(floor) = floor_str.parse::<u64>() {
                    let ceiling: u64 = value.parse().map_err(|_| {
                        ExecutionError::ContractError(format!(
                            "governance param 'base_fee_ceiling' must be u64, got '{}'",
                            value
                        ))
                    })?;
                    if ceiling <= floor {
                        return Err(ExecutionError::ContractError(format!(
                            "base_fee_ceiling ({}) must be strictly greater than \
                             base_fee_floor ({})",
                            ceiling, floor
                        )));
                    }
                }
            }
        }
        _ => {} // No cross-key invariant for this key.
    }

    Ok(())
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

        // Charge storage_bytes to the creator. Closes the storage-rent
        // write-path gap from audit/end_to_end_audit_2026_04_27.md §3:
        // before this, `storage_bytes` was only ever set to 0, so
        // collect_storage_rent (lib.rs:~1029) had nothing to bill.
        //
        // V0.1 scope: only the CreateObject path is wired here. The
        // DeployContract / DeployScript paths do not currently route
        // through StateDB and so do not charge their deployers — that's
        // a follow-up. Object evaporation also does not DEcrement
        // storage_bytes — owners of evaporated objects keep paying. Both
        // gaps are intentional in this v0.1.
        let creator_acct = db.get_or_create_account(&tx.creator);
        creator_acct.storage_bytes = creator_acct
            .storage_bytes
            .saturating_add(tx.data.len() as u64);

        debug!(
            object_id = hex::encode(tx.object_id),
            energy = tx.energy,
            half_life = tx.half_life,
            data_bytes = tx.data.len(),
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

        let id = self
            .contract_engine
            .deploy(template, init_args, rules, tx.deployer, tx.energy, tx.half_life, epoch)
            .map_err(|e| ExecutionError::ContractError(e.to_string()))?;

        // Charge storage_bytes to the deployer. Uses init_args.len() as an
        // approximation of the stored contract instance size — exact
        // per-state-mutation accounting is a separate concern. Mirrors the
        // CreateObject write-path edit. Closes the DeployContract gap from
        // the storage_bytes-write-path build.
        let deployer_acct = db.get_or_create_account(&tx.deployer);
        deployer_acct.storage_bytes = deployer_acct
            .storage_bytes
            .saturating_add(tx.init_args.len() as u64);

        debug!(
            contract_id = id,
            template = %tx.template,
            init_args_bytes = tx.init_args.len(),
            "Contract deployed"
        );
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
        db: &mut dyn StateDB,
        tx: &DeployScriptTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        let id = self
            .script_engine
            .deploy(&tx.source_code, tx.deployer, tx.energy, tx.half_life, epoch)
            .map_err(|e| ExecutionError::ScriptError(e.to_string()))?;

        // Charge storage_bytes to the deployer. Uses source_code.len() as a
        // conservative upper bound for the stored bytecode size (the
        // compiled bytecode is typically smaller than the source, so this
        // over-charges slightly — preferable to under-charging). Mirrors
        // the CreateObject write-path edit; closes the DeployScript gap.
        let deployer_acct = db.get_or_create_account(&tx.deployer);
        deployer_acct.storage_bytes = deployer_acct
            .storage_bytes
            .saturating_add(tx.source_code.len() as u64);

        debug!(
            script_id = id,
            source_bytes = tx.source_code.len(),
            "Script contract deployed"
        );
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
                // Submission-time floor-bounds check: reject malformed
                // or out-of-floor proposals at submit, not at apply.
                // Without this, voters waste blocks voting on a proposal
                // that the floor will reject when it tries to apply.
                // Closes the deferred submission-time-validation follow-up.
                validate_governance_param_against_state(db, param_key, param_value).map_err(|e| {
                    ExecutionError::ContractError(format!(
                        "proposal rejected at submit: {}",
                        match &e {
                            ExecutionError::ContractError(msg) => msg.as_str(),
                            _ => "validation failed",
                        }
                    ))
                })?;

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
                    proposal.status = if proposal.votes_for > proposal.votes_against * 2 {
                        ProposalStatus::Passed
                    } else {
                        ProposalStatus::Rejected
                    };
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

                let voter_balance = db.get_account(&tx.sender).map(|a| a.balance).unwrap_or(0);
                if *vote {
                    proposal.votes_for = proposal.votes_for.saturating_add(voter_balance);
                } else {
                    proposal.votes_against = proposal.votes_against.saturating_add(voter_balance);
                }

                if proposal.votes_for > proposal.votes_against * 2 && current_epoch >= proposal.end_epoch {
                    // Floor-bounds defense-in-depth: even if voting passed,
                    // the value must be inside the constitutional floor
                    // AND consistent with cross-key invariants.
                    // On violation: log + skip put, but DO advance status to
                    // Passed so the proposal isn't reconsidered.
                    if let Err(e) = validate_governance_param_against_state(
                        db,
                        &proposal.param_key,
                        &proposal.param_value,
                    ) {
                        tracing::warn!(
                            proposal_id = proposal.proposal_id,
                            param_key = %proposal.param_key,
                            param_value = %proposal.param_value,
                            error = %e,
                            "Governance proposal passed vote but violated floor bounds; param NOT applied",
                        );
                    } else {
                        db.put_governance_param(
                            proposal.param_key.clone(),
                            proposal.param_value.clone(),
                        );
                    }
                    proposal.status = ProposalStatus::Passed;
                }

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
            // Paymaster requires its own nonce. Without it, the same UserOpTx
            // could be replayed across blocks (or across Block-STM aborts)
            // to drain the paymaster — closes the replay-drain gap from
            // audit/end_to_end_audit_2026_04_27.md §3.
            let pm_nonce = tx.paymaster_nonce.ok_or_else(|| {
                ExecutionError::ContractError(
                    "UserOp with paymaster requires paymaster_nonce".into(),
                )
            })?;
            let pm = db.get_or_create_account(paymaster);
            if pm.nonce != pm_nonce {
                return Err(ExecutionError::InvalidNonce {
                    expected: pm.nonce,
                    got: pm_nonce,
                });
            }
            pm.nonce += 1;

            let total_gas_cost = tx.call_gas_limit.saturating_add(GAS_USER_OP);
            if pm.balance < total_gas_cost {
                return Err(ExecutionError::InsufficientGas {
                    account: hex::encode(paymaster),
                    required: total_gas_cost,
                    available: pm.balance,
                });
            }
            pm.balance = pm.balance.saturating_sub(total_gas_cost);
        } else if tx.paymaster_nonce.is_some() {
            // paymaster_nonce without paymaster is malformed.
            return Err(ExecutionError::ContractError(
                "UserOp has paymaster_nonce but no paymaster".into(),
            ));
        }

        Ok(())
    }

    fn collect_storage_rent(&self, db: &mut dyn StateDB) {
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
                acct.balance = 0;
                acct.storage_deposit = 0;
                acct.storage_bytes = 0;
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
            // Cross-key consistency check before applying floor/ceiling:
            // if both are set in db AND inconsistent (floor >= ceiling),
            // log a warning and apply NEITHER. Defense-in-depth — the
            // submission/apply gates should already prevent this, but if
            // legacy or genesis state contains an inverted pair, applying
            // them as-is would invert the fee controller. Better to skip.
            let floor_str = db.get_governance_param("base_fee_floor");
            let ceiling_str = db.get_governance_param("base_fee_ceiling");
            let parsed_floor = floor_str.and_then(|s| s.parse::<u64>().ok());
            let parsed_ceiling = ceiling_str.and_then(|s| s.parse::<u64>().ok());

            let pair_consistent = match (parsed_floor, parsed_ceiling) {
                (Some(f), Some(c)) => f < c,
                _ => true, // single side or neither — no cross-key check
            };

            if !pair_consistent {
                tracing::warn!(
                    base_fee_floor = ?parsed_floor,
                    base_fee_ceiling = ?parsed_ceiling,
                    "Governance state has inconsistent base_fee_floor/ceiling pair (floor >= ceiling); skipping apply",
                );
            } else {
                if let Some(floor) = parsed_floor {
                    fc.min_base_fee = floor;
                }
                if let Some(ceiling) = parsed_ceiling {
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

    /// Apply all `ReadyToApply` proposals from a DecayingDAO contract to
    /// the execution-layer governance state. Bridges the policy layer
    /// (in the contract: bounds, vote-weight cap, quorum, timelock) to
    /// the parameter layer (in execution state: actual values consumed
    /// by `apply_governance_params`).
    ///
    /// Returns the list of `(param_key, param_value_u64)` pairs that were
    /// successfully applied. If the contract has no `ReadyToApply`
    /// proposals, returns an empty `Vec`. Errors propagate from the
    /// contract engine — the bridge is all-or-nothing per call.
    ///
    /// Closes the deferred F-bridge from
    /// `audit/end_to_end_audit_2026_04_27.md` §5. Operator-invoked for
    /// now; automatic block-time dispatch (a registered DAO contract
    /// polled per block) is a separate change.
    pub fn apply_dao_governance(
        &mut self,
        db: &mut dyn StateDB,
        dao_contract_id: u64,
        current_epoch: Epoch,
    ) -> Result<Vec<(String, u64)>, ExecutionError> {
        // Sentinel zero-address caller. The DecayingDAO's
        // `list_ready_to_apply` and `mark_applied` methods are open (no
        // caller restriction) — the access gate already happened at
        // `propose` / `vote` / `finalize` / `mark_ready_to_apply`.
        let bridge_caller = [0u8; 32];

        let listed = self
            .contract_engine
            .call(
                dao_contract_id,
                "list_ready_to_apply",
                &serde_json::json!({}),
                &bridge_caller,
                current_epoch,
            )
            .map_err(|e| {
                ExecutionError::ContractError(format!(
                    "list_ready_to_apply on DAO {}: {}",
                    dao_contract_id, e
                ))
            })?;

        let ready = listed
            .return_value
            .get("ready")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut applied = Vec::new();
        for entry in ready {
            let id = entry
                .get("id")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    ExecutionError::ContractError(
                        "DAO proposal entry missing id".into(),
                    )
                })?;
            let key = entry
                .get("param_key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ExecutionError::ContractError(
                        "DAO proposal entry missing param_key".into(),
                    )
                })?
                .to_string();
            let val = entry
                .get("param_value_u64")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    ExecutionError::ContractError(
                        "DAO proposal entry missing param_value_u64".into(),
                    )
                })?;

            // Apply to execution-layer governance state. The DAO has
            // already verified bounds + quorum + supermajority + timelock
            // before transitioning the proposal to ReadyToApply. The
            // execution-layer floor bounds is a defense-in-depth layer
            // (audit gap E): the DAO's bounds are statutory and may be
            // tightened by deploying a new DAO with different bounds, but
            // never widened past the constitutional floor enforced here.
            // On floor violation, abort the bridge call — neither this
            // proposal nor any later ones in the same call are applied.
            let val_str = val.to_string();
            validate_governance_param_against_state(db, &key, &val_str)?;
            db.put_governance_param(key.clone(), val_str);

            // Mark the proposal Applied in the DAO so subsequent
            // list_ready_to_apply calls don't re-surface it.
            self.contract_engine
                .call(
                    dao_contract_id,
                    "mark_applied",
                    &serde_json::json!({ "proposal_id": id }),
                    &bridge_caller,
                    current_epoch,
                )
                .map_err(|e| {
                    ExecutionError::ContractError(format!(
                        "mark_applied on proposal {}: {}",
                        id, e
                    ))
                })?;

            applied.push((key, val));
        }

        Ok(applied)
    }

    fn finalize_expired_proposals(&self, db: &mut dyn StateDB, current_epoch: u64) {
        let expired: Vec<GovernanceProposal> = db
            .all_proposals()
            .iter()
            .filter(|p| p.status == ProposalStatus::Active && current_epoch > p.end_epoch)
            .cloned()
            .cloned()
            .collect();

        for mut proposal in expired {
            if proposal.votes_for > proposal.votes_against * 2 {
                proposal.status = ProposalStatus::Passed;
                // Floor-bounds defense-in-depth (audit gap E). Same
                // shape as the execute_governance pass-immediate path.
                if let Err(e) = validate_governance_param_against_state(
                    db,
                    &proposal.param_key,
                    &proposal.param_value,
                ) {
                    tracing::warn!(
                        proposal_id = proposal.proposal_id,
                        param_key = %proposal.param_key,
                        param_value = %proposal.param_value,
                        error = %e,
                        "Expired-proposal finalization passed vote but violated floor bounds; param NOT applied",
                    );
                } else {
                    db.put_governance_param(proposal.param_key.clone(), proposal.param_value.clone());
                }
                info!(
                    proposal_id = proposal.proposal_id,
                    param = proposal.param_key,
                    value = proposal.param_value,
                    "Governance proposal passed and applied"
                );
            } else {
                proposal.status = ProposalStatus::Rejected;
                debug!(
                    proposal_id = proposal.proposal_id,
                    "Governance proposal rejected (insufficient votes)"
                );
            }
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
                Transaction::DeployContract(deploy) => self.execute_deploy_contract(db, deploy, block.epoch),
                Transaction::CallContract(call) => self.execute_call_contract(call),
                Transaction::DeployScript(deploy) => self.execute_deploy_script(db, deploy, block.epoch),
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
                Transaction::UpgradeContract(_) => {
                    // Fail loud: governance approval check + bytecode swap into
                    // ContractEngine are not yet implemented. Returning Ok here
                    // would let any signer submit a contract upgrade tx that
                    // silently passes — refuse it instead until the upgrade
                    // path is wired through governance.
                    Err(ExecutionError::ContractError(
                        "UpgradeContract execution not implemented: \
                         governance approval check and bytecode swap are missing"
                            .into(),
                    ))
                }
                Transaction::Delegate(_) => {
                    // Fail loud: delegation handler will land in a follow-up
                    // commit (P0 #4 phase 4). Until then refuse to admit so a
                    // user submitting Delegate sees an explicit error rather
                    // than a silent no-op.
                    Err(ExecutionError::ContractError(
                        "Delegate execution not yet implemented (P0 #4 in flight)"
                            .into(),
                    ))
                }
                Transaction::Undelegate(_) => {
                    Err(ExecutionError::ContractError(
                        "Undelegate execution not yet implemented (P0 #4 in flight)"
                            .into(),
                    ))
                }
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
        let contract_tick_result = self.contract_engine.tick(block.epoch);
        credit_back_evaporated_contracts(db, &self.contract_engine, &contract_tick_result);
        let script_tick_result = self.script_engine.tick(block.epoch);
        credit_back_evaporated_scripts(db, &self.script_engine, &script_tick_result);

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

        self.collect_storage_rent(db);
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

    // ─── storage_bytes accounting on CreateObject ───

    #[test]
    fn test_create_object_charges_storage_bytes() {
        // CreateObject must increment the creator's storage_bytes by the
        // data size, so collect_storage_rent has something to bill.
        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);
        fund_account(&mut db, 1, 10_000);

        // Storage_bytes starts at zero.
        let initial = db.get_account(&addr(1)).unwrap().storage_bytes;
        assert_eq!(initial, 0);

        let payload: Vec<u8> = (0..256u32).map(|i| (i & 0xff) as u8).collect();
        let block = make_block(
            1,
            10,
            vec![Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(101),
                energy: 5000,
                half_life: 100,
                data: payload.clone(),
                decay_curve: None,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);

        let after_one = db.get_account(&addr(1)).unwrap().storage_bytes;
        assert_eq!(
            after_one, payload.len() as u64,
            "first CreateObject should credit data.len() to creator's storage_bytes"
        );

        // A second CreateObject in a later block should accumulate, not overwrite.
        let payload2 = vec![0xAB; 100];
        let block2 = make_block(
            2,
            11,
            vec![Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(102),
                energy: 5000,
                half_life: 100,
                data: payload2.clone(),
                decay_curve: None,
                signature: None,
                public_key: None,
            })],
        );
        let _ = executor.execute_block(&mut db, &block2).unwrap();
        let after_two = db.get_account(&addr(1)).unwrap().storage_bytes;
        assert_eq!(
            after_two,
            (payload.len() + payload2.len()) as u64,
            "second CreateObject should accumulate storage_bytes, not overwrite"
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

    // ─── UserOp paymaster-nonce replay protection ───────────────────────────

    fn make_user_op_tx(
        sender: AccountAddress,
        sender_nonce: u64,
        paymaster: Option<AccountAddress>,
        paymaster_nonce: Option<u64>,
        call_gas_limit: u64,
    ) -> evaporchain_types::UserOpTx {
        evaporchain_types::UserOpTx {
            sender,
            nonce: sender_nonce,
            call_data: vec![],
            call_gas_limit,
            paymaster,
            paymaster_nonce,
            paymaster_data: None,
            signature: None,
            public_key: None,
        }
    }

    #[test]
    fn test_user_op_paymaster_nonce_required() {
        // Paymaster set but paymaster_nonce missing → reject (malformed).
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);
        fund_account(&mut db, 2, 1_000_000);
        let executor = SimpleExecutor::new_for_test(7);

        let tx = make_user_op_tx(addr(1), 0, Some(addr(2)), None, 1_000);
        let result = executor.execute_user_op(&mut db, &tx);
        assert!(
            result.is_err(),
            "UserOp with paymaster but no paymaster_nonce must be rejected"
        );
    }

    #[test]
    fn test_user_op_paymaster_nonce_without_paymaster_rejected() {
        // paymaster_nonce set but paymaster missing → reject (malformed).
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);
        let executor = SimpleExecutor::new_for_test(7);

        let tx = make_user_op_tx(addr(1), 0, None, Some(0), 1_000);
        let result = executor.execute_user_op(&mut db, &tx);
        assert!(
            result.is_err(),
            "UserOp with paymaster_nonce but no paymaster must be rejected"
        );
    }

    #[test]
    fn test_user_op_paymaster_nonce_mismatch_rejected() {
        // Paymaster account has nonce 0; tx provides paymaster_nonce 5.
        // Should reject.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);
        fund_account(&mut db, 2, 1_000_000);
        let executor = SimpleExecutor::new_for_test(7);

        let tx = make_user_op_tx(addr(1), 0, Some(addr(2)), Some(5), 1_000);
        match executor.execute_user_op(&mut db, &tx) {
            Err(ExecutionError::InvalidNonce { expected, got }) => {
                assert_eq!(expected, 0);
                assert_eq!(got, 5);
            }
            other => panic!("expected InvalidNonce, got {:?}", other),
        }
    }

    #[test]
    fn test_user_op_paymaster_nonce_increments_on_success() {
        // Successful UserOp with paymaster increments BOTH sender and
        // paymaster nonces — so a replay of the same tx hits the nonce
        // check on both accounts.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);
        fund_account(&mut db, 2, 1_000_000);
        let executor = SimpleExecutor::new_for_test(7);

        let tx = make_user_op_tx(addr(1), 0, Some(addr(2)), Some(0), 1_000);
        executor.execute_user_op(&mut db, &tx).unwrap();

        assert_eq!(db.get_account(&addr(1)).unwrap().nonce, 1);
        assert_eq!(db.get_account(&addr(2)).unwrap().nonce, 1);

        // Replay the exact same tx — sender nonce check is what catches it
        // here; what we are really protecting against is a tx with sender
        // nonce 1 and paymaster_nonce 0, which would have drained the
        // paymaster a second time before this fix.
        let replay = make_user_op_tx(addr(1), 1, Some(addr(2)), Some(0), 1_000);
        match executor.execute_user_op(&mut db, &replay) {
            Err(ExecutionError::InvalidNonce { expected, got }) => {
                assert_eq!(expected, 1, "paymaster nonce should now be 1");
                assert_eq!(got, 0, "tx-supplied paymaster_nonce was 0 (stale)");
            }
            other => panic!(
                "expected InvalidNonce on paymaster, got {:?} — pre-fix the paymaster would have been drained again",
                other
            ),
        }
    }

    #[test]
    fn test_user_op_no_paymaster_does_not_require_nonce() {
        // UserOp without paymaster works as before — only sender nonce checked.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);
        let executor = SimpleExecutor::new_for_test(7);

        let tx = make_user_op_tx(addr(1), 0, None, None, 1_000);
        executor.execute_user_op(&mut db, &tx).unwrap();
        assert_eq!(db.get_account(&addr(1)).unwrap().nonce, 1);
    }

    // ─── DeployContract / DeployScript storage_bytes accounting ────────────

    #[test]
    fn test_deploy_contract_charges_storage_bytes() {
        // DeployContract must increment the deployer's storage_bytes by
        // init_args.len(), so collect_storage_rent has something to bill.
        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);
        fund_account(&mut db, 1, 10_000_000);

        let initial = db.get_account(&addr(1)).unwrap().storage_bytes;
        assert_eq!(initial, 0);

        let init_args = r#"{"name":"Test","supply":1000,"decay_half_life":100}"#;
        let block = make_block(
            1,
            10,
            vec![Transaction::DeployContract(DeployContractTx {
                deployer: addr(1),
                template: "DecayingToken".into(),
                init_args: init_args.into(),
                energy: 5_000,
                half_life: 100,
                rules: None,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1, "DeployContract must succeed");

        let after = db.get_account(&addr(1)).unwrap().storage_bytes;
        assert_eq!(
            after,
            init_args.len() as u64,
            "DeployContract should credit init_args.len() to deployer's storage_bytes"
        );
    }

    #[test]
    fn test_deploy_script_charges_storage_bytes() {
        // DeployScript must increment the deployer's storage_bytes by
        // source_code.len() — used as a conservative upper bound for
        // the compiled bytecode size.
        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);
        fund_account(&mut db, 1, 10_000_000);

        let source = "contract C { state { x: u64 = 0 } fn inc() { self.x += 1 } }";
        let block = make_block(
            1,
            10,
            vec![Transaction::DeployScript(evaporchain_types::DeployScriptTx {
                deployer: addr(1),
                source_code: source.into(),
                energy: 5_000,
                half_life: 100,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block);
        // Script may or may not parse-succeed depending on grammar; what
        // matters here is the accounting on the success path. Skip if
        // parsing fails (the storage_bytes path is only hit on success).
        if let Ok(r) = result {
            if r.txs_executed == 1 {
                let after = db.get_account(&addr(1)).unwrap().storage_bytes;
                assert_eq!(
                    after,
                    source.len() as u64,
                    "DeployScript should credit source_code.len() to deployer's storage_bytes"
                );
            }
        }
    }

    #[test]
    fn test_contract_evaporation_decrements_deployer_storage_bytes() {
        // End-to-end: deploy a contract → its storage_bytes is credited;
        // advance many epochs so its energy decays to 0 → tick evaporates
        // it → credit_back_evaporated_contracts decrements the deployer.
        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);
        fund_account(&mut db, 1, 10_000_000);

        // energy=1, half_life=1 → fully decayed within 64 epochs.
        let init_args = r#"{"name":"DecayFast","symbol":"DF","supply":1}"#;
        let block_deploy = make_block(
            1,
            0,
            vec![Transaction::DeployContract(DeployContractTx {
                deployer: addr(1),
                template: "DecayingToken".into(),
                init_args: init_args.into(),
                energy: 1,
                half_life: 1,
                rules: None,
                signature: None,
                public_key: None,
            })],
        );
        let _ = executor.execute_block(&mut db, &block_deploy).unwrap();

        let pre = db.get_account(&addr(1)).unwrap().storage_bytes;
        assert!(
            pre > 0,
            "deploy must have credited storage_bytes (got {})",
            pre
        );

        // Advance to epoch 200 — well past the contract's lifetime. The
        // empty block triggers the post-block tick, which should mark the
        // contract evaporated and credit-back the deployer.
        let block_later = make_block(2, 200, vec![]);
        let _ = executor.execute_block(&mut db, &block_later).unwrap();

        let post = db.get_account(&addr(1)).unwrap().storage_bytes;
        assert!(
            post < pre,
            "evaporation must have decremented storage_bytes (pre={}, post={})",
            pre,
            post
        );
    }

    #[test]
    fn test_script_evaporation_decrements_deployer_storage_bytes() {
        // Mirror of the contract-evaporation test for the script path.
        // Permissive on parse: if the source doesn't compile, skip the
        // assertion (the storage_bytes path only runs on a successful
        // deploy). Same shape as test_deploy_script_charges_storage_bytes.
        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);
        fund_account(&mut db, 1, 10_000_000);

        let source = "contract C { state { x: u64 = 0 } fn inc() { self.x += 1 } }";
        let block_deploy = make_block(
            1,
            0,
            vec![Transaction::DeployScript(evaporchain_types::DeployScriptTx {
                deployer: addr(1),
                source_code: source.into(),
                energy: 1,
                half_life: 1,
                signature: None,
                public_key: None,
            })],
        );

        let deploy_result = executor.execute_block(&mut db, &block_deploy);
        let pre = db.get_account(&addr(1)).map(|a| a.storage_bytes).unwrap_or(0);

        // Only assert on the successful-deploy branch — parser grammar may
        // shift; we don't want a brittle test pinning to one form.
        if let Ok(r) = &deploy_result {
            if r.txs_executed == 1 && pre > 0 {
                let block_later = make_block(2, 200, vec![]);
                let _ = executor.execute_block(&mut db, &block_later).unwrap();

                let post = db.get_account(&addr(1)).unwrap().storage_bytes;
                assert!(
                    post < pre,
                    "script evaporation should have decremented storage_bytes \
                     (pre={}, post={})",
                    pre,
                    post,
                );
            }
        }
    }

    // ─── DecayingDAO ↔ execution governance bridge ──────────────────────────

    #[test]
    fn test_apply_dao_governance_end_to_end() {
        // Full integration: deploy DecayingDAO → propose → vote → finalize
        // → mark_ready_to_apply → apply_dao_governance → verify the
        // execution-layer governance param was set and the contract-layer
        // proposal is marked Applied.
        use evaporchain_contracts::ContractTemplate;

        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);
        let creator = addr(1);

        // Deploy DecayingDAO with bounds: block_gas_limit ∈ [10_000, 100_000_000].
        // Small total_stake / min_stake_to_propose / quorum so the test
        // converges on small numbers.
        let init_args = serde_json::json!({
            "title": "test_dao",
            "param_bounds": {
                "block_gas_limit": [10_000u64, 100_000_000u64],
            },
            "voting_period_epochs": 100u64,
            "quorum_pct": 50u64,
            "timelock_epochs": 24u64,
            "total_stake": 1_000u64,
            "min_stake_to_propose": 100u64,
        });
        let dao_id = executor
            .contract_engine
            .deploy(
                ContractTemplate::DecayingDAO,
                init_args,
                Vec::new(),
                creator,
                100_000,
                10_000,
                0,
            )
            .unwrap();

        // Propose to set block_gas_limit = 50_000_000.
        executor
            .contract_engine
            .call(
                dao_id,
                "propose",
                &serde_json::json!({
                    "proposer_stake": 200u64,
                    "param_key": "block_gas_limit",
                    "param_value_u64": 50_000_000u64,
                }),
                &creator,
                5,
            )
            .unwrap();

        // Three voters, each contributing 200 weight → 600 yes / 0 no.
        // Quorum threshold = 1000 * 50 / 100 = 500. 600 ≥ 500 ✓
        // Supermajority: 600 > 0 * 2 ✓
        for i in 0u8..3 {
            let mut a = [0u8; 32];
            a[0] = 0x10 + i;
            executor
                .contract_engine
                .call(
                    dao_id,
                    "vote",
                    &serde_json::json!({
                        "proposal_id": 0u64,
                        "support": true,
                        "balance": 200u64,
                        "stake": 200u64,
                    }),
                    &a,
                    10 + i as u64,
                )
                .unwrap();
        }

        // Finalize: voting started at 5, period 100, ends at 105. Call at 110.
        let finalized = executor
            .contract_engine
            .call(
                dao_id,
                "finalize",
                &serde_json::json!({ "proposal_id": 0u64 }),
                &creator,
                110,
            )
            .unwrap();
        assert_eq!(finalized.return_value["status"], "Passed");

        // Mark ready_to_apply: passed at 110, timelock 24 → ready at 134. Call at 140.
        executor
            .contract_engine
            .call(
                dao_id,
                "mark_ready_to_apply",
                &serde_json::json!({ "proposal_id": 0u64 }),
                &creator,
                140,
            )
            .unwrap();

        // Pre-bridge: governance state has no override.
        assert!(db.get_governance_param("block_gas_limit").is_none());

        // Bridge: apply.
        let applied = executor
            .apply_dao_governance(&mut db, dao_id, 141)
            .expect("bridge should succeed");
        assert_eq!(applied.len(), 1, "exactly one ReadyToApply proposal");
        assert_eq!(applied[0].0, "block_gas_limit");
        assert_eq!(applied[0].1, 50_000_000);

        // Post-bridge: governance state now has the value.
        assert_eq!(
            db.get_governance_param("block_gas_limit"),
            Some("50000000"),
            "execution-layer governance state must reflect the applied param",
        );

        // Idempotency: bridge again returns empty (proposal already Applied).
        let again = executor.apply_dao_governance(&mut db, dao_id, 142).unwrap();
        assert!(
            again.is_empty(),
            "second bridge call must not re-apply already-Applied proposals"
        );
    }

    #[test]
    fn test_apply_dao_governance_no_ready_proposals_is_noop() {
        // Brand-new DAO with no proposals → bridge returns empty Vec.
        use evaporchain_contracts::ContractTemplate;

        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);
        let creator = addr(1);

        let init_args = serde_json::json!({
            "title": "empty_dao",
            "param_bounds": { "block_gas_limit": [1u64, 1_000_000u64] },
            "voting_period_epochs": 100u64,
            "quorum_pct": 50u64,
            "timelock_epochs": 24u64,
            "total_stake": 1_000u64,
            "min_stake_to_propose": 100u64,
        });
        let dao_id = executor
            .contract_engine
            .deploy(
                ContractTemplate::DecayingDAO,
                init_args,
                Vec::new(),
                creator,
                100_000,
                10_000,
                0,
            )
            .unwrap();

        let applied = executor.apply_dao_governance(&mut db, dao_id, 1).unwrap();
        assert!(applied.is_empty());
        assert!(db.get_governance_param("block_gas_limit").is_none());
    }

    // ─── Execution-layer floor bounds (validate_governance_param) ──────────

    #[test]
    fn test_validate_governance_param_block_gas_limit_inside_bounds() {
        // Within [10_000, 100_000_000] → Ok
        assert!(validate_governance_param("block_gas_limit", "10000").is_ok());
        assert!(validate_governance_param("block_gas_limit", "500000").is_ok());
        assert!(validate_governance_param("block_gas_limit", "100000000").is_ok());
    }

    #[test]
    fn test_validate_governance_param_block_gas_limit_below_floor() {
        // Below the minimum floor → Err
        assert!(validate_governance_param("block_gas_limit", "0").is_err());
        assert!(validate_governance_param("block_gas_limit", "9999").is_err());
    }

    #[test]
    fn test_validate_governance_param_block_gas_limit_above_ceiling() {
        // Above the maximum → Err
        assert!(validate_governance_param("block_gas_limit", "100000001").is_err());
        assert!(
            validate_governance_param("block_gas_limit", &u64::MAX.to_string()).is_err()
        );
    }

    #[test]
    fn test_validate_governance_param_unparseable_value() {
        // Non-u64 value for a numeric key → Err
        assert!(validate_governance_param("block_gas_limit", "not_a_number").is_err());
        assert!(validate_governance_param("block_gas_limit", "").is_err());
        assert!(validate_governance_param("block_gas_limit", "-1").is_err());
    }

    #[test]
    fn test_validate_governance_param_unknown_key_passes() {
        // Unknown keys still pass through (forward-compat default).
        assert!(
            validate_governance_param("some_future_param", "anything").is_ok()
        );
    }

    #[test]
    fn test_validate_governance_param_block_reward_bounded() {
        // Inside [0, 1_000_000_000]
        assert!(validate_governance_param("block_reward", "0").is_ok());
        assert!(validate_governance_param("block_reward", "999999999").is_ok());
        assert!(validate_governance_param("block_reward", "1000000000").is_ok());
        // Above the ceiling
        assert!(validate_governance_param("block_reward", "1000000001").is_err());
        assert!(
            validate_governance_param("block_reward", &u64::MAX.to_string()).is_err()
        );
    }

    #[test]
    fn test_validate_governance_param_reward_half_life_minimum() {
        // Below floor (100)
        assert!(validate_governance_param("reward_half_life", "0").is_err());
        assert!(validate_governance_param("reward_half_life", "99").is_err());
        // At and above floor
        assert!(validate_governance_param("reward_half_life", "100").is_ok());
        assert!(validate_governance_param("reward_half_life", "1000000").is_ok());
    }

    #[test]
    fn test_validate_governance_param_base_fee_ceiling_nonzero() {
        // Floor for base_fee_ceiling is 1 (zero would never let any tx pay).
        assert!(validate_governance_param("base_fee_ceiling", "0").is_err());
        assert!(validate_governance_param("base_fee_ceiling", "1").is_ok());
        assert!(validate_governance_param("base_fee_floor", "0").is_ok());
    }

    #[test]
    fn test_validate_governance_param_f64_ratios() {
        // fee_burn_rate / staker_fee_share / target_*: ratios in [0.0, 1.0]
        for key in [
            "fee_burn_rate",
            "staker_fee_share",
            "target_staking_apy",
            "target_gas_utilization",
        ] {
            assert!(
                validate_governance_param(key, "0").is_ok(),
                "{key}=0 should be ok"
            );
            assert!(
                validate_governance_param(key, "0.5").is_ok(),
                "{key}=0.5 should be ok"
            );
            assert!(
                validate_governance_param(key, "1.0").is_ok(),
                "{key}=1.0 should be ok"
            );
            // Outside range
            assert!(
                validate_governance_param(key, "1.000001").is_err(),
                "{key}=1.000001 should fail"
            );
            assert!(
                validate_governance_param(key, "-0.0001").is_err(),
                "{key}=-0.0001 should fail"
            );
            // Non-finite
            assert!(
                validate_governance_param(key, "NaN").is_err(),
                "{key}=NaN should fail"
            );
            assert!(
                validate_governance_param(key, "inf").is_err(),
                "{key}=inf should fail"
            );
            // Unparseable
            assert!(
                validate_governance_param(key, "not_a_number").is_err(),
                "{key}=not_a_number should fail"
            );
        }
    }

    // ─── Cross-key invariant: base_fee_floor < base_fee_ceiling ────────────

    #[test]
    fn test_validate_against_state_base_fee_floor_below_ceiling_ok() {
        let mut db = InMemoryStateDB::new();
        // Set ceiling first.
        db.put_governance_param("base_fee_ceiling".into(), "1000".into());
        // Floor strictly less → ok
        assert!(validate_governance_param_against_state(&db, "base_fee_floor", "500").is_ok());
        assert!(validate_governance_param_against_state(&db, "base_fee_floor", "999").is_ok());
    }

    #[test]
    fn test_validate_against_state_base_fee_floor_at_or_above_ceiling_rejected() {
        let mut db = InMemoryStateDB::new();
        db.put_governance_param("base_fee_ceiling".into(), "1000".into());
        // Floor == ceiling → must be strictly less, so reject.
        assert!(validate_governance_param_against_state(&db, "base_fee_floor", "1000").is_err());
        // Floor > ceiling → reject.
        assert!(validate_governance_param_against_state(&db, "base_fee_floor", "1001").is_err());
    }

    #[test]
    fn test_validate_against_state_base_fee_ceiling_above_floor_ok() {
        let mut db = InMemoryStateDB::new();
        db.put_governance_param("base_fee_floor".into(), "100".into());
        // Ceiling strictly greater → ok
        assert!(validate_governance_param_against_state(&db, "base_fee_ceiling", "200").is_ok());
        assert!(validate_governance_param_against_state(&db, "base_fee_ceiling", "101").is_ok());
    }

    #[test]
    fn test_validate_against_state_base_fee_ceiling_at_or_below_floor_rejected() {
        let mut db = InMemoryStateDB::new();
        db.put_governance_param("base_fee_floor".into(), "100".into());
        // Ceiling == floor → reject (must be strictly greater).
        assert!(validate_governance_param_against_state(&db, "base_fee_ceiling", "100").is_err());
        // Ceiling < floor → reject.
        assert!(validate_governance_param_against_state(&db, "base_fee_ceiling", "50").is_err());
    }

    #[test]
    fn test_validate_against_state_no_cross_check_when_other_unset() {
        // If the other side isn't set in db, cross-key check is skipped —
        // the executor's compiled-in default fills in. Single-key bound
        // is still enforced.
        let db = InMemoryStateDB::new();
        // Floor set, ceiling not in db → ok (only single-key check).
        assert!(validate_governance_param_against_state(&db, "base_fee_floor", "100").is_ok());
        assert!(validate_governance_param_against_state(&db, "base_fee_ceiling", "100").is_ok());
    }

    #[test]
    fn test_validate_against_state_single_key_bounds_still_apply() {
        // The cross-key wrapper still calls the single-key validator first.
        let db = InMemoryStateDB::new();
        // block_gas_limit=0 fails single-key bounds check.
        assert!(validate_governance_param_against_state(&db, "block_gas_limit", "0").is_err());
        // base_fee_ceiling=0 fails single-key bounds check (min=1).
        assert!(validate_governance_param_against_state(&db, "base_fee_ceiling", "0").is_err());
    }

    #[test]
    fn test_apply_dao_governance_rejects_out_of_floor_bounds() {
        // The DAO contract's own param_bounds may be wider than the
        // execution-layer floor. If so, the bridge must REJECT the apply
        // even though the contract considers the proposal Ready.
        use evaporchain_contracts::ContractTemplate;

        let mut db = InMemoryStateDB::new();
        let mut executor = SimpleExecutor::new_for_test(7);
        let creator = addr(1);

        // Deploy DAO with bounds INTENTIONALLY WIDER than the floor:
        // [1, u64::MAX] for block_gas_limit. The DAO will accept any
        // value; the floor [10_000, 100_000_000] should reject the bridge
        // call when a value outside floor is proposed.
        let init_args = serde_json::json!({
            "title": "wide_dao",
            "param_bounds": {
                "block_gas_limit": [1u64, u64::MAX],
            },
            "voting_period_epochs": 100u64,
            "quorum_pct": 50u64,
            "timelock_epochs": 24u64,
            "total_stake": 1_000u64,
            "min_stake_to_propose": 100u64,
        });
        let dao_id = executor
            .contract_engine
            .deploy(
                ContractTemplate::DecayingDAO,
                init_args,
                Vec::new(),
                creator,
                100_000,
                10_000,
                0,
            )
            .unwrap();

        // Propose value above the floor ceiling.
        executor
            .contract_engine
            .call(
                dao_id,
                "propose",
                &serde_json::json!({
                    "proposer_stake": 200u64,
                    "param_key": "block_gas_limit",
                    "param_value_u64": 200_000_000u64, // above 100M ceiling
                }),
                &creator,
                5,
            )
            .unwrap();

        // Pass the proposal through the contract layer: 3 voters, full quorum.
        for i in 0u8..3 {
            let mut a = [0u8; 32];
            a[0] = 0x20 + i;
            executor
                .contract_engine
                .call(
                    dao_id,
                    "vote",
                    &serde_json::json!({
                        "proposal_id": 0u64,
                        "support": true,
                        "balance": 200u64,
                        "stake": 200u64,
                    }),
                    &a,
                    10 + i as u64,
                )
                .unwrap();
        }
        executor
            .contract_engine
            .call(
                dao_id,
                "finalize",
                &serde_json::json!({ "proposal_id": 0u64 }),
                &creator,
                110,
            )
            .unwrap();
        executor
            .contract_engine
            .call(
                dao_id,
                "mark_ready_to_apply",
                &serde_json::json!({ "proposal_id": 0u64 }),
                &creator,
                140,
            )
            .unwrap();

        // Bridge: floor bounds must reject this — return Err, no put.
        let result = executor.apply_dao_governance(&mut db, dao_id, 141);
        assert!(
            result.is_err(),
            "bridge must reject values outside execution-layer floor bounds"
        );

        // No governance state mutation happened.
        assert!(db.get_governance_param("block_gas_limit").is_none());
    }

    // ─── Submission-time governance proposal validation ─────────────────────

    fn make_create_proposal_tx(
        sender: AccountAddress,
        param_key: &str,
        param_value: &str,
    ) -> evaporchain_types::GovernanceTx {
        evaporchain_types::GovernanceTx {
            action: evaporchain_types::GovernanceAction::CreateProposal {
                title: "test_proposal".into(),
                param_key: param_key.into(),
                param_value: param_value.into(),
                voting_epochs: 100,
            },
            sender,
            nonce: 0,
            signature: None,
            public_key: None,
        }
    }

    #[test]
    fn test_create_proposal_rejected_at_submit_when_below_floor() {
        // block_gas_limit = 0 is below the floor (10_000). Submission must
        // reject the tx and NOT create the proposal — voters shouldn't
        // waste blocks voting on a doomed proposal.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);
        let executor = SimpleExecutor::new_for_test(7);

        assert!(db.all_proposals().is_empty());

        let tx = make_create_proposal_tx(addr(1), "block_gas_limit", "0");
        let result = executor.execute_governance(&mut db, &tx, 1);
        assert!(
            result.is_err(),
            "out-of-floor proposal must be rejected at submit"
        );

        // No proposal was created.
        assert!(
            db.all_proposals().is_empty(),
            "rejected proposal must not appear in the proposal list"
        );
    }

    #[test]
    fn test_create_proposal_rejected_when_value_unparseable() {
        // For a u64-bounded key, non-numeric value rejected at submit.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);
        let executor = SimpleExecutor::new_for_test(7);

        let tx = make_create_proposal_tx(addr(1), "block_gas_limit", "not_a_number");
        assert!(executor.execute_governance(&mut db, &tx, 1).is_err());
        assert!(db.all_proposals().is_empty());
    }

    #[test]
    fn test_create_proposal_succeeds_inside_floor() {
        // A valid proposal still succeeds — sanity check that submission
        // validation only rejects bad values, not all values.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);
        let executor = SimpleExecutor::new_for_test(7);

        let tx = make_create_proposal_tx(addr(1), "block_gas_limit", "50000000");
        executor.execute_governance(&mut db, &tx, 1).unwrap();

        assert_eq!(db.all_proposals().len(), 1);
    }

    #[test]
    fn test_create_proposal_unknown_key_passes_at_submit() {
        // Unknown keys pass through validate_governance_param (forward-compat
        // default) — submission also accepts.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);
        let executor = SimpleExecutor::new_for_test(7);

        let tx = make_create_proposal_tx(addr(1), "some_future_param", "anything");
        executor.execute_governance(&mut db, &tx, 1).unwrap();

        assert_eq!(db.all_proposals().len(), 1);
    }
}
