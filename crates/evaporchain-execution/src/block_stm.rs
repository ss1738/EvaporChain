//! Block-STM: Optimistic Parallel Execution Engine for EvaporChain
//!
//! Implements the Block-STM algorithm (as pioneered by Aptos/Diem) adapted for
//! EvaporChain's thermodynamic object model. All transactions in a block execute
//! optimistically in parallel. A multi-version data structure (MVCC) tracks
//! per-transaction reads and writes. A collaborative scheduler detects conflicts
//! and re-executes aborted transactions until a fixed point is reached.
//!
//! **Determinism guarantee**: The final state always matches sequential execution
//! in block order, regardless of thread scheduling.
//!
//! Architecture:
//!   1. Multi-Version Memory (MVMemory) — versioned key-value store
//!   2. Collaborative Scheduler — coordinates execution, validation, abort
//!   3. Block-STM Executor — orchestrates parallel execution loop
//!
//! Key insight for EvaporChain: the object-based state model gives natural
//! key granularity (AccountAddress, ObjectId) — most transactions touch
//! independent objects, yielding high parallelism.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::RwLock;

use evaporchain_contracts::{ContractEngine, ContractTemplate};
use evaporchain_crypto::MerkleMountainRange;
use evaporchain_script::ScriptEngine;
use evaporchain_state::db::StateDB;
use evaporchain_state::EvaporationEngine;
use evaporchain_types::{
    Account, AccountAddress, Block, CreateObjectTx, Epoch, GhostRecord, ObjectId, ObjectState,
    RefreshTx, StateObject, Transaction, TransferTx, ValidatorExitTx, ValidatorStakeTx,
};
use rayon::prelude::*;
use tracing::{debug, info};

use crate::{
    fees, BlockExecutionResult, ExecutionEngine, ExecutionError,
    GAS_CALL_CONTRACT, GAS_CALL_SCRIPT, GAS_CREATE_OBJECT_BASE, GAS_CREATE_OBJECT_PER_BYTE,
    GAS_DEPLOY_CONTRACT, GAS_DEPLOY_SCRIPT, GAS_REFRESH, GAS_TRANSFER, GAS_VALIDATOR_CLAIM_STAKE,
    GAS_VALIDATOR_EXIT, GAS_VALIDATOR_STAKE,
};

// ═══════════════════════════════════════════════════════════════════════════
// §1  MVCC Data Structures
// ═══════════════════════════════════════════════════════════════════════════

/// A location in the state that can be read or written.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Location {
    AccountBalance(AccountAddress),
    AccountNonce(AccountAddress),
    AccountStorageDeposit(AccountAddress),
    AccountStorageBytes(AccountAddress),
    Object(ObjectId),
    Ghost(ObjectId),
}

/// A versioned value written by transaction `tx_index` at `incarnation`.
#[derive(Debug, Clone)]
enum MVValue {
    /// The transaction wrote this concrete value.
    Value(ValuePayload),
    /// The transaction's write was aborted — readers must wait/re-execute.
    #[allow(dead_code)]
    Estimate,
}

/// Concrete state values stored in multi-version memory.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum ValuePayload {
    Balance(u64),
    Nonce(u64),
    StorageDeposit(u64),
    StorageBytes(u64),
    Object(StateObject),
    ObjectDeleted,
    Ghost(GhostRecord),
    GhostRemoved,
}

/// Version identifier: (tx_index, incarnation).
type Version = (u32, u32);

/// Number of shards for MVMemory. Must be a power of 2.
const MV_SHARDS: usize = 64;

/// Multi-version memory: the core MVCC data structure.
/// For each Location, stores a BTreeMap of tx_index → (incarnation, MVValue).
/// Readers find the highest tx_index < their own that has a committed write.
///
/// Sharded into 64 independent RwLocks to minimize contention during parallel
/// execution. Each Location is hashed to a shard; concurrent reads/writes to
/// different shards never contend.
pub(crate) struct MVMemory {
    #[allow(clippy::type_complexity)]
    shards: Vec<RwLock<HashMap<Location, BTreeMap<u32, (u32, MVValue)>>>>,
}

impl MVMemory {
    fn new() -> Self {
        Self {
            shards: (0..MV_SHARDS)
                .map(|_| RwLock::new(HashMap::new()))
                .collect(),
        }
    }

    /// Hash a location to a shard index.
    fn shard_index(loc: &Location) -> usize {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        loc.hash(&mut hasher);
        hasher.finish() as usize & (MV_SHARDS - 1)
    }

    /// Write a value at (location, tx_index, incarnation).
    fn write(&self, loc: Location, tx_index: u32, incarnation: u32, value: ValuePayload) {
        let shard = Self::shard_index(&loc);
        let mut map = self.shards[shard].write().unwrap();
        let versions = map.entry(loc).or_default();
        versions.insert(tx_index, (incarnation, MVValue::Value(value)));
    }

    /// Mark a location as "estimate" for a given tx_index (used on abort).
    #[allow(dead_code)]
    fn mark_estimate(&self, loc: Location, tx_index: u32) {
        let shard = Self::shard_index(&loc);
        let mut map = self.shards[shard].write().unwrap();
        if let Some(versions) = map.get_mut(&loc) {
            if let Some(entry) = versions.get_mut(&tx_index) {
                entry.1 = MVValue::Estimate;
            }
        }
    }

    /// Delete all writes by tx_index (used before re-execution).
    fn delete_writes(&self, tx_index: u32, locations: &[Location]) {
        // Group locations by shard to minimize lock acquisitions
        let mut by_shard: HashMap<usize, Vec<&Location>> = HashMap::new();
        for loc in locations {
            by_shard
                .entry(Self::shard_index(loc))
                .or_default()
                .push(loc);
        }
        for (shard, locs) in by_shard {
            let mut map = self.shards[shard].write().unwrap();
            for loc in locs {
                if let Some(versions) = map.get_mut(loc) {
                    versions.remove(&tx_index);
                }
            }
        }
    }

    /// Read the latest version of `loc` written by any transaction < `tx_index`.
    /// Returns:
    ///   Ok(Some((version, payload))) — found a committed write
    ///   Ok(None) — no prior transaction wrote to this location
    ///   Err(blocking_tx) — found an Estimate (abort marker), caller should wait
    fn read(
        &self,
        loc: &Location,
        tx_index: u32,
    ) -> Result<Option<(Version, ValuePayload)>, u32> {
        let shard = Self::shard_index(loc);
        let map = self.shards[shard].read().unwrap();
        if let Some(versions) = map.get(loc) {
            if let Some((&writer_tx, (incarnation, mv_val))) = versions.range(..tx_index).next_back() {
                match mv_val {
                    MVValue::Value(payload) => {
                        return Ok(Some(((writer_tx, *incarnation), payload.clone())));
                    }
                    MVValue::Estimate => {
                        return Err(writer_tx);
                    }
                }
            }
        }
        Ok(None)
    }

    /// Iterate all locations and their versions (for materialization).
    fn for_each_location<F>(&self, mut f: F)
    where
        F: FnMut(&Location, &BTreeMap<u32, (u32, MVValue)>),
    {
        for shard in &self.shards {
            let map = shard.read().unwrap();
            for (loc, versions) in map.iter() {
                f(loc, versions);
            }
        }
    }
}

/// Read-set entry: records what a transaction read and from which version.
#[derive(Debug, Clone)]
pub(crate) struct ReadEntry {
    pub location: Location,
    /// None = read from base state; Some(version) = read from MVMemory.
    pub version: Option<Version>,
}

/// Write-set entry: records what a transaction wrote.
#[derive(Debug, Clone)]
pub(crate) struct WriteEntry {
    pub location: Location,
    pub(crate) value: ValuePayload,
}

// ═══════════════════════════════════════════════════════════════════════════
// §2  Transaction View — MVCC-aware state access layer
// ═══════════════════════════════════════════════════════════════════════════

/// A view of state for a single transaction execution.
/// Reads go through MVMemory (finding writes by earlier txs) then fall back
/// to the base StateDB. Writes are buffered locally and flushed to MVMemory
/// after execution.
struct TxView<'a> {
    tx_index: u32,
    incarnation: u32,
    mv_memory: &'a MVMemory,
    base_db: &'a dyn StateDB,

    // Buffered writes and read tracking
    write_buffer: Vec<WriteEntry>,
    read_set: Vec<ReadEntry>,
    /// Checkpoint index in write_buffer after fee deduction.
    /// On tx failure, truncate to this point to keep fee writes but revert execution writes.
    fee_checkpoint: usize,

    // Local overlay for this transaction's writes (so it can read its own writes)
    local_accounts: HashMap<AccountAddress, Account>,
    local_objects: HashMap<ObjectId, StateObject>,
    local_objects_deleted: HashSet<ObjectId>,
    local_ghosts: HashMap<ObjectId, GhostRecord>,
    local_ghosts_removed: HashSet<ObjectId>,
}

impl<'a> TxView<'a> {
    fn new(
        tx_index: u32,
        incarnation: u32,
        mv_memory: &'a MVMemory,
        base_db: &'a dyn StateDB,
    ) -> Self {
        Self {
            tx_index,
            incarnation,
            mv_memory,
            base_db,
            write_buffer: Vec::new(),
            read_set: Vec::new(),
            fee_checkpoint: 0,
            local_accounts: HashMap::new(),
            local_objects: HashMap::new(),
            local_objects_deleted: HashSet::new(),
            local_ghosts: HashMap::new(),
            local_ghosts_removed: HashSet::new(),
        }
    }

    /// Mark the current write buffer position as the fee checkpoint.
    /// Call this after fee deduction, before tx execution.
    fn checkpoint_after_fees(&mut self) {
        self.fee_checkpoint = self.write_buffer.len();
    }

    /// Revert all writes after the fee checkpoint (keeps fee deduction).
    /// Also reverts local_accounts to only reflect fee deduction state.
    fn revert_to_fee_checkpoint(&mut self) {
        self.write_buffer.truncate(self.fee_checkpoint);
        // Rebuild local state from remaining write buffer
        self.local_accounts.clear();
        self.local_objects.clear();
        self.local_objects_deleted.clear();
        self.local_ghosts.clear();
        self.local_ghosts_removed.clear();
        for entry in &self.write_buffer {
            match (&entry.location, &entry.value) {
                (Location::AccountBalance(addr), ValuePayload::Balance(bal)) => {
                    let acct = self.local_accounts.entry(*addr).or_insert(Account {
                        address: *addr,
                        balance: 0,
                        nonce: 0,
                    storage_deposit: 0,
                    storage_bytes: 0,
                    last_touched_epoch: 0,
                    });
                    acct.balance = *bal;
                }
                (Location::AccountNonce(addr), ValuePayload::Nonce(nonce)) => {
                    let acct = self.local_accounts.entry(*addr).or_insert(Account {
                        address: *addr,
                        balance: 0,
                        nonce: 0,
                    storage_deposit: 0,
                    storage_bytes: 0,
                    last_touched_epoch: 0,
                    });
                    acct.nonce = *nonce;
                }
                _ => {}
            }
        }
    }

    /// Read an account's balance. Checks: local writes → MVMemory → base DB.
    fn read_balance(&mut self, addr: &AccountAddress) -> Result<u64, u32> {
        // Check local writes first
        if let Some(acct) = self.local_accounts.get(addr) {
            return Ok(acct.balance);
        }

        let loc = Location::AccountBalance(*addr);
        match self.mv_memory.read(&loc, self.tx_index)? {
            Some((version, ValuePayload::Balance(bal))) => {
                self.read_set.push(ReadEntry {
                    location: loc,
                    version: Some(version),
                });
                Ok(bal)
            }
            _ => {
                self.read_set.push(ReadEntry {
                    location: loc,
                    version: None,
                });
                Ok(self.base_db.get_account(addr).map_or(0, |a| a.balance))
            }
        }
    }

    /// Read an account's nonce.
    fn read_nonce(&mut self, addr: &AccountAddress) -> Result<u64, u32> {
        if let Some(acct) = self.local_accounts.get(addr) {
            return Ok(acct.nonce);
        }

        let loc = Location::AccountNonce(*addr);
        match self.mv_memory.read(&loc, self.tx_index)? {
            Some((version, ValuePayload::Nonce(n))) => {
                self.read_set.push(ReadEntry {
                    location: loc,
                    version: Some(version),
                });
                Ok(n)
            }
            _ => {
                self.read_set.push(ReadEntry {
                    location: loc,
                    version: None,
                });
                Ok(self.base_db.get_account(addr).map_or(0, |a| a.nonce))
            }
        }
    }

    /// Read a state object.
    fn read_object(&mut self, id: &ObjectId) -> Result<Option<StateObject>, u32> {
        if self.local_objects_deleted.contains(id) {
            return Ok(None);
        }
        if let Some(obj) = self.local_objects.get(id) {
            return Ok(Some(obj.clone()));
        }

        let loc = Location::Object(*id);
        match self.mv_memory.read(&loc, self.tx_index)? {
            Some((version, ValuePayload::Object(obj))) => {
                self.read_set.push(ReadEntry {
                    location: loc,
                    version: Some(version),
                });
                Ok(Some(obj))
            }
            Some((version, ValuePayload::ObjectDeleted)) => {
                self.read_set.push(ReadEntry {
                    location: loc,
                    version: Some(version),
                });
                Ok(None)
            }
            _ => {
                self.read_set.push(ReadEntry {
                    location: loc,
                    version: None,
                });
                Ok(self.base_db.get_object(id).cloned())
            }
        }
    }

    /// Read a ghost record.
    fn read_ghost(&mut self, id: &ObjectId) -> Result<Option<GhostRecord>, u32> {
        if self.local_ghosts_removed.contains(id) {
            return Ok(None);
        }
        if let Some(ghost) = self.local_ghosts.get(id) {
            return Ok(Some(ghost.clone()));
        }

        let loc = Location::Ghost(*id);
        match self.mv_memory.read(&loc, self.tx_index)? {
            Some((version, ValuePayload::Ghost(g))) => {
                self.read_set.push(ReadEntry {
                    location: loc,
                    version: Some(version),
                });
                Ok(Some(g))
            }
            Some((version, ValuePayload::GhostRemoved)) => {
                self.read_set.push(ReadEntry {
                    location: loc,
                    version: Some(version),
                });
                Ok(None)
            }
            _ => {
                self.read_set.push(ReadEntry {
                    location: loc,
                    version: None,
                });
                Ok(self.base_db.get_ghost(id).cloned())
            }
        }
    }

    /// Write an account (balance + nonce). Stamps `last_touched_epoch` to
    /// `epoch` since this writer is only invoked when balance/nonce just
    /// mutated under a tx — i.e. the per-account demurrage anchor needs
    /// to slide forward to `epoch`.
    fn write_account(&mut self, addr: AccountAddress, balance: u64, nonce: u64, epoch: u64) {
        let acct = Account {
            address: addr,
            balance,
            nonce,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: epoch,
        };
        self.local_accounts.insert(addr, acct);
        self.write_buffer.push(WriteEntry {
            location: Location::AccountBalance(addr),
            value: ValuePayload::Balance(balance),
        });
        self.write_buffer.push(WriteEntry {
            location: Location::AccountNonce(addr),
            value: ValuePayload::Nonce(nonce),
        });
    }

    /// Read storage deposit for an account.
    fn read_storage_deposit(&mut self, addr: &AccountAddress) -> Result<u64, u32> {
        if let Some(acct) = self.local_accounts.get(addr) {
            return Ok(acct.storage_deposit);
        }
        let loc = Location::AccountStorageDeposit(*addr);
        match self.mv_memory.read(&loc, self.tx_index)? {
            Some((version, ValuePayload::StorageDeposit(d))) => {
                self.read_set.push(ReadEntry { location: loc, version: Some(version) });
                Ok(d)
            }
            _ => {
                self.read_set.push(ReadEntry { location: loc, version: None });
                Ok(self.base_db.get_account(addr).map_or(0, |a| a.storage_deposit))
            }
        }
    }

    /// Read storage bytes for an account.
    fn read_storage_bytes(&mut self, addr: &AccountAddress) -> Result<u64, u32> {
        if let Some(acct) = self.local_accounts.get(addr) {
            return Ok(acct.storage_bytes);
        }
        let loc = Location::AccountStorageBytes(*addr);
        match self.mv_memory.read(&loc, self.tx_index)? {
            Some((version, ValuePayload::StorageBytes(b))) => {
                self.read_set.push(ReadEntry { location: loc, version: Some(version) });
                Ok(b)
            }
            _ => {
                self.read_set.push(ReadEntry { location: loc, version: None });
                Ok(self.base_db.get_account(addr).map_or(0, |a| a.storage_bytes))
            }
        }
    }

    /// Write updated storage deposit and bytes for an account.
    fn write_storage_fields(&mut self, addr: AccountAddress, deposit: u64, bytes: u64) {
        if let Some(acct) = self.local_accounts.get_mut(&addr) {
            acct.storage_deposit = deposit;
            acct.storage_bytes = bytes;
        }
        self.write_buffer.push(WriteEntry {
            location: Location::AccountStorageDeposit(addr),
            value: ValuePayload::StorageDeposit(deposit),
        });
        self.write_buffer.push(WriteEntry {
            location: Location::AccountStorageBytes(addr),
            value: ValuePayload::StorageBytes(bytes),
        });
    }

    /// Write a state object.
    fn write_object(&mut self, obj: StateObject) {
        let id = obj.id;
        self.local_objects_deleted.remove(&id);
        self.write_buffer.push(WriteEntry {
            location: Location::Object(id),
            value: ValuePayload::Object(obj.clone()),
        });
        self.local_objects.insert(id, obj);
    }

    /// Delete a state object.
    #[allow(dead_code)]
    fn delete_object(&mut self, id: ObjectId) {
        self.local_objects.remove(&id);
        self.local_objects_deleted.insert(id);
        self.write_buffer.push(WriteEntry {
            location: Location::Object(id),
            value: ValuePayload::ObjectDeleted,
        });
    }

    /// Write a ghost record.
    #[allow(dead_code)]
    fn write_ghost(&mut self, record: GhostRecord) {
        let id = record.object_id;
        self.local_ghosts_removed.remove(&id);
        self.write_buffer.push(WriteEntry {
            location: Location::Ghost(id),
            value: ValuePayload::Ghost(record.clone()),
        });
        self.local_ghosts.insert(id, record);
    }

    /// Remove a ghost record.
    fn remove_ghost(&mut self, id: ObjectId) {
        self.local_ghosts.remove(&id);
        self.local_ghosts_removed.insert(id);
        self.write_buffer.push(WriteEntry {
            location: Location::Ghost(id),
            value: ValuePayload::GhostRemoved,
        });
    }

    /// Flush all buffered writes to MVMemory. Returns the write locations.
    fn flush_writes(self) -> (Vec<ReadEntry>, Vec<Location>) {
        let mut write_locs = Vec::with_capacity(self.write_buffer.len());
        for entry in &self.write_buffer {
            self.mv_memory.write(
                entry.location.clone(),
                self.tx_index,
                self.incarnation,
                entry.value.clone(),
            );
            write_locs.push(entry.location.clone());
        }
        (self.read_set, write_locs)
    }
}

// §3  Scheduler removed — using wave-based execution (see execute_parallel)

// ═══════════════════════════════════════════════════════════════════════════
// §4  Transaction Execution Logic
// ═══════════════════════════════════════════════════════════════════════════

/// Result of executing a single transaction in Block-STM.
#[derive(Debug)]
enum TxExecResult {
    /// Transaction executed successfully.
    Success { gas_used: u64, fee: u64 },
    /// Transaction failed but fee is still charged.
    Failed { fee: u64 },
    /// Need to wait for another transaction (dependency on Estimate).
    #[allow(dead_code)]
    Blocked(u32),
}

/// Execute a single transaction against a TxView.
fn execute_tx(
    view: &mut TxView,
    tx: &Transaction,
    epoch: Epoch,
    fee_controller: &Option<fees::PidFeeController>,
) -> TxExecResult {
    let tx_gas = estimate_gas(tx);

    // Fee deduction
    let tx_fee = if let Some(fc) = fee_controller {
        let gas_fee = fc.compute_gas_fee(tx_gas, 0);
        let extra_fee = match tx {
            Transaction::CreateObject(create) => fc.compute_creation_deposit(create.data.len()),
            Transaction::Refresh(refresh) => fc.compute_refresh_fee(refresh.energy_deposit),
            _ => 0,
        };
        let total_tx_fee = match gas_fee.checked_add(extra_fee) {
            Some(v) => v,
            None => return TxExecResult::Failed { fee: 0 },
        };

        if let Some(sender_addr) = tx.sender() {
            let balance = match view.read_balance(sender_addr) {
                Ok(b) => b,
                Err(blocked) => return TxExecResult::Blocked(blocked),
            };
            if balance < total_tx_fee {
                return TxExecResult::Failed { fee: 0 };
            }
            let nonce = match view.read_nonce(sender_addr) {
                Ok(n) => n,
                Err(blocked) => return TxExecResult::Blocked(blocked),
            };
            let new_balance = match balance.checked_sub(total_tx_fee) {
                Some(v) => v,
                None => return TxExecResult::Failed { fee: 0 },
            };
            view.write_account(*sender_addr, new_balance, nonce, epoch);
        }
        total_tx_fee
    } else {
        0
    };

    // Checkpoint: everything before this is fee deduction (persists on failure)
    view.checkpoint_after_fees();

    // Execute the transaction
    let result = match tx {
        Transaction::Transfer(t) => exec_transfer(view, t, epoch),
        Transaction::CreateObject(t) => exec_create_object(view, t, epoch),
        Transaction::Refresh(t) => exec_refresh(view, t, epoch),
        Transaction::ValidatorStake(t) => exec_validator_stake(view, t, epoch),
        Transaction::ValidatorExit(t) => exec_validator_exit(view, t, epoch),
        Transaction::ValidatorClaimStake(_) => {
            Err(TxViewError::ExecutionError(ExecutionError::ContractError(
                "validator claim stake executes in serial phase".into(),
            )))
        }
        // Contract/script/privacy txs cannot run in parallel (global mutable state)
        Transaction::DeployContract(_)
        | Transaction::CallContract(_)
        | Transaction::DeployScript(_)
        | Transaction::CallScript(_)
        | Transaction::Shield(_)
        | Transaction::Unshield(_)
        | Transaction::PrivateTransfer(_)
        | Transaction::Deferred(_) => {
            Err(TxViewError::ExecutionError(ExecutionError::ContractError(
                "contract/script/privacy/deferred txs execute in serial phase".into(),
            )))
        }
        Transaction::Blob(blob) => {
            if blob.data.is_empty() {
                Err(TxViewError::ExecutionError(ExecutionError::ContractError("blob data cannot be empty".into())))
            } else if blob.data.len() > crate::MAX_BLOB_SIZE {
                Err(TxViewError::ExecutionError(ExecutionError::ContractError(format!(
                    "blob size {} exceeds limit {}", blob.data.len(), crate::MAX_BLOB_SIZE
                ))))
            } else if blob.namespace_id == 0 {
                Err(TxViewError::ExecutionError(ExecutionError::ContractError("reserved namespace_id 0".into())))
            } else {
                Ok(())
            }
        }
        Transaction::Governance(_) => {
            Err(TxViewError::ExecutionError(ExecutionError::ContractError(
                "governance txs execute in serial phase".into(),
            )))
        }
        Transaction::MultiSig(_) => {
            Err(TxViewError::ExecutionError(ExecutionError::ContractError(
                "multi-sig txs execute in serial phase".into(),
            )))
        }
        Transaction::UserOp(_) => {
            Err(TxViewError::ExecutionError(ExecutionError::ContractError(
                "user-op txs execute in serial phase".into(),
            )))
        }
        Transaction::UpgradeContract(_) => {
            Err(TxViewError::ExecutionError(ExecutionError::ContractError(
                "upgrade txs execute in serial phase".into(),
            )))
        }
        Transaction::Delegate(_) => {
            Err(TxViewError::ExecutionError(ExecutionError::ContractError(
                "delegation txs execute in serial phase".into(),
            )))
        }
        Transaction::Undelegate(_) => {
            Err(TxViewError::ExecutionError(ExecutionError::ContractError(
                "delegation txs execute in serial phase".into(),
            )))
        }
        Transaction::RotateValidatorKey(_) => {
            Err(TxViewError::ExecutionError(ExecutionError::ContractError(
                "validator key rotation executes in serial phase".into(),
            )))
        }
        Transaction::ClaimDelegation(_) => {
            Err(TxViewError::ExecutionError(ExecutionError::ContractError(
                "delegation txs execute in serial phase".into(),
            )))
        }
    };

    match result {
        Ok(()) => TxExecResult::Success {
            gas_used: tx_gas,
            fee: tx_fee,
        },
        Err(TxViewError::Blocked(blocked_by)) => TxExecResult::Blocked(blocked_by),
        Err(TxViewError::ExecutionError(_e)) => {
            // Revert execution writes but keep fee deduction
            view.revert_to_fee_checkpoint();
            TxExecResult::Failed { fee: tx_fee }
        }
    }
}

/// Errors that can occur during tx execution in Block-STM.
#[derive(Debug)]
enum TxViewError {
    Blocked(u32),
    ExecutionError(ExecutionError),
}

impl From<ExecutionError> for TxViewError {
    fn from(e: ExecutionError) -> Self {
        TxViewError::ExecutionError(e)
    }
}

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
        Transaction::Shield(_) => crate::privacy_exec::GAS_SHIELD,
        Transaction::Unshield(_) => crate::privacy_exec::GAS_UNSHIELD,
        Transaction::PrivateTransfer(ptx) => {
            crate::privacy_exec::PrivacyExecutor::estimate_private_transfer_gas(ptx)
        }
        Transaction::Deferred(dtx) => {
            crate::temporal::GAS_DEFERRED_SUBMIT
                .saturating_add(crate::temporal::GAS_PER_GUARD.saturating_mul(dtx.guards.len() as u64))
        }
        Transaction::Blob(tx) => {
            crate::GAS_CREATE_OBJECT_BASE.saturating_add(crate::GAS_CREATE_OBJECT_PER_BYTE.saturating_mul(tx.data.len() as u64))
        }
        Transaction::Governance(_) => GAS_VALIDATOR_CLAIM_STAKE,
        Transaction::MultiSig(_) => GAS_VALIDATOR_CLAIM_STAKE,
        Transaction::UserOp(tx) => crate::GAS_USER_OP.saturating_add(tx.call_data.len() as u64 * 16),
        Transaction::UpgradeContract(tx) => crate::GAS_UPGRADE_CONTRACT.saturating_add(tx.new_bytecode.len() as u64 * 200),
        Transaction::Delegate(_) => crate::GAS_DELEGATE,
        Transaction::Undelegate(_) => crate::GAS_UNDELEGATE,
        Transaction::RotateValidatorKey(_) => crate::GAS_ROTATE_VALIDATOR_KEY,
        Transaction::ClaimDelegation(_) => crate::GAS_CLAIM_DELEGATION,
    }
}

// ── Per-transaction execution functions ──

fn exec_transfer(view: &mut TxView, tx: &TransferTx, epoch: Epoch) -> Result<(), TxViewError> {
    if tx.from == tx.to {
        return Err(ExecutionError::SelfTransfer.into());
    }
    if tx.amount == 0 {
        return Err(ExecutionError::ZeroAmount.into());
    }

    let is_faucet = tx.from == [0u8; 32];
    let sender_balance = view.read_balance(&tx.from).map_err(TxViewError::Blocked)?;
    let sender_nonce = view.read_nonce(&tx.from).map_err(TxViewError::Blocked)?;

    if !is_faucet && sender_nonce != tx.nonce {
        return Err(ExecutionError::InvalidNonce {
            expected: sender_nonce,
            got: tx.nonce,
        }
        .into());
    }
    if sender_balance < tx.amount {
        return Err(ExecutionError::InsufficientBalance {
            account: hex::encode(tx.from),
            available: sender_balance,
            required: tx.amount,
        }
        .into());
    }

    // Note: fee was already deducted, so balance may already be reduced
    let new_sender_balance = sender_balance.checked_sub(tx.amount).ok_or_else(|| {
        TxViewError::ExecutionError(ExecutionError::InsufficientBalance {
            account: hex::encode(tx.from),
            available: sender_balance,
            required: tx.amount,
        })
    })?;
    let new_sender_nonce = if is_faucet {
        sender_nonce
    } else {
        sender_nonce.checked_add(1).ok_or_else(|| {
            TxViewError::ExecutionError(ExecutionError::ContractError(
                "nonce overflow".into(),
            ))
        })?
    };
    view.write_account(tx.from, new_sender_balance, new_sender_nonce, epoch);

    let recv_balance = view.read_balance(&tx.to).map_err(TxViewError::Blocked)?;
    let recv_nonce = view.read_nonce(&tx.to).map_err(TxViewError::Blocked)?;
    let new_recv_balance = recv_balance.checked_add(tx.amount).ok_or_else(|| {
        TxViewError::ExecutionError(ExecutionError::InsufficientBalance {
            account: hex::encode(tx.to),
            available: recv_balance,
            required: tx.amount,
        })
    })?;
    view.write_account(tx.to, new_recv_balance, recv_nonce, epoch);

    Ok(())
}

fn exec_create_object(
    view: &mut TxView,
    tx: &CreateObjectTx,
    epoch: Epoch,
) -> Result<(), TxViewError> {
    let existing = view.read_object(&tx.object_id).map_err(TxViewError::Blocked)?;
    if existing.is_some() {
        return Err(ExecutionError::ObjectAlreadyExists(hex::encode(tx.object_id)).into());
    }

    // Storage rent enforcement: require MIN_STORAGE_DEPOSIT from creator.
    let object_bytes = {
        const BASE_OBJECT_BYTES: u64 = 97;
        BASE_OBJECT_BYTES.saturating_add(tx.data.len() as u64)
    };
    let bal = view.read_balance(&tx.creator).map_err(TxViewError::Blocked)?;
    if bal < evaporchain_types::MIN_STORAGE_DEPOSIT {
        return Err(ExecutionError::InsufficientBalance {
            account: hex::encode(tx.creator),
            available: bal,
            required: evaporchain_types::MIN_STORAGE_DEPOSIT,
        }.into());
    }
    let nonce = view.read_nonce(&tx.creator).map_err(TxViewError::Blocked)?;
    view.write_account(tx.creator, bal - evaporchain_types::MIN_STORAGE_DEPOSIT, nonce, epoch);

    let cur_deposit = view.read_storage_deposit(&tx.creator).map_err(TxViewError::Blocked)?;
    let cur_bytes = view.read_storage_bytes(&tx.creator).map_err(TxViewError::Blocked)?;
    view.write_storage_fields(
        tx.creator,
        cur_deposit.saturating_add(evaporchain_types::MIN_STORAGE_DEPOSIT),
        cur_bytes.saturating_add(object_bytes),
    );

    view.write_object(StateObject {
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
    });

    Ok(())
}

fn exec_refresh(view: &mut TxView, tx: &RefreshTx, epoch: Epoch) -> Result<(), TxViewError> {
    // Try refresh on active/grace object
    let obj = view.read_object(&tx.object_id).map_err(TxViewError::Blocked)?;
    if let Some(mut obj) = obj {
        // Inline refresh logic (can't use RefreshEngine directly with TxView)
        obj.energy = obj.energy.saturating_add(tx.energy_deposit);
        obj.last_refreshed = epoch;
        if obj.state == ObjectState::Grace {
            obj.state = ObjectState::Active;
            obj.grace_epoch = None;
        }
        view.write_object(obj);
        return Ok(());
    }

    // Try resurrection from ghost
    let ghost = view.read_ghost(&tx.object_id).map_err(TxViewError::Blocked)?;
    if let Some(ghost) = ghost {
        // Resurrect: create new object from ghost data.
        // Matches RefreshEngine::resurrect — preserves evaporated_at as created_at.
        let data = ghost.original_data.clone().unwrap_or_default();
        let obj = StateObject {
            id: ghost.object_id,
            owner: ghost.owner,
            energy: tx.energy_deposit,
            half_life: 100,
            created_at: ghost.evaporated_at,
            last_refreshed: epoch,
            state: ObjectState::Resurrected,
            grace_epoch: None,
            data,
            decay_curve: None,
        };
        view.write_object(obj);
        view.remove_ghost(tx.object_id);
        return Ok(());
    }

    Err(ExecutionError::ObjectNotFound(hex::encode(tx.object_id)).into())
}

fn exec_validator_stake(view: &mut TxView, tx: &ValidatorStakeTx, epoch: Epoch) -> Result<(), TxViewError> {
    if tx.stake_amount == 0 {
        return Err(ExecutionError::ZeroAmount.into());
    }

    let balance = view
        .read_balance(&tx.validator_address)
        .map_err(TxViewError::Blocked)?;
    let nonce = view
        .read_nonce(&tx.validator_address)
        .map_err(TxViewError::Blocked)?;

    if nonce != tx.nonce {
        return Err(ExecutionError::InvalidNonce {
            expected: nonce,
            got: tx.nonce,
        }
        .into());
    }
    if balance < tx.stake_amount {
        return Err(ExecutionError::InsufficientBalance {
            account: hex::encode(tx.validator_address),
            available: balance,
            required: tx.stake_amount,
        }
        .into());
    }

    let new_balance = balance.checked_sub(tx.stake_amount).ok_or_else(|| {
        TxViewError::ExecutionError(ExecutionError::InsufficientBalance {
            account: hex::encode(tx.validator_address),
            available: balance,
            required: tx.stake_amount,
        })
    })?;
    let new_nonce = nonce.checked_add(1).ok_or_else(|| {
        TxViewError::ExecutionError(ExecutionError::ContractError(
            "nonce overflow".into(),
        ))
    })?;
    view.write_account(tx.validator_address, new_balance, new_nonce, epoch);
    Ok(())
}

fn exec_validator_exit(view: &mut TxView, tx: &ValidatorExitTx, epoch: Epoch) -> Result<(), TxViewError> {
    let balance = view
        .read_balance(&tx.validator_address)
        .map_err(TxViewError::Blocked)?;
    let nonce = view
        .read_nonce(&tx.validator_address)
        .map_err(TxViewError::Blocked)?;

    if nonce != tx.nonce {
        return Err(ExecutionError::InvalidNonce {
            expected: nonce,
            got: tx.nonce,
        }
        .into());
    }

    let new_nonce = nonce.checked_add(1).ok_or_else(|| {
        TxViewError::ExecutionError(ExecutionError::ContractError(
            "nonce overflow".into(),
        ))
    })?;
    view.write_account(tx.validator_address, balance, new_nonce, epoch);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// §5  Block-STM Executor
// ═══════════════════════════════════════════════════════════════════════════

/// Block-STM parallel execution engine.
///
/// Uses optimistic concurrency control with multi-version memory to execute
/// all transactions in a block in parallel, while guaranteeing deterministic
/// results identical to sequential execution.
///
/// Performance characteristics:
/// - Best case (no conflicts): O(N/P) where P = thread count
/// - Moderate conflicts: O(N/P + R) where R = re-executions from aborts
/// - Worst case (total conflict): O(N) sequential with overhead
///
/// The Block-STM approach is strictly superior to static partitioning for
/// workloads with partial conflicts, as it extracts maximum parallelism
/// dynamically rather than conservatively grouping by access keys.
pub struct BlockStmExecutor {
    evaporation_engine: EvaporationEngine,
    mmr: MerkleMountainRange,
    verify_signatures: bool,
    fee_controller: Option<fees::PidFeeController>,
    pub block_gas_limit: u64,
    pub contract_engine: ContractEngine,
    pub script_engine: ScriptEngine,
    /// Zero-knowledge privacy execution engine.
    pub privacy_executor: crate::privacy_exec::PrivacyExecutor,
    /// Deferred transaction queue (temporal engine).
    pub deferred_queue: crate::temporal::DeferredQueue,
    /// Decay watcher engine (energy threshold triggers).
    pub decay_watchers: crate::temporal::DecayWatcherEngine,
    /// Minimum number of transactions to trigger parallel execution.
    /// Below this threshold, sequential execution is used (less overhead).
    pub parallel_threshold: usize,
    pub chain_id: String,
}

impl BlockStmExecutor {
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
            parallel_threshold: 4,
            chain_id: String::new(),
        }
    }

    /// Test-friendly constructor with a small privacy tree.
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
            privacy_executor: crate::privacy_exec::PrivacyExecutor::with_depth(4),
            deferred_queue: crate::temporal::DeferredQueue::new(),
            decay_watchers: crate::temporal::DecayWatcherEngine::new(),
            parallel_threshold: 4,
            chain_id: String::new(),
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
            parallel_threshold: 4,
            chain_id: String::new(),
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
            parallel_threshold: 4,
            chain_id: String::new(),
        }
    }

    pub fn fee_controller(&self) -> Option<&fees::PidFeeController> {
        self.fee_controller.as_ref()
    }

    pub fn fee_controller_mut(&mut self) -> Option<&mut fees::PidFeeController> {
        self.fee_controller.as_mut()
    }

    /// Execute a block's parallelizable transactions using Block-STM.
    ///
    /// Wave-based approach:
    ///   Wave 1: Execute ALL txs optimistically in parallel
    ///   Wave 2+: Validate in order; re-execute any tx whose reads are stale
    ///   Repeat until all txs validate (convergence guaranteed — at worst,
    ///   becomes sequential after N waves).
    fn execute_parallel(
        &self,
        db: &dyn StateDB,
        parallel_txs: &[&Transaction],
        epoch: Epoch,
        block_gas_limit: u64,
    ) -> ParallelResult {
        let num_txs = parallel_txs.len() as u32;
        if num_txs == 0 {
            return ParallelResult::default();
        }

        let mv_memory = MVMemory::new();
        let verify_sigs = self.verify_signatures;
        let fee_ctrl = &self.fee_controller;
        let chain_id = &self.chain_id;

        // Per-transaction state
        let mut incarnations: Vec<u32> = vec![0; num_txs as usize];
        let mut read_sets: Vec<Vec<ReadEntry>> = vec![Vec::new(); num_txs as usize];
        let mut write_locs: Vec<Vec<Location>> = vec![Vec::new(); num_txs as usize];
        let mut results: Vec<Option<TxExecResult>> = (0..num_txs).map(|_| None).collect();

        // ── Wave 1: Execute ALL transactions in parallel ──
        let wave1: Vec<(usize, Vec<ReadEntry>, Vec<Location>, TxExecResult)> = (0..num_txs as usize)
            .into_par_iter()
            .map(|tx_idx| {
                let tx = parallel_txs[tx_idx];

                // Signature check
                if verify_sigs
                    && crate::parallel::ParallelExecutor::verify_tx_signature(true, tx, chain_id).is_err()
                {
                    return (tx_idx, Vec::new(), Vec::new(), TxExecResult::Failed { fee: 0 });
                }

                let mut view = TxView::new(tx_idx as u32, 0, &mv_memory, db);
                let result = execute_tx(&mut view, tx, epoch, fee_ctrl);

                match result {
                    TxExecResult::Blocked(_) => {
                        // Can't block on wave 1 (no prior writes) — treat as failed
                        (tx_idx, Vec::new(), Vec::new(), TxExecResult::Failed { fee: 0 })
                    }
                    _ => {
                        let (rs, wl) = view.flush_writes();
                        (tx_idx, rs, wl, result)
                    }
                }
            })
            .collect();

        // Store wave 1 results
        for (tx_idx, rs, wl, result) in wave1 {
            read_sets[tx_idx] = rs;
            write_locs[tx_idx] = wl;
            results[tx_idx] = Some(result);
        }

        // ── Wave 2+: Validate and re-execute until convergence ──
        // M-20: Track abort counts per tx for serial fallback; sort re-exec set
        // by dependency depth (lowest tx_idx first = deepest dependency).
        const MAX_ABORTS_BEFORE_SERIAL: u32 = 3;
        let mut abort_counts = vec![0u32; num_txs as usize];
        let max_waves = num_txs + 2; // Convergence bound
        for wave in 0..max_waves {
            let mut needs_reexec: Vec<u32> = Vec::new();

            // Validate all txs in order
            for tx_idx in 0..num_txs {
                let rs = &read_sets[tx_idx as usize];
                let mut valid = true;

                for entry in rs {
                    match mv_memory.read(&entry.location, tx_idx) {
                        Ok(current_val) => {
                            let current_version = current_val.as_ref().map(|(v, _)| *v);
                            if current_version != entry.version {
                                valid = false;
                                break;
                            }
                        }
                        Err(_) => {
                            valid = false;
                            break;
                        }
                    }
                }

                if !valid {
                    needs_reexec.push(tx_idx);
                }
            }

            if needs_reexec.is_empty() {
                debug!(wave, "Block-STM converged");
                break;
            }

            // M-20: sort by tx_idx (dependency order — lower indices first)
            needs_reexec.sort_unstable();

            // Separate serial-fallback txs from parallel re-exec
            let mut serial_txs = Vec::new();
            let mut parallel_reexec = Vec::new();
            for &tx_idx in &needs_reexec {
                abort_counts[tx_idx as usize] += 1;
                if abort_counts[tx_idx as usize] >= MAX_ABORTS_BEFORE_SERIAL {
                    serial_txs.push(tx_idx);
                } else {
                    parallel_reexec.push(tx_idx);
                }
            }

            debug!(
                wave,
                reexec_count = needs_reexec.len(),
                serial_fallback = serial_txs.len(),
                "Block-STM: re-executing invalidated txs"
            );

            // Clear old writes for all invalidated txs and bump incarnations
            for &tx_idx in &needs_reexec {
                mv_memory.delete_writes(tx_idx, &write_locs[tx_idx as usize]);
                incarnations[tx_idx as usize] += 1;
            }

            // M-20: Serial fallback for repeatedly-aborting txs (ordered execution)
            for &tx_idx in &serial_txs {
                let inc = incarnations[tx_idx as usize];
                let tx = parallel_txs[tx_idx as usize];

                if verify_sigs
                    && crate::parallel::ParallelExecutor::verify_tx_signature(true, tx, chain_id)
                        .is_err()
                {
                    results[tx_idx as usize] = Some(TxExecResult::Failed { fee: 0 });
                    continue;
                }

                let mut view = TxView::new(tx_idx, inc, &mv_memory, db);
                let result = execute_tx(&mut view, tx, epoch, fee_ctrl);
                match result {
                    TxExecResult::Blocked(_) => {
                        results[tx_idx as usize] = Some(TxExecResult::Failed { fee: 0 });
                    }
                    _ => {
                        let (rs, wl) = view.flush_writes();
                        read_sets[tx_idx as usize] = rs;
                        write_locs[tx_idx as usize] = wl;
                        results[tx_idx as usize] = Some(result);
                    }
                }
            }

            // Re-execute remaining invalidated txs in parallel.
            let reexec_results: Vec<(u32, Vec<ReadEntry>, Vec<Location>, TxExecResult)> =
                parallel_reexec
                    .par_iter()
                    .map(|&tx_idx| {
                        let inc = incarnations[tx_idx as usize];
                        let tx = parallel_txs[tx_idx as usize];

                        if verify_sigs
                            && crate::parallel::ParallelExecutor::verify_tx_signature(true, tx, chain_id)
                                .is_err()
                        {
                            return (tx_idx, Vec::new(), Vec::new(), TxExecResult::Failed { fee: 0 });
                        }

                        let mut view = TxView::new(tx_idx, inc, &mv_memory, db);
                        let result = execute_tx(&mut view, tx, epoch, fee_ctrl);

                        match result {
                            TxExecResult::Blocked(_) => {
                                (tx_idx, Vec::new(), Vec::new(), TxExecResult::Failed { fee: 0 })
                            }
                            _ => {
                                let (rs, wl) = view.flush_writes();
                                (tx_idx, rs, wl, result)
                            }
                        }
                    })
                    .collect();

            for (tx_idx, rs, wl, result) in reexec_results {
                read_sets[tx_idx as usize] = rs;
                write_locs[tx_idx as usize] = wl;
                results[tx_idx as usize] = Some(result);
            }
        }

        // Collect results, enforcing block gas limit in order
        let mut total_gas = 0u64;
        let mut total_fees = 0u64;
        let mut txs_executed = 0usize;
        let mut txs_failed = 0usize;
        let mut gas_exceeded_from: Option<u32> = None;

        for i in 0..num_txs {
            // Check if this tx would exceed the block gas limit
            if gas_exceeded_from.is_some() {
                // All remaining txs are over the limit — remove their writes
                mv_memory.delete_writes(i, &write_locs[i as usize]);
                txs_failed += 1;
                continue;
            }

            match results[i as usize].take() {
                Some(TxExecResult::Success { gas_used, fee }) => {
                    if block_gas_limit > 0 && total_gas + gas_used > block_gas_limit {
                        // Over limit — remove this tx's writes and fail all remaining
                        mv_memory.delete_writes(i, &write_locs[i as usize]);
                        gas_exceeded_from = Some(i);
                        txs_failed += 1;
                    } else {
                        txs_executed += 1;
                        total_gas += gas_used;
                        total_fees += fee;
                    }
                }
                Some(TxExecResult::Failed { fee }) => {
                    txs_failed += 1;
                    total_fees += fee;
                }
                _ => {
                    txs_failed += 1;
                }
            }
        }

        let final_writes = self.materialize_final_state(&mv_memory, db, num_txs, epoch);

        ParallelResult {
            txs_executed,
            txs_failed,
            gas_used: total_gas,
            total_fees,
            final_writes,
        }
    }

    /// Materialize the final committed state from MVMemory.
    /// For each location, takes the write from the highest tx_index.
    fn materialize_final_state(
        &self,
        mv_memory: &MVMemory,
        base_db: &dyn StateDB,
        _num_txs: u32,
        epoch: u64,
    ) -> FinalWrites {
        let mut writes = FinalWrites::default();

        mv_memory.for_each_location(|loc, versions| {
            // Find the highest tx_index with a committed value
            if let Some((&_tx_idx, (_inc, MVValue::Value(payload)))) = versions.iter().next_back()
            {
                match (loc, payload) {
                    (Location::AccountBalance(addr), ValuePayload::Balance(bal)) => {
                        // Seed from base DB so we don't clobber nonce with 0
                        let base = base_db.get_account(addr);
                        let entry = writes.accounts.entry(*addr).or_insert_with(|| {
                            base.map_or((0, 0, 0), |a| (a.balance, a.nonce, a.last_touched_epoch))
                        });
                        entry.0 = *bal;
                        // Balance just mutated under a tx — slide demurrage
                        // anchor forward to this block's epoch.
                        entry.2 = epoch;
                    }
                    (Location::AccountNonce(addr), ValuePayload::Nonce(nonce)) => {
                        let base = base_db.get_account(addr);
                        let entry = writes.accounts.entry(*addr).or_insert_with(|| {
                            base.map_or((0, 0, 0), |a| (a.balance, a.nonce, a.last_touched_epoch))
                        });
                        entry.1 = *nonce;
                        // Nonce mutated — slide anchor forward.
                        entry.2 = epoch;
                    }
                    (Location::AccountStorageDeposit(addr), ValuePayload::StorageDeposit(d)) => {
                        let base = base_db.get_account(addr);
                        let entry = writes.storage.entry(*addr).or_insert_with(|| {
                            base.map_or((0, 0), |a| (a.storage_deposit, a.storage_bytes))
                        });
                        entry.0 = *d;
                    }
                    (Location::AccountStorageBytes(addr), ValuePayload::StorageBytes(b)) => {
                        let base = base_db.get_account(addr);
                        let entry = writes.storage.entry(*addr).or_insert_with(|| {
                            base.map_or((0, 0), |a| (a.storage_deposit, a.storage_bytes))
                        });
                        entry.1 = *b;
                    }
                    (Location::Object(id), ValuePayload::Object(obj)) => {
                        writes.objects.insert(*id, Some(obj.clone()));
                    }
                    (Location::Object(id), ValuePayload::ObjectDeleted) => {
                        writes.objects.insert(*id, None);
                    }
                    (Location::Ghost(id), ValuePayload::Ghost(g)) => {
                        writes.ghosts.insert(*id, Some(g.clone()));
                    }
                    (Location::Ghost(id), ValuePayload::GhostRemoved) => {
                        writes.ghosts.insert(*id, None);
                    }
                    _ => {}
                }
            }
        });

        writes
    }

    /// Apply finalized writes to the main state DB.
    fn apply_writes(db: &mut dyn StateDB, writes: FinalWrites) {
        // Apply account balance/nonce changes
        for (addr, (balance, nonce, last_touched_epoch)) in writes.accounts {
            let acct = db.get_or_create_account(&addr);
            acct.balance = balance;
            acct.nonce = nonce;
            acct.last_touched_epoch = last_touched_epoch;
        }
        // Apply storage deposit/bytes changes
        for (addr, (deposit, bytes)) in writes.storage {
            let acct = db.get_or_create_account(&addr);
            acct.storage_deposit = deposit;
            acct.storage_bytes = bytes;
        }

        // Apply object changes
        for (id, maybe_obj) in writes.objects {
            match maybe_obj {
                Some(obj) => db.put_object(obj),
                None => {
                    db.delete_object(&id);
                }
            }
        }

        // Apply ghost changes
        for (id, maybe_ghost) in writes.ghosts {
            match maybe_ghost {
                Some(ghost) => db.put_ghost(ghost),
                None => {
                    db.remove_ghost(&id);
                }
            }
        }
    }
}

/// Result of parallel execution phase.
#[derive(Debug, Default)]
struct ParallelResult {
    txs_executed: usize,
    txs_failed: usize,
    gas_used: u64,
    total_fees: u64,
    final_writes: FinalWrites,
}

/// Materialized final writes from Block-STM execution.
#[derive(Debug, Default)]
struct FinalWrites {
    /// addr → (balance, nonce, last_touched_epoch).  The `last_touched_epoch`
    /// component is set to the block epoch whenever a per-tx write to either
    /// balance or nonce is observed during materialisation; it is the
    /// demurrage anchor and slides forward in lock-step with balance/nonce.
    accounts: HashMap<AccountAddress, (u64, u64, u64)>,
    /// addr → (storage_deposit, storage_bytes)
    storage: HashMap<AccountAddress, (u64, u64)>,
    /// id → Some(obj) for write/create, None for delete
    objects: HashMap<ObjectId, Option<StateObject>>,
    /// id → Some(ghost) for write, None for remove
    ghosts: HashMap<ObjectId, Option<GhostRecord>>,
}

impl ExecutionEngine for BlockStmExecutor {
    fn execute_block(
        &mut self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<BlockExecutionResult, ExecutionError> {
        let base_fee = self.fee_controller.as_ref().map_or(0, |fc| fc.base_fee);

        // ── Phase 1: Separate contract/script txs (must be serial) ──
        let mut parallel_txs: Vec<&Transaction> = Vec::new();
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
                | Transaction::ValidatorExit(_)
                | Transaction::ValidatorClaimStake(_) => serial_txs.push((i, tx)),
                _ => parallel_txs.push(tx),
            }
        }

        // ── Phase 2: Execute parallelizable txs via Block-STM ──
        let par_result = if parallel_txs.len() >= self.parallel_threshold {
            info!(
                block = block.number,
                parallel_txs = parallel_txs.len(),
                serial_txs = serial_txs.len(),
                "Block-STM: executing {} txs in parallel",
                parallel_txs.len()
            );
            self.execute_parallel(db, &parallel_txs, block.epoch, self.block_gas_limit)
        } else {
            // Fall back to sequential for small blocks
            self.execute_sequential(db, &parallel_txs, block.epoch)
        };

        // Apply parallel results to main DB
        BlockStmExecutor::apply_writes(db, par_result.final_writes);

        let mut total_txs_executed = par_result.txs_executed;
        let mut total_txs_failed = par_result.txs_failed;
        let mut total_gas_used = par_result.gas_used;
        let mut total_fees = par_result.total_fees;

        // ── Phase 3: Execute serial txs (contract/script) ──
        let mut serial_call_depth: usize = 0;
        for &(idx, tx) in &serial_txs {
            if self.verify_signatures {
                if let Err(e) =
                    crate::parallel::ParallelExecutor::verify_tx_signature(true, tx, &self.chain_id)
                {
                    debug!(tx_idx = idx, error = %e, "Serial: sig verification failed");
                    total_txs_failed += 1;
                    continue;
                }
            }

            let tx_gas = estimate_gas(tx);
            if self.block_gas_limit > 0 && total_gas_used + tx_gas > self.block_gas_limit {
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
                    sender.balance = match sender.balance.checked_sub(total_tx_fee) {
                        Some(v) => v,
                        None => {
                            total_txs_failed += 1;
                            continue;
                        }
                    };
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
                                            tmpl, args, rules, deploy.deployer,
                                            deploy.energy, deploy.half_life, block.epoch,
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
                            Err(e) => Err(ExecutionError::ContractError(format!("invalid args: {e}"))),
                        };
                        serial_call_depth = serial_call_depth.saturating_sub(1);
                        r
                    }
                }
                Transaction::DeployScript(deploy) => self
                    .script_engine
                    .deploy(
                        &deploy.source_code, deploy.deployer, deploy.energy,
                        deploy.half_life, block.epoch,
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
                        let r = self.script_engine
                            .call(call.contract_id, &call.method, args, call.caller, call.epoch)
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
                Transaction::Deferred(dtx) => {
                    self.deferred_queue
                        .submit(dtx.clone())
                        .map(|_| ())
                        .map_err(|e| ExecutionError::ContractError(e.to_string()))
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
                        let mut stake = db.get_stake(exit.validator_id).cloned().ok_or_else(|| {
                            ExecutionError::ObjectNotFound(
                                format!("no stake record for validator {}", exit.validator_id),
                            )
                        })?;
                        if stake.validator_address != exit.validator_address {
                            return Err(ExecutionError::InvalidSignature);
                        }
                        if stake.unbonding_epoch.is_some() {
                            Err(ExecutionError::ContractError("validator already exiting".to_string()))
                        } else {
                            stake.unbonding_epoch = Some(block.epoch + crate::UNBONDING_PERIOD_EPOCHS);
                            db.put_stake(stake);
                            Ok(())
                        }
                    }
                }
                Transaction::ValidatorClaimStake(claim) => {
                    let stake = db.get_stake(claim.validator_id).cloned().ok_or_else(|| {
                        ExecutionError::ObjectNotFound(
                            format!("no stake record for validator {}", claim.validator_id),
                        )
                    })?;
                    if stake.validator_address != claim.validator_address {
                        return Err(ExecutionError::InvalidSignature);
                    }
                    let unbonding = stake.unbonding_epoch.ok_or_else(|| {
                        ExecutionError::ContractError(
                            "validator has not exited — cannot claim stake".to_string(),
                        )
                    })?;
                    if block.epoch < unbonding {
                        Err(ExecutionError::ContractError(format!(
                            "unbonding period not complete: current epoch {} < unbonding epoch {}",
                            block.epoch, unbonding
                        )))
                    } else {
                        let sender = db.get_or_create_account(&claim.validator_address);
                        if sender.nonce != claim.nonce {
                            return Err(ExecutionError::InvalidNonce {
                                expected: sender.nonce,
                                got: claim.nonce,
                            });
                        }
                        let claimable = stake.staked_amount.saturating_sub(stake.slashed_amount);
                        sender.balance += claimable;
                        sender.nonce += 1;
                        // Balance + nonce mutated — stamp anchor.
                        sender.last_touched_epoch = block.epoch;
                        db.remove_stake(claim.validator_id);
                        Ok(())
                    }
                }
                _ => unreachable!("only serial-phase txs expected here"),
            };

            match result {
                Ok(()) => {
                    total_txs_executed += 1;
                    total_gas_used += tx_gas;
                    total_fees += tx_fee;
                }
                Err(e) => {
                    debug!(tx_idx = idx, error = %e, "Serial: tx failed");
                    total_txs_failed += 1;
                    total_fees += tx_fee;
                }
            }
        }

        // ── Phase 4: Evaporation + contract/script ticks ──
        let evap_result = self.evaporation_engine.process_epoch_with_mmr(db, block.epoch, &mut self.mmr);
        self.contract_engine.tick(block.epoch);
        self.script_engine.tick(block.epoch);

        // Punch-list 6: storage-rent collection. Inline (same shape as in
        // parallel.rs / SimpleExecutor) and gated on `last_rent_epoch` so
        // rent fires exactly once per epoch.
        if block.epoch > db.get_last_rent_epoch() {
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
                        evaporchain_types::STORAGE_RENT_PER_BYTE_PER_EPOCH,
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
            db.put_last_rent_epoch(block.epoch);
        }

        let state_root = db.compute_state_root();

        info!(
            block = block.number,
            epoch = block.epoch,
            txs_executed = total_txs_executed,
            txs_failed = total_txs_failed,
            gas_used = total_gas_used,
            entered_grace = evap_result.entered_grace.len(),
            evaporated = evap_result.evaporated.len(),
            "Block-STM block executed"
        );

        let mera_root = crate::mera_integration::compute_mera_commitment(db);
        let mera_commitment = if mera_root == [0u8; 32] { None } else { Some(mera_root) };

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
            cross_shard_processed: 0,
            cross_shard_receipts: Vec::new(),
            // RotateValidatorKey is dispatched to serial fallback (see the
            // arm in `execute_tx_view`) so Block-STM never accumulates them.
            validator_key_rotations: Vec::new(),
            mera_commitment,
        })
    }

    fn mmr_root(&self) -> [u8; 32] {
        self.mmr.root()
    }

    fn mmr_size(&self) -> usize {
        self.mmr.size()
    }
}

impl BlockStmExecutor {
    /// Sequential fallback for small blocks (avoids Block-STM overhead).
    fn execute_sequential(
        &self,
        db: &dyn StateDB,
        txs: &[&Transaction],
        epoch: Epoch,
    ) -> ParallelResult {
        let mut accounts: HashMap<AccountAddress, Account> = HashMap::new();
        let mut objects: HashMap<ObjectId, Option<StateObject>> = HashMap::new();
        let ghosts: HashMap<ObjectId, Option<GhostRecord>> = HashMap::new();
        let mut txs_executed = 0;
        let mut txs_failed = 0;
        let mut gas_used = 0u64;
        let mut total_fees = 0u64;

        // Helper closures to read from local overlay → base DB
        let get_balance = |addr: &AccountAddress,
                           accounts: &HashMap<AccountAddress, Account>|
         -> u64 {
            accounts
                .get(addr)
                .map_or_else(|| db.get_account(addr).map_or(0, |a| a.balance), |a| a.balance)
        };

        let get_nonce = |addr: &AccountAddress,
                         accounts: &HashMap<AccountAddress, Account>|
         -> u64 {
            accounts
                .get(addr)
                .map_or_else(|| db.get_account(addr).map_or(0, |a| a.nonce), |a| a.nonce)
        };

        for tx in txs {
            let tx_gas = estimate_gas(tx);

            // Fee deduction
            let tx_fee = if let Some(fc) = &self.fee_controller {
                let gas_fee = fc.compute_gas_fee(tx_gas, 0);
                let extra_fee = match *tx {
                    Transaction::CreateObject(ref create) => {
                        fc.compute_creation_deposit(create.data.len())
                    }
                    Transaction::Refresh(ref refresh) => {
                        fc.compute_refresh_fee(refresh.energy_deposit)
                    }
                    _ => 0,
                };
                let total_tx_fee = match gas_fee.checked_add(extra_fee) {
                    Some(v) => v,
                    None => {
                        txs_failed += 1;
                        continue;
                    }
                };
                if let Some(sender_addr) = tx.sender() {
                    let bal = get_balance(sender_addr, &accounts);
                    if bal < total_tx_fee {
                        txs_failed += 1;
                        continue;
                    }
                    let nonce = get_nonce(sender_addr, &accounts);
                    let new_bal = match bal.checked_sub(total_tx_fee) {
                        Some(v) => v,
                        None => {
                            txs_failed += 1;
                            continue;
                        }
                    };
                    accounts.insert(
                        *sender_addr,
                        Account {
                            address: *sender_addr,
                            balance: new_bal,
                            nonce,
                            storage_deposit: 0,
                            storage_bytes: 0,
                            // Fee deduction is a balance mutation — stamp the
                            // demurrage anchor so the next sweep starts from
                            // the current epoch.
                            last_touched_epoch: epoch,
                        },
                    );
                }
                total_tx_fee
            } else {
                0
            };

            // Simple sequential execution using local overlay
            let success = match *tx {
                Transaction::Transfer(ref t) => {
                    if t.from == t.to || t.amount == 0 {
                        false
                    } else {
                        let sb = get_balance(&t.from, &accounts);
                        let sn = get_nonce(&t.from, &accounts);
                        if sn != t.nonce || sb < t.amount {
                            false
                        } else {
                            match (sb.checked_sub(t.amount), sn.checked_add(1)) {
                                (Some(new_sb), Some(new_sn)) => {
                                    accounts.insert(
                                        t.from,
                                        Account {
                                            address: t.from,
                                            balance: new_sb,
                                            nonce: new_sn,
                            storage_deposit: 0,
                            storage_bytes: 0,
                                            // Sender balance + nonce mutated.
                                            last_touched_epoch: epoch,
                                        },
                                    );
                                    let rb = get_balance(&t.to, &accounts);
                                    let rn = get_nonce(&t.to, &accounts);
                                    match rb.checked_add(t.amount) {
                                        Some(new_rb) => {
                                            accounts.insert(
                                                t.to,
                                                Account {
                                                    address: t.to,
                                                    balance: new_rb,
                                                    nonce: rn,
                            storage_deposit: 0,
                            storage_bytes: 0,
                                                    // Receiver balance mutated.
                                                    last_touched_epoch: epoch,
                                                },
                                            );
                                            true
                                        }
                                        None => false, // receiver balance overflow
                                    }
                                }
                                _ => false, // arithmetic overflow
                            }
                        }
                    }
                }
                Transaction::CreateObject(ref t) => {
                    let exists = objects
                        .get(&t.object_id)
                        .map_or_else(|| db.get_object(&t.object_id).is_some(), |o| o.is_some());
                    if exists {
                        false
                    } else {
                        objects.insert(
                            t.object_id,
                            Some(StateObject {
                                id: t.object_id,
                                owner: t.creator,
                                energy: t.energy,
                                half_life: t.half_life,
                                created_at: epoch,
                                last_refreshed: epoch,
                                state: ObjectState::Active,
                                grace_epoch: None,
                                data: t.data.clone(),
                                decay_curve: t.decay_curve.clone(),
                            }),
                        );
                        true
                    }
                }
                Transaction::Refresh(ref t) => {
                    let obj = objects
                        .get(&t.object_id)
                        .cloned()
                        .unwrap_or_else(|| db.get_object(&t.object_id).cloned());
                    if let Some(Some(mut obj)) = Some(obj) {
                        obj.energy = obj.energy.saturating_add(t.energy_deposit);
                        obj.last_refreshed = epoch;
                        if obj.state == ObjectState::Grace {
                            obj.state = ObjectState::Active;
                            obj.grace_epoch = None;
                        }
                        objects.insert(t.object_id, Some(obj));
                        true
                    } else {
                        false
                    }
                }
                Transaction::ValidatorStake(ref t) => {
                    let bal = get_balance(&t.validator_address, &accounts);
                    let nonce = get_nonce(&t.validator_address, &accounts);
                    if t.stake_amount == 0 || nonce != t.nonce || bal < t.stake_amount {
                        false
                    } else {
                        match (bal.checked_sub(t.stake_amount), nonce.checked_add(1)) {
                            (Some(new_bal), Some(new_nonce)) => {
                                accounts.insert(
                                    t.validator_address,
                                    Account {
                                        address: t.validator_address,
                                        balance: new_bal,
                                        nonce: new_nonce,
                            storage_deposit: 0,
                            storage_bytes: 0,
                                        // Stake locks balance + bumps nonce.
                                        last_touched_epoch: epoch,
                                    },
                                );
                                true
                            }
                            _ => false, // arithmetic overflow
                        }
                    }
                }
                Transaction::ValidatorExit(ref t) => {
                    let bal = get_balance(&t.validator_address, &accounts);
                    let nonce = get_nonce(&t.validator_address, &accounts);
                    if nonce != t.nonce {
                        false
                    } else {
                        match nonce.checked_add(1) {
                            Some(new_nonce) => {
                                accounts.insert(
                                    t.validator_address,
                                    Account {
                                        address: t.validator_address,
                                        balance: bal,
                                        nonce: new_nonce,
                            storage_deposit: 0,
                            storage_bytes: 0,
                                        // Validator-exit bumps nonce.
                                        last_touched_epoch: epoch,
                                    },
                                );
                                true
                            }
                            None => false, // nonce overflow
                        }
                    }
                }
                _ => false,
            };

            if success {
                txs_executed += 1;
                gas_used += tx_gas;
                total_fees += tx_fee;
            } else {
                txs_failed += 1;
                total_fees += tx_fee;
            }
        }

        ParallelResult {
            txs_executed,
            txs_failed,
            gas_used,
            total_fees,
            final_writes: FinalWrites {
                accounts: accounts
                    .iter()
                    .map(|(addr, acct)| (*addr, (acct.balance, acct.nonce)))
                    .collect(),
                storage: accounts
                    .into_iter()
                    .filter(|(_, acct)| acct.storage_deposit > 0 || acct.storage_bytes > 0)
                    .map(|(addr, acct)| (addr, (acct.storage_deposit, acct.storage_bytes)))
                    .collect(),
                objects,
                ghosts,
            },
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// §6  Tests
// ═══════════════════════════════════════════════════════════════════════════

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
        });
    }

    // ── MVCC Tests ──

    #[test]
    fn test_mvmemory_write_read() {
        let mv = MVMemory::new();
        let loc = Location::AccountBalance(addr(1));

        // Tx 0 writes balance 100
        mv.write(loc.clone(), 0, 0, ValuePayload::Balance(100));

        // Tx 1 reads and should see tx 0's write
        let result = mv.read(&loc, 1).unwrap();
        assert!(result.is_some());
        let ((writer, _inc), payload) = result.unwrap();
        assert_eq!(writer, 0);
        match payload {
            ValuePayload::Balance(b) => assert_eq!(b, 100),
            _ => panic!("expected balance"),
        }
    }

    #[test]
    fn test_mvmemory_no_prior_write() {
        let mv = MVMemory::new();
        let loc = Location::AccountBalance(addr(1));

        // Tx 0 reads — no prior write
        let result = mv.read(&loc, 0).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_mvmemory_estimate_blocks_reader() {
        let mv = MVMemory::new();
        let loc = Location::AccountBalance(addr(1));

        // Tx 0 writes, then gets marked as estimate
        mv.write(loc.clone(), 0, 0, ValuePayload::Balance(100));
        mv.mark_estimate(loc.clone(), 0);

        // Tx 1 reads — should get blocked
        let result = mv.read(&loc, 1);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), 0);
    }

    #[test]
    fn test_mvmemory_version_ordering() {
        let mv = MVMemory::new();
        let loc = Location::AccountBalance(addr(1));

        // Tx 0 writes 100, tx 2 writes 200
        mv.write(loc.clone(), 0, 0, ValuePayload::Balance(100));
        mv.write(loc.clone(), 2, 0, ValuePayload::Balance(200));

        // Tx 1 reads — should see tx 0 (highest < 1)
        let result = mv.read(&loc, 1).unwrap().unwrap();
        match result.1 {
            ValuePayload::Balance(b) => assert_eq!(b, 100),
            _ => panic!("expected balance"),
        }

        // Tx 3 reads — should see tx 2 (highest < 3)
        let result = mv.read(&loc, 3).unwrap().unwrap();
        match result.1 {
            ValuePayload::Balance(b) => assert_eq!(b, 200),
            _ => panic!("expected balance"),
        }
    }

    // ── Block-STM Executor Tests ──

    #[test]
    fn test_block_stm_independent_transfers() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
        fund_account(&mut db, 3, 2000);

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

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 1; // Force parallel execution
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 2);
        assert_eq!(result.txs_failed, 0);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 900);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 100);
        assert_eq!(db.get_account(&addr(3)).unwrap().balance, 1800);
        assert_eq!(db.get_account(&addr(4)).unwrap().balance, 200);
    }

    #[test]
    fn test_block_stm_conflicting_transfers() {
        // A→B then B→C — must serialize correctly
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
        fund_account(&mut db, 2, 500);

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
                amount: 300,
                nonce: 0,
                signature: None,
                public_key: None,
            }),
        ];

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 1;
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 2);
        // Account 1: 1000 - 100 = 900
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 900);
        // Account 2: 500 + 100 - 300 = 300
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 300);
        // Account 3: 0 + 300 = 300
        assert_eq!(db.get_account(&addr(3)).unwrap().balance, 300);
    }

    #[test]
    fn test_block_stm_create_object() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);
        fund_account(&mut db, 2, 10_000);

        let txs = vec![
            Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(1),
                energy: 1000,
                half_life: 100,
                data: vec![1, 2, 3],
                decay_curve: None,
                signature: None,
                public_key: None,
            }),
            Transaction::CreateObject(CreateObjectTx {
                creator: addr(2),
                object_id: obj_id(2),
                energy: 2000,
                half_life: 200,
                data: vec![4, 5, 6],
                decay_curve: None,
                signature: None,
                public_key: None,
            }),
        ];

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 1;
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 2);
        assert!(db.get_object(&obj_id(1)).is_some());
        assert!(db.get_object(&obj_id(2)).is_some());
        assert_eq!(db.get_object(&obj_id(1)).unwrap().energy, 1000);
        assert_eq!(db.get_object(&obj_id(2)).unwrap().energy, 2000);
    }

    #[test]
    fn test_block_stm_determinism() {
        // Run the same block multiple times — results must be identical
        for _ in 0..10 {
            let mut db = InMemoryStateDB::new();
            fund_account(&mut db, 1, 10000);
            fund_account(&mut db, 2, 10000);
            fund_account(&mut db, 3, 10000);

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
                    amount: 200,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(3),
                    to: addr(1),
                    amount: 50,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(3),
                    amount: 75,
                    nonce: 1,
                    signature: None,
                    public_key: None,
                }),
            ];

            let block = make_block(1, 1, txs);
            let mut executor = BlockStmExecutor::new_for_test(7);
            executor.parallel_threshold = 1;
            let result = executor.execute_block(&mut db, &block).unwrap();

            // Must always produce the same result
            assert_eq!(result.txs_executed, 4);
            assert_eq!(db.get_account(&addr(1)).unwrap().balance, 9875);
            assert_eq!(db.get_account(&addr(2)).unwrap().balance, 9900);
            assert_eq!(db.get_account(&addr(3)).unwrap().balance, 10225);
        }
    }

    #[test]
    fn test_block_stm_nonce_validation() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10000);

        // Second tx has wrong nonce
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
                from: addr(1),
                to: addr(3),
                amount: 200,
                nonce: 5, // Wrong nonce
                signature: None,
                public_key: None,
            }),
        ];

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 1;
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 1);
        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 9900);
    }

    #[test]
    fn test_block_stm_empty_block() {
        let mut db = InMemoryStateDB::new();
        let block = make_block(1, 1, vec![]);
        let mut executor = BlockStmExecutor::new_for_test(7);
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 0);
    }

    #[test]
    fn test_block_stm_mixed_tx_types() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10000);
        fund_account(&mut db, 2, 10000);

        let txs = vec![
            Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 500,
                nonce: 0,
                signature: None,
                public_key: None,
            }),
            Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(1),
                energy: 1000,
                half_life: 100,
                data: vec![0xDE, 0xAD],
                decay_curve: None,
                signature: None,
                public_key: None,
            }),
            Transaction::ValidatorStake(ValidatorStakeTx {
                validator_address: addr(2),
                stake_amount: 1000,
                validator_id: 1,
                nonce: 0,
                bls_public_key: None,
                vrf_public_key: None,
                signature: None,
                public_key: None,
            }),
        ];

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 1;
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 3);
        // addr(1): 10000 - 500 (transfer) - 1000 (storage deposit) = 8500
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 8500);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 9500); // +500 -1000
        assert!(db.get_object(&obj_id(1)).is_some());
    }

    #[test]
    fn test_block_stm_high_contention() {
        // All txs from same sender — maximum contention, should still work
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 100_000);

        let txs: Vec<Transaction> = (0..10)
            .map(|i| {
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(10 + i),
                    amount: 100,
                    nonce: i as u64,
                    signature: None,
                    public_key: None,
                })
            })
            .collect();

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 1;
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 10);
        assert_eq!(
            db.get_account(&addr(1)).unwrap().balance,
            100_000 - 10 * 100
        );
        assert_eq!(db.get_account(&addr(1)).unwrap().nonce, 10);

        // All receivers should have 100
        for i in 0..10u8 {
            assert_eq!(db.get_account(&addr(10 + i)).unwrap().balance, 100);
        }
    }

    #[test]
    fn test_block_stm_sequential_matches_parallel() {
        // Compare Block-STM execution with SimpleExecutor
        let mut db_par = InMemoryStateDB::new();
        let mut db_seq = InMemoryStateDB::new();

        for byte in 1..=5u8 {
            fund_account(&mut db_par, byte, 50000);
            fund_account(&mut db_seq, byte, 50000);
        }

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
                from: addr(2),
                to: addr(5),
                amount: 150,
                nonce: 0,
                signature: None,
                public_key: None,
            }),
            Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(1),
                energy: 500,
                half_life: 50,
                data: vec![1, 2, 3, 4],
                decay_curve: None,
                signature: None,
                public_key: None,
            }),
        ];

        let block_par = make_block(1, 1, txs.clone());
        let block_seq = make_block(1, 1, txs);

        let mut par_exec = BlockStmExecutor::new_for_test(7);
        par_exec.parallel_threshold = 1;
        let par_result = par_exec.execute_block(&mut db_par, &block_par).unwrap();

        let mut seq_exec = crate::SimpleExecutor::new_for_test(7);
        let seq_result = seq_exec.execute_block(&mut db_seq, &block_seq).unwrap();

        // Results must match
        assert_eq!(par_result.txs_executed, seq_result.txs_executed);
        assert_eq!(par_result.txs_failed, seq_result.txs_failed);

        // State must match
        for byte in 1..=5u8 {
            let par_acct = db_par.get_account(&addr(byte));
            let seq_acct = db_seq.get_account(&addr(byte));
            assert_eq!(
                par_acct.map(|a| (a.balance, a.nonce)),
                seq_acct.map(|a| (a.balance, a.nonce)),
                "account {} mismatch",
                byte
            );
        }

        // State root must match
        assert_eq!(par_result.state_root, seq_result.state_root);
    }

    // ── Bug regression tests ──

    #[test]
    fn test_block_gas_limit_enforced_in_parallel() {
        // BUG 2 regression: gas limit must be enforced even for parallel txs
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 100_000);
        fund_account(&mut db, 2, 100_000);
        fund_account(&mut db, 3, 100_000);

        // 3 independent transfers, each costs GAS_TRANSFER (21000)
        // Set gas limit to allow only 2 (42000)
        let txs = vec![
            Transaction::Transfer(TransferTx {
                from: addr(1), to: addr(10), amount: 100, nonce: 0,
                signature: None, public_key: None,
            }),
            Transaction::Transfer(TransferTx {
                from: addr(2), to: addr(20), amount: 200, nonce: 0,
                signature: None, public_key: None,
            }),
            Transaction::Transfer(TransferTx {
                from: addr(3), to: addr(30), amount: 300, nonce: 0,
                signature: None, public_key: None,
            }),
        ];

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 1;
        executor.block_gas_limit = 42_000; // Only fits 2 transfers
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 2, "only 2 txs should fit in gas limit");
        assert_eq!(result.txs_failed, 1, "third tx should be over gas limit");
        assert!(result.gas_used <= 42_000, "gas must not exceed limit");
    }

    #[test]
    fn test_failed_tx_keeps_fee_reverts_execution() {
        // BUG 3 regression: failed tx must keep fee writes but revert execution writes
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 500); // Just enough for fee but not for transfer

        let txs = vec![
            Transaction::Transfer(TransferTx {
                from: addr(1), to: addr(2),
                amount: 1_000_000, // Way more than balance — will fail
                nonce: 0,
                signature: None, public_key: None,
            }),
        ];

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 1;
        // No fee controller — so no fee deduction to test
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_failed, 1);
        // Account should be unchanged (no fee controller, tx failed)
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 500);
        assert_eq!(db.get_account(&addr(1)).unwrap().nonce, 0);
        // Receiver should NOT exist (tx reverted)
        assert!(
            db.get_account(&addr(2)).is_none()
                || db.get_account(&addr(2)).unwrap().balance == 0
        );
    }

    #[test]
    fn test_sharded_mvmemory_correctness() {
        // BUG 6 regression: sharded MVMemory must be functionally correct
        let mv = MVMemory::new();

        // Write to many different locations (should spread across shards)
        for i in 0..100u8 {
            let loc = Location::AccountBalance(addr(i));
            mv.write(loc, 0, 0, ValuePayload::Balance(i as u64 * 100));
        }

        // Read them all back
        for i in 0..100u8 {
            let loc = Location::AccountBalance(addr(i));
            let result = mv.read(&loc, 1).unwrap().unwrap();
            match result.1 {
                ValuePayload::Balance(b) => assert_eq!(b, i as u64 * 100),
                _ => panic!("expected balance"),
            }
        }

        // Delete some and verify
        let locs: Vec<Location> = (50..60u8)
            .map(|i| Location::AccountBalance(addr(i)))
            .collect();
        mv.delete_writes(0, &locs);

        for i in 50..60u8 {
            let loc = Location::AccountBalance(addr(i));
            let result = mv.read(&loc, 1).unwrap();
            assert!(result.is_none(), "deleted write should not be visible");
        }

        // Non-deleted writes should still be there
        for i in 0..50u8 {
            let loc = Location::AccountBalance(addr(i));
            let result = mv.read(&loc, 1).unwrap();
            assert!(result.is_some(), "non-deleted write should still exist");
        }
    }

    // ── C-12 regression: balance overflow protection ──

    #[test]
    fn test_receiver_balance_overflow_parallel() {
        // C-12 regression: receiving u64::MAX + 1 must fail, not wrap to zero
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
        // Give the receiver near-max balance so adding any amount overflows
        db.put_account(Account {
            address: addr(2),
            balance: u64::MAX,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        });

        let txs = vec![Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 1,
            nonce: 0,
            signature: None,
            public_key: None,
        })];

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 1; // Force parallel path
        let result = executor.execute_block(&mut db, &block).unwrap();

        // The transfer must fail — receiver balance would overflow
        assert_eq!(result.txs_failed, 1, "overflow transfer must fail");
        assert_eq!(result.txs_executed, 0);
        // Receiver balance must NOT have wrapped to zero
        assert_eq!(
            db.get_account(&addr(2)).unwrap().balance,
            u64::MAX,
            "receiver balance must not wrap on overflow"
        );
        // Sender balance unchanged (tx reverted)
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1000);
    }

    #[test]
    fn test_receiver_balance_overflow_sequential() {
        // C-12 regression: same test but via sequential fallback path
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);
        db.put_account(Account {
            address: addr(2),
            balance: u64::MAX,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        });

        let txs = vec![Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 1,
            nonce: 0,
            signature: None,
            public_key: None,
        })];

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 100; // Force sequential path
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_failed, 1, "overflow transfer must fail in sequential");
        assert_eq!(result.txs_executed, 0);
        assert_eq!(
            db.get_account(&addr(2)).unwrap().balance,
            u64::MAX,
            "receiver balance must not wrap on overflow (sequential)"
        );
        let sender_bal = db.get_account(&addr(1)).unwrap().balance;
        assert!(sender_bal >= 999 && sender_bal <= 1000, "sender balance {sender_bal} after failed tx");
    }

    #[test]
    fn test_large_transfer_near_max_balance() {
        // Verify that transfers near u64::MAX boundary work correctly
        // when they don't actually overflow
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: u64::MAX - 100,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        });
        fund_account(&mut db, 2, 0);

        let txs = vec![Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 500,
            nonce: 0,
            signature: None,
            public_key: None,
        })];

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 1;
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_executed, 1);
        assert_eq!(
            db.get_account(&addr(1)).unwrap().balance,
            u64::MAX - 100 - 500
        );
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 500);
    }

    #[test]
    fn test_concurrent_overflow_attack() {
        // C-12 regression: multiple concurrent transfers to a near-max receiver
        // Only one should succeed if the second would cause overflow
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);
        fund_account(&mut db, 3, 10_000);
        db.put_account(Account {
            address: addr(2),
            balance: u64::MAX - 5_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        });

        // Two independent senders both transferring to addr(2)
        // First: 5000 (fits exactly at u64::MAX)
        // Second: 1 (would overflow past u64::MAX)
        let txs = vec![
            Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 5_000,
                nonce: 0,
                signature: None,
                public_key: None,
            }),
            Transaction::Transfer(TransferTx {
                from: addr(3),
                to: addr(2),
                amount: 1,
                nonce: 0,
                signature: None,
                public_key: None,
            }),
        ];

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 1;
        let result = executor.execute_block(&mut db, &block).unwrap();

        // First tx should succeed, second must fail (would overflow)
        assert_eq!(result.txs_executed, 1, "only one transfer should succeed");
        assert_eq!(result.txs_failed, 1, "overflow transfer must fail");
        assert_eq!(
            db.get_account(&addr(2)).unwrap().balance,
            u64::MAX,
            "receiver should be at max, not wrapped"
        );
    }

    #[test]
    fn test_validator_stake_checked_subtraction() {
        // Verify validator stake uses checked arithmetic
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 100);

        let txs = vec![Transaction::ValidatorStake(ValidatorStakeTx {
            validator_address: addr(1),
            stake_amount: 200, // More than balance
            validator_id: 1,
            nonce: 0,
            bls_public_key: None,
            vrf_public_key: None,
            signature: None,
            public_key: None,
        })];

        let block = make_block(1, 1, txs);
        let mut executor = BlockStmExecutor::new_for_test(7);
        executor.parallel_threshold = 1;
        let result = executor.execute_block(&mut db, &block).unwrap();

        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 100);
    }
}
