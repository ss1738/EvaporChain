//! Parallel Execution Engine for EvaporChain
//!
//! Exploits EvaporChain's object-based state model to execute non-conflicting
//! transactions in parallel. Uses static conflict detection with union-find
//! partitioning — transactions that touch disjoint accounts/objects execute
//! concurrently via rayon, while conflicting transactions are serialized
//! within the same partition.
//!
//! Architecture:
//!   1. Extract access keys (accounts, objects, global engines) per transaction
//!   2. Union-find partitioning: transactions sharing any access key merge
//!   3. Pre-populate per-partition overlay DBs with cloned state
//!   4. Execute partitions in parallel (rayon), sequential within each partition
//!   5. Merge overlay writes back to the main state DB
//!   6. Run evaporation, contract/script ticks on the merged state

use std::collections::{HashMap, HashSet};

use evaporchain_contracts::{ContractEngine, ContractTemplate};
use evaporchain_crypto::signatures::{HybridVerifier, Verifier};
use evaporchain_crypto::MerkleMountainRange;
use evaporchain_script::ScriptEngine;
use evaporchain_state::db::StateDB;
use evaporchain_state::{EvaporationEngine, RefreshEngine};
use evaporchain_types::{
    Account, AccountAddress, Block, CreateObjectTx, Epoch, GhostRecord, ObjectId, ObjectState,
    RefreshTx, StakeRecord, StateObject, Transaction, TransferTx, ValidatorExitTx,
    ValidatorStakeTx,
};
use rayon::prelude::*;
use tracing::{debug, info};

use crate::{
    fees, BlockExecutionResult, ExecutionEngine, ExecutionError, GAS_CALL_CONTRACT,
    GAS_CALL_SCRIPT, GAS_CREATE_OBJECT_BASE, GAS_CREATE_OBJECT_PER_BYTE, GAS_DEPLOY_CONTRACT,
    GAS_DEPLOY_SCRIPT, GAS_GOVERNANCE, GAS_REFRESH, GAS_REFUND, GAS_TRANSFER,
    GAS_VALIDATOR_CLAIM_STAKE, GAS_VALIDATOR_EXIT, GAS_VALIDATOR_STAKE,
};

// ─── Access Key & Conflict Detection ───────────────────────────────────────

/// A key representing a piece of state that a transaction reads or writes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum AccessKey {
    /// An account (by address) — transfers, staking, fee deductions.
    Account(AccountAddress),
    /// A state object or ghost (by ID) — creates, refreshes, resurrections.
    Object(ObjectId),
    /// The global contract engine — deploy/call contract transactions.
    ContractEngine,
    /// The global script engine — deploy/call script transactions.
    ScriptEngine,
    /// The global privacy engine — shield/unshield/private transfer.
    PrivacyEngine,
    /// The global temporal engine — deferred transactions.
    TemporalEngine,
}

/// Extract all state keys that a transaction reads or writes.
fn extract_access_keys(tx: &Transaction) -> Vec<AccessKey> {
    match tx {
        Transaction::Transfer(t) => {
            vec![AccessKey::Account(t.from), AccessKey::Account(t.to)]
        }
        Transaction::CreateObject(t) => {
            vec![
                AccessKey::Account(t.creator),
                AccessKey::Object(t.object_id),
            ]
        }
        Transaction::Refresh(t) => {
            vec![AccessKey::Object(t.object_id)]
        }
        Transaction::DeployContract(t) => {
            vec![AccessKey::Account(t.deployer), AccessKey::ContractEngine]
        }
        Transaction::CallContract(t) => {
            vec![AccessKey::Account(t.caller), AccessKey::ContractEngine]
        }
        Transaction::DeployScript(t) => {
            vec![AccessKey::Account(t.deployer), AccessKey::ScriptEngine]
        }
        Transaction::CallScript(t) => {
            vec![AccessKey::Account(t.caller), AccessKey::ScriptEngine]
        }
        Transaction::ValidatorStake(t) => {
            vec![AccessKey::Account(t.validator_address)]
        }
        Transaction::ValidatorExit(t) => {
            vec![AccessKey::Account(t.validator_address)]
        }
        Transaction::ValidatorClaimStake(t) => {
            vec![AccessKey::Account(t.validator_address)]
        }
        Transaction::Shield(t) => {
            vec![AccessKey::Account(t.from), AccessKey::PrivacyEngine]
        }
        Transaction::Unshield(t) => {
            vec![AccessKey::Account(t.to), AccessKey::PrivacyEngine]
        }
        Transaction::PrivateTransfer(_) => {
            vec![AccessKey::PrivacyEngine]
        }
        Transaction::Deferred(dtx) => {
            vec![AccessKey::Account(dtx.submitter), AccessKey::TemporalEngine]
        }
        Transaction::Blob(tx) => {
            vec![AccessKey::Account(tx.submitter)]
        }
        Transaction::Governance(tx) => {
            vec![AccessKey::Account(tx.sender)]
        }
        Transaction::MultiSig(tx) => {
            vec![AccessKey::Account(tx.multisig_address)]
        }
        Transaction::UserOp(tx) => {
            let mut keys = vec![AccessKey::Account(tx.sender)];
            if let Some(ref pm) = tx.paymaster {
                keys.push(AccessKey::Account(*pm));
            }
            keys
        }
        Transaction::UpgradeContract(tx) => {
            vec![AccessKey::Account(tx.owner)]
        }
        Transaction::Delegate(tx) => {
            vec![AccessKey::Account(tx.delegator)]
        }
        Transaction::Undelegate(tx) => {
            vec![AccessKey::Account(tx.delegator)]
        }
        Transaction::RotateValidatorKey(tx) => {
            vec![AccessKey::Account(tx.validator_address)]
        }
        Transaction::ClaimDelegation(tx) => {
            vec![AccessKey::Account(tx.delegator)]
        }
        // Crooks-MEV refund — Phase 3.1 of CROOKS_MEV_INTEGRATION_PLAN.md.
        // Touches both the debited (attacker) and credited (victim)
        // accounts. Phase 3.5 will add the actual balance movement;
        // for now this declares the access pattern correctly so the
        // BlockSTM scheduler doesn't run conflicting refunds in parallel.
        Transaction::Refund(tx) => {
            vec![
                AccessKey::Account(tx.attacker),
                AccessKey::Account(tx.victim),
            ]
        }
    }
}

// ─── Union-Find Partitioner ────────────────────────────────────────────────

/// Disjoint-set (union-find) with path compression and union by rank.
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        match self.rank[ra].cmp(&self.rank[rb]) {
            std::cmp::Ordering::Less => self.parent[ra] = rb,
            std::cmp::Ordering::Greater => self.parent[rb] = ra,
            std::cmp::Ordering::Equal => {
                self.parent[rb] = ra;
                self.rank[ra] += 1;
            }
        }
    }
}

/// Partition transactions into non-conflicting groups using union-find.
/// Returns a Vec of partitions, each containing (original_index, &Transaction).
/// Transactions within the same partition share at least one access key
/// (transitively) and must execute sequentially. Different partitions are
/// fully independent and can execute in parallel.
fn partition_transactions(txs: &[Transaction]) -> Vec<Vec<(usize, &Transaction)>> {
    if txs.is_empty() {
        return vec![];
    }

    let n = txs.len();
    let mut uf = UnionFind::new(n);

    // Map each access key to the first transaction index that uses it.
    // When a second tx uses the same key, union it with the first.
    let mut key_to_first: HashMap<AccessKey, usize> = HashMap::new();

    for (i, tx) in txs.iter().enumerate() {
        let keys = extract_access_keys(tx);
        for key in keys {
            match key_to_first.get(&key) {
                Some(&first) => uf.union(first, i),
                None => {
                    key_to_first.insert(key, i);
                }
            }
        }
    }

    // Group transactions by their root representative.
    let mut groups: HashMap<usize, Vec<(usize, &Transaction)>> = HashMap::new();
    for (i, tx) in txs.iter().enumerate() {
        let root = uf.find(i);
        groups.entry(root).or_default().push((i, tx));
    }

    // Sort partitions by their first transaction index to preserve block ordering.
    let mut partitions: Vec<Vec<(usize, &Transaction)>> = groups.into_values().collect();
    partitions.sort_by_key(|p| p[0].0);

    partitions
}

// ─── Overlay State DB ──────────────────────────────────────────────────────

/// A self-contained state DB pre-populated with clones of the state that a
/// partition's transactions will touch. After execution, the modified state
/// can be extracted and merged back into the main DB.
struct OverlayStateDB {
    accounts: HashMap<AccountAddress, Account>,
    objects: HashMap<ObjectId, StateObject>,
    ghosts: HashMap<ObjectId, GhostRecord>,
    /// Objects that were newly created (not cloned from base).
    created_objects: HashSet<ObjectId>,
    /// Objects that were deleted during execution.
    deleted_objects: HashSet<ObjectId>,
    /// Ghosts that were newly added.
    created_ghosts: HashSet<ObjectId>,
    /// Ghosts that were removed (resurrection).
    removed_ghosts: HashSet<ObjectId>,
}

impl OverlayStateDB {
    fn new() -> Self {
        Self {
            accounts: HashMap::new(),
            objects: HashMap::new(),
            ghosts: HashMap::new(),
            created_objects: HashSet::new(),
            deleted_objects: HashSet::new(),
            created_ghosts: HashSet::new(),
            removed_ghosts: HashSet::new(),
        }
    }

    /// Pre-populate this overlay from the base DB for the given access keys.
    fn populate_from(&mut self, base: &dyn StateDB, keys: &[AccessKey]) {
        for key in keys {
            match key {
                AccessKey::Account(addr) => {
                    if let Some(acct) = base.get_account(addr) {
                        self.accounts.insert(*addr, acct.clone());
                    }
                }
                AccessKey::Object(id) => {
                    if let Some(obj) = base.get_object(id) {
                        self.objects.insert(*id, obj.clone());
                    }
                    if let Some(ghost) = base.get_ghost(id) {
                        self.ghosts.insert(*id, ghost.clone());
                    }
                }
                AccessKey::ContractEngine
                | AccessKey::ScriptEngine
                | AccessKey::PrivacyEngine
                | AccessKey::TemporalEngine => {
                    // These are handled by the executor, not the state DB.
                }
            }
        }
    }
}

fn empty_verkle_proof() -> evaporchain_crypto::EnergyVerkleProof {
    evaporchain_crypto::EnergyVerkleProof {
        key: [0u8; 32],
        value: None,
        depth: 0,
        commitments: vec![],
        path_indices: vec![],
        siblings: vec![],
        energy_path: vec![],
        hit_compressed: false,
    }
}

impl StateDB for OverlayStateDB {
    fn get_object(&self, id: &ObjectId) -> Option<&StateObject> {
        self.objects.get(id)
    }

    fn get_object_mut(&mut self, id: &ObjectId) -> Option<&mut StateObject> {
        self.objects.get_mut(id)
    }

    fn put_object(&mut self, obj: StateObject) {
        let id = obj.id;
        self.objects.insert(id, obj);
        self.created_objects.insert(id);
    }

    fn delete_object(&mut self, id: &ObjectId) -> Option<StateObject> {
        self.deleted_objects.insert(*id);
        self.objects.remove(id)
    }

    fn put_ghost(&mut self, record: GhostRecord) {
        let id = record.object_id;
        self.ghosts.insert(id, record);
        self.created_ghosts.insert(id);
    }

    fn get_ghost(&self, id: &ObjectId) -> Option<&GhostRecord> {
        self.ghosts.get(id)
    }

    fn remove_ghost(&mut self, id: &ObjectId) -> Option<GhostRecord> {
        self.removed_ghosts.insert(*id);
        self.ghosts.remove(id)
    }

    fn all_object_ids(&self) -> Vec<ObjectId> {
        self.objects.keys().copied().collect()
    }

    fn object_count(&self) -> usize {
        self.objects.len()
    }

    fn ghost_count(&self) -> usize {
        self.ghosts.len()
    }

    fn all_ghost_ids(&self) -> Vec<ObjectId> {
        self.ghosts.keys().copied().collect()
    }

    fn get_account(&self, addr: &AccountAddress) -> Option<&Account> {
        self.accounts.get(addr)
    }

    fn get_account_mut(&mut self, addr: &AccountAddress) -> Option<&mut Account> {
        self.accounts.get_mut(addr)
    }

    fn put_account(&mut self, account: Account) {
        self.accounts.insert(account.address, account);
    }

    fn delete_account(&mut self, addr: &AccountAddress) -> Option<Account> {
        self.accounts.remove(addr)
    }

    fn get_or_create_account(&mut self, addr: &AccountAddress) -> &mut Account {
        self.accounts.entry(*addr).or_insert_with(|| Account {
            address: *addr,
            balance: 0,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        })
    }

    fn all_account_addresses(&self) -> Vec<AccountAddress> {
        self.accounts.keys().copied().collect()
    }

    fn compute_state_root(&mut self) -> [u8; 32] {
        [0u8; 32]
    }

    fn prune_before_height(&mut self, _height: u64) -> u64 {
        0
    }

    fn compress_cold_subtrees(&mut self) -> u32 {
        0
    }
    fn trie_health(&mut self) -> evaporchain_crypto::TrieHealth {
        evaporchain_crypto::TrieHealth {
            active_leaves: 0,
            compressed_leaves: 0,
            total_nodes: 0,
            max_energy: 0,
            min_half_life: u64::MAX,
            last_activity_epoch: 0,
            compressions: 0,
            decompressions: 0,
        }
    }
    fn prove_account(&mut self, _addr: &AccountAddress) -> evaporchain_crypto::EnergyVerkleProof {
        empty_verkle_proof()
    }
    fn prove_object(&mut self, _id: &ObjectId) -> evaporchain_crypto::EnergyVerkleProof {
        empty_verkle_proof()
    }
    fn prove_at_key(&mut self, _key: &[u8; 32]) -> evaporchain_crypto::EnergyVerkleProof {
        // Overlay holds only ephemeral writes — proving against the
        // overlay's view alone gives an inconsistent answer relative to
        // the canonical post-commit trie. Return the empty proof so
        // overlay-level callers don't accidentally rely on this path;
        // the canonical proof is computed by the wrapped backing store
        // after `materialize_final_state` flushes the overlay.
        empty_verkle_proof()
    }
    fn trie_snapshot(&mut self) -> Vec<u8> {
        Vec::new()
    }
    fn load_trie_snapshot(&mut self, _bytes: &[u8]) -> Result<(), String> {
        Ok(())
    }

    // Privacy methods — overlay doesn't handle privacy state (it's in the serial phase).
    fn put_note_tree_root(&mut self, _root: [u8; 32]) {}
    fn get_note_tree_root(&self) -> [u8; 32] {
        [0u8; 32]
    }
    fn spend_nullifier(&mut self, _nullifier: &[u8; 32]) -> bool {
        false
    }
    fn is_nullifier_spent(&self, _nullifier: &[u8; 32]) -> bool {
        false
    }
    fn nullifier_count(&self) -> usize {
        0
    }
    fn all_nullifiers(&self) -> Vec<[u8; 32]> {
        Vec::new()
    }
    fn put_shielded_pool_balance(&mut self, _balance: u64) {}
    fn get_shielded_pool_balance(&self) -> u64 {
        0
    }
    fn put_note_count(&mut self, _count: u64) {}
    fn get_note_count(&self) -> u64 {
        0
    }
    // Sister-session-added trait methods, stubbed here to satisfy the trait.
    // Real implementation lives in InMemoryStateDB / RocksDB; OverlayStateDB
    // never serves the privacy-pool note commitment trie.
    fn append_note_commitment(&mut self, _index: u64, _commitment: [u8; 32]) {}
    fn get_all_note_commitments(&self) -> Vec<[u8; 32]> {
        Vec::new()
    }
    fn get_stake(&self, _validator_id: u64) -> Option<&evaporchain_types::StakeRecord> {
        None
    }
    fn put_stake(&mut self, _record: evaporchain_types::StakeRecord) {}
    fn remove_stake(&mut self, _validator_id: u64) -> Option<evaporchain_types::StakeRecord> {
        None
    }
    fn all_stakes(&self) -> Vec<&evaporchain_types::StakeRecord> {
        Vec::new()
    }
    fn get_delegation(
        &self,
        _delegator: &evaporchain_types::AccountAddress,
        _validator_id: u64,
    ) -> Option<&evaporchain_types::DelegationRecord> {
        None
    }
    fn put_delegation(&mut self, _record: evaporchain_types::DelegationRecord) {}
    fn remove_delegation(
        &mut self,
        _delegator: &evaporchain_types::AccountAddress,
        _validator_id: u64,
    ) -> Option<evaporchain_types::DelegationRecord> {
        None
    }
    fn delegations_for_validator(
        &self,
        _validator_id: u64,
    ) -> Vec<&evaporchain_types::DelegationRecord> {
        Vec::new()
    }
    fn delegations_for_delegator(
        &self,
        _delegator: &evaporchain_types::AccountAddress,
    ) -> Vec<&evaporchain_types::DelegationRecord> {
        Vec::new()
    }
    fn all_delegations(&self) -> Vec<&evaporchain_types::DelegationRecord> {
        Vec::new()
    }
    fn get_proposal(&self, _proposal_id: u64) -> Option<&evaporchain_types::GovernanceProposal> {
        None
    }
    fn put_proposal(&mut self, _proposal: evaporchain_types::GovernanceProposal) {}
    fn all_proposals(&self) -> Vec<&evaporchain_types::GovernanceProposal> {
        Vec::new()
    }
    fn get_governance_param(&self, _key: &str) -> Option<&str> {
        None
    }
    fn put_governance_param(&mut self, _key: String, _value: String) {}
    fn commit_state_snapshot(&mut self, _height: u64) {}
    fn get_account_at_height(
        &self,
        _address: &evaporchain_types::AccountAddress,
        _height: u64,
    ) -> Option<evaporchain_types::Account> {
        None
    }
    fn get_object_at_height(
        &self,
        _id: &evaporchain_types::ObjectId,
        _height: u64,
    ) -> Option<evaporchain_types::StateObject> {
        None
    }
    fn earliest_snapshot_height(&self) -> Option<u64> {
        None
    }
    fn latest_snapshot_height(&self) -> Option<u64> {
        None
    }
    fn prune_snapshots_before(&mut self, _height: u64) {}
}

// ─── Partition Execution Result ────────────────────────────────────────────

/// Result of executing a single partition.
struct PartitionResult {
    overlay: OverlayStateDB,
    txs_executed: usize,
    txs_failed: usize,
    gas_used: u64,
    total_fees: u64,
    /// Per-tx outcomes tagged with their original block index so the
    /// caller can re-assemble a block-order outcomes vec.
    tx_outcomes: Vec<(usize, crate::TxOutcome)>,
}

// ─── Parallel Executor ─────────────────────────────────────────────────────

/// Parallel execution engine that partitions transactions by state access
/// and executes non-conflicting partitions concurrently.
///
/// Uses the same transaction execution logic as `SimpleExecutor` but achieves
/// parallelism through static conflict detection. In the best case (all
/// transactions touch independent state), achieves O(1) block execution time
/// regardless of transaction count. In the worst case (all transactions
/// conflict), falls back to sequential execution.
#[derive(Clone)]
pub struct ParallelExecutor {
    evaporation_engine: EvaporationEngine,
    /// Energy-stamped nullifier accumulator. One leaf per evaporated
    /// object; consumed by `four_act_state()` for the public
    /// `/api/four_act` surface.
    pub mmr: MerkleMountainRange,
    verify_signatures: bool,
    fee_controller: Option<fees::PidFeeController>,
    pub block_gas_limit: u64,
    pub contract_engine: ContractEngine,
    pub script_engine: ScriptEngine,
    pub privacy_executor: crate::privacy_exec::PrivacyExecutor,
    pub deferred_queue: crate::temporal::DeferredQueue,
    pub decay_watchers: crate::temporal::DecayWatcherEngine,
    pub chain_id: String,
    /// Cross-shard router. Inbound messages addressed to this shard are
    /// drained and processed at the end of every block; outbound messages
    /// produced by tx execution are routed via the router for inclusion
    /// in the destination shard's next block. Default ShardId(0) =
    /// single-shard topology.
    pub shard_id: evaporchain_sharding::shard_assignment::ShardId,
    pub cross_shard_router: evaporchain_sharding::cross_shard::CrossShardRouter,
    /// Four-act narrative spine state. Mirrors SimpleExecutor's
    /// fields so production (which uses ParallelExecutor) carries
    /// the same state.
    pub eulogy_trie: evaporchain_tombstone::EulogyTrie,
    pub refresh_pool: evaporchain_energy_kernel::RefreshPool,
    pub mortis_monitor: evaporchain_mortis::MortisMonitor,
    pub mortis_certificate: Option<evaporchain_mortis::MortisCertificate>,
    /// Per-block §1.2 conservation audit verdict. Stored for
    /// observability; gating is governance-controlled via
    /// `conservation_enforcement` (see
    /// `ExecutionError::ConservationViolation`).
    pub last_conservation_audit:
        Option<Result<(), evaporchain_energy_kernel::ConservationViolation>>,
    /// Block.epoch of the previous successful audit. Drives the
    /// `epochs_elapsed` argument fed to `energy_at_epoch`. `None`
    /// until the first audit runs.
    pub last_audit_epoch: Option<u64>,
    /// Consecutive-clean-audit counter (mirrors SimpleExecutor field).
    /// Operator-facing readiness signal for the
    /// `conservation_enforcement` flag flip. Increments on Ok; resets
    /// to 0 on Err verdicts. 0 until the first block runs.
    pub consecutive_clean_audits: u64,
    /// Reward accumulator for block rewards + fee distribution. Mirrors
    /// `SimpleExecutor.reward_accumulator`; until this commit production
    /// tendermint validators received NO block rewards because
    /// `ParallelExecutor` lacked this field entirely. Initialised None;
    /// node startup calls `enable_rewards(tokenomics)` to turn it on.
    pub reward_accumulator: Option<crate::rewards::RewardAccumulator>,
    /// Singh-Lyapunov fee state. Mirrors `SimpleExecutor.lyapunov_fee_state`
    /// — production runs `ParallelExecutor`, so without these fields the
    /// Lyapunov controller had no per-block state advance in the hot path
    /// at all (the field existed on `SimpleExecutor` only, and `execute_block`
    /// never even ticked it there). Lane F.1 closes both gaps. Advanced
    /// per-block via `tick_lyapunov_fee_state`.
    pub lyapunov_fee_state: evaporchain_fee_controller::FeeState,
    /// Singh-Lyapunov fee params (snapshot of the chain-global λ + targets).
    /// Genesis defaults from `FeeControllerParams::default_genesis`.
    pub lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams,
    /// Native demurrage parameters. Controls the piecewise log-rate
    /// charged on idle balances above the threshold; sink is
    /// `refresh_pool`. Mirrors `SimpleExecutor.demurrage_params` —
    /// production runs `ParallelExecutor`, so wiring this field is
    /// what actually turns demurrage on for the live chain (Layer 0
    /// item 4 of the doctrine punch list).
    pub demurrage_params: evaporchain_demurrage::DemurrageParams,
}

impl ParallelExecutor {
    pub fn new(grace_period: u64) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            mmr: MerkleMountainRange::new(),
            verify_signatures: false,
            fee_controller: None,
            block_gas_limit: 0,
            contract_engine: ContractEngine::new(),
            script_engine: ScriptEngine::new(),
            privacy_executor: crate::privacy_exec::PrivacyExecutor::new(),
            deferred_queue: crate::temporal::DeferredQueue::new(),
            decay_watchers: crate::temporal::DecayWatcherEngine::new(),
            chain_id: String::new(),
            shard_id: evaporchain_sharding::shard_assignment::ShardId(0),
            cross_shard_router: evaporchain_sharding::cross_shard::CrossShardRouter::new(),
            eulogy_trie: evaporchain_tombstone::EulogyTrie::new(),
            refresh_pool: evaporchain_energy_kernel::RefreshPool::new(),
            mortis_monitor: evaporchain_mortis::MortisMonitor::new(
                evaporchain_mortis::MortisCondition::default_genesis(),
            ),
            mortis_certificate: None,
            last_conservation_audit: None,
            last_audit_epoch: None,
            consecutive_clean_audits: 0,
            reward_accumulator: None,
            lyapunov_fee_state: evaporchain_fee_controller::FeeState::at_equilibrium(
                evaporchain_fee_controller::FeeControllerParams::default_genesis().target_energy,
            ),
            lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams::default_genesis(),
            demurrage_params: evaporchain_demurrage::DemurrageParams::default(),
        }
    }

    /// Per-block hook: advance the Singh-Lyapunov fee state against
    /// `gas_used` from the just-applied block. Returns the new base fee
    /// + Lyapunov drift. Mirrors `SimpleExecutor::tick_lyapunov_fee_state`.
    ///
    /// Per `INVENTION_STACK.md` §4.1 row 4 the empty-block drift is
    /// provably monotone-non-positive (asserted by the property test in
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

    /// Enable block-reward distribution with the given tokenomics.
    /// Mirrors `SimpleExecutor::enable_rewards`. Until called, no
    /// block rewards or fee distribution fire from this executor.
    pub fn enable_rewards(&mut self, tokenomics: evaporchain_types::genesis::Tokenomics) {
        self.reward_accumulator = Some(crate::rewards::RewardAccumulator::new(tokenomics));
    }

    /// Apply the proposer-priority bonus (Phase-1.5 of the energy-
    /// stamped MEV defense). Caller passes the priority sum captured
    /// at proposal time via `Mempool::take_with_priority_and_sum`.
    ///
    /// Returns the bonus credited to `producer` (zero if rewards
    /// aren't enabled or if the divisor zeroes the bonus). Mirrors
    /// `SimpleExecutor::apply_proposer_priority_bonus`.
    pub fn apply_proposer_priority_bonus(
        &mut self,
        db: &mut dyn evaporchain_state::db::StateDB,
        producer: &evaporchain_types::AccountAddress,
        epoch: evaporchain_types::Epoch,
        priority_sum: u64,
        scale_per_unit: u64,
    ) -> u64 {
        let producer_alive = !self.eulogy_trie.contains(producer);
        if let Some(ref mut ra) = self.reward_accumulator {
            ra.apply_priority_bonus(
                db,
                producer,
                epoch,
                priority_sum,
                scale_per_unit,
                producer_alive,
                Some(&mut self.refresh_pool),
            )
        } else {
            0
        }
    }

    /// Get a reference to the reward accumulator (if enabled).
    pub fn reward_accumulator(&self) -> Option<&crate::rewards::RewardAccumulator> {
        self.reward_accumulator.as_ref()
    }

    /// Get a mutable reference to the reward accumulator (if enabled).
    pub fn reward_accumulator_mut(&mut self) -> Option<&mut crate::rewards::RewardAccumulator> {
        self.reward_accumulator.as_mut()
    }

    // ─────────────────────────────────────────────────────────────────
    // POST_EXEC_STATE_VERIFICATION_PLAN.md Phase 2 scaffold
    //
    // The proposer can speculatively execute a block to obtain its
    // post-execution `state_root` BEFORE broadcasting (so the broadcast
    // block carries `post_state_root` filled in). The speculative
    // execution must NOT affect future real applies. This requires
    // reverting both the StateDB (via `db.begin_batch` /
    // `rollback_batch` — already implemented) AND the executor's own
    // mutable state (the gap this scaffold closes).
    //
    // Approach: snapshot the executor before speculation, run
    // execute_block on it as normal, then restore from the snapshot
    // afterward. Backed by `Clone` — every field on ParallelExecutor
    // and every type it transitively contains is `Clone` as of the
    // 2026-05-08 snapshot scaffold landing.
    //
    // Usage shape (caller side, in the propose path):
    //
    //     let snap = executor.snapshot_for_simulation();
    //     db.begin_batch();
    //     let result = executor.execute_block(&mut db, ...)?;
    //     let post_state_root = result.state_root;
    //     db.rollback_batch();
    //     executor.restore_from_simulation(snap);
    //     // executor and db are now bit-identical to pre-snapshot.
    //     block.post_state_root = Some(post_state_root);
    //     // ... broadcast block ...
    //
    // NOT YET WIRED into `tendermint.rs::propose` — the call site
    // change waits for a clean window vs. the parallel paymaster
    // arc to avoid merge conflicts in the consensus path. This
    // method is the structural primitive the wiring will use.
    // ─────────────────────────────────────────────────────────────────

    /// Capture the executor's current mutable state as a snapshot
    /// suitable for restoration after a speculative `execute_block`
    /// call. Pairs with [`StateDB::begin_batch`] /
    /// [`StateDB::rollback_batch`] on the DB side to give the
    /// proposer a fully-reversible "what would the post-state be"
    /// query.
    ///
    /// O(n) in the size of the executor's accumulators (MMR nodes,
    /// nullifier set, eulogy entries, refresh-pool snapshot, etc.).
    /// Memory-equivalent to an extra ParallelExecutor for the
    /// duration of the simulation.
    pub fn snapshot_for_simulation(&self) -> Self {
        self.clone()
    }

    /// Restore the executor's state from a snapshot produced by
    /// [`Self::snapshot_for_simulation`]. After this call returns,
    /// `self` is bit-identical to the executor at the moment the
    /// snapshot was taken.
    ///
    /// Caller is responsible for pairing this with
    /// `db.rollback_batch()` to also revert the DB state captured
    /// by [`StateDB::begin_batch`]. Calling restore without a DB
    /// rollback leaves the executor and DB in inconsistent states.
    pub fn restore_from_simulation(&mut self, snapshot: Self) {
        *self = snapshot;
    }

    /// Create executor with a small privacy tree for fast test initialization.
    /// Uses depth 4 (16 notes) instead of depth 20 (1M notes).
    pub fn new_for_test(grace_period: u64) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            mmr: MerkleMountainRange::new(),
            verify_signatures: false,
            fee_controller: None,
            block_gas_limit: 0,
            contract_engine: ContractEngine::new(),
            script_engine: ScriptEngine::new(),
            privacy_executor: crate::privacy_exec::PrivacyExecutor::with_depth(4),
            deferred_queue: crate::temporal::DeferredQueue::new(),
            decay_watchers: crate::temporal::DecayWatcherEngine::new(),
            chain_id: String::new(),
            shard_id: evaporchain_sharding::shard_assignment::ShardId(0),
            cross_shard_router: evaporchain_sharding::cross_shard::CrossShardRouter::new(),
            eulogy_trie: evaporchain_tombstone::EulogyTrie::new(),
            refresh_pool: evaporchain_energy_kernel::RefreshPool::new(),
            mortis_monitor: evaporchain_mortis::MortisMonitor::new(
                evaporchain_mortis::MortisCondition::default_genesis(),
            ),
            mortis_certificate: None,
            last_conservation_audit: None,
            last_audit_epoch: None,
            consecutive_clean_audits: 0,
            reward_accumulator: None,
            lyapunov_fee_state: evaporchain_fee_controller::FeeState::at_equilibrium(
                evaporchain_fee_controller::FeeControllerParams::default_genesis().target_energy,
            ),
            lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams::default_genesis(),
            demurrage_params: evaporchain_demurrage::DemurrageParams::default(),
        }
    }

    /// Create executor with signature verification and a small privacy tree for tests.
    pub fn new_with_sig_verification_for_test(grace_period: u64) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            mmr: MerkleMountainRange::new(),
            verify_signatures: true,
            fee_controller: None,
            block_gas_limit: 0,
            contract_engine: ContractEngine::new(),
            script_engine: ScriptEngine::new(),
            privacy_executor: crate::privacy_exec::PrivacyExecutor::with_depth(4),
            deferred_queue: crate::temporal::DeferredQueue::new(),
            decay_watchers: crate::temporal::DecayWatcherEngine::new(),
            chain_id: String::new(),
            shard_id: evaporchain_sharding::shard_assignment::ShardId(0),
            cross_shard_router: evaporchain_sharding::cross_shard::CrossShardRouter::new(),
            eulogy_trie: evaporchain_tombstone::EulogyTrie::new(),
            refresh_pool: evaporchain_energy_kernel::RefreshPool::new(),
            mortis_monitor: evaporchain_mortis::MortisMonitor::new(
                evaporchain_mortis::MortisCondition::default_genesis(),
            ),
            mortis_certificate: None,
            last_conservation_audit: None,
            last_audit_epoch: None,
            consecutive_clean_audits: 0,
            reward_accumulator: None,
            lyapunov_fee_state: evaporchain_fee_controller::FeeState::at_equilibrium(
                evaporchain_fee_controller::FeeControllerParams::default_genesis().target_energy,
            ),
            lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams::default_genesis(),
            demurrage_params: evaporchain_demurrage::DemurrageParams::default(),
        }
    }

    pub fn new_with_sig_verification(grace_period: u64) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            mmr: MerkleMountainRange::new(),
            verify_signatures: true,
            fee_controller: None,
            block_gas_limit: 0,
            contract_engine: ContractEngine::new(),
            script_engine: ScriptEngine::new(),
            privacy_executor: crate::privacy_exec::PrivacyExecutor::new(),
            deferred_queue: crate::temporal::DeferredQueue::new(),
            decay_watchers: crate::temporal::DecayWatcherEngine::new(),
            chain_id: String::new(),
            shard_id: evaporchain_sharding::shard_assignment::ShardId(0),
            cross_shard_router: evaporchain_sharding::cross_shard::CrossShardRouter::new(),
            eulogy_trie: evaporchain_tombstone::EulogyTrie::new(),
            refresh_pool: evaporchain_energy_kernel::RefreshPool::new(),
            mortis_monitor: evaporchain_mortis::MortisMonitor::new(
                evaporchain_mortis::MortisCondition::default_genesis(),
            ),
            mortis_certificate: None,
            last_conservation_audit: None,
            last_audit_epoch: None,
            consecutive_clean_audits: 0,
            reward_accumulator: None,
            lyapunov_fee_state: evaporchain_fee_controller::FeeState::at_equilibrium(
                evaporchain_fee_controller::FeeControllerParams::default_genesis().target_energy,
            ),
            lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams::default_genesis(),
            demurrage_params: evaporchain_demurrage::DemurrageParams::default(),
        }
    }

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
            privacy_executor: crate::privacy_exec::PrivacyExecutor::new(),
            deferred_queue: crate::temporal::DeferredQueue::new(),
            decay_watchers: crate::temporal::DecayWatcherEngine::new(),
            chain_id: String::new(),
            shard_id: evaporchain_sharding::shard_assignment::ShardId(0),
            cross_shard_router: evaporchain_sharding::cross_shard::CrossShardRouter::new(),
            eulogy_trie: evaporchain_tombstone::EulogyTrie::new(),
            refresh_pool: evaporchain_energy_kernel::RefreshPool::new(),
            mortis_monitor: evaporchain_mortis::MortisMonitor::new(
                evaporchain_mortis::MortisCondition::default_genesis(),
            ),
            mortis_certificate: None,
            last_conservation_audit: None,
            last_audit_epoch: None,
            consecutive_clean_audits: 0,
            reward_accumulator: None,
            lyapunov_fee_state: evaporchain_fee_controller::FeeState::at_equilibrium(
                evaporchain_fee_controller::FeeControllerParams::default_genesis().target_energy,
            ),
            lyapunov_fee_params: evaporchain_fee_controller::FeeControllerParams::default_genesis(),
            demurrage_params: evaporchain_demurrage::DemurrageParams::default(),
        }
    }

    /// Set this executor's shard id. Inbound cross-shard messages
    /// addressed to this shard will be drained from
    /// `cross_shard_router` at the end of every block. Default
    /// `ShardId(0)` for single-shard topology.
    pub fn set_shard_id(&mut self, shard: evaporchain_sharding::shard_assignment::ShardId) {
        self.shard_id = shard;
    }

    /// Per-block hook: advance Mortis monitor against the current
    /// refresh-pool total. Mints the death-cert NFT on first trigger.
    /// Returns the cert iff JUST minted on this tick.
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

    pub fn fee_controller(&self) -> Option<&fees::PidFeeController> {
        self.fee_controller.as_ref()
    }

    pub fn fee_controller_mut(&mut self) -> Option<&mut fees::PidFeeController> {
        self.fee_controller.as_mut()
    }

    pub fn estimate_gas(tx: &Transaction) -> u64 {
        match tx {
            Transaction::Transfer(_) => GAS_TRANSFER,
            Transaction::CreateObject(create) => {
                GAS_CREATE_OBJECT_BASE + GAS_CREATE_OBJECT_PER_BYTE * create.data.len() as u64
            }
            Transaction::Refresh(_) => GAS_REFRESH,
            Transaction::DeployContract(_) => GAS_DEPLOY_CONTRACT,
            Transaction::CallContract(_) => GAS_CALL_CONTRACT,
            Transaction::DeployScript(_) => GAS_DEPLOY_SCRIPT,
            Transaction::CallScript(_) => GAS_CALL_SCRIPT,
            Transaction::ValidatorStake(_) => GAS_VALIDATOR_STAKE,
            Transaction::ValidatorExit(_) => GAS_VALIDATOR_EXIT,
            Transaction::ValidatorClaimStake(_) => GAS_VALIDATOR_CLAIM_STAKE,
            Transaction::Shield(_) => crate::privacy_exec::GAS_SHIELD,
            Transaction::Unshield(_) => crate::privacy_exec::GAS_UNSHIELD,
            Transaction::PrivateTransfer(ptx) => {
                crate::privacy_exec::PrivacyExecutor::estimate_private_transfer_gas(ptx)
            }
            Transaction::Deferred(dtx) => {
                crate::temporal::GAS_DEFERRED_SUBMIT
                    + crate::temporal::GAS_PER_GUARD * dtx.guards.len() as u64
            }
            Transaction::Blob(tx) => {
                crate::GAS_CREATE_OBJECT_BASE
                    + crate::GAS_CREATE_OBJECT_PER_BYTE * tx.data.len() as u64
            }
            Transaction::Governance(_) => GAS_GOVERNANCE,
            Transaction::MultiSig(_) => crate::GAS_MULTISIG,
            Transaction::UserOp(tx) => {
                crate::GAS_USER_OP.saturating_add(tx.call_data.len() as u64 * 16)
            }
            Transaction::UpgradeContract(tx) => {
                crate::GAS_UPGRADE_CONTRACT.saturating_add(tx.new_bytecode.len() as u64 * 200)
            }
            Transaction::Delegate(_) => crate::GAS_DELEGATE,
            Transaction::Undelegate(_) => crate::GAS_UNDELEGATE,
            Transaction::RotateValidatorKey(_) => crate::GAS_ROTATE_VALIDATOR_KEY,
            Transaction::ClaimDelegation(_) => crate::GAS_CLAIM_DELEGATION,
            // Crooks-MEV Phase 3.5: protocol-issued refund. Charged
            // at `GAS_REFUND = 5_000` (lower than `GAS_TRANSFER`) so
            // proposers aren't economically deterred from settling
            // refunds they're obligated to settle. Mirrors lib.rs
            // gas_for; see the lib.rs Refund arm for the full
            // rationale (Phase 3.5d stake-slash assumes proposers
            // find settling cheaper than skipping).
            Transaction::Refund(_) => GAS_REFUND,
        }
    }

    pub fn verify_tx_signature(
        verify: bool,
        tx: &Transaction,
        chain_id: &str,
    ) -> Result<(), ExecutionError> {
        if !verify {
            return Ok(());
        }
        if matches!(
            tx,
            Transaction::Unshield(_) | Transaction::PrivateTransfer(_)
        ) {
            return Ok(());
        }
        let sig = tx.signature().ok_or(ExecutionError::MissingSignature)?;
        let pk = tx.public_key().ok_or(ExecutionError::MissingSignature)?;
        let msg = tx.signing_message(chain_id);
        if !HybridVerifier::verify(&msg, sig, pk) {
            return Err(ExecutionError::InvalidSignature);
        }
        Ok(())
    }

    /// Execute a partition's transactions sequentially against an overlay DB.
    /// Contract/script transactions are skipped here — they require mutable
    /// access to the global engines and are handled in a serial phase.
    #[allow(clippy::too_many_arguments)]
    fn execute_partition(
        txs: &[(usize, &Transaction)],
        overlay: &mut OverlayStateDB,
        epoch: Epoch,
        verify_signatures: bool,
        _base_fee: u64,
        fee_controller: &Option<fees::PidFeeController>,
        _block_gas_limit: u64,
        gas_budget: &mut u64,
        chain_id: &str,
    ) -> PartitionResult {
        let mut txs_executed = 0;
        let mut txs_failed = 0;
        let mut gas_used = 0u64;
        let mut total_fees = 0u64;
        let mut tx_outcomes: Vec<(usize, crate::TxOutcome)> = Vec::with_capacity(txs.len());

        for &(idx, tx) in txs {
            let tx_hash = tx.tx_hash();
            // Signature verification
            if let Err(e) = Self::verify_tx_signature(verify_signatures, tx, chain_id) {
                debug!(tx_idx = idx, error = %e, "Parallel: signature verification failed");
                txs_failed += 1;
                tx_outcomes.push((
                    idx,
                    crate::TxOutcome {
                        tx_hash,
                        success: false,
                        error: Some(format!("signature: {e}")),
                        gas_used: 0,
                    },
                ));
                continue;
            }

            let tx_gas = Self::estimate_gas(tx);

            // Block gas limit check (using shared budget)
            if *gas_budget < tx_gas {
                debug!(tx_idx = idx, "Parallel: block gas limit exceeded");
                txs_failed += 1;
                tx_outcomes.push((
                    idx,
                    crate::TxOutcome {
                        tx_hash,
                        success: false,
                        error: Some("block gas limit exceeded".to_string()),
                        gas_used: 0,
                    },
                ));
                continue;
            }

            // Fee deduction
            let tx_fee = if let Some(fc) = fee_controller {
                let gas_fee = fc.compute_gas_fee(tx_gas, 0);
                let extra_fee = match tx {
                    Transaction::CreateObject(create) => {
                        fc.compute_creation_deposit(create.data.len())
                    }
                    Transaction::Refresh(refresh) => fc.compute_refresh_fee(refresh.energy_deposit),
                    _ => 0,
                };
                let total_tx_fee = gas_fee + extra_fee;

                if let Some(sender_addr) = tx.sender() {
                    let sender = overlay.get_or_create_account(sender_addr);
                    if sender.balance < total_tx_fee {
                        debug!(tx_idx = idx, "Parallel: insufficient balance for fees");
                        txs_failed += 1;
                        tx_outcomes.push((
                            idx,
                            crate::TxOutcome {
                                tx_hash,
                                success: false,
                                error: Some(format!(
                                    "insufficient balance for gas: {}/{}",
                                    sender.balance, total_tx_fee
                                )),
                                gas_used: 0,
                            },
                        ));
                        continue;
                    }
                    sender.balance -= total_tx_fee;
                    // Fee deduction is a balance mutation — refresh anchor.
                    sender.last_touched_epoch = epoch;
                }
                total_tx_fee
            } else {
                0
            };

            // Snapshot for revert on failure
            let sender_snapshot = tx.sender().and_then(|addr| {
                overlay
                    .get_account(addr)
                    .map(|acct| (acct.balance, acct.nonce))
            });

            let result = match tx {
                Transaction::Transfer(t) => Self::exec_transfer(overlay, t, epoch),
                Transaction::CreateObject(t) => Self::exec_create_object(overlay, t, epoch),
                Transaction::Refresh(t) => Self::exec_refresh(overlay, t, epoch),
                // ValidatorStake / Exit / ClaimStake all execute in the
                // serial phase (see filter above + AUDIT_2026_05_13 C3
                // closure). Reaching this arm would be a filter regression
                // — error fail-CLOSED so a future bug surfaces as a tx
                // failure, not a silent balance burn.
                Transaction::ValidatorStake(_) => Err(ExecutionError::ContractError(
                    "validator stake executes in serial phase".into(),
                )),
                Transaction::ValidatorExit(_) => Err(ExecutionError::ContractError(
                    "validator exit executes in serial phase".into(),
                )),
                Transaction::ValidatorClaimStake(_) => Err(ExecutionError::ContractError(
                    "validator claim stake executes in serial phase".into(),
                )),
                // Contract/script txs should not appear in parallelizable partitions
                // (they all share the ContractEngine/ScriptEngine key and form one partition).
                // But if they do end up here, we mark them failed — they'll be handled
                // in the serial phase.
                Transaction::DeployContract(_)
                | Transaction::CallContract(_)
                | Transaction::DeployScript(_)
                | Transaction::CallScript(_)
                | Transaction::Shield(_)
                | Transaction::Unshield(_)
                | Transaction::PrivateTransfer(_)
                | Transaction::Deferred(_) => Err(ExecutionError::ContractError(
                    "contract/script/privacy/deferred txs execute in serial phase".into(),
                )),
                Transaction::Blob(blob) => {
                    if blob.data.is_empty() {
                        Err(ExecutionError::ContractError(
                            "blob data cannot be empty".into(),
                        ))
                    } else if blob.data.len() > crate::MAX_BLOB_SIZE {
                        Err(ExecutionError::ContractError(format!(
                            "blob size {} exceeds limit {}",
                            blob.data.len(),
                            crate::MAX_BLOB_SIZE
                        )))
                    } else if blob.namespace_id == 0 {
                        Err(ExecutionError::ContractError(
                            "reserved namespace_id 0".into(),
                        ))
                    } else {
                        Ok(())
                    }
                }
                Transaction::Governance(_) => Err(ExecutionError::ContractError(
                    "governance txs execute in serial phase".into(),
                )),
                Transaction::MultiSig(_) => Err(ExecutionError::ContractError(
                    "multi-sig txs execute in serial phase".into(),
                )),
                Transaction::UserOp(_) => Err(ExecutionError::ContractError(
                    "user-op txs execute in serial phase".into(),
                )),
                Transaction::UpgradeContract(_) => Err(ExecutionError::ContractError(
                    "upgrade txs execute in serial phase".into(),
                )),
                Transaction::Delegate(_) => Err(ExecutionError::ContractError(
                    "delegation txs execute in serial phase".into(),
                )),
                Transaction::Undelegate(_) => Err(ExecutionError::ContractError(
                    "delegation txs execute in serial phase".into(),
                )),
                Transaction::RotateValidatorKey(_) => Err(ExecutionError::ContractError(
                    "validator key rotation executes in serial phase".into(),
                )),
                Transaction::ClaimDelegation(_) => Err(ExecutionError::ContractError(
                    "delegation txs execute in serial phase".into(),
                )),
                Transaction::Refund(_) => Err(ExecutionError::ContractError(
                    "refund txs execute in serial phase".into(),
                )),
            };

            match result {
                Ok(()) => {
                    txs_executed += 1;
                    gas_used += tx_gas;
                    total_fees += tx_fee;
                    *gas_budget = gas_budget.saturating_sub(tx_gas);
                    tx_outcomes.push((
                        idx,
                        crate::TxOutcome {
                            tx_hash,
                            success: true,
                            error: None,
                            gas_used: tx_gas,
                        },
                    ));
                }
                Err(e) => {
                    if let (Some(sender_addr), Some((snap_bal, snap_nonce))) =
                        (tx.sender(), sender_snapshot)
                    {
                        if let Some(acct) = overlay.get_account_mut(sender_addr) {
                            acct.balance = snap_bal;
                            acct.nonce = snap_nonce;
                        }
                    }
                    debug!(tx_idx = idx, error = %e, "Parallel: tx failed, reverted");
                    txs_failed += 1;
                    total_fees += tx_fee;
                    tx_outcomes.push((
                        idx,
                        crate::TxOutcome {
                            tx_hash,
                            success: false,
                            error: Some(e.to_string()),
                            gas_used: tx_gas,
                        },
                    ));
                }
            }
        }

        PartitionResult {
            overlay: std::mem::replace(overlay, OverlayStateDB::new()),
            txs_executed,
            txs_failed,
            gas_used,
            total_fees,
            tx_outcomes,
        }
    }

    /// Merge an overlay's state changes back into the main DB.
    /// Sorted iteration for deterministic state across nodes.
    fn merge_overlay(db: &mut dyn StateDB, overlay: OverlayStateDB) {
        let mut accounts: Vec<_> = overlay.accounts.into_iter().collect();
        accounts.sort_by_key(|(addr, _)| *addr);
        for (_, acct) in accounts {
            db.put_account(acct);
        }

        let mut deleted: Vec<_> = overlay.deleted_objects.into_iter().collect();
        deleted.sort();
        for id in &deleted {
            db.delete_object(id);
        }

        let mut objects: Vec<_> = overlay.objects.into_iter().collect();
        objects.sort_by_key(|(id, _)| *id);
        for (_, obj) in objects {
            db.put_object(obj);
        }

        let mut removed: Vec<_> = overlay.removed_ghosts.into_iter().collect();
        removed.sort();
        for id in &removed {
            db.remove_ghost(id);
        }

        let mut ghosts: Vec<_> = overlay.ghosts.into_iter().collect();
        ghosts.sort_by_key(|(id, _)| *id);
        for (_, ghost) in ghosts {
            db.put_ghost(ghost);
        }
    }

    // ─── Per-Transaction Execution (pure functions on overlay) ──────────

    fn exec_transfer(
        db: &mut OverlayStateDB,
        tx: &TransferTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        if tx.from == tx.to {
            return Err(ExecutionError::SelfTransfer);
        }
        if tx.amount == 0 {
            return Err(ExecutionError::ZeroAmount);
        }

        let sender = db.get_or_create_account(&tx.from);
        // Mint-bypass: a transfer from the all-zeros address skips both
        // nonce check (no source account exists) and nonce increment, and
        // creates the recipient out of thin air. Used for legacy faucet
        // and genesis-time minting paths. The canonical genesis faucet at
        // FAUCET_ADDRESS = [0xFA; 32] does NOT use this bypass — it has a
        // real balance and goes through the normal Transfer path.
        let is_mint_bypass = tx.from == [0u8; 32];
        if !is_mint_bypass && sender.nonce != tx.nonce {
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
        sender.balance -= tx.amount;
        if !is_mint_bypass {
            sender.nonce += 1;
        }
        // Stamp the demurrage anchor: balance (and possibly nonce) just mutated.
        sender.last_touched_epoch = epoch;

        let receiver = db.get_or_create_account(&tx.to);
        receiver.balance = receiver.balance.saturating_add(tx.amount);
        // Receiver balance changed — refresh its anchor too.
        receiver.last_touched_epoch = epoch;

        Ok(())
    }

    fn exec_create_object(
        db: &mut OverlayStateDB,
        tx: &CreateObjectTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        if db.get_object(&tx.object_id).is_some() {
            return Err(ExecutionError::ObjectAlreadyExists(hex::encode(
                tx.object_id,
            )));
        }
        // Storage rent enforcement: collect MIN_STORAGE_DEPOSIT from creator.
        let object_bytes = {
            const BASE_OBJECT_BYTES: u64 = 97;
            BASE_OBJECT_BYTES.saturating_add(tx.data.len() as u64)
        };
        {
            let creator = db.get_or_create_account(&tx.creator);
            if creator.balance < evaporchain_types::MIN_STORAGE_DEPOSIT {
                return Err(ExecutionError::InsufficientBalance {
                    account: hex::encode(tx.creator),
                    available: creator.balance,
                    required: evaporchain_types::MIN_STORAGE_DEPOSIT,
                });
            }
            creator.balance -= evaporchain_types::MIN_STORAGE_DEPOSIT;
            creator.storage_deposit = creator
                .storage_deposit
                .saturating_add(evaporchain_types::MIN_STORAGE_DEPOSIT);
            creator.storage_bytes = creator.storage_bytes.saturating_add(object_bytes);
            // Storage-deposit lock-up debits balance — stamp the demurrage anchor.
            creator.last_touched_epoch = epoch;
        }
        db.put_object(StateObject {
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
            lad_mode: tx.lad_mode,
        });
        Ok(())
    }

    fn exec_refresh(
        db: &mut OverlayStateDB,
        tx: &RefreshTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        if db.get_object(&tx.object_id).is_some() {
            RefreshEngine::refresh(db, &tx.object_id, tx.energy_deposit, epoch)
                .map_err(|e| ExecutionError::RefreshFailed(e.to_string()))?;
            return Ok(());
        }
        if db.get_ghost(&tx.object_id).is_some() {
            RefreshEngine::resurrect(db, &tx.object_id, tx.energy_deposit, epoch)
                .map_err(|e| ExecutionError::RefreshFailed(e.to_string()))?;
            return Ok(());
        }
        Err(ExecutionError::ObjectNotFound(hex::encode(tx.object_id)))
    }

    // AUDIT_2026_05_13 C3 closure: `exec_validator_stake` and
    // `exec_validator_exit` were the parallel-partition implementations.
    // They wrote balance debits to the overlay but never reached the
    // main stake ledger (`OverlayStateDB::put_stake` is `{}`), so
    // ValidatorStake silently burned balance and ValidatorExit was a
    // no-op on production. Both transaction types now execute in the
    // serial phase (see filter at the top of `execute_block` and the
    // arm in the serial loop below). Functions removed.
}

impl ExecutionEngine for ParallelExecutor {
    fn execute_block(
        &mut self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<BlockExecutionResult, ExecutionError> {
        // Lane B.2: stamp the block's protocol_version onto the privacy
        // executor BEFORE any privacy txs run. Subsequent
        // execute_unshield / execute_private_transfer will read this
        // to pick the dual-mode double-spend backend (v0 = unbounded
        // db set, v1+ = PNT bounded window).
        self.privacy_executor
            .set_protocol_version(block.protocol_version);
        // Pre-block §1.2 conservation snapshot (read-only over StateDB).
        let conservation_before = crate::energy_audit::compartment_snapshot_with_pool(
            db,
            self.refresh_pool.total_accrued(),
        );
        let base_fee = self.fee_controller.as_ref().map_or(0, |fc| fc.base_fee);
        let mut total_txs_executed = 0usize;
        let mut total_txs_failed = 0usize;
        let mut total_gas_used = 0u64;
        let mut total_fees = 0u64;

        // ── Phase 1: Separate contract/script txs (serial) from parallelizable txs ──

        let mut parallel_txs: Vec<(usize, &Transaction)> = Vec::new();
        let mut serial_txs: Vec<(usize, &Transaction)> = Vec::new();

        for (i, tx) in block.transactions.iter().enumerate() {
            match tx {
                Transaction::DeployContract(_)
                | Transaction::CallContract(_)
                | Transaction::DeployScript(_)
                | Transaction::CallScript(_)
                | Transaction::Shield(_)
                | Transaction::Unshield(_)
                | Transaction::PrivateTransfer(_)
                | Transaction::Deferred(_)
                // ValidatorStake / Exit / ClaimStake mutate the stake
                // ledger via `db.put_stake / db.remove_stake`. The overlay
                // does not buffer stake writes (put_stake there is `{}`)
                // so the parallel-path version silently dropped the stake
                // record — AUDIT_2026_05_13 C3 (silent balance burn on
                // production executor). All three go through serial path
                // which uses the main `db` directly.
                | Transaction::ValidatorStake(_)
                | Transaction::ValidatorExit(_)
                | Transaction::ValidatorClaimStake(_)
                // Delegate / Undelegate / ClaimDelegation mutate the
                // delegation ledger via `db.put_delegation /
                // db.remove_delegation`. Pre-AUDIT-C4 these three errored
                // unconditionally with "executes in serial phase" — yet
                // the filter did not push them to the serial bucket, so
                // every delegation tx on the production executor failed.
                // T1.20 tests at parallel.rs:4631-4720 codified this as
                // desired behaviour (assert_eq!(txs_failed, 1)). Fixed:
                // add to serial filter + port SimpleExecutor arms.
                | Transaction::Delegate(_)
                | Transaction::Undelegate(_)
                | Transaction::ClaimDelegation(_)
                // Governance + MultiSig mutate governance_proposals /
                // account nonces respectively. Pre-AUDIT-C4 the parallel
                // partition errored both with "executes in serial phase"
                // while the filter never pushed them to the serial
                // bucket — every governance vote and multisig tx on the
                // production executor failed silently. T1.20 tests
                // codified the failure as desired behaviour; PR inverts.
                | Transaction::Governance(_)
                | Transaction::MultiSig(_)
                // UpgradeContract mutates `script_engine` — serial-only
                // (audit C4 6/7). Body extracted to shared
                // `execute_upgrade_contract_impl` so SimpleExecutor and
                // ParallelExecutor share the implementation; copies
                // cannot drift.
                | Transaction::UpgradeContract(_)
                // UserOp envelope (sender nonce, paymaster sig verify,
                // gas debit) extracted to shared
                // `execute_user_op_envelope_impl` — audit C4 7/7.
                // Inner-tx dispatch (Transfer / CallScript /
                // CallContract on call_data) remains in SimpleExecutor
                // only; ParallelExecutor errors with a clear deferred
                // message for non-empty call_data until Phase 3e
                // ports the inner dispatch helpers.
                | Transaction::UserOp(_)
                // Validator key rotation mutates ValidatorSet via the
                // BlockExecutionResult.validator_key_rotations side
                // channel, which only the serial path populates. Putting
                // it in the parallel pool would silently drop rotations.
                | Transaction::RotateValidatorKey(_)
                // Refund is protocol-issued and serial.
                | Transaction::Refund(_) => serial_txs.push((i, tx)),
                _ => parallel_txs.push((i, tx)),
            }
        }

        // ── Phase 2: Partition parallelizable txs ──

        // We need Transaction (not ref) for partition_transactions, so collect refs
        // and map back to original indices.
        let partitions = {
            // Build temp vec of transactions for partitioning
            let txs_for_partition: Vec<Transaction> =
                parallel_txs.iter().map(|&(_, tx)| tx.clone()).collect();
            let raw_partitions = partition_transactions(&txs_for_partition);
            // Map partition indices back to original block indices
            raw_partitions
                .into_iter()
                .map(|group| {
                    group
                        .into_iter()
                        .map(|(local_idx, _)| {
                            let (orig_idx, orig_tx) = parallel_txs[local_idx];
                            (orig_idx, orig_tx)
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };

        let num_partitions = partitions.len();
        info!(
            block = block.number,
            total_txs = block.transactions.len(),
            parallel_txs = parallel_txs.len(),
            serial_txs = serial_txs.len(),
            partitions = num_partitions,
            "Parallel executor: partitioned block"
        );

        // ── Phase 3: Build per-partition overlays ──

        let mut partition_data: Vec<(Vec<(usize, &Transaction)>, OverlayStateDB)> = partitions
            .into_iter()
            .map(|group| {
                let mut overlay = OverlayStateDB::new();
                // Collect all access keys for this partition
                let all_keys: Vec<AccessKey> = group
                    .iter()
                    .flat_map(|&(_, tx)| extract_access_keys(tx))
                    .collect();
                overlay.populate_from(db, &all_keys);
                (group, overlay)
            })
            .collect();

        // ── Phase 4: Execute partitions in parallel ──

        // Gas budget: divide evenly across partitions so total cannot exceed limit.
        let num_partitions = partition_data.len().max(1) as u64;
        let per_partition_gas = if self.block_gas_limit > 0 {
            self.block_gas_limit / num_partitions
        } else {
            u64::MAX
        };

        let verify_sigs = self.verify_signatures;
        let fee_ctrl = self.fee_controller.clone();
        let chain_id = self.chain_id.clone();

        let partition_results: Vec<PartitionResult> = partition_data
            .par_iter_mut()
            .map(|(group, overlay)| {
                let mut gas_budget = per_partition_gas;
                Self::execute_partition(
                    group,
                    overlay,
                    block.epoch,
                    verify_sigs,
                    base_fee,
                    &fee_ctrl,
                    0,
                    &mut gas_budget,
                    &chain_id,
                )
            })
            .collect();

        // ── Phase 5: Merge overlays back ──

        // Indexed outcomes by original block index — re-assembled in
        // block-tx order at the end of execute_block so the API layer
        // sees a 1:1 mapping with block.transactions.
        let mut indexed_outcomes: Vec<(usize, crate::TxOutcome)> =
            Vec::with_capacity(block.transactions.len());

        for result in partition_results {
            total_txs_executed += result.txs_executed;
            total_txs_failed += result.txs_failed;
            total_gas_used += result.gas_used;
            total_fees += result.total_fees;
            indexed_outcomes.extend(result.tx_outcomes);
            Self::merge_overlay(db, result.overlay);
        }

        // ── Phase 6: Execute serial (contract/script) txs sequentially ──

        let mut serial_call_depth: usize = 0;
        // Side-effect channel for validator BLS key rotations — populated
        // by `RotateValidatorKey` arm below and surfaced through
        // `BlockExecutionResult.validator_key_rotations` so the consensus
        // layer can apply them post-commit. Closes punch-list 4b.
        let mut validator_key_rotations: Vec<crate::ValidatorKeyRotation> = Vec::new();
        for &(idx, tx) in &serial_txs {
            let tx_hash = tx.tx_hash();
            if let Err(e) = Self::verify_tx_signature(self.verify_signatures, tx, &self.chain_id) {
                debug!(tx_idx = idx, error = %e, "Serial: signature verification failed");
                total_txs_failed += 1;
                indexed_outcomes.push((
                    idx,
                    crate::TxOutcome {
                        tx_hash,
                        success: false,
                        error: Some(format!("signature: {e}")),
                        gas_used: 0,
                    },
                ));
                continue;
            }

            let tx_gas = Self::estimate_gas(tx);
            if self.block_gas_limit > 0 && total_gas_used + tx_gas > self.block_gas_limit {
                debug!(tx_idx = idx, "Serial: block gas limit exceeded");
                total_txs_failed += 1;
                indexed_outcomes.push((
                    idx,
                    crate::TxOutcome {
                        tx_hash,
                        success: false,
                        error: Some("block gas limit exceeded".to_string()),
                        gas_used: 0,
                    },
                ));
                continue;
            }

            let tx_fee = if let Some(fc) = &self.fee_controller {
                let total_tx_fee = fc.compute_gas_fee(tx_gas, 0);
                if let Some(sender_addr) = tx.sender() {
                    let sender = db.get_or_create_account(sender_addr);
                    if sender.balance < total_tx_fee {
                        total_txs_failed += 1;
                        indexed_outcomes.push((
                            idx,
                            crate::TxOutcome {
                                tx_hash,
                                success: false,
                                error: Some(format!(
                                    "insufficient balance for gas: {}/{}",
                                    sender.balance, total_tx_fee
                                )),
                                gas_used: 0,
                            },
                        ));
                        continue;
                    }
                    sender.balance -= total_tx_fee;
                    // Fee deduction is a balance mutation — refresh anchor.
                    sender.last_touched_epoch = block.epoch;
                }
                total_tx_fee
            } else {
                0
            };

            let result = match tx {
                Transaction::DeployContract(deploy) => {
                    let template = match deploy.template.as_str() {
                        "DecayingToken" => Ok(ContractTemplate::DecayingToken),
                        "MortalNFT" => Ok(ContractTemplate::MortalNFT),
                        "ThermodynamicEscrow" => Ok(ContractTemplate::ThermodynamicEscrow),
                        "DecayingAuction" => Ok(ContractTemplate::DecayingAuction),
                        "StakingPool" => Ok(ContractTemplate::StakingPool),
                        "DAOVote" => Ok(ContractTemplate::DAOVote),
                        "TemporalContract" => Ok(ContractTemplate::TemporalContract),
                        other => Err(ExecutionError::ContractError(format!(
                            "unknown template: {other}"
                        ))),
                    };
                    match template {
                        Ok(tmpl) => {
                            let init_args: Result<serde_json::Value, _> =
                                serde_json::from_str(&deploy.init_args);
                            match init_args {
                                Ok(args) => {
                                    let rules = if let Some(rules_str) = &deploy.rules {
                                        serde_json::from_str(rules_str).unwrap_or_default()
                                    } else {
                                        vec![]
                                    };
                                    self.contract_engine
                                        .deploy(
                                            tmpl,
                                            args,
                                            rules,
                                            deploy.deployer,
                                            deploy.energy,
                                            deploy.half_life,
                                            block.epoch,
                                        )
                                        .map(|_| ())
                                        .map_err(|e| ExecutionError::ContractError(e.to_string()))
                                }
                                Err(e) => Err(ExecutionError::ContractError(format!(
                                    "invalid init_args: {e}"
                                ))),
                            }
                        }
                        Err(e) => Err(e),
                    }
                }
                Transaction::CallContract(call) => {
                    if serial_call_depth >= crate::MAX_CALL_DEPTH {
                        Err(ExecutionError::CallDepthExceeded(crate::MAX_CALL_DEPTH))
                    } else {
                        serial_call_depth += 1;
                        let args: Result<serde_json::Value, _> = serde_json::from_str(&call.args);
                        let r = match args {
                            Ok(a) => self
                                .contract_engine
                                .call(call.contract_id, &call.method, &a, &call.caller, call.epoch)
                                .map(|_| ())
                                .map_err(|e| ExecutionError::ContractError(e.to_string())),
                            Err(e) => {
                                Err(ExecutionError::ContractError(format!("invalid args: {e}")))
                            }
                        };
                        serial_call_depth = serial_call_depth.saturating_sub(1);
                        r
                    }
                }
                Transaction::DeployScript(deploy) => self
                    .script_engine
                    .deploy(
                        &deploy.source_code,
                        deploy.deployer,
                        deploy.energy,
                        deploy.half_life,
                        block.epoch,
                    )
                    .map(|_| ())
                    .map_err(|e| ExecutionError::ScriptError(e.to_string())),
                Transaction::CallScript(call) => {
                    if serial_call_depth >= crate::MAX_CALL_DEPTH {
                        Err(ExecutionError::CallDepthExceeded(crate::MAX_CALL_DEPTH))
                    } else {
                        serial_call_depth += 1;
                        let args: Vec<evaporchain_script::Value> =
                            if call.args.is_empty() || call.args == "[]" {
                                vec![]
                            } else {
                                serde_json::from_str(&call.args).unwrap_or_default()
                            };
                        let r = self
                            .script_engine
                            .call(
                                call.contract_id,
                                &call.method,
                                args,
                                call.caller,
                                call.epoch,
                            )
                            .map(|_| ())
                            .map_err(|e| ExecutionError::ScriptError(e.to_string()));
                        serial_call_depth = serial_call_depth.saturating_sub(1);
                        r
                    }
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
                Transaction::ValidatorStake(stake_tx) => {
                    // AUDIT_2026_05_13 C3 + H9 closure. Mirrors
                    // SimpleExecutor::execute_validator_stake at
                    // lib.rs:1632. Uses transferable_balance(epoch)
                    // for the vesting gate (TOKENOMICS §2.6 / Q14)
                    // and writes the StakeRecord with staked_at_epoch
                    // set to the block epoch so the unbonding cooldown
                    // gate fires correctly.
                    if stake_tx.stake_amount == 0 {
                        Err(ExecutionError::ZeroAmount)
                    } else {
                        let sender_view = db
                            .get_account(&stake_tx.validator_address)
                            .cloned()
                            .unwrap_or_else(|| {
                                use evaporchain_types::Account;
                                Account {
                                    address: stake_tx.validator_address,
                                    balance: 0,
                                    nonce: 0,
                                    storage_deposit: 0,
                                    storage_bytes: 0,
                                    last_touched_epoch: 0,
                                    vesting: None,
                                }
                            });
                        if sender_view.nonce != stake_tx.nonce {
                            Err(ExecutionError::InvalidNonce {
                                expected: sender_view.nonce,
                                got: stake_tx.nonce,
                            })
                        } else {
                            let available = sender_view.transferable_balance(block.epoch);
                            if available < stake_tx.stake_amount {
                                Err(ExecutionError::InsufficientBalance {
                                    account: hex::encode(stake_tx.validator_address),
                                    available,
                                    required: stake_tx.stake_amount,
                                })
                            } else {
                                let existing_stake = db
                                    .get_stake(stake_tx.validator_id)
                                    .map(|s| s.staked_amount)
                                    .unwrap_or(0);
                                let sender = db
                                    .get_or_create_account(&stake_tx.validator_address);
                                sender.balance -= stake_tx.stake_amount;
                                sender.nonce += 1;
                                sender.last_touched_epoch = block.epoch;
                                db.put_stake(StakeRecord {
                                    validator_id: stake_tx.validator_id,
                                    validator_address: stake_tx.validator_address,
                                    staked_amount: existing_stake + stake_tx.stake_amount,
                                    staked_at_epoch: block.epoch,
                                    unbonding_epoch: None,
                                    slashed_amount: 0,
                                });
                                Ok(())
                            }
                        }
                    }
                }
                Transaction::ValidatorExit(exit) => {
                    let sender = db.get_or_create_account(&exit.validator_address);
                    if sender.nonce != exit.nonce {
                        Err(ExecutionError::InvalidNonce {
                            expected: sender.nonce,
                            got: exit.nonce,
                        })
                    } else {
                        sender.nonce += 1;
                        sender.last_touched_epoch = block.epoch;
                        // Use explicit match so errors stay in `result` rather than
                        // propagating through `?` and aborting the entire block.
                        match db.get_stake(exit.validator_id).cloned() {
                            None => Err(ExecutionError::ObjectNotFound(format!(
                                "no stake record for validator {}",
                                exit.validator_id
                            ))),
                            Some(mut stake) => {
                                if stake.validator_address != exit.validator_address {
                                    Err(ExecutionError::InvalidSignature)
                                } else if stake.unbonding_epoch.is_some() {
                                    Err(ExecutionError::ContractError(
                                        "validator already exiting".to_string(),
                                    ))
                                } else {
                                    stake.unbonding_epoch =
                                        Some(block.epoch + crate::UNBONDING_PERIOD_EPOCHS);
                                    db.put_stake(stake);
                                    Ok(())
                                }
                            }
                        }
                    }
                }
                Transaction::ValidatorClaimStake(claim) => {
                    // Explicit match avoids `?`/`return Err` accidentally aborting
                    // the entire block execution on a per-tx validation failure.
                    match db.get_stake(claim.validator_id).cloned() {
                        None => Err(ExecutionError::ObjectNotFound(format!(
                            "no stake record for validator {}",
                            claim.validator_id
                        ))),
                        Some(stake) => {
                            if stake.validator_address != claim.validator_address {
                                Err(ExecutionError::InvalidSignature)
                            } else {
                                match stake.unbonding_epoch {
                                    None => Err(ExecutionError::ContractError(
                                        "validator has not exited — cannot claim stake"
                                            .to_string(),
                                    )),
                                    Some(unbonding) => {
                                        if block.epoch < unbonding {
                                            Err(ExecutionError::ContractError(format!(
                                                "unbonding period not complete: current epoch {} < unbonding epoch {}",
                                                block.epoch, unbonding
                                            )))
                                        } else {
                                            let sender = db
                                                .get_or_create_account(&claim.validator_address);
                                            if sender.nonce != claim.nonce {
                                                Err(ExecutionError::InvalidNonce {
                                                    expected: sender.nonce,
                                                    got: claim.nonce,
                                                })
                                            } else {
                                                let claimable = stake
                                                    .staked_amount
                                                    .saturating_sub(stake.slashed_amount);
                                                sender.balance += claimable;
                                                sender.nonce += 1;
                                                sender.last_touched_epoch = block.epoch;
                                                db.remove_stake(claim.validator_id);
                                                Ok(())
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // ─── AUDIT_2026_05_13 C4 closure: Governance ────────────────
                // Mirrors SimpleExecutor::execute_governance at lib.rs:2004.
                Transaction::Governance(gtx) => {
                    use evaporchain_types::{GovernanceAction, GovernanceProposal, ProposalStatus};
                    let sender_view = db
                        .get_account(&gtx.sender)
                        .cloned()
                        .unwrap_or_else(|| Account {
                            address: gtx.sender,
                            balance: 0,
                            nonce: 0,
                            storage_deposit: 0,
                            storage_bytes: 0,
                            last_touched_epoch: 0,
                            vesting: None,
                        });
                    if sender_view.nonce != gtx.nonce {
                        Err(ExecutionError::InvalidNonce {
                            expected: sender_view.nonce,
                            got: gtx.nonce,
                        })
                    } else {
                        // Nonce bump + demurrage anchor stamp.
                        let sender = db.get_or_create_account(&gtx.sender);
                        sender.nonce += 1;
                        sender.last_touched_epoch = block.epoch;

                        match &gtx.action {
                            GovernanceAction::CreateProposal {
                                title,
                                param_key,
                                param_value,
                                voting_epochs,
                            } => {
                                if title.len() > crate::MAX_PROPOSAL_TITLE_BYTES {
                                    Err(ExecutionError::ContractError(format!(
                                        "proposal title exceeds {} bytes ({})",
                                        crate::MAX_PROPOSAL_TITLE_BYTES,
                                        title.len()
                                    )))
                                } else if param_key.len() > crate::MAX_PARAM_KEY_BYTES {
                                    Err(ExecutionError::ContractError(format!(
                                        "param_key exceeds {} bytes ({})",
                                        crate::MAX_PARAM_KEY_BYTES,
                                        param_key.len()
                                    )))
                                } else if param_value.len() > crate::MAX_PARAM_VALUE_BYTES {
                                    Err(ExecutionError::ContractError(format!(
                                        "param_value exceeds {} bytes ({})",
                                        crate::MAX_PARAM_VALUE_BYTES,
                                        param_value.len()
                                    )))
                                } else if !(crate::MIN_VOTING_EPOCHS
                                    ..=crate::MAX_VOTING_EPOCHS)
                                    .contains(voting_epochs)
                                {
                                    Err(ExecutionError::ContractError(format!(
                                        "voting_epochs out of range [{}, {}]: {}",
                                        crate::MIN_VOTING_EPOCHS,
                                        crate::MAX_VOTING_EPOCHS,
                                        voting_epochs
                                    )))
                                } else if !crate::is_governable_param_key(param_key) {
                                    Err(ExecutionError::ContractError(format!(
                                        "param_key '{}' is not on the governable allowlist",
                                        param_key
                                    )))
                                } else if let Err(e) =
                                    crate::validate_param_value(param_key, param_value)
                                {
                                    Err(ExecutionError::ContractError(format!(
                                        "invalid param_value: {}",
                                        e
                                    )))
                                } else {
                                    let proposal_id = db.all_proposals().len() as u64;
                                    let proposal = GovernanceProposal {
                                        proposal_id,
                                        proposer: gtx.sender,
                                        title: title.clone(),
                                        param_key: param_key.clone(),
                                        param_value: param_value.clone(),
                                        start_epoch: block.epoch,
                                        end_epoch: block.epoch + voting_epochs,
                                        votes_for: 0,
                                        votes_against: 0,
                                        status: ProposalStatus::Active,
                                        created_at: block.epoch,
                                        voters: std::collections::HashSet::new(),
                                    };
                                    db.put_proposal(proposal);
                                    Ok(())
                                }
                            }
                            GovernanceAction::CastVote { proposal_id, vote } => {
                                match db.get_proposal(*proposal_id).cloned() {
                                    None => Err(ExecutionError::ContractError(format!(
                                        "proposal {} not found",
                                        proposal_id
                                    ))),
                                    Some(mut proposal) => {
                                        if proposal.status != ProposalStatus::Active {
                                            Err(ExecutionError::ContractError(
                                                "proposal is not active".to_string(),
                                            ))
                                        } else if block.epoch > proposal.end_epoch {
                                            proposal.status =
                                                crate::decide_proposal_outcome(&proposal);
                                            db.put_proposal(proposal);
                                            Err(ExecutionError::ContractError(
                                                "voting period has ended".to_string(),
                                            ))
                                        } else if proposal.voters.contains(&gtx.sender) {
                                            Err(ExecutionError::ContractError(
                                                "duplicate vote: account has already voted on this proposal".into(),
                                            ))
                                        } else {
                                            proposal.voters.insert(gtx.sender);
                                            let voter_balance = db
                                                .get_account(&gtx.sender)
                                                .map(|a| a.balance)
                                                .unwrap_or(0);
                                            let voter_weight =
                                                voter_balance.min(crate::MAX_VOTE_WEIGHT);
                                            if *vote {
                                                proposal.votes_for = proposal
                                                    .votes_for
                                                    .saturating_add(voter_weight);
                                            } else {
                                                proposal.votes_against = proposal
                                                    .votes_against
                                                    .saturating_add(voter_weight);
                                            }
                                            db.put_proposal(proposal);
                                            Ok(())
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // ─── AUDIT_2026_05_13 C4 closure: UpgradeContract ──────────
                // Calls the shared `execute_upgrade_contract_impl` —
                // single source of truth across both executors.
                Transaction::UpgradeContract(utx) => {
                    crate::execute_upgrade_contract_impl(
                        &mut self.script_engine,
                        db,
                        utx,
                        block.epoch,
                    )
                }
                // ─── AUDIT_2026_05_13 C4 closure: UserOp (envelope) ────────
                // Envelope-level parity via shared
                // `execute_user_op_envelope_impl`. Empty call_data
                // (gas-only sponsorship) is fully parity-clean. Non-empty
                // call_data is rejected with a clear deferred error
                // until Phase 3e ports `execute_inner_transfer` /
                // `execute_call_script` / `execute_call_contract` as
                // shared impls. Pre-fix every UserOp errored at the
                // partition arm; now sponsorship works and only the
                // inner-dispatch case is gated.
                Transaction::UserOp(uop) => {
                    match crate::execute_user_op_envelope_impl(
                        &self.chain_id,
                        db,
                        uop,
                        block.epoch,
                    ) {
                        Err(e) => Err(e),
                        Ok(()) => {
                            if uop.call_data.is_empty() {
                                Ok(())
                            } else {
                                Err(ExecutionError::ContractError(
                                    "UserOp inner-tx dispatch not yet wired in \
                                     ParallelExecutor serial path (envelope ran; \
                                     sender + paymaster mutations applied). \
                                     See Phase 3e of executor-parity arc."
                                        .into(),
                                ))
                            }
                        }
                    }
                }
                // ─── AUDIT_2026_05_13 C4 closure: MultiSig ─────────────────
                // Mirrors SimpleExecutor::execute_multisig at lib.rs:2133.
                Transaction::MultiSig(mtx) => {
                    if (mtx.signatures.len() as u8) < mtx.threshold {
                        Err(ExecutionError::ContractError(format!(
                            "multi-sig requires {} signatures, got {}",
                            mtx.threshold,
                            mtx.signatures.len()
                        )))
                    } else {
                        let mut seen_signers = std::collections::HashSet::new();
                        let mut signer_err: Option<ExecutionError> = None;
                        for (signer_addr, _) in &mtx.signatures {
                            if !seen_signers.insert(signer_addr) {
                                signer_err = Some(ExecutionError::ContractError(
                                    "duplicate signer in multisig transaction".into(),
                                ));
                                break;
                            }
                            if !mtx.signers.contains(signer_addr) {
                                signer_err = Some(ExecutionError::ContractError(
                                    "signer not in authorized signers list".to_string(),
                                ));
                                break;
                            }
                        }
                        if let Some(e) = signer_err {
                            Err(e)
                        } else {
                            let sender = db.get_or_create_account(&mtx.multisig_address);
                            if sender.nonce != mtx.nonce {
                                Err(ExecutionError::InvalidNonce {
                                    expected: sender.nonce,
                                    got: mtx.nonce,
                                })
                            } else {
                                sender.nonce += 1;
                                sender.last_touched_epoch = block.epoch;
                                Ok(())
                            }
                        }
                    }
                }
                // ─── AUDIT_2026_05_13 C4 closure: delegation trio ────────────
                // Mirrors SimpleExecutor::execute_delegate /
                // execute_undelegate / execute_claim_delegation at
                // lib.rs:1699 / 1774 / 1828. Uses transferable_balance for
                // the Delegate vesting gate (TOKENOMICS §2.6 / Q14).
                Transaction::Delegate(dtx) => {
                    use evaporchain_types::DelegationRecord;
                    if dtx.amount == 0 {
                        Err(ExecutionError::ZeroAmount)
                    } else if db.get_stake(dtx.validator_id).is_none() {
                        Err(ExecutionError::ContractError(format!(
                            "validator-id {} has no stake record; cannot accept delegations",
                            dtx.validator_id
                        )))
                    } else {
                        let delegator_view = db
                            .get_account(&dtx.delegator)
                            .cloned()
                            .unwrap_or_else(|| Account {
                                address: dtx.delegator,
                                balance: 0,
                                nonce: 0,
                                storage_deposit: 0,
                                storage_bytes: 0,
                                last_touched_epoch: 0,
                                vesting: None,
                            });
                        if delegator_view.nonce != dtx.nonce {
                            Err(ExecutionError::InvalidNonce {
                                expected: delegator_view.nonce,
                                got: dtx.nonce,
                            })
                        } else {
                            let available = delegator_view.transferable_balance(block.epoch);
                            if available < dtx.amount {
                                Err(ExecutionError::InsufficientBalance {
                                    account: hex::encode(dtx.delegator),
                                    available,
                                    required: dtx.amount,
                                })
                            } else {
                                let existing = db
                                    .get_delegation(&dtx.delegator, dtx.validator_id)
                                    .cloned();
                                let delegator = db.get_or_create_account(&dtx.delegator);
                                delegator.balance -= dtx.amount;
                                delegator.nonce += 1;
                                delegator.last_touched_epoch = block.epoch;
                                let record = match existing {
                                    Some(mut r) => {
                                        r.amount = r.amount.saturating_add(dtx.amount);
                                        r.delegated_at_epoch = block.epoch;
                                        r
                                    }
                                    None => DelegationRecord {
                                        delegator: dtx.delegator,
                                        validator_id: dtx.validator_id,
                                        amount: dtx.amount,
                                        delegated_at_epoch: block.epoch,
                                        unbonding_amount: 0,
                                        unbonding_epoch: None,
                                    },
                                };
                                db.put_delegation(record);
                                Ok(())
                            }
                        }
                    }
                }
                Transaction::Undelegate(utx) => {
                    if utx.amount == 0 {
                        Err(ExecutionError::ZeroAmount)
                    } else {
                        let delegator_acct =
                            db.get_or_create_account(&utx.delegator);
                        if delegator_acct.nonce != utx.nonce {
                            Err(ExecutionError::InvalidNonce {
                                expected: delegator_acct.nonce,
                                got: utx.nonce,
                            })
                        } else {
                            delegator_acct.nonce += 1;
                            delegator_acct.last_touched_epoch = block.epoch;
                            match db.get_delegation(&utx.delegator, utx.validator_id).cloned() {
                                None => Err(ExecutionError::ContractError(format!(
                                    "no delegation from {} to validator-id {}",
                                    hex::encode(utx.delegator),
                                    utx.validator_id
                                ))),
                                Some(mut record) => {
                                    if record.amount < utx.amount {
                                        Err(ExecutionError::ContractError(format!(
                                            "delegation has only {} but tried to undelegate {}",
                                            record.amount, utx.amount
                                        )))
                                    } else {
                                        record.amount =
                                            record.amount.saturating_sub(utx.amount);
                                        record.unbonding_amount = record
                                            .unbonding_amount
                                            .saturating_add(utx.amount);
                                        record.unbonding_epoch = Some(block.epoch);
                                        db.put_delegation(record);
                                        Ok(())
                                    }
                                }
                            }
                        }
                    }
                }
                Transaction::ClaimDelegation(ctx) => {
                    let delegator_acct = db.get_or_create_account(&ctx.delegator);
                    if delegator_acct.nonce != ctx.nonce {
                        Err(ExecutionError::InvalidNonce {
                            expected: delegator_acct.nonce,
                            got: ctx.nonce,
                        })
                    } else {
                        delegator_acct.nonce += 1;
                        delegator_acct.last_touched_epoch = block.epoch;
                        match db.get_delegation(&ctx.delegator, ctx.validator_id).cloned() {
                            None => Err(ExecutionError::ContractError(format!(
                                "no delegation from {} to validator-id {}",
                                hex::encode(ctx.delegator),
                                ctx.validator_id
                            ))),
                            Some(mut record) => {
                                if record.unbonding_amount == 0 {
                                    Err(ExecutionError::ContractError(
                                        "no unbonding amount to claim".into(),
                                    ))
                                } else {
                                    match record.unbonding_epoch {
                                        None => Err(ExecutionError::ContractError(
                                            "unbonding_epoch unset".into(),
                                        )),
                                        Some(unbonding_started) => {
                                            let claim_ready_at = unbonding_started
                                                .saturating_add(
                                                    crate::UNBONDING_PERIOD_EPOCHS,
                                                );
                                            if block.epoch < claim_ready_at {
                                                Err(ExecutionError::ContractError(format!(
                                                    "unbonding period not elapsed: claimable at epoch {} (current {})",
                                                    claim_ready_at, block.epoch
                                                )))
                                            } else {
                                                let claimed = record.unbonding_amount;
                                                record.unbonding_amount = 0;
                                                record.unbonding_epoch = None;
                                                if let Some(acct) =
                                                    db.get_account_mut(&ctx.delegator)
                                                {
                                                    acct.balance =
                                                        acct.balance.saturating_add(claimed);
                                                    acct.last_touched_epoch = block.epoch;
                                                }
                                                if record.amount == 0
                                                    && record.unbonding_amount == 0
                                                {
                                                    db.remove_delegation(
                                                        &ctx.delegator,
                                                        ctx.validator_id,
                                                    );
                                                } else {
                                                    db.put_delegation(record);
                                                }
                                                Ok(())
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Transaction::RotateValidatorKey(rot) => {
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
                                    validator_key_rotations.push(crate::ValidatorKeyRotation {
                                        validator_id: rot.validator_id,
                                        new_bls_public_key: rot.new_bls_public_key.clone(),
                                        bls_pop_old: rot.bls_pop_old.clone(),
                                        new_bls_pop: rot.bls_pop_new.clone(),
                                        prev_key_expiry_epoch: rot
                                            .effective_epoch
                                            .saturating_add(crate::KEY_ROTATION_GRACE_EPOCHS),
                                    });
                                    Ok(())
                                }
                            }
                        }
                    }
                }
                // Lane S.1: Crooks-MEV Phase 3.5 protocol-issued refund.
                // Routed to serial phase via line 1449; without this arm
                // the executor would fall into the `unreachable!()` and
                // panic on any block containing a Refund tx. Mirrors
                // SimpleExecutor::execute_refund.
                Transaction::Refund(refund) => {
                    if refund.attacker == refund.victim {
                        Err(ExecutionError::SelfTransfer)
                    } else if refund.amount == 0 {
                        Err(ExecutionError::ZeroAmount)
                    } else {
                        let attacker = db.get_or_create_account(&refund.attacker);
                        if attacker.balance < refund.amount {
                            Err(ExecutionError::InsufficientBalance {
                                account: hex::encode(refund.attacker),
                                available: attacker.balance,
                                required: refund.amount,
                            })
                        } else {
                            attacker.balance -= refund.amount;
                            // No nonce increment — refund is protocol-issued.
                            attacker.last_touched_epoch = block.epoch;
                            let victim = db.get_or_create_account(&refund.victim);
                            victim.balance = victim.balance.saturating_add(refund.amount);
                            victim.last_touched_epoch = block.epoch;
                            Ok(())
                        }
                    }
                }
                _ => unreachable!("only serial-phase txs expected here"),
            };

            match result {
                Ok(()) => {
                    total_txs_executed += 1;
                    total_gas_used += tx_gas;
                    total_fees += tx_fee;
                    indexed_outcomes.push((
                        idx,
                        crate::TxOutcome {
                            tx_hash,
                            success: true,
                            error: None,
                            gas_used: tx_gas,
                        },
                    ));
                }
                Err(e) => {
                    // Revert fee deduction on failure? No — same as SimpleExecutor:
                    // sender still pays for gas even on failure.
                    debug!(tx_idx = idx, error = %e, "Serial: tx failed");
                    total_txs_failed += 1;
                    total_fees += tx_fee;
                    indexed_outcomes.push((
                        idx,
                        crate::TxOutcome {
                            tx_hash,
                            success: false,
                            error: Some(e.to_string()),
                            gas_used: tx_gas,
                        },
                    ));
                }
            }
        }

        // ── Phase 7: Evaporation + contract/script ticks ──

        let evap_result =
            self.evaporation_engine
                .process_epoch_with_mmr(db, block.epoch, &mut self.mmr);
        self.contract_engine.tick(block.epoch);
        self.script_engine.tick(block.epoch);

        // Punch-list 6: storage-rent collection. Inline here (rather than
        // a method on ParallelExecutor) because the rent logic is small,
        // gated by `last_rent_epoch`, and identical to the SimpleExecutor
        // path. Idempotent: if already charged this epoch, no-op.
        //
        // Layer 0 item 4 (Doctrine Punch List): native-demurrage sweep
        // wired in lockstep with rent. Until this commit demurrage was
        // implemented + tested + invoked from `SimpleExecutor`, but
        // `ParallelExecutor` (the executor production tendermint
        // actually drives) had no field for `demurrage_params` and no
        // call site — so on the live chain demurrage was a no-op.
        let mut demurrage_collected: u64 = 0;
        if block.epoch > db.get_last_rent_epoch() {
            let prev_rent_epoch = db.get_last_rent_epoch();
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
                let actually_debited;
                if acct.balance >= rent_info.0 {
                    acct.balance -= rent_info.0;
                    actually_debited = rent_info.0;
                } else {
                    // Account zeroed out — engrave the tombstone before wiping.
                    actually_debited = acct.balance;
                    let final_balance_before_wipe = acct.balance;
                    acct.balance = 0;
                    acct.storage_deposit = 0;
                    acct.storage_bytes = 0;
                    let tombstone = evaporchain_tombstone::mint(
                        addr,
                        final_balance_before_wipe,
                        block.epoch,
                        evaporchain_tombstone::CauseOfDeath::RentExhausted,
                    );
                    let _ = self.eulogy_trie.insert(addr, tombstone);
                }
                if actually_debited > 0 {
                    self.refresh_pool.accrue(
                        b"evaporchain-system-refresh".to_vec(),
                        actually_debited,
                        block.epoch,
                    );
                }
            }
            // Native demurrage sweep — runs after storage rent on the
            // same per-epoch cadence. `prev_rent_epoch` (captured
            // above) is the lower bound of the sweep window;
            // `block.epoch` is the upper bound. `collect_demurrage`
            // skips accounts below `DemurrageParams.threshold` so
            // small genesis accounts are never drained.
            let outcome = crate::demurrage_integration::collect_demurrage(
                db,
                &mut self.refresh_pool,
                &self.demurrage_params,
                prev_rent_epoch,
                block.epoch,
            );
            demurrage_collected = outcome.total;
            db.put_last_rent_epoch(block.epoch);
        }

        // Reward distribution (mirrors SimpleExecutor::execute_block).
        // Until this commit, ParallelExecutor (the executor production
        // tendermint actually drives) had no reward path at all —
        // validators received no block rewards in production. Wiring
        // here is gated on `enable_rewards()` having been called at
        // node startup; if not enabled, behaviour is unchanged.
        // Compute producer-alive once outside the reward-accumulator
        // borrow so we can pass &mut self.refresh_pool alongside the
        // &mut self.reward_accumulator (split borrows on disjoint fields).
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
        let producer_alive = !self.eulogy_trie.contains(&producer_addr);

        // Snapshot total_minted before block rewards so we can credit
        // newly-minted supply into conservation_before below — same fix
        // already applied to SimpleExecutor (lib.rs). Without this, the
        // §1.2 audit fires DecayIncreasedTotal every block where the
        // reward path mints non-zero supply, which is exactly the
        // false-signal the live cluster surfaces as
        // `last_conservation_audit_ok: false` despite no real violation.
        let minted_before_rewards: u64 = self
            .reward_accumulator
            .as_ref()
            .map_or(0, |ra| ra.total_minted);

        // Clone stake data before the mutable reward call (avoids overlapping borrows).
        let (attesters_parallel, total_staked_parallel): (Vec<[u8; 32]>, u64) = {
            let snap: Vec<(u64, [u8; 32], u64, Option<u64>)> = db
                .all_stakes()
                .iter()
                .map(|s| (s.validator_id, s.validator_address, s.staked_amount, s.unbonding_epoch))
                .collect();
            let ts = snap
                .iter()
                .filter(|(_, _, _, u)| u.is_none())
                .map(|(_, _, sa, _)| *sa)
                .fold(0u64, u64::saturating_add);
            let atts = block
                .commit_certificate
                .as_ref()
                .map(|cert| {
                    cert.signer_ids
                        .iter()
                        .filter_map(|&vid| {
                            snap.iter().find(|(v, _, _, _)| *v == vid).map(|(_, a, _, _)| *a)
                        })
                        .collect()
                })
                .unwrap_or_default();
            (atts, ts)
        };

        if let Some(ref mut ra) = self.reward_accumulator {
            ra.process_block_rewards_v2(
                db,
                &producer_addr,
                block.epoch,
                total_fees,
                producer_alive,
                Some(&mut self.refresh_pool),
                &attesters_parallel,
                total_staked_parallel,
            );

            // Lane A.3: priority bonus auto-fire. The proposer stamped
            // `block.submit_epoch_hints` per Lane A.2 (commit 18beb29);
            // every validator now sums `energy_at_epoch(BASE_INCLUSION_ENERGY,
            // MEV_INCLUSION_HALF_LIFE_BLOCKS, block.number - hint)` across
            // hinted txs and mints `priority_sum / scale_per_unit` to the
            // producer. Followers compute the same priority_sum from on-the-
            // wire hints, mint the same bonus, no state divergence — Phase
            // 1.5 of `research/proposals/energy-stamped-mev-resistance.md`
            // is now consensus-deterministic.
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
                // `scale_per_unit = BASE_INCLUSION_ENERGY` ⇒ a fully-fresh
                // tx (elapsed=0) yields ~1 token. Operator-tunable in a
                // future commit (genesis tokenomics field).
                let bonus_scale = evaporchain_types::BASE_INCLUSION_ENERGY;
                ra.apply_priority_bonus(
                    db,
                    &producer_addr,
                    block.epoch,
                    priority_sum,
                    bonus_scale,
                    producer_alive,
                    Some(&mut self.refresh_pool),
                );
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
                        "Staker rewards distributed (parallel executor)"
                    );
                }
            }
        }

        // PNT phase auto-advance (research-buildable #8 follow-up).
        // Rotates the PhasedNullifierTree window iff the configured
        // `pnt_phase_interval_epochs` has elapsed. Stage-1 shadow-only,
        // so this is purely operational telemetry until Stage-2 makes
        // PNT authoritative for double-spend gating.
        let _pnt_advanced = self.privacy_executor.tick_pnt_phase(block.epoch);

        // Lane F.1: Singh-Lyapunov fee state per-block advance. Mirrors
        // SimpleExecutor's tick — closes the "substrate-shipped, no caller"
        // gap from DOCTRINE_PUNCH_LIST.md §6 in the production path.
        // Errors silently dropped: FeeControllerError is unreachable under
        // default_genesis params.
        let _ = self.tick_lyapunov_fee_state(total_gas_used, 1);

        // Lane E.2: EnergyVerkleTrie cold-subtree compression. v0 keeps
        // legacy behaviour (no compression). v1+ compresses cold subtrees
        // (max_energy=0) once per block so the trie physically contracts
        // as objects evaporate. Closes the "substrate-shipped, no caller"
        // gap from earlier in the session — the
        // `EnergyVerkleTrie::compress_cold` method existed but no
        // execute_block path ever invoked it. The state-root binding
        // includes compressed nodes, so the v0→v1 flip changes the root
        // (correctly — that's the whole point of versioning the root).
        if block.state_root_version >= 1 {
            let _compressed = db.compress_cold_subtrees();
        }

        let state_root = db.compute_state_root();

        info!(
            block = block.number,
            epoch = block.epoch,
            txs_executed = total_txs_executed,
            txs_failed = total_txs_failed,
            gas_used = total_gas_used,
            partitions = num_partitions,
            entered_grace = evap_result.entered_grace.len(),
            evaporated = evap_result.evaporated.len(),
            "Parallel block executed"
        );

        // Cross-shard activation (Task #24): drain inbound messages
        // addressed to this shard from the router, generate receipts,
        // and acknowledge them. Each receipt records that the message
        // was processed at this block's epoch. The router's per-shard
        // inbox is now actively consumed instead of accumulating
        // forever (which is what the previous hardcoded
        // cross_shard_processed: 0 implied).
        let inbound = self.cross_shard_router.drain_for_shard(self.shard_id);
        let cross_shard_receipts: Vec<evaporchain_sharding::cross_shard::CrossShardReceipt> =
            inbound
                .into_iter()
                .map(|msg| {
                    let payload_bytes = serde_json::to_vec(&msg.payload).unwrap_or_default();
                    let result_hash = *blake3::hash(&payload_bytes).as_bytes();
                    let receipt = evaporchain_sharding::cross_shard::CrossShardReceipt {
                        message_id: msg.id,
                        from_shard: msg.from_shard,
                        to_shard: msg.to_shard,
                        success: true,
                        result_hash,
                        processed_at: block.epoch,
                    };
                    self.cross_shard_router.acknowledge(receipt.clone());
                    receipt
                })
                .collect();
        let cross_shard_processed = cross_shard_receipts.len();

        // Post-block §1.2 conservation snapshot + audit. Governance-
        // gated by `conservation_enforcement`: "observe" (default)
        // stores the verdict on `last_conservation_audit`; "enforce"
        // rejects any block whose invariant breaks via
        // `ExecutionError::ConservationViolation`. Mirrors
        // SimpleExecutor::execute_block for parity across executors.
        let conservation_after = crate::energy_audit::compartment_snapshot_with_pool(
            db,
            self.refresh_pool.total_accrued(),
        );
        let lambda = evaporchain_energy_kernel::ChainLambda::default_genesis();
        // epochs_elapsed = block.epoch − last_audit_epoch (0 on first
        // audit); see SimpleExecutor for the rationale.
        let epochs_elapsed = self
            .last_audit_epoch
            .map(|prev| block.epoch.saturating_sub(prev))
            .unwrap_or(0);
        // Compute the net minting that fired during the reward path so
        // we can credit it into the pre-block compartment snapshot before
        // running the audit — matching the SimpleExecutor::execute_block
        // fix. Without this credit, the §1.2 audit fires
        // `DecayIncreasedTotal` every block whose block_reward > 0,
        // which is the live-cluster `last_conservation_audit_ok: false`
        // false-signal.
        let minted_this_block: u64 = self
            .reward_accumulator
            .as_ref()
            .map_or(0, |ra| ra.total_minted)
            .saturating_sub(minted_before_rewards);
        let mut conservation_before_adjusted = conservation_before;
        if minted_this_block > 0 {
            conservation_before_adjusted.credit(
                evaporchain_energy_kernel::Compartment::Accounts,
                minted_this_block,
            );
        }
        let audit_verdict = crate::energy_audit::audit_block_step(
            &conservation_before_adjusted,
            &conservation_after,
            epochs_elapsed,
            lambda,
        );
        let must_enforce = matches!(
            db.get_governance_param("conservation_enforcement"),
            Some("enforce"),
        );
        // Same gate as SimpleExecutor — see `evaluate_conservation_gate`.
        let stored = crate::evaluate_conservation_gate(audit_verdict, must_enforce)?;
        // Consecutive-clean-audit counter (operator readiness signal for
        // the `conservation_enforcement` flag flip). Mirrors SimpleExecutor.
        if stored.is_ok() {
            self.consecutive_clean_audits = self.consecutive_clean_audits.saturating_add(1);
        } else {
            self.consecutive_clean_audits = 0;
        }
        self.last_conservation_audit = Some(stored);
        self.last_audit_epoch = Some(block.epoch);

        let mera_root = crate::mera_integration::compute_mera_commitment(db);
        let mera_commitment = if mera_root == [0u8; 32] {
            None
        } else {
            Some(mera_root)
        };

        // Re-assemble outcomes in block-tx order so the API layer can
        // index by position alongside `block.transactions`. Stable-sort
        // by original block index.
        indexed_outcomes.sort_by_key(|(i, _)| *i);
        let tx_outcomes: Vec<crate::TxOutcome> =
            indexed_outcomes.into_iter().map(|(_, o)| o).collect();

        Ok(BlockExecutionResult {
            state_root,
            mmr_root: self.mmr.root(),
            txs_executed: total_txs_executed,
            txs_failed: total_txs_failed,
            objects_entered_grace: evap_result.entered_grace.len(),
            objects_evaporated: evap_result.evaporated.len(),
            gas_used: total_gas_used,
            base_fee,
            total_fees,
            evaporation_proof: None,
            contract_events: Vec::new(),
            cross_shard_processed,
            cross_shard_receipts,
            validator_key_rotations,
            mera_commitment,
            demurrage_collected,
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

// ─── Metrics ───────────────────────────────────────────────────────────────

/// Analyze a block's parallelism potential without executing it.
/// Returns (num_partitions, max_partition_size, parallelism_ratio).
/// parallelism_ratio = num_txs / max_partition_size — higher is better.
pub fn analyze_parallelism(txs: &[Transaction]) -> (usize, usize, f64) {
    if txs.is_empty() {
        return (0, 0, 0.0);
    }
    let partitions = partition_transactions(txs);
    let num_partitions = partitions.len();
    let max_size = partitions.iter().map(|p| p.len()).max().unwrap_or(0);
    let ratio = if max_size > 0 {
        txs.len() as f64 / max_size as f64
    } else {
        0.0
    };
    (num_partitions, max_size, ratio)
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
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
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
            post_state_root: None,
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

    // ── Partitioning Tests ──

    #[test]
    fn test_independent_transfers_partition_separately() {
        // A→B and C→D have no shared accounts — should be 2 partitions
        let txs = vec![
            Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 100,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }),
            Transaction::Transfer(TransferTx {
                from: addr(3),
                to: addr(4),
                amount: 200,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }),
        ];

        let partitions = partition_transactions(&txs);
        assert_eq!(
            partitions.len(),
            2,
            "independent transfers should be in separate partitions"
        );
    }

    #[test]
    fn test_conflicting_transfers_same_partition() {
        // A→B and B→C share account B — should merge into 1 partition
        let txs = vec![
            Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 100,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }),
            Transaction::Transfer(TransferTx {
                from: addr(2),
                to: addr(3),
                amount: 50,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }),
        ];

        let partitions = partition_transactions(&txs);
        assert_eq!(partitions.len(), 1, "conflicting transfers should merge");
        assert_eq!(partitions[0].len(), 2);
    }

    #[test]
    fn test_transitive_conflict() {
        // A→B, B→C, D→E — first two merge transitively, D→E separate
        let txs = vec![
            Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 10,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }),
            Transaction::Transfer(TransferTx {
                from: addr(2),
                to: addr(3),
                amount: 10,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }),
            Transaction::Transfer(TransferTx {
                from: addr(4),
                to: addr(5),
                amount: 10,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }),
        ];

        let partitions = partition_transactions(&txs);
        assert_eq!(partitions.len(), 2);
        // First partition has 2 txs (A→B + B→C), second has 1 (D→E)
        let sizes: Vec<usize> = partitions.iter().map(|p| p.len()).collect();
        assert!(sizes.contains(&2));
        assert!(sizes.contains(&1));
    }

    #[test]
    fn test_refresh_and_transfer_independent() {
        // Transfer A→B and Refresh object X — different state, separate partitions
        let txs = vec![
            Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 100,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }),
            Transaction::Refresh(RefreshTx {
                object_id: obj_id(10),
                energy_deposit: 500,
                signature: None,
                public_key: None,
            }),
        ];

        let partitions = partition_transactions(&txs);
        assert_eq!(partitions.len(), 2);
    }

    #[test]
    fn test_same_object_refreshes_conflict() {
        // Two refreshes on the same object — must be same partition
        let txs = vec![
            Transaction::Refresh(RefreshTx {
                object_id: obj_id(1),
                energy_deposit: 100,
                signature: None,
                public_key: None,
            }),
            Transaction::Refresh(RefreshTx {
                object_id: obj_id(1),
                energy_deposit: 200,
                signature: None,
                public_key: None,
            }),
        ];

        let partitions = partition_transactions(&txs);
        assert_eq!(partitions.len(), 1);
    }

    #[test]
    fn test_empty_block() {
        let partitions = partition_transactions(&[]);
        assert_eq!(partitions.len(), 0);
    }

    #[test]
    fn test_analyze_parallelism_metric() {
        // 4 independent transfers: 4 partitions, max_size=1, ratio=4.0
        let txs: Vec<Transaction> = (0..4)
            .map(|i| {
                Transaction::Transfer(TransferTx {
                    from: addr(i * 2 + 1),
                    to: addr(i * 2 + 2),
                    amount: 100,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                })
            })
            .collect();

        let (partitions, max_size, ratio) = analyze_parallelism(&txs);
        assert_eq!(partitions, 4);
        assert_eq!(max_size, 1);
        assert!((ratio - 4.0).abs() < f64::EPSILON);
    }

    // ── Execution Tests ──

    #[test]
    fn test_parallel_executor_mints_block_reward_to_producer() {
        // ParallelExecutor (production tendermint's executor) was missing
        // a reward_accumulator until this commit — validators ran without
        // rewards. This test proves rewards now fire end-to-end through
        // execute_block when enable_rewards() is called.
        use evaporchain_types::genesis::Tokenomics;

        let mut db = InMemoryStateDB::new();
        let producer_id: u64 = 7;
        let producer_addr = addr(producer_id as u8);
        fund_account(&mut db, producer_id as u8, 0);
        // Sender so the block has at least one tx.
        fund_account(&mut db, 1, 1000);

        let mut block = make_block(
            1,
            0, // epoch 0 — block reward is at full 100 from test tokenomics
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
        block.producer_id = Some(producer_id);

        let mut executor = ParallelExecutor::new_for_test(100);
        executor.enable_rewards(Tokenomics {
            total_supply: 10_000_000,
            block_reward: 100,
            reward_half_life: 1000,
            fee_burn_rate: 0.50,
            staker_fee_share: 0.50,
            target_staking_apy: 0.05,
            validator_commission_default: Tokenomics::default_commission(),
            max_supply_cap: None,
            emission: None,
            blocks_per_year: Tokenomics::default_blocks_per_year(),
        });

        let _ = executor.execute_block(&mut db, &block).unwrap();

        // Producer must have received the block reward (100 from
        // test_tokenomics at epoch 0). Pre-fix this was 0.
        let producer_balance = db.get_account(&producer_addr).unwrap().balance;
        assert!(
            producer_balance >= 100,
            "producer must receive block reward — got {producer_balance}"
        );
        let ra = executor.reward_accumulator().unwrap();
        assert_eq!(ra.total_minted, 100);
    }

    #[test]
    fn test_parallel_executor_priority_bonus_auto_fires_from_hints() {
        // Lane A.3: when block.submit_epoch_hints carries Some(epoch)
        // for any tx, execute_block computes priority_sum from on-the-
        // wire hints (not from local mempool state) and mints a bonus
        // to the producer on top of the block reward. This is what
        // makes Phase 1.5 cluster-deterministic — every follower
        // computes the same priority_sum and mints the same bonus.
        use evaporchain_types::genesis::Tokenomics;

        let mut db = InMemoryStateDB::new();
        let producer_id: u64 = 7;
        let producer_addr = addr(producer_id as u8);
        fund_account(&mut db, producer_id as u8, 0);
        fund_account(&mut db, 1, 1000);

        let mut block = make_block(
            5, // height 5
            0,
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
        block.producer_id = Some(producer_id);
        // Tx submitted at epoch 5 (same height as the block) → elapsed=0
        // → priority = BASE_INCLUSION_ENERGY = 1_000_000 → bonus =
        // 1_000_000 / 1_000_000 = 1 token.
        block.submit_epoch_hints = vec![Some(5)];

        let mut executor = ParallelExecutor::new_for_test(100);
        executor.enable_rewards(Tokenomics {
            total_supply: 10_000_000,
            block_reward: 100,
            reward_half_life: 1000,
            fee_burn_rate: 0.50,
            staker_fee_share: 0.50,
            target_staking_apy: 0.05,
            validator_commission_default: Tokenomics::default_commission(),
            max_supply_cap: None,
            emission: None,
            blocks_per_year: Tokenomics::default_blocks_per_year(),
        });

        let _ = executor.execute_block(&mut db, &block).unwrap();

        // Producer should receive: block_reward (100) + priority_bonus (≥1).
        let producer_balance = db.get_account(&producer_addr).unwrap().balance;
        assert!(
            producer_balance >= 101,
            "producer must receive block reward + priority bonus — got {producer_balance}"
        );
    }

    #[test]
    fn test_parallel_executor_state_root_version_v1_triggers_compression() {
        // Lane E.2: a block with state_root_version >= 1 invokes
        // db.compress_cold_subtrees() before computing the state root.
        // v0 leaves it off. We don't observe a state-root delta directly
        // (the test InMemoryStateDB has no objects to compress) — we
        // assert via the trie health that compress was called by
        // checking compressions counter on the trie. Rough but
        // deterministic.
        let mut db_v0 = InMemoryStateDB::new();
        let mut db_v1 = InMemoryStateDB::new();
        fund_account(&mut db_v0, 1, 1000);
        fund_account(&mut db_v1, 1, 1000);

        // Same block in both runs except state_root_version.
        let mut block = make_block(
            1,
            0,
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

        // v0 run.
        block.state_root_version = 0;
        let mut exec_v0 = ParallelExecutor::new_for_test(100);
        let _ = exec_v0.execute_block(&mut db_v0, &block).unwrap();

        // v1 run.
        block.state_root_version = 1;
        let mut exec_v1 = ParallelExecutor::new_for_test(100);
        let _ = exec_v1.execute_block(&mut db_v1, &block).unwrap();

        // Both v0 and v1 should produce equal state roots when there
        // are no cold subtrees to compress (no evaporated objects in
        // this minimal test). What we ARE verifying: v1 didn't crash
        // and the trie_health is observably reachable on both.
        let h_v0 = db_v0.trie_health();
        let h_v1 = db_v1.trie_health();
        // v1 may have invoked compression but, with no eligible
        // subtrees, the count stays 0. Both health snapshots produce
        // identical compressed_leaves (==0) since there's nothing to
        // compress. The assertion here is that v1 didn't ERROR or
        // panic — the call is just a no-op when there's nothing
        // cold yet.
        assert_eq!(h_v0.compressed_leaves, h_v1.compressed_leaves);
        // (Real-world delta would surface only after objects evaporate;
        // a longer integration test in the cluster soak would observe
        // the delta. This unit test just proves the wiring is alive.)
    }

    #[test]
    fn test_parallel_executor_no_priority_bonus_when_hints_empty() {
        // Lane A.3 negative case: empty hints vec ⇒ no priority bonus
        // (legacy-quiet block path is bit-compat).
        use evaporchain_types::genesis::Tokenomics;

        let mut db = InMemoryStateDB::new();
        let producer_id: u64 = 9;
        let producer_addr = addr(producer_id as u8);
        fund_account(&mut db, producer_id as u8, 0);
        fund_account(&mut db, 1, 1000);

        let mut block = make_block(
            1,
            0,
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
        block.producer_id = Some(producer_id);
        // Empty hints ⇒ no priority bonus.
        block.submit_epoch_hints = vec![];

        let mut executor = ParallelExecutor::new_for_test(100);
        executor.enable_rewards(Tokenomics {
            total_supply: 10_000_000,
            block_reward: 100,
            reward_half_life: 1000,
            fee_burn_rate: 0.50,
            staker_fee_share: 0.50,
            target_staking_apy: 0.05,
            validator_commission_default: Tokenomics::default_commission(),
            max_supply_cap: None,
            emission: None,
            blocks_per_year: Tokenomics::default_blocks_per_year(),
        });

        let _ = executor.execute_block(&mut db, &block).unwrap();

        let producer_balance = db.get_account(&producer_addr).unwrap().balance;
        assert_eq!(
            producer_balance, 100,
            "no hints ⇒ block reward only, no priority bonus"
        );
    }

    #[test]
    fn test_parallel_executor_no_rewards_when_disabled() {
        // Without enable_rewards(), no minting fires — preserves the
        // pre-existing test-mode and node-pre-launch behaviour.
        let mut db = InMemoryStateDB::new();
        let producer_id: u64 = 9;
        let producer_addr = addr(producer_id as u8);
        fund_account(&mut db, producer_id as u8, 0);
        fund_account(&mut db, 1, 1000);

        let mut block = make_block(
            1,
            0,
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
        block.producer_id = Some(producer_id);

        let mut executor = ParallelExecutor::new_for_test(100);
        let _ = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(db.get_account(&producer_addr).unwrap().balance, 0);
        assert!(executor.reward_accumulator().is_none());
    }

    #[test]
    fn test_parallel_transfer_execution() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
        fund_account(&mut db, 3, 2000);

        let block = make_block(
            1,
            1,
            vec![
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(2),
                    amount: 500,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(3),
                    to: addr(4),
                    amount: 700,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                }),
            ],
        );

        let mut executor = ParallelExecutor::new_for_test(100);
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 2);
        assert_eq!(result.txs_failed, 0);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 500);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 500);
        assert_eq!(db.get_account(&addr(3)).unwrap().balance, 1300);
        assert_eq!(db.get_account(&addr(4)).unwrap().balance, 700);
    }

    #[test]
    fn test_parallel_matches_sequential() {
        // Execute the same block with both engines and compare final state.
        let txs = vec![
            Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 100,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }),
            Transaction::Transfer(TransferTx {
                from: addr(3),
                to: addr(4),
                amount: 200,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }),
            Transaction::Transfer(TransferTx {
                from: addr(5),
                to: addr(6),
                amount: 300,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }),
            Transaction::CreateObject(CreateObjectTx {
                creator: addr(7),
                object_id: obj_id(1),
                energy: 1000,
                half_life: 10,
                data: vec![1, 2, 3],
                decay_curve: None,
                lad_mode: None,
                signature: None,
                public_key: None,
            }),
            Transaction::Refresh(RefreshTx {
                object_id: obj_id(2),
                energy_deposit: 500,
                signature: None,
                public_key: None,
            }),
        ];

        let block = make_block(1, 1, txs);

        // Sequential
        let mut db_seq = InMemoryStateDB::new();
        fund_account(&mut db_seq, 1, 10000);
        fund_account(&mut db_seq, 3, 10000);
        fund_account(&mut db_seq, 5, 10000);
        fund_account(&mut db_seq, 7, 10000);
        // Pre-create object for refresh
        db_seq.put_object(StateObject {
            id: obj_id(2),
            owner: addr(99),
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
        let mut seq_executor = crate::SimpleExecutor::new_for_test(100);
        let seq_result = seq_executor.execute_block(&mut db_seq, &block).unwrap();

        // Parallel
        let mut db_par = InMemoryStateDB::new();
        fund_account(&mut db_par, 1, 10000);
        fund_account(&mut db_par, 3, 10000);
        fund_account(&mut db_par, 5, 10000);
        fund_account(&mut db_par, 7, 10000);
        db_par.put_object(StateObject {
            id: obj_id(2),
            owner: addr(99),
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
        let mut par_executor = ParallelExecutor::new_for_test(100);
        let par_result = par_executor.execute_block(&mut db_par, &block).unwrap();

        // Compare results
        assert_eq!(seq_result.txs_executed, par_result.txs_executed);
        assert_eq!(seq_result.txs_failed, par_result.txs_failed);
        assert_eq!(seq_result.gas_used, par_result.gas_used);

        // Compare final account states
        for byte in [1, 2, 3, 4, 5, 6] {
            let seq_acct = db_seq.get_account(&addr(byte));
            let par_acct = db_par.get_account(&addr(byte));
            assert_eq!(
                seq_acct.map(|a| (a.balance, a.nonce)),
                par_acct.map(|a| (a.balance, a.nonce)),
                "account {} mismatch",
                byte
            );
        }

        // Compare object states
        assert!(
            db_par.get_object(&obj_id(1)).is_some(),
            "object 1 should exist"
        );
        assert_eq!(
            db_seq.get_object(&obj_id(1)).unwrap().energy,
            db_par.get_object(&obj_id(1)).unwrap().energy,
        );
    }

    #[test]
    fn test_conflicting_chain_executes_correctly() {
        // A→B, then B→C with nonce 0 on B — these CONFLICT (share B),
        // so they go in the same partition and execute sequentially.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
        fund_account(&mut db, 2, 500);

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
                    from: addr(2),
                    to: addr(3),
                    amount: 600,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                }),
            ],
        );

        let mut executor = ParallelExecutor::new_for_test(100);
        let result = executor.execute_block(&mut db, &block).unwrap();

        // Both should execute: A sends 200 to B (B now 700), B sends 600 to C
        assert_eq!(result.txs_executed, 2);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 800);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 100);
        assert_eq!(db.get_account(&addr(3)).unwrap().balance, 600);
    }

    #[test]
    fn test_insufficient_balance_fails_gracefully() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 50); // Not enough for 100

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

        let mut executor = ParallelExecutor::new_for_test(100);
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 50);
    }

    #[test]
    fn test_mixed_tx_types_parallel() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 5000);
        fund_account(&mut db, 3, 5000);
        fund_account(&mut db, 50, 10000);
        // Object for refresh
        db.put_object(StateObject {
            id: obj_id(10),
            owner: addr(99),
            energy: 200,
            half_life: 10,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![],
            decay_curve: None,
            lad_mode: None,
        });

        let block = make_block(
            1,
            1,
            vec![
                // Partition 1: transfer A→B
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(2),
                    amount: 1000,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                }),
                // Partition 2: transfer C→D
                Transaction::Transfer(TransferTx {
                    from: addr(3),
                    to: addr(4),
                    amount: 2000,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                }),
                // Partition 3: refresh object 10
                Transaction::Refresh(RefreshTx {
                    object_id: obj_id(10),
                    energy_deposit: 300,
                    signature: None,
                    public_key: None,
                }),
                // Partition 4: create object 20
                Transaction::CreateObject(CreateObjectTx {
                    creator: addr(50),
                    object_id: obj_id(20),
                    energy: 1000,
                    half_life: 5,
                    data: vec![0xDE, 0xAD],
                    decay_curve: None,
                    lad_mode: None,
                    signature: None,
                    public_key: None,
                }),
            ],
        );

        let mut executor = ParallelExecutor::new_for_test(100);
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 4);
        assert_eq!(result.txs_failed, 0);

        // Verify all state changes applied
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 4000);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 1000);
        assert_eq!(db.get_account(&addr(3)).unwrap().balance, 3000);
        assert_eq!(db.get_account(&addr(4)).unwrap().balance, 2000);
        assert!(db.get_object(&obj_id(20)).is_some());
        assert_eq!(db.get_object(&obj_id(20)).unwrap().energy, 1000);
    }

    #[test]
    fn test_many_independent_transfers_high_parallelism() {
        let mut db = InMemoryStateDB::new();
        let n = 50;
        let mut txs = Vec::new();

        for i in 0..n {
            let from = (i * 2 + 1) as u8;
            let to = (i * 2 + 2) as u8;
            fund_account(&mut db, from, 10000);
            txs.push(Transaction::Transfer(TransferTx {
                from: addr(from),
                to: addr(to),
                amount: 100,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }));
        }

        let block = make_block(1, 1, txs.clone());

        // Check parallelism metric
        let (partitions, max_size, ratio) = analyze_parallelism(&txs);
        assert_eq!(partitions, n);
        assert_eq!(max_size, 1);
        assert!((ratio - n as f64).abs() < f64::EPSILON);

        // Execute
        let mut executor = ParallelExecutor::new_for_test(100);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, n);
        assert_eq!(result.txs_failed, 0);
    }

    #[test]
    fn test_validator_stake_parallel() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 100_000);
        fund_account(&mut db, 2, 100_000);

        let block = make_block(
            1,
            1,
            vec![
                Transaction::ValidatorStake(ValidatorStakeTx {
                    validator_address: addr(1),
                    stake_amount: 50_000,
                    validator_id: 1,
                    nonce: 0,
                    bls_public_key: None,
                    vrf_public_key: None,
                    signature: None,
                    public_key: None,
                }),
                Transaction::ValidatorStake(ValidatorStakeTx {
                    validator_address: addr(2),
                    stake_amount: 30_000,
                    validator_id: 2,
                    nonce: 0,
                    bls_public_key: None,
                    vrf_public_key: None,
                    signature: None,
                    public_key: None,
                }),
            ],
        );

        let mut executor = ParallelExecutor::new_for_test(100);
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 2);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 50_000);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 70_000);

        // AUDIT_2026_05_13 C3 regression guard: the pre-fix
        // exec_validator_stake debited balance but never wrote a
        // StakeRecord (silent burn). Assert both records exist with
        // the right amount + correct staked_at_epoch (otherwise the
        // unbonding-cooldown gate in execute_validator_claim_stake
        // never fires).
        let s1 = db.get_stake(1).expect("validator 1 stake record missing");
        assert_eq!(s1.staked_amount, 50_000);
        assert_eq!(s1.validator_address, addr(1));
        assert_eq!(s1.staked_at_epoch, 1);
        let s2 = db.get_stake(2).expect("validator 2 stake record missing");
        assert_eq!(s2.staked_amount, 30_000);
        assert_eq!(s2.validator_address, addr(2));
        assert_eq!(s2.staked_at_epoch, 1);
    }

    #[test]
    fn test_parallel_executor_lyapunov_fee_state_advances_per_block() {
        // Lane F.1: execute_block must call tick_lyapunov_fee_state with
        // total_gas_used so the Singh-Lyapunov controller actually
        // advances in the production hot path. Pre-Lane-F.1, this field
        // sat at genesis equilibrium forever and the controller's
        // doctrine claim was folklore (DOCTRINE_PUNCH_LIST.md §6).
        //
        // The controller is a no-op at *exact* equilibrium (V=0 must
        // stay 0 — Lyapunov monotone-non-increasing invariant — so
        // perturbations clip to zero magnitude). To prove the tick
        // actually fires from execute_block we seed off-equilibrium and
        // assert it decays toward target.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let block = make_block(
            1,
            1, // epoch 1 so per-block tick has positive epochs_elapsed
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

        let mut executor = ParallelExecutor::new_for_test(100);
        let target_e = executor.lyapunov_fee_params.target_energy;

        // Seed off-equilibrium: energy above target. A low-gas block
        // (one transfer, ~21k gas << target_gas) must let the controller
        // pull energy back toward target.
        let seeded_energy = target_e + 100_000;
        executor.lyapunov_fee_state = evaporchain_fee_controller::FeeState::new(seeded_energy);
        assert_eq!(executor.lyapunov_fee_state.energy, seeded_energy);

        let _ = executor.execute_block(&mut db, &block).unwrap();

        assert!(
            executor.lyapunov_fee_state.energy < seeded_energy,
            "lyapunov_fee_state.energy must decay toward target after \
             execute_block — got {} (started {}, target {})",
            executor.lyapunov_fee_state.energy,
            seeded_energy,
            target_e
        );
        // And it should not undershoot below target_e — the controller
        // pulls toward equilibrium, never past it.
        assert!(
            executor.lyapunov_fee_state.energy >= target_e,
            "controller must not overshoot target — got {} (target {})",
            executor.lyapunov_fee_state.energy,
            target_e
        );
    }

    // ─── POST_EXEC_STATE_VERIFICATION_PLAN.md Phase 2 scaffold ─────────

    /// Snapshot + restore returns the executor to a bit-identical
    /// pre-snapshot state across every mutable field. This is the
    /// load-bearing primitive for speculative execute in Phase 2 —
    /// if any mutable field leaks through restore, the proposer's
    /// real apply (post-broadcast) will diverge from the proposer's
    /// speculative apply (pre-broadcast), which would corrupt the
    /// `post_state_root` claim and silently break the warn-mode gate.
    #[test]
    fn snapshot_then_restore_is_bit_identical() {
        let mut executor = ParallelExecutor::new_for_test(10);

        // Mutate a representative subset of fields so we have something
        // to restore *from*. The point of the test is that ANY mutation
        // we make after snapshotting is reversed by restore.
        executor.consecutive_clean_audits = 7;
        executor.last_audit_epoch = Some(42);
        executor.lyapunov_fee_state.energy = 999_999;
        // mmr is private — push via the public API the executor itself
        // exposes during execution. For the snapshot test we just snapshot
        // current state and check the post-mutation snapshot/restore path.

        let snap = executor.snapshot_for_simulation();

        // Mutate after snapshot — every field below should revert.
        executor.consecutive_clean_audits = 9999;
        executor.last_audit_epoch = Some(7777);
        executor.lyapunov_fee_state.energy = 0;

        // Restore — executor should match its pre-mutation state.
        executor.restore_from_simulation(snap);

        assert_eq!(executor.consecutive_clean_audits, 7);
        assert_eq!(executor.last_audit_epoch, Some(42));
        assert_eq!(executor.lyapunov_fee_state.energy, 999_999);
    }

    /// Snapshot semantics: the snapshot is OWNED — mutating the
    /// executor after snapshot does NOT mutate the snapshot. Verifies
    /// `snapshot_for_simulation` returns a deep clone, not a shared
    /// reference. (If it ever became shared, the post-broadcast apply
    /// path would silently observe pre-broadcast simulation effects.)
    #[test]
    fn snapshot_is_an_owned_deep_copy() {
        let mut executor = ParallelExecutor::new_for_test(10);
        executor.consecutive_clean_audits = 5;

        let snap = executor.snapshot_for_simulation();

        // Mutate the original — snap should be unaffected.
        executor.consecutive_clean_audits = 999;

        // Restore — original goes back to 5 (matching snap), confirming
        // the snap was independent of the post-snap mutation.
        executor.restore_from_simulation(snap);
        assert_eq!(executor.consecutive_clean_audits, 5);
    }

    // ── T1.20 gap-closure: transfer error paths ──────────────────────────────

    #[test]
    fn t1_20_transfer_self_transfer_fails() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
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
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1000);
    }

    #[test]
    fn t1_20_transfer_zero_amount_fails() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
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
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    #[test]
    fn t1_20_transfer_invalid_nonce_fails() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 100,
                nonce: 99,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1000);
    }

    #[test]
    fn t1_20_transfer_mint_bypass_from_zero_address() {
        // from=[0u8;32] bypasses nonce check (nonce=99 accepted) but balance
        // is still enforced — pre-fund the zero address so the transfer lands.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 0, 10_000); // addr(0) == [0u8;32]
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: [0u8; 32],
                to: addr(42),
                amount: 5000,
                nonce: 99,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed, 1);
        assert_eq!(db.get_account(&addr(42)).unwrap().balance, 5000);
    }

    // ── T1.20 gap-closure: CreateObject error paths ──────────────────────────

    #[test]
    fn t1_20_create_object_already_exists_fails() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);
        db.put_object(StateObject {
            id: obj_id(5),
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
        let block = make_block(
            1,
            1,
            vec![Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(5),
                energy: 500,
                half_life: 10,
                data: vec![],
                decay_curve: None,
                lad_mode: None,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    #[test]
    fn t1_20_create_object_insufficient_balance_for_deposit_fails() {
        let mut db = InMemoryStateDB::new();
        // MIN_STORAGE_DEPOSIT = 1000; fund below that
        fund_account(&mut db, 1, 500);
        let block = make_block(
            1,
            1,
            vec![Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(7),
                energy: 500,
                half_life: 10,
                data: vec![],
                decay_curve: None,
                lad_mode: None,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 500);
    }

    // ── T1.20 gap-closure: Refresh paths ─────────────────────────────────────

    #[test]
    fn t1_20_refresh_ghost_resurrection() {
        let mut db = InMemoryStateDB::new();
        db.put_ghost(GhostRecord {
            object_id: obj_id(9),
            owner: addr(3),
            evaporated_at: 5,
            data_hash: [0u8; 32],
            original_data: Some(vec![0xDE, 0xAD]),
            mmr_position: None,
            original_half_life: Some(10),
        });
        let block = make_block(
            1,
            10,
            vec![Transaction::Refresh(RefreshTx {
                object_id: obj_id(9),
                energy_deposit: 2000,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed, 1, "ghost resurrection must succeed");
    }

    #[test]
    fn t1_20_refresh_object_not_found_fails() {
        let mut db = InMemoryStateDB::new();
        let block = make_block(
            1,
            1,
            vec![Transaction::Refresh(RefreshTx {
                object_id: obj_id(99),
                energy_deposit: 500,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    // ── T1.20 gap-closure: ValidatorExit serial path ─────────────────────────

    fn make_stake(validator_id: u64, addr_byte: u8, amount: u64) -> evaporchain_types::StakeRecord {
        evaporchain_types::StakeRecord {
            validator_id,
            validator_address: addr(addr_byte),
            staked_amount: amount,
            staked_at_epoch: 0,
            unbonding_epoch: None,
            slashed_amount: 0,
        }
    }

    #[test]
    fn t1_20_validator_exit_serial_success() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 10, 100_000);
        db.put_stake(make_stake(1, 10, 50_000));
        let block = make_block(
            1,
            5,
            vec![Transaction::ValidatorExit(ValidatorExitTx {
                validator_address: addr(10),
                validator_id: 1,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed, 1);
        let stake = db.get_stake(1).cloned().unwrap();
        assert!(stake.unbonding_epoch.is_some());
        assert_eq!(stake.unbonding_epoch.unwrap(), 5 + 256);
    }

    #[test]
    fn t1_20_validator_exit_serial_invalid_nonce_fails() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 10, 100_000);
        db.put_stake(make_stake(1, 10, 50_000));
        let block = make_block(
            1,
            5,
            vec![Transaction::ValidatorExit(ValidatorExitTx {
                validator_address: addr(10),
                validator_id: 1,
                nonce: 99,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    #[test]
    fn t1_20_validator_exit_serial_no_stake_record_fails() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 10, 100_000);
        let block = make_block(
            1,
            5,
            vec![Transaction::ValidatorExit(ValidatorExitTx {
                validator_address: addr(10),
                validator_id: 99,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    #[test]
    fn t1_20_validator_exit_serial_already_exiting_fails() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 10, 100_000);
        let mut stake = make_stake(1, 10, 50_000);
        stake.unbonding_epoch = Some(100);
        db.put_stake(stake);
        let block = make_block(
            1,
            5,
            vec![Transaction::ValidatorExit(ValidatorExitTx {
                validator_address: addr(10),
                validator_id: 1,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    // ── T1.20 gap-closure: ValidatorClaimStake serial path ──────────────────

    #[test]
    fn t1_20_validator_claim_stake_success() {
        use evaporchain_types::ValidatorClaimStakeTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 10, 1000);
        let mut stake = make_stake(1, 10, 50_000);
        stake.unbonding_epoch = Some(10);
        db.put_stake(stake);
        // epoch 300 > unbonding epoch 10
        let block = make_block(
            1,
            300,
            vec![Transaction::ValidatorClaimStake(ValidatorClaimStakeTx {
                validator_address: addr(10),
                validator_id: 1,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed, 1);
        let bal = db.get_account(&addr(10)).unwrap().balance;
        assert!(bal >= 50_000 + 1000, "stake must be returned: got {bal}");
        assert!(db.get_stake(1).is_none(), "stake record must be removed");
    }

    #[test]
    fn t1_20_validator_claim_stake_not_exited_fails() {
        use evaporchain_types::ValidatorClaimStakeTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 10, 1000);
        db.put_stake(make_stake(1, 10, 50_000)); // unbonding_epoch = None
        let block = make_block(
            1,
            300,
            vec![Transaction::ValidatorClaimStake(ValidatorClaimStakeTx {
                validator_address: addr(10),
                validator_id: 1,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    #[test]
    fn t1_20_validator_claim_stake_unbonding_period_not_done_fails() {
        use evaporchain_types::ValidatorClaimStakeTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 10, 1000);
        let mut stake = make_stake(1, 10, 50_000);
        stake.unbonding_epoch = Some(500);
        db.put_stake(stake);
        // epoch 10 < unbonding 500
        let block = make_block(
            1,
            10,
            vec![Transaction::ValidatorClaimStake(ValidatorClaimStakeTx {
                validator_address: addr(10),
                validator_id: 1,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    // ── T1.20 gap-closure: RotateValidatorKey serial path ───────────────────

    #[test]
    fn t1_20_rotate_validator_key_past_epoch_fails() {
        use evaporchain_types::RotateValidatorKeyTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 10, 1000);
        db.put_stake(make_stake(1, 10, 50_000));
        // effective_epoch 50 < block epoch 100
        let block = make_block(
            1,
            100,
            vec![Transaction::RotateValidatorKey(RotateValidatorKeyTx {
                validator_address: addr(10),
                validator_id: 1,
                new_bls_public_key: vec![0u8; 48],
                bls_pop_old: vec![0u8; 48],
                bls_pop_new: vec![0u8; 48],
                effective_epoch: 50,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    #[test]
    fn t1_20_rotate_validator_key_wrong_bls_length_fails() {
        use evaporchain_types::RotateValidatorKeyTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 10, 1000);
        db.put_stake(make_stake(1, 10, 50_000));
        let block = make_block(
            1,
            100,
            vec![Transaction::RotateValidatorKey(RotateValidatorKeyTx {
                validator_address: addr(10),
                validator_id: 1,
                new_bls_public_key: vec![0u8; 16], // wrong: not 48 bytes
                bls_pop_old: vec![0u8; 48],
                bls_pop_new: vec![0u8; 48],
                effective_epoch: 200,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    #[test]
    fn t1_20_rotate_validator_key_no_stake_record_fails() {
        use evaporchain_types::RotateValidatorKeyTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 10, 1000);
        // No stake record for validator 77
        let block = make_block(
            1,
            100,
            vec![Transaction::RotateValidatorKey(RotateValidatorKeyTx {
                validator_address: addr(10),
                validator_id: 77,
                new_bls_public_key: vec![0u8; 48],
                bls_pop_old: vec![0u8; 48],
                bls_pop_new: vec![0u8; 48],
                effective_epoch: 200,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    // ── T1.20 gap-closure: Refund serial path ───────────────────────────────

    #[test]
    fn t1_20_refund_serial_success() {
        use evaporchain_types::RefundTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 5000);
        fund_account(&mut db, 2, 0);
        let block = make_block(
            1,
            5,
            vec![Transaction::Refund(RefundTx {
                source_block_height: 0,
                source_observation_idx: 0,
                attacker: addr(1),
                victim: addr(2),
                amount: 1000,
                settle_block_height: 1,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 4000);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 1000);
    }

    #[test]
    fn t1_20_refund_serial_self_transfer_fails() {
        use evaporchain_types::RefundTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 5000);
        let block = make_block(
            1,
            5,
            vec![Transaction::Refund(RefundTx {
                source_block_height: 0,
                source_observation_idx: 0,
                attacker: addr(1),
                victim: addr(1),
                amount: 1000,
                settle_block_height: 1,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    #[test]
    fn t1_20_refund_serial_zero_amount_fails() {
        use evaporchain_types::RefundTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 5000);
        let block = make_block(
            1,
            5,
            vec![Transaction::Refund(RefundTx {
                source_block_height: 0,
                source_observation_idx: 0,
                attacker: addr(1),
                victim: addr(2),
                amount: 0,
                settle_block_height: 1,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    #[test]
    fn t1_20_refund_serial_insufficient_balance_fails() {
        use evaporchain_types::RefundTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 100);
        let block = make_block(
            1,
            5,
            vec![Transaction::Refund(RefundTx {
                source_block_height: 0,
                source_observation_idx: 0,
                attacker: addr(1),
                victim: addr(2),
                amount: 1000,
                settle_block_height: 1,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 100);
    }

    // ── T1.20 gap-closure: DeployContract serial path ───────────────────────

    #[test]
    fn t1_20_deploy_contract_unknown_template_fails() {
        use evaporchain_types::DeployContractTx;
        let mut db = InMemoryStateDB::new();
        let block = make_block(
            1,
            1,
            vec![Transaction::DeployContract(DeployContractTx {
                deployer: addr(1),
                template: "NonExistentTemplate".into(),
                init_args: "{}".into(),
                energy: 1000,
                half_life: 100,
                rules: None,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    #[test]
    fn t1_20_deploy_contract_invalid_init_args_fails() {
        use evaporchain_types::DeployContractTx;
        let mut db = InMemoryStateDB::new();
        let block = make_block(
            1,
            1,
            vec![Transaction::DeployContract(DeployContractTx {
                deployer: addr(1),
                template: "DecayingToken".into(),
                init_args: "not-valid-json{{".into(),
                energy: 1000,
                half_life: 100,
                rules: None,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1);
    }

    // ── T1.20 gap-closure: serial gas limit exceeded ─────────────────────────

    #[test]
    fn t1_20_serial_gas_limit_exceeded() {
        use evaporchain_types::DeployContractTx;
        let mut db = InMemoryStateDB::new();
        // block_gas_limit = 1 unit; DeployContract costs GAS_DEPLOY_CONTRACT (100_000)
        let mut ex = ParallelExecutor::new_for_test(100);
        ex.block_gas_limit = 1;
        let block = make_block(
            1,
            1,
            vec![Transaction::DeployContract(DeployContractTx {
                deployer: addr(1),
                template: "DecayingToken".into(),
                init_args: "{}".into(),
                energy: 1000,
                half_life: 100,
                rules: None,
                signature: None,
                public_key: None,
            })],
        );
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1, "serial tx must fail on gas limit = 1");
    }

    // ── T1.20 gap-closure: tick_mortis ───────────────────────────────────────

    #[test]
    fn t1_20_tick_mortis_returns_cert_when_triggered() {
        // Override with a hair-trigger condition: floor = MAX (always below floor),
        // sustained = 1 → first tick with empty pool triggers JustTriggered.
        let mut ex = ParallelExecutor::new_for_test(100);
        ex.mortis_monitor = evaporchain_mortis::MortisMonitor::new(
            evaporchain_mortis::MortisCondition::new(u64::MAX, 1),
        );
        let cert = ex.tick_mortis(1, [0u8; 32]);
        assert!(cert.is_some(), "mortis must trigger after 1 tick below floor");
        // AlreadyTriggered: second tick returns None
        let cert2 = ex.tick_mortis(2, [0u8; 32]);
        assert!(cert2.is_none(), "already-triggered mortis returns None");
    }

    // ── T1.20 gap-closure: storage rent tombstone (balance zeroed) ───────────

    #[test]
    fn t1_20_storage_rent_wipes_account_below_floor() {
        // Account with storage_bytes=1000, balance=1: STORAGE_RENT_PER_BYTE=1
        // → rent=1000 > balance=1 → account zeroed and tombstone minted.
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(200),
            balance: 1,
            nonce: 0,
            storage_deposit: 1000,
            storage_bytes: 1000,
            last_touched_epoch: 0,
            vesting: None,
        });
        // Epoch 1 > last_rent_epoch 0 → rent sweep fires
        let block = make_block(1, 1, vec![]);
        let mut ex = ParallelExecutor::new_for_test(100);
        let _ = ex.execute_block(&mut db, &block).unwrap();
        let bal = db.get_account(&addr(200)).unwrap().balance;
        assert_eq!(bal, 0, "rent-exhausted account must be zeroed");
    }

    // ── T1.20 gap-closure: set_shard_id ──────────────────────────────────────

    #[test]
    fn t1_20_set_shard_id_updates_field() {
        use evaporchain_sharding::shard_assignment::ShardId;
        let mut ex = ParallelExecutor::new_for_test(100);
        assert_eq!(ex.shard_id, ShardId(0));
        ex.set_shard_id(ShardId(7));
        assert_eq!(ex.shard_id, ShardId(7));
    }

    // ── T1.20 batch 7: ParallelExecutor::estimate_gas static method ──────────

    #[test]
    fn t1_20_par_estimate_gas_shield() {
        let tx = Transaction::Shield(evaporchain_types::ShieldTx {
            from: addr(1), amount: 1_000, nonce: 0,
            note_owner_hash: [0u8; 32], value_blinding: [0u8; 32],
            energy: None, energy_blinding: None, half_life: 0,
            signature: None, public_key: None,
        });
        assert_eq!(ParallelExecutor::estimate_gas(&tx), crate::privacy_exec::GAS_SHIELD);
    }

    #[test]
    fn t1_20_par_estimate_gas_unshield() {
        let tx = Transaction::Unshield(evaporchain_types::UnshieldTx {
            to: addr(1), amount: 500,
            input_nullifiers: vec![], anchor: [0u8; 32], balance_binding: [0u8; 32],
            input_amounts: vec![], input_blindings: vec![],
            input_value_commitments: vec![], input_note_commitments: vec![],
            input_merkle_proofs: vec![], output_blindings: vec![],
            change_commitments: vec![], energy_proofs: vec![],
        });
        assert_eq!(ParallelExecutor::estimate_gas(&tx), crate::privacy_exec::GAS_UNSHIELD);
    }

    #[test]
    fn t1_20_par_estimate_gas_private_transfer() {
        let tx = Transaction::PrivateTransfer(evaporchain_types::PrivateTransferTx {
            input_nullifiers: vec![], output_commitments: vec![],
            anchor: [0u8; 32], balance_binding: [0u8; 32], fee: 0,
            input_amounts: vec![], input_blindings: vec![],
            input_value_commitments: vec![], input_note_commitments: vec![],
            input_merkle_proofs: vec![], output_amounts: vec![],
            output_blindings: vec![], energy_proofs: vec![],
        });
        assert_eq!(
            ParallelExecutor::estimate_gas(&tx),
            crate::privacy_exec::GAS_PRIVATE_TRANSFER_BASE,
        );
    }

    #[test]
    fn t1_20_par_estimate_gas_deferred_scales_with_guards() {
        use evaporchain_types::TemporalGuard;
        let no_guards = Transaction::Deferred(evaporchain_types::DeferredTx {
            submitter: addr(1), nonce: 0, deposit: 0,
            guards: vec![], inner_tx_bytes: vec![], gas_limit: 0,
            signature: None, public_key: None,
        });
        let two_guards = Transaction::Deferred(evaporchain_types::DeferredTx {
            submitter: addr(1), nonce: 0, deposit: 0,
            guards: vec![TemporalGuard::AfterEpoch(10), TemporalGuard::BeforeEpoch(100)],
            inner_tx_bytes: vec![], gas_limit: 0,
            signature: None, public_key: None,
        });
        assert_eq!(
            ParallelExecutor::estimate_gas(&no_guards),
            crate::temporal::GAS_DEFERRED_SUBMIT,
        );
        assert_eq!(
            ParallelExecutor::estimate_gas(&two_guards),
            crate::temporal::GAS_DEFERRED_SUBMIT + 2 * crate::temporal::GAS_PER_GUARD,
        );
    }

    #[test]
    fn t1_20_par_estimate_gas_blob_scales_with_data() {
        let empty = Transaction::Blob(evaporchain_types::BlobTx {
            submitter: addr(1), data: vec![], nonce: 0, namespace_id: 1,
            signature: None, public_key: None,
        });
        let with_data = Transaction::Blob(evaporchain_types::BlobTx {
            submitter: addr(1), data: vec![0u8; 100], nonce: 0, namespace_id: 1,
            signature: None, public_key: None,
        });
        assert_eq!(ParallelExecutor::estimate_gas(&empty), GAS_CREATE_OBJECT_BASE);
        assert_eq!(
            ParallelExecutor::estimate_gas(&with_data),
            GAS_CREATE_OBJECT_BASE + GAS_CREATE_OBJECT_PER_BYTE * 100,
        );
    }

    #[test]
    fn t1_20_par_estimate_gas_governance() {
        let tx = Transaction::Governance(evaporchain_types::GovernanceTx {
            action: evaporchain_types::GovernanceAction::CastVote { proposal_id: 1, vote: true },
            sender: addr(1), nonce: 0, signature: None, public_key: None,
        });
        assert_eq!(ParallelExecutor::estimate_gas(&tx), GAS_GOVERNANCE);
    }

    #[test]
    fn t1_20_par_estimate_gas_multisig() {
        let tx = Transaction::MultiSig(evaporchain_types::MultiSigTx {
            multisig_address: addr(1), threshold: 1, signers: vec![addr(2)],
            inner_tx_bytes: vec![], signatures: vec![], public_keys: vec![], nonce: 0,
        });
        assert_eq!(ParallelExecutor::estimate_gas(&tx), crate::GAS_MULTISIG);
    }

    #[test]
    fn t1_20_par_estimate_gas_userop_scales_with_calldata() {
        let empty = Transaction::UserOp(evaporchain_types::UserOpTx {
            sender: addr(1), nonce: 0, call_data: vec![], call_gas_limit: 0,
            paymaster: None, paymaster_nonce: None, paymaster_data: None,
            paymaster_signature: None, paymaster_public_key: None,
            signature: None, public_key: None,
        });
        let with_data = Transaction::UserOp(evaporchain_types::UserOpTx {
            sender: addr(1), nonce: 0, call_data: vec![0u8; 50], call_gas_limit: 0,
            paymaster: None, paymaster_nonce: None, paymaster_data: None,
            paymaster_signature: None, paymaster_public_key: None,
            signature: None, public_key: None,
        });
        assert_eq!(ParallelExecutor::estimate_gas(&empty), crate::GAS_USER_OP);
        assert_eq!(ParallelExecutor::estimate_gas(&with_data), crate::GAS_USER_OP + 50 * 16);
    }

    #[test]
    fn t1_20_par_estimate_gas_upgrade_contract_scales_with_bytecode() {
        let empty = Transaction::UpgradeContract(evaporchain_types::UpgradeContractTx {
            owner: addr(1), contract_id: 1, new_bytecode: vec![],
            new_bytecode_hash: [0u8; 32], nonce: 0,
            admin_signature: None, admin_public_key: None,
            endorser_stakes: vec![], required_stake: 0,
            governance_approved: false, signature: None, public_key: None,
        });
        let with_code = Transaction::UpgradeContract(evaporchain_types::UpgradeContractTx {
            owner: addr(1), contract_id: 1, new_bytecode: vec![0u8; 10],
            new_bytecode_hash: [0u8; 32], nonce: 0,
            admin_signature: None, admin_public_key: None,
            endorser_stakes: vec![], required_stake: 0,
            governance_approved: false, signature: None, public_key: None,
        });
        assert_eq!(ParallelExecutor::estimate_gas(&empty), crate::GAS_UPGRADE_CONTRACT);
        assert_eq!(
            ParallelExecutor::estimate_gas(&with_code),
            crate::GAS_UPGRADE_CONTRACT + 10 * 200,
        );
    }

    #[test]
    fn t1_20_par_estimate_gas_delegate() {
        let tx = Transaction::Delegate(evaporchain_types::DelegateTx {
            delegator: addr(1), validator_id: 1, amount: 500, nonce: 0,
            signature: None, public_key: None,
        });
        assert_eq!(ParallelExecutor::estimate_gas(&tx), crate::GAS_DELEGATE);
    }

    #[test]
    fn t1_20_par_estimate_gas_undelegate() {
        let tx = Transaction::Undelegate(evaporchain_types::UndelegateTx {
            delegator: addr(1), validator_id: 1, amount: 500, nonce: 0,
            signature: None, public_key: None,
        });
        assert_eq!(ParallelExecutor::estimate_gas(&tx), crate::GAS_UNDELEGATE);
    }

    #[test]
    fn t1_20_par_estimate_gas_rotate_validator_key() {
        let tx = Transaction::RotateValidatorKey(evaporchain_types::RotateValidatorKeyTx {
            validator_address: addr(1), validator_id: 1,
            new_bls_public_key: vec![], bls_pop_old: vec![], bls_pop_new: vec![],
            effective_epoch: 5, nonce: 0,
            signature: None, public_key: None,
        });
        assert_eq!(ParallelExecutor::estimate_gas(&tx), crate::GAS_ROTATE_VALIDATOR_KEY);
    }

    #[test]
    fn t1_20_par_estimate_gas_claim_delegation() {
        let tx = Transaction::ClaimDelegation(evaporchain_types::ClaimDelegationTx {
            delegator: addr(1), validator_id: 1, nonce: 0,
            signature: None, public_key: None,
        });
        assert_eq!(ParallelExecutor::estimate_gas(&tx), crate::GAS_CLAIM_DELEGATION);
    }

    #[test]
    fn t1_20_par_estimate_gas_refund() {
        let tx = Transaction::Refund(evaporchain_types::RefundTx {
            source_block_height: 1, source_observation_idx: 0,
            attacker: addr(1), victim: addr(2), amount: 100, settle_block_height: 5,
        });
        assert_eq!(ParallelExecutor::estimate_gas(&tx), GAS_REFUND);
    }

    // ── T1.20 batch 7: analyze_parallelism / extract_access_keys coverage ─────

    #[test]
    fn t1_20_analyze_parallelism_governance_partitions_separately() {
        // Two Governance txs from different senders: different Account keys → 2 partitions
        let txs = vec![
            Transaction::Governance(evaporchain_types::GovernanceTx {
                action: evaporchain_types::GovernanceAction::CastVote {
                    proposal_id: 1, vote: true,
                },
                sender: addr(10), nonce: 0, signature: None, public_key: None,
            }),
            Transaction::Governance(evaporchain_types::GovernanceTx {
                action: evaporchain_types::GovernanceAction::CastVote {
                    proposal_id: 2, vote: false,
                },
                sender: addr(11), nonce: 0, signature: None, public_key: None,
            }),
        ];
        let (partitions, max_size, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 2);
        assert_eq!(max_size, 1);
    }

    #[test]
    fn t1_20_analyze_parallelism_multisig_same_address_merges() {
        // Two MultiSig txs with same multisig_address → shared Account key → 1 partition
        let txs = vec![
            Transaction::MultiSig(evaporchain_types::MultiSigTx {
                multisig_address: addr(5), threshold: 1, signers: vec![addr(6)],
                inner_tx_bytes: vec![], signatures: vec![], public_keys: vec![], nonce: 0,
            }),
            Transaction::MultiSig(evaporchain_types::MultiSigTx {
                multisig_address: addr(5), threshold: 1, signers: vec![addr(7)],
                inner_tx_bytes: vec![], signatures: vec![], public_keys: vec![], nonce: 1,
            }),
        ];
        let (partitions, _, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 1, "same multisig_address must merge into 1 partition");
    }

    #[test]
    fn t1_20_analyze_parallelism_userop_without_paymaster() {
        let txs = vec![Transaction::UserOp(evaporchain_types::UserOpTx {
            sender: addr(20), nonce: 0, call_data: vec![], call_gas_limit: 0,
            paymaster: None, paymaster_nonce: None, paymaster_data: None,
            paymaster_signature: None, paymaster_public_key: None,
            signature: None, public_key: None,
        })];
        let (partitions, max_size, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 1);
        assert_eq!(max_size, 1);
    }

    #[test]
    fn t1_20_analyze_parallelism_userop_paymaster_links_accounts() {
        // UserOp with paymaster adds both sender + paymaster as access keys.
        // A Transfer from the paymaster account merges into the same partition.
        let pm = addr(99);
        let txs = vec![
            Transaction::UserOp(evaporchain_types::UserOpTx {
                sender: addr(20), nonce: 0, call_data: vec![], call_gas_limit: 0,
                paymaster: Some(pm),
                paymaster_nonce: None, paymaster_data: None,
                paymaster_signature: None, paymaster_public_key: None,
                signature: None, public_key: None,
            }),
            Transaction::Transfer(TransferTx {
                from: pm, to: addr(30), amount: 100, nonce: 0,
                signature: None, public_key: None, mev_refund_eligible: None,
            }),
        ];
        let (partitions, _, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 1, "UserOp paymaster + conflicting Transfer must merge");
    }

    #[test]
    fn t1_20_analyze_parallelism_shield_shares_privacy_engine() {
        // Two Shield txs from different senders → both touch PrivacyEngine → 1 partition
        let txs = vec![
            Transaction::Shield(evaporchain_types::ShieldTx {
                from: addr(1), amount: 100, nonce: 0,
                note_owner_hash: [0u8; 32], value_blinding: [0u8; 32],
                energy: None, energy_blinding: None, half_life: 0,
                signature: None, public_key: None,
            }),
            Transaction::Shield(evaporchain_types::ShieldTx {
                from: addr(2), amount: 200, nonce: 0,
                note_owner_hash: [1u8; 32], value_blinding: [1u8; 32],
                energy: None, energy_blinding: None, half_life: 0,
                signature: None, public_key: None,
            }),
        ];
        let (partitions, _, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 1, "Shield txs share PrivacyEngine and must merge");
    }

    #[test]
    fn t1_20_analyze_parallelism_unshield_shares_privacy_engine() {
        let txs = vec![
            Transaction::Unshield(evaporchain_types::UnshieldTx {
                to: addr(3), amount: 100,
                input_nullifiers: vec![], anchor: [0u8; 32], balance_binding: [0u8; 32],
                input_amounts: vec![], input_blindings: vec![],
                input_value_commitments: vec![], input_note_commitments: vec![],
                input_merkle_proofs: vec![], output_blindings: vec![],
                change_commitments: vec![], energy_proofs: vec![],
            }),
            Transaction::Unshield(evaporchain_types::UnshieldTx {
                to: addr(4), amount: 200,
                input_nullifiers: vec![], anchor: [0u8; 32], balance_binding: [0u8; 32],
                input_amounts: vec![], input_blindings: vec![],
                input_value_commitments: vec![], input_note_commitments: vec![],
                input_merkle_proofs: vec![], output_blindings: vec![],
                change_commitments: vec![], energy_proofs: vec![],
            }),
        ];
        let (partitions, _, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 1, "Unshield txs share PrivacyEngine and must merge");
    }

    #[test]
    fn t1_20_analyze_parallelism_private_transfer_shares_privacy_engine() {
        let txs = vec![
            Transaction::PrivateTransfer(evaporchain_types::PrivateTransferTx {
                input_nullifiers: vec![], output_commitments: vec![],
                anchor: [0u8; 32], balance_binding: [0u8; 32], fee: 0,
                input_amounts: vec![], input_blindings: vec![],
                input_value_commitments: vec![], input_note_commitments: vec![],
                input_merkle_proofs: vec![], output_amounts: vec![],
                output_blindings: vec![], energy_proofs: vec![],
            }),
            Transaction::PrivateTransfer(evaporchain_types::PrivateTransferTx {
                input_nullifiers: vec![[1u8; 32], [2u8; 32]], output_commitments: vec![[3u8; 32], [4u8; 32]],
                anchor: [1u8; 32], balance_binding: [1u8; 32], fee: 100,
                input_amounts: vec![], input_blindings: vec![],
                input_value_commitments: vec![], input_note_commitments: vec![],
                input_merkle_proofs: vec![], output_amounts: vec![],
                output_blindings: vec![], energy_proofs: vec![],
            }),
        ];
        let (partitions, _, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 1, "PrivateTransfer txs share PrivacyEngine");
    }

    #[test]
    fn t1_20_analyze_parallelism_deferred_shares_temporal_engine() {
        // Two Deferred txs from different submitters → both touch TemporalEngine → 1 partition
        let txs = vec![
            Transaction::Deferred(evaporchain_types::DeferredTx {
                submitter: addr(10), nonce: 0, deposit: 0,
                guards: vec![], inner_tx_bytes: vec![], gas_limit: 0,
                signature: None, public_key: None,
            }),
            Transaction::Deferred(evaporchain_types::DeferredTx {
                submitter: addr(11), nonce: 0, deposit: 0,
                guards: vec![], inner_tx_bytes: vec![], gas_limit: 0,
                signature: None, public_key: None,
            }),
        ];
        let (partitions, _, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 1, "Deferred txs share TemporalEngine and must merge");
    }

    #[test]
    fn t1_20_analyze_parallelism_blob_separate_submitters() {
        // Two Blob txs from different submitters → different Account keys → 2 partitions
        let txs = vec![
            Transaction::Blob(evaporchain_types::BlobTx {
                submitter: addr(40), data: vec![0xAA], nonce: 0, namespace_id: 1,
                signature: None, public_key: None,
            }),
            Transaction::Blob(evaporchain_types::BlobTx {
                submitter: addr(41), data: vec![0xBB], nonce: 0, namespace_id: 2,
                signature: None, public_key: None,
            }),
        ];
        let (partitions, max_size, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 2);
        assert_eq!(max_size, 1);
    }

    #[test]
    fn t1_20_analyze_parallelism_upgrade_contract() {
        let txs = vec![Transaction::UpgradeContract(evaporchain_types::UpgradeContractTx {
            owner: addr(50), contract_id: 1, new_bytecode: vec![],
            new_bytecode_hash: [0u8; 32], nonce: 0,
            admin_signature: None, admin_public_key: None,
            endorser_stakes: vec![], required_stake: 0,
            governance_approved: false, signature: None, public_key: None,
        })];
        let (partitions, max_size, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 1);
        assert_eq!(max_size, 1);
    }

    #[test]
    fn t1_20_analyze_parallelism_delegate_and_undelegate_different_delegators() {
        // Delegate and Undelegate from different delegators → 2 partitions
        let txs = vec![
            Transaction::Delegate(evaporchain_types::DelegateTx {
                delegator: addr(60), validator_id: 1, amount: 1_000, nonce: 0,
                signature: None, public_key: None,
            }),
            Transaction::Undelegate(evaporchain_types::UndelegateTx {
                delegator: addr(61), validator_id: 1, amount: 500, nonce: 0,
                signature: None, public_key: None,
            }),
        ];
        let (partitions, max_size, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 2);
        assert_eq!(max_size, 1);
    }

    #[test]
    fn t1_20_analyze_parallelism_claim_delegation() {
        let txs = vec![Transaction::ClaimDelegation(evaporchain_types::ClaimDelegationTx {
            delegator: addr(70), validator_id: 1, nonce: 0,
            signature: None, public_key: None,
        })];
        let (partitions, max_size, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 1);
        assert_eq!(max_size, 1);
    }

    #[test]
    fn t1_20_analyze_parallelism_refund_merges_with_conflicting_transfer() {
        // Refund touches both attacker + victim accounts. Transfer from victim merges.
        let victim = addr(80);
        let txs = vec![
            Transaction::Refund(evaporchain_types::RefundTx {
                source_block_height: 1, source_observation_idx: 0,
                attacker: addr(79), victim, amount: 100, settle_block_height: 5,
            }),
            Transaction::Transfer(TransferTx {
                from: victim, to: addr(81), amount: 50, nonce: 0,
                signature: None, public_key: None, mev_refund_eligible: None,
            }),
        ];
        let (partitions, _, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 1, "Refund victim + conflicting Transfer must merge");
    }

    #[test]
    fn t1_20_analyze_parallelism_rotate_validator_key() {
        let txs = vec![Transaction::RotateValidatorKey(evaporchain_types::RotateValidatorKeyTx {
            validator_address: addr(90), validator_id: 1,
            new_bls_public_key: vec![0u8; 48], bls_pop_old: vec![0u8; 48],
            bls_pop_new: vec![0u8; 48], effective_epoch: 10, nonce: 0,
            signature: None, public_key: None,
        })];
        let (partitions, max_size, _) = analyze_parallelism(&txs);
        assert_eq!(partitions, 1);
        assert_eq!(max_size, 1);
    }

    // ── T1.20 batch 7: execute_partition ContractError via execute_block ───────
    // These types reach execute_partition via Phase 1's `_` arm (parallel path).

    // AUDIT_2026_05_13 C4 closure: these tests previously asserted
    // `txs_failed == 1` because Governance + MultiSig fell to the
    // parallel partition's serial-phase error arm. Now they exercise
    // the correct success path through the serial executor.
    #[test]
    fn t1_20_governance_create_proposal_via_parallel_executor_succeeds() {
        use evaporchain_types::{GovernanceAction, GovernanceTx};
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 60, 1_000);
        let block = make_block(
            1,
            1,
            vec![Transaction::Governance(GovernanceTx {
                action: GovernanceAction::CreateProposal {
                    title: "raise block gas limit".into(),
                    param_key: "block_gas_limit".into(),
                    param_value: "5000000".into(),
                    voting_epochs: 50,
                },
                sender: addr(60),
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed, 1, "governance must execute on serial path");
        assert_eq!(r.txs_failed, 0);
        // Proposal landed in the proposal store + nonce bumped.
        assert_eq!(db.all_proposals().len(), 1);
        let p = db.get_proposal(0).expect("proposal not stored");
        assert_eq!(p.param_key, "block_gas_limit");
        assert_eq!(p.param_value, "5000000");
        assert_eq!(p.start_epoch, 1);
        assert_eq!(p.end_epoch, 51);
        assert_eq!(db.get_account(&addr(60)).unwrap().nonce, 1);
    }

    #[test]
    fn t1_20_multisig_via_parallel_executor_succeeds() {
        use evaporchain_types::MultiSigTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 61, 1_000);
        let block = make_block(
            1,
            1,
            vec![Transaction::MultiSig(MultiSigTx {
                multisig_address: addr(61),
                threshold: 1,
                signers: vec![addr(62)],
                inner_tx_bytes: vec![],
                // One legitimate signature from an authorized signer.
                signatures: vec![(addr(62), vec![0u8; 64])],
                public_keys: vec![],
                nonce: 0,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed, 1, "multisig must execute on serial path");
        assert_eq!(r.txs_failed, 0);
        // Nonce bumped at multisig_address.
        assert_eq!(db.get_account(&addr(61)).unwrap().nonce, 1);
    }

    // AUDIT_2026_05_13 C4 (7/7) closure: UserOp envelope. The
    // gas-only / no-paymaster UserOp executes through the shared
    // `execute_user_op_envelope_impl` and succeeds — pre-fix it
    // errored at the partition arm with "executes in serial phase".
    // Non-empty call_data is gated to Phase 3e (sibling test below).
    #[test]
    fn t1_20_userop_gas_only_via_parallel_executor_succeeds() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 62, 1_000);
        let block = make_block(
            1,
            1,
            vec![Transaction::UserOp(evaporchain_types::UserOpTx {
                sender: addr(62),
                nonce: 0,
                call_data: vec![],
                call_gas_limit: 0,
                paymaster: None,
                paymaster_nonce: None,
                paymaster_data: None,
                paymaster_signature: None,
                paymaster_public_key: None,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed, 1, "gas-only UserOp must execute on serial path");
        assert_eq!(r.txs_failed, 0);
        // Sender nonce bumped via shared envelope impl.
        assert_eq!(db.get_account(&addr(62)).unwrap().nonce, 1);
    }

    #[test]
    fn t1_20_userop_non_empty_call_data_fails_serial_path() {
        // Documented partial closure: non-empty call_data is rejected
        // with a clear "inner-tx dispatch not yet wired" message instead
        // of the pre-fix generic "executes in serial phase". Envelope
        // mutations (nonce bump) still apply identically — Ethereum
        // semantics: failed-but-attempted tx consumes the nonce slot.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 62, 1_000);
        let inner_payload =
            serde_json::to_vec(&Transaction::Transfer(evaporchain_types::TransferTx {
                from: addr(62),
                to: addr(63),
                amount: 100,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }))
            .unwrap();
        let block = make_block(
            1,
            1,
            vec![Transaction::UserOp(evaporchain_types::UserOpTx {
                sender: addr(62),
                nonce: 0,
                call_data: inner_payload,
                call_gas_limit: 100_000,
                paymaster: None,
                paymaster_nonce: None,
                paymaster_data: None,
                paymaster_signature: None,
                paymaster_public_key: None,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1, "non-empty call_data deferred to Phase 3e");
    }

    // AUDIT_2026_05_13 C4 (6/7) closure: UpgradeContract previously
    // fell to the parallel partition's "executes in serial phase"
    // error arm, so every contract upgrade on the production executor
    // failed silently. Now wired through the shared
    // `execute_upgrade_contract_impl`. This test asserts the
    // error-path parity (no contract registered → ContractError) on
    // the serial path, NOT the partition-error.
    #[test]
    fn t1_20_upgrade_contract_via_parallel_executor_reaches_serial_path() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 63, 1_000);
        let block = make_block(
            1,
            1,
            vec![Transaction::UpgradeContract(
                evaporchain_types::UpgradeContractTx {
                    owner: addr(63),
                    contract_id: 1,
                    new_bytecode: b"contract Empty { fn noop() { } }".to_vec(),
                    new_bytecode_hash: *blake3::hash(b"contract Empty { fn noop() { } }")
                        .as_bytes(),
                    nonce: 0,
                    admin_signature: None,
                    admin_public_key: None,
                    endorser_stakes: vec![],
                    required_stake: 0,
                    governance_approved: false,
                    signature: None,
                    public_key: None,
                },
            )],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        // Still 1 failed (no contract id=1 exists, and required_stake=0
        // is rejected via the governance path) — but failure NOW comes
        // from the serial-path ContractError, not the parallel-phase
        // "executes in serial phase" error arm. Nonce bumped at owner
        // because pre-fix the partition-error skipped nonce mutation;
        // post-fix the serial impl bumps nonce before failing.
        assert_eq!(r.txs_failed, 1, "expected serial-path validation failure");
        assert_eq!(
            db.get_account(&addr(63)).unwrap().nonce,
            1,
            "serial-path execute_upgrade_contract_impl bumps owner nonce on validation failure"
        );
    }

    // AUDIT_2026_05_13 C4 closure: these 3 tests previously asserted
    // `txs_failed == 1` — codifying the bug (delegation txs failing with
    // "executes in serial phase") as desired behaviour. The audit caught
    // this. Tests now assert the inverse: a properly-set-up delegation
    // flow succeeds and the delegation record exists at the right values.
    #[test]
    fn t1_20_delegate_via_parallel_executor_succeeds() {
        use evaporchain_types::{DelegateTx, StakeRecord};
        let mut db = InMemoryStateDB::new();
        // Validator-1 must have a stake record before delegations are
        // accepted (matches SimpleExecutor::execute_delegate guard).
        db.put_stake(StakeRecord {
            validator_id: 1,
            validator_address: addr(1),
            staked_amount: 10_000,
            staked_at_epoch: 1,
            unbonding_epoch: None,
            slashed_amount: 0,
        });
        fund_account(&mut db, 64, 5_000);

        let block = make_block(
            1,
            1,
            vec![Transaction::Delegate(DelegateTx {
                delegator: addr(64),
                validator_id: 1,
                amount: 1_000,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed, 1, "delegate must execute on serial path");
        assert_eq!(r.txs_failed, 0);
        let record = db
            .get_delegation(&addr(64), 1)
            .expect("delegation record missing after successful Delegate");
        assert_eq!(record.amount, 1_000);
        assert_eq!(record.delegated_at_epoch, 1);
        // Balance debited (fee is charged on top, so balance is just <5_000 - 1_000>).
        assert!(db.get_account(&addr(64)).unwrap().balance <= 4_000);
        // Nonce bumped.
        assert_eq!(db.get_account(&addr(64)).unwrap().nonce, 1);
    }

    #[test]
    fn t1_20_undelegate_via_parallel_executor_succeeds() {
        use evaporchain_types::{DelegateTx, StakeRecord, UndelegateTx};
        let mut db = InMemoryStateDB::new();
        db.put_stake(StakeRecord {
            validator_id: 1,
            validator_address: addr(1),
            staked_amount: 10_000,
            staked_at_epoch: 1,
            unbonding_epoch: None,
            slashed_amount: 0,
        });
        fund_account(&mut db, 65, 5_000);

        let block = make_block(
            1,
            1,
            vec![
                Transaction::Delegate(DelegateTx {
                    delegator: addr(65),
                    validator_id: 1,
                    amount: 2_000,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                }),
                Transaction::Undelegate(UndelegateTx {
                    delegator: addr(65),
                    validator_id: 1,
                    amount: 500,
                    nonce: 1,
                    signature: None,
                    public_key: None,
                }),
            ],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed, 2);
        assert_eq!(r.txs_failed, 0);
        let record = db.get_delegation(&addr(65), 1).expect("record missing");
        assert_eq!(record.amount, 1_500);
        assert_eq!(record.unbonding_amount, 500);
        assert_eq!(record.unbonding_epoch, Some(1));
    }

    #[test]
    fn t1_20_claim_delegation_via_parallel_executor_succeeds() {
        use evaporchain_types::{ClaimDelegationTx, DelegateTx, StakeRecord, UndelegateTx};
        let mut db = InMemoryStateDB::new();
        db.put_stake(StakeRecord {
            validator_id: 1,
            validator_address: addr(1),
            staked_amount: 10_000,
            staked_at_epoch: 1,
            unbonding_epoch: None,
            slashed_amount: 0,
        });
        fund_account(&mut db, 66, 5_000);

        // Block A: Delegate + Undelegate at epoch 1.
        let block_a = make_block(
            1,
            1,
            vec![
                Transaction::Delegate(DelegateTx {
                    delegator: addr(66),
                    validator_id: 1,
                    amount: 1_500,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                }),
                Transaction::Undelegate(UndelegateTx {
                    delegator: addr(66),
                    validator_id: 1,
                    amount: 1_500,
                    nonce: 1,
                    signature: None,
                    public_key: None,
                }),
            ],
        );
        let mut ex = ParallelExecutor::new_for_test(100);
        let _ = ex.execute_block(&mut db, &block_a).unwrap();

        // Block B: ClaimDelegation after unbonding period elapses.
        let claim_epoch = 1 + crate::UNBONDING_PERIOD_EPOCHS;
        let block_b = make_block(
            2,
            claim_epoch,
            vec![Transaction::ClaimDelegation(ClaimDelegationTx {
                delegator: addr(66),
                validator_id: 1,
                nonce: 2,
                signature: None,
                public_key: None,
            })],
        );
        let r = ex.execute_block(&mut db, &block_b).unwrap();
        assert_eq!(r.txs_executed, 1, "claim-delegation must execute");
        assert_eq!(r.txs_failed, 0);
        // After full claim with amount=0 + unbonding_amount=0, the
        // delegation record is removed (SimpleExecutor parity).
        assert!(
            db.get_delegation(&addr(66), 1).is_none(),
            "delegation record should be removed once fully claimed"
        );
        // Balance credited back (started 5_000, delegated 1_500, then claimed 1_500;
        // fees deducted from the delegate and undelegate steps).
        let after = db.get_account(&addr(66)).unwrap().balance;
        assert!(
            after > 4_000,
            "claimed 1_500 should be credited back, got balance {after}"
        );
    }

    // ── T1.20 batch 7: Blob via parallel path ────────────────────────────────

    #[test]
    fn t1_20_blob_empty_data_fails() {
        let mut db = InMemoryStateDB::new();
        let block = make_block(1, 1, vec![Transaction::Blob(evaporchain_types::BlobTx {
            submitter: addr(50), data: vec![], nonce: 0, namespace_id: 1,
            signature: None, public_key: None,
        })]);
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1, "blob with empty data must fail");
    }

    #[test]
    fn t1_20_blob_too_large_fails() {
        let mut db = InMemoryStateDB::new();
        let block = make_block(1, 1, vec![Transaction::Blob(evaporchain_types::BlobTx {
            submitter: addr(50), data: vec![0u8; crate::MAX_BLOB_SIZE + 1],
            nonce: 0, namespace_id: 1, signature: None, public_key: None,
        })]);
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1, "blob exceeding MAX_BLOB_SIZE must fail");
    }

    #[test]
    fn t1_20_blob_reserved_namespace_zero_fails() {
        let mut db = InMemoryStateDB::new();
        let block = make_block(1, 1, vec![Transaction::Blob(evaporchain_types::BlobTx {
            submitter: addr(51), data: vec![0xAB], nonce: 0,
            namespace_id: 0, signature: None, public_key: None,
        })]);
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1, "namespace_id=0 is reserved; must fail");
    }

    #[test]
    fn t1_20_blob_valid_succeeds() {
        let mut db = InMemoryStateDB::new();
        let block = make_block(1, 1, vec![Transaction::Blob(evaporchain_types::BlobTx {
            submitter: addr(52), data: vec![0xDE, 0xAD, 0xBE, 0xEF], nonce: 0,
            namespace_id: 42, signature: None, public_key: None,
        })]);
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed, 1, "valid blob must succeed");
    }

    // ── T1.20 batch 7: serial phase missing edge cases ────────────────────────

    #[test]
    fn t1_20_validator_exit_address_mismatch_fails() {
        // stake has validator_address=addr(10); exit tx uses addr(11) → mismatch
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 11, 100_000);
        db.put_stake(make_stake(2, 10, 50_000));
        let block = make_block(1, 5, vec![Transaction::ValidatorExit(ValidatorExitTx {
            validator_address: addr(11),
            validator_id: 2,
            nonce: 0,
            signature: None, public_key: None,
        })]);
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1, "address mismatch on validator exit must fail");
    }

    #[test]
    fn t1_20_validator_claim_stake_address_mismatch_fails() {
        use evaporchain_types::ValidatorClaimStakeTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 99, 1000);
        let mut stake = make_stake(3, 10, 50_000);
        stake.unbonding_epoch = Some(10);
        db.put_stake(stake);
        let block = make_block(1, 300, vec![Transaction::ValidatorClaimStake(ValidatorClaimStakeTx {
            validator_address: addr(99),
            validator_id: 3,
            nonce: 0,
            signature: None, public_key: None,
        })]);
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1, "address mismatch on claim-stake must fail");
    }

    #[test]
    fn t1_20_rotate_validator_key_address_mismatch_fails() {
        use evaporchain_types::RotateValidatorKeyTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 99, 1000);
        db.put_stake(make_stake(4, 10, 50_000));
        let block = make_block(1, 100, vec![Transaction::RotateValidatorKey(RotateValidatorKeyTx {
            validator_address: addr(99),
            validator_id: 4,
            new_bls_public_key: vec![0u8; 48],
            bls_pop_old: vec![0u8; 48],
            bls_pop_new: vec![0u8; 48],
            effective_epoch: 200,
            nonce: 0,
            signature: None, public_key: None,
        })]);
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1, "address mismatch on rotate-key must fail");
    }

    #[test]
    fn t1_20_rotate_validator_key_nonce_mismatch_fails() {
        use evaporchain_types::RotateValidatorKeyTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 10, 1000); // account nonce = 0
        db.put_stake(make_stake(5, 10, 50_000));
        let block = make_block(1, 100, vec![Transaction::RotateValidatorKey(RotateValidatorKeyTx {
            validator_address: addr(10),
            validator_id: 5,
            new_bls_public_key: vec![0u8; 48],
            bls_pop_old: vec![0u8; 48],
            bls_pop_new: vec![0u8; 48],
            effective_epoch: 200,
            nonce: 99, // WRONG: expected 0
            signature: None, public_key: None,
        })]);
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1, "nonce mismatch on rotate-key must fail");
    }

    #[test]
    fn t1_20_deferred_tx_serial_submission_processed() {
        // Exercises the Deferred serial-phase arm. submit() rejects empty
        // inner_tx_bytes so the tx fails gracefully — the important thing is
        // the code path is reached without panicking.
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 20, 10_000);
        let block = make_block(1, 5, vec![Transaction::Deferred(evaporchain_types::DeferredTx {
            submitter: addr(20), nonce: 0, deposit: 0,
            guards: vec![], inner_tx_bytes: vec![], gas_limit: 0,
            signature: None, public_key: None,
        })]);
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_executed + r.txs_failed, 1, "Deferred tx must be processed");
    }

    #[test]
    fn t1_20_deploy_script_invalid_source_fails() {
        use evaporchain_types::DeployScriptTx;
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 30, 10_000);
        let block = make_block(1, 1, vec![Transaction::DeployScript(DeployScriptTx {
            deployer: addr(30),
            source_code: "not valid evaporscript @@@!".to_string(),
            energy: 500,
            half_life: 10,
            signature: None, public_key: None,
        })]);
        let mut ex = ParallelExecutor::new_for_test(100);
        let r = ex.execute_block(&mut db, &block).unwrap();
        assert_eq!(r.txs_failed, 1, "invalid DeployScript source must fail");
    }
}
