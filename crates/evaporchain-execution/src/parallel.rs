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
use evaporchain_crypto::signatures::{MlDsaVerifier, Verifier};
use evaporchain_crypto::MerkleMountainRange;
use evaporchain_script::ScriptEngine;
use evaporchain_state::db::StateDB;
use evaporchain_state::{EvaporationEngine, RefreshEngine};
use evaporchain_types::{
    Account, AccountAddress, Block, CreateObjectTx, Epoch, GhostRecord, ObjectId, ObjectState,
    RefreshTx, StateObject, Transaction, TransferTx, ValidatorExitTx, ValidatorStakeTx,
};
use rayon::prelude::*;
use tracing::{debug, info};

use crate::{
    fees, BlockExecutionResult, ExecutionEngine, ExecutionError,
    GAS_CALL_CONTRACT, GAS_CALL_SCRIPT, GAS_CREATE_OBJECT_BASE, GAS_CREATE_OBJECT_PER_BYTE,
    GAS_DEPLOY_CONTRACT, GAS_DEPLOY_SCRIPT, GAS_REFRESH, GAS_TRANSFER, GAS_VALIDATOR_EXIT,
    GAS_VALIDATOR_STAKE,
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
            vec![AccessKey::Account(t.creator), AccessKey::Object(t.object_id)]
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
                AccessKey::ContractEngine | AccessKey::ScriptEngine | AccessKey::PrivacyEngine | AccessKey::TemporalEngine => {
                    // These are handled by the executor, not the state DB.
                }
            }
        }
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

    fn get_or_create_account(&mut self, addr: &AccountAddress) -> &mut Account {
        self.accounts.entry(*addr).or_insert_with(|| Account {
            address: *addr,
            balance: 0,
            nonce: 0,
        })
    }

    fn all_account_addresses(&self) -> Vec<AccountAddress> {
        self.accounts.keys().copied().collect()
    }

    fn compute_state_root(&mut self) -> [u8; 32] {
        [0u8; 32]
    }

    fn compress_cold_subtrees(&mut self) -> u32 { 0 }
    fn trie_health(&mut self) -> evaporchain_crypto::TrieHealth {
        evaporchain_crypto::TrieHealth {
            active_leaves: 0, compressed_leaves: 0, total_nodes: 0,
            max_energy: 0, min_half_life: u64::MAX, last_activity_epoch: 0,
            compressions: 0, decompressions: 0,
        }
    }
    fn trie_snapshot(&mut self) -> Vec<u8> { Vec::new() }
    fn load_trie_snapshot(&mut self, _bytes: &[u8]) -> Result<(), String> { Ok(()) }

    // Privacy methods — overlay doesn't handle privacy state (it's in the serial phase).
    fn put_note_tree_root(&mut self, _root: [u8; 32]) {}
    fn get_note_tree_root(&self) -> [u8; 32] { [0u8; 32] }
    fn spend_nullifier(&mut self, _nullifier: &[u8; 32]) -> bool { false }
    fn is_nullifier_spent(&self, _nullifier: &[u8; 32]) -> bool { false }
    fn nullifier_count(&self) -> usize { 0 }
    fn all_nullifiers(&self) -> Vec<[u8; 32]> { Vec::new() }
    fn put_shielded_pool_balance(&mut self, _balance: u64) {}
    fn get_shielded_pool_balance(&self) -> u64 { 0 }
    fn put_note_count(&mut self, _count: u64) {}
    fn get_note_count(&self) -> u64 { 0 }
}

// ─── Partition Execution Result ────────────────────────────────────────────

/// Result of executing a single partition.
struct PartitionResult {
    overlay: OverlayStateDB,
    txs_executed: usize,
    txs_failed: usize,
    gas_used: u64,
    total_fees: u64,
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
pub struct ParallelExecutor {
    evaporation_engine: EvaporationEngine,
    mmr: MerkleMountainRange,
    verify_signatures: bool,
    fee_controller: Option<fees::PidFeeController>,
    pub block_gas_limit: u64,
    pub contract_engine: ContractEngine,
    pub script_engine: ScriptEngine,
    pub privacy_executor: crate::privacy_exec::PrivacyExecutor,
    pub deferred_queue: crate::temporal::DeferredQueue,
    pub decay_watchers: crate::temporal::DecayWatcherEngine,
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
        }
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
        }
    }

    pub fn fee_controller(&self) -> Option<&fees::PidFeeController> {
        self.fee_controller.as_ref()
    }

    pub fn fee_controller_mut(&mut self) -> Option<&mut fees::PidFeeController> {
        self.fee_controller.as_mut()
    }

    fn estimate_gas(tx: &Transaction) -> u64 {
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
                crate::GAS_CREATE_OBJECT_BASE + crate::GAS_CREATE_OBJECT_PER_BYTE * tx.data.len() as u64
            }
        }
    }

    pub fn verify_tx_signature(verify: bool, tx: &Transaction) -> Result<(), ExecutionError> {
        if !verify {
            return Ok(());
        }
        // ZK-authenticated transactions don't use signatures
        if matches!(tx, Transaction::Unshield(_) | Transaction::PrivateTransfer(_)) {
            return Ok(());
        }
        let sig = tx.signature().ok_or(ExecutionError::MissingSignature)?;
        let pk = tx.public_key().ok_or(ExecutionError::MissingSignature)?;
        let msg = tx.signable_bytes();
        if !MlDsaVerifier::verify(&msg, sig, pk) {
            return Err(ExecutionError::InvalidSignature);
        }
        Ok(())
    }

    /// Execute a partition's transactions sequentially against an overlay DB.
    /// Contract/script transactions are skipped here — they require mutable
    /// access to the global engines and are handled in a serial phase.
    fn execute_partition(
        txs: &[(usize, &Transaction)],
        overlay: &mut OverlayStateDB,
        epoch: Epoch,
        verify_signatures: bool,
        _base_fee: u64,
        fee_controller: &Option<fees::PidFeeController>,
        _block_gas_limit: u64,
        gas_budget: &mut u64,
    ) -> PartitionResult {
        let mut txs_executed = 0;
        let mut txs_failed = 0;
        let mut gas_used = 0u64;
        let mut total_fees = 0u64;

        for &(idx, tx) in txs {
            // Signature verification
            if let Err(e) = Self::verify_tx_signature(verify_signatures, tx) {
                debug!(tx_idx = idx, error = %e, "Parallel: signature verification failed");
                txs_failed += 1;
                continue;
            }

            let tx_gas = Self::estimate_gas(tx);

            // Block gas limit check (using shared budget)
            if *gas_budget < tx_gas {
                debug!(tx_idx = idx, "Parallel: block gas limit exceeded");
                txs_failed += 1;
                continue;
            }

            // Fee deduction
            let tx_fee = if let Some(fc) = fee_controller {
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

                if let Some(sender_addr) = tx.sender() {
                    let sender = overlay.get_or_create_account(sender_addr);
                    if sender.balance < total_tx_fee {
                        debug!(tx_idx = idx, "Parallel: insufficient balance for fees");
                        txs_failed += 1;
                        continue;
                    }
                    sender.balance -= total_tx_fee;
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
                Transaction::Transfer(t) => Self::exec_transfer(overlay, t),
                Transaction::CreateObject(t) => Self::exec_create_object(overlay, t, epoch),
                Transaction::Refresh(t) => Self::exec_refresh(overlay, t, epoch),
                Transaction::ValidatorStake(t) => Self::exec_validator_stake(overlay, t),
                Transaction::ValidatorExit(t) => Self::exec_validator_exit(overlay, t),
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
                | Transaction::Deferred(_) => {
                    Err(ExecutionError::ContractError(
                        "contract/script/privacy/deferred txs execute in serial phase".into(),
                    ))
                }
                Transaction::Blob(_) => {
                    // Blob transactions are handled by the DA layer
                    Ok(())
                }
            };

            match result {
                Ok(()) => {
                    txs_executed += 1;
                    gas_used += tx_gas;
                    total_fees += tx_fee;
                    *gas_budget = gas_budget.saturating_sub(tx_gas);
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
                }
            }
        }

        PartitionResult {
            overlay: std::mem::replace(overlay, OverlayStateDB::new()),
            txs_executed,
            txs_failed,
            gas_used,
            total_fees,
        }
    }

    /// Merge an overlay's state changes back into the main DB.
    fn merge_overlay(db: &mut dyn StateDB, overlay: OverlayStateDB) {
        // Merge accounts (all modified accounts in overlay replace base).
        for (_, acct) in overlay.accounts {
            db.put_account(acct);
        }

        // Merge deleted objects.
        for id in &overlay.deleted_objects {
            db.delete_object(id);
        }

        // Merge created/modified objects.
        for (_, obj) in overlay.objects {
            db.put_object(obj);
        }

        // Merge removed ghosts.
        for id in &overlay.removed_ghosts {
            db.remove_ghost(id);
        }

        // Merge created ghosts.
        for (_, ghost) in overlay.ghosts {
            db.put_ghost(ghost);
        }
    }

    // ─── Per-Transaction Execution (pure functions on overlay) ──────────

    fn exec_transfer(
        db: &mut OverlayStateDB,
        tx: &TransferTx,
    ) -> Result<(), ExecutionError> {
        if tx.from == tx.to {
            return Err(ExecutionError::SelfTransfer);
        }
        if tx.amount == 0 {
            return Err(ExecutionError::ZeroAmount);
        }

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
        sender.balance -= tx.amount;
        sender.nonce += 1;

        let receiver = db.get_or_create_account(&tx.to);
        receiver.balance += tx.amount;

        Ok(())
    }

    fn exec_create_object(
        db: &mut OverlayStateDB,
        tx: &CreateObjectTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        if db.get_object(&tx.object_id).is_some() {
            return Err(ExecutionError::ObjectAlreadyExists(hex::encode(tx.object_id)));
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

    fn exec_validator_stake(
        db: &mut OverlayStateDB,
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
        sender.balance -= tx.stake_amount;
        sender.nonce += 1;
        Ok(())
    }

    fn exec_validator_exit(
        db: &mut OverlayStateDB,
        tx: &ValidatorExitTx,
    ) -> Result<(), ExecutionError> {
        let sender = db.get_or_create_account(&tx.validator_address);
        if sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        sender.nonce += 1;
        Ok(())
    }
}

impl ExecutionEngine for ParallelExecutor {
    fn execute_block(
        &mut self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<BlockExecutionResult, ExecutionError> {
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
                | Transaction::Deferred(_) => serial_txs.push((i, tx)),
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
                )
            })
            .collect();

        // ── Phase 5: Merge overlays back ──

        for result in partition_results {
            total_txs_executed += result.txs_executed;
            total_txs_failed += result.txs_failed;
            total_gas_used += result.gas_used;
            total_fees += result.total_fees;
            Self::merge_overlay(db, result.overlay);
        }

        // ── Phase 6: Execute serial (contract/script) txs sequentially ──

        for &(idx, tx) in &serial_txs {
            if let Err(e) = Self::verify_tx_signature(self.verify_signatures, tx) {
                debug!(tx_idx = idx, error = %e, "Serial: signature verification failed");
                total_txs_failed += 1;
                continue;
            }

            let tx_gas = Self::estimate_gas(tx);
            if self.block_gas_limit > 0 && total_gas_used + tx_gas > self.block_gas_limit {
                debug!(tx_idx = idx, "Serial: block gas limit exceeded");
                total_txs_failed += 1;
                continue;
            }

            let tx_fee = if let Some(fc) = &self.fee_controller {
                let total_tx_fee = fc.compute_gas_fee(tx_gas, 0);
                if let Some(sender_addr) = tx.sender() {
                    let sender = db.get_or_create_account(sender_addr);
                    if sender.balance < total_tx_fee {
                        total_txs_failed += 1;
                        continue;
                    }
                    sender.balance -= total_tx_fee;
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
                                        .map_err(|e| {
                                            ExecutionError::ContractError(e.to_string())
                                        })
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
                    let args: Result<serde_json::Value, _> = serde_json::from_str(&call.args);
                    match args {
                        Ok(a) => self
                            .contract_engine
                            .call(call.contract_id, &call.method, &a, &call.caller, call.epoch)
                            .map(|_| ())
                            .map_err(|e| ExecutionError::ContractError(e.to_string())),
                        Err(e) => Err(ExecutionError::ContractError(format!(
                            "invalid args: {e}"
                        ))),
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
                    let args: Vec<evaporchain_script::Value> =
                        if call.args.is_empty() || call.args == "[]" {
                            vec![]
                        } else {
                            serde_json::from_str(&call.args).unwrap_or_default()
                        };
                    self.script_engine
                        .call(call.contract_id, &call.method, args, call.caller, call.epoch)
                        .map(|_| ())
                        .map_err(|e| ExecutionError::ScriptError(e.to_string()))
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
                Transaction::Deferred(dtx) => {
                    self.deferred_queue
                        .submit(dtx.clone())
                        .map(|_| ())
                        .map_err(|e| ExecutionError::ContractError(e.to_string()))
                }
                _ => unreachable!("only contract/script/privacy/deferred txs in serial phase"),
            };

            match result {
                Ok(()) => {
                    total_txs_executed += 1;
                    total_gas_used += tx_gas;
                    total_fees += tx_fee;
                }
                Err(e) => {
                    // Revert fee deduction on failure? No — same as SimpleExecutor:
                    // sender still pays for gas even on failure.
                    debug!(tx_idx = idx, error = %e, "Serial: tx failed");
                    total_txs_failed += 1;
                    total_fees += tx_fee;
                }
            }
        }

        // ── Phase 7: Evaporation + contract/script ticks ──

        let evap_result = self.evaporation_engine.process_epoch_with_mmr(db, block.epoch, &mut self.mmr);
        self.contract_engine.tick(block.epoch);
        self.script_engine.tick(block.epoch);

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
        })
    }

    fn mmr_root(&self) -> [u8; 32] {
        self.mmr.root()
    }

    fn mmr_size(&self) -> usize {
        self.mmr.size()
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
            da_row_roots: vec![],
            da_col_roots: vec![],
        }
    }

    fn fund_account(db: &mut InMemoryStateDB, byte: u8, balance: u64) {
        db.put_account(Account {
            address: addr(byte),
            balance,
            nonce: 0,
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
            }),
            Transaction::Transfer(TransferTx {
                from: addr(3),
                to: addr(4),
                amount: 200,
                nonce: 0,
                signature: None,
                public_key: None,
            }),
        ];

        let partitions = partition_transactions(&txs);
        assert_eq!(partitions.len(), 2, "independent transfers should be in separate partitions");
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
            }),
            Transaction::Transfer(TransferTx {
                from: addr(2),
                to: addr(3),
                amount: 50,
                nonce: 0,
                signature: None,
                public_key: None,
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
            }),
            Transaction::Transfer(TransferTx {
                from: addr(2),
                to: addr(3),
                amount: 10,
                nonce: 0,
                signature: None,
                public_key: None,
            }),
            Transaction::Transfer(TransferTx {
                from: addr(4),
                to: addr(5),
                amount: 10,
                nonce: 0,
                signature: None,
                public_key: None,
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
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(3),
                    to: addr(4),
                    amount: 700,
                    nonce: 0,
                    signature: None,
                    public_key: None,
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
            }),
            Transaction::Transfer(TransferTx {
                from: addr(3),
                to: addr(4),
                amount: 200,
                nonce: 0,
                signature: None,
                public_key: None,
            }),
            Transaction::Transfer(TransferTx {
                from: addr(5),
                to: addr(6),
                amount: 300,
                nonce: 0,
                signature: None,
                public_key: None,
            }),
            Transaction::CreateObject(CreateObjectTx {
                creator: addr(7),
                object_id: obj_id(1),
                energy: 1000,
                half_life: 10,
                data: vec![1, 2, 3],
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
        });
        let mut seq_executor = crate::SimpleExecutor::new_for_test(100);
        let seq_result = seq_executor.execute_block(&mut db_seq, &block).unwrap();

        // Parallel
        let mut db_par = InMemoryStateDB::new();
        fund_account(&mut db_par, 1, 10000);
        fund_account(&mut db_par, 3, 10000);
        fund_account(&mut db_par, 5, 10000);
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
        assert!(db_par.get_object(&obj_id(1)).is_some(), "object 1 should exist");
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
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(2),
                    to: addr(3),
                    amount: 600,
                    nonce: 0,
                    signature: None,
                    public_key: None,
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
                }),
                // Partition 2: transfer C→D
                Transaction::Transfer(TransferTx {
                    from: addr(3),
                    to: addr(4),
                    amount: 2000,
                    nonce: 0,
                    signature: None,
                    public_key: None,
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
    }
}
