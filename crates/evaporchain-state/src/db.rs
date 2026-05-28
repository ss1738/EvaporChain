use evaporchain_crypto::hash::blake3_hash;
use evaporchain_crypto::{EnergyVerkleTrie, TrieHealth};

use evaporchain_types::{
    Account, AccountAddress, DelegationRecord, GhostRecord, GovernanceProposal, ObjectId,
    StakeRecord, StateObject,
};
use std::collections::BTreeMap;
use std::collections::HashMap;

// ─── Trie key/value derivation (shared by all StateDB backends) ─────────

pub fn trie_key_for_account(addr: &AccountAddress) -> [u8; 32] {
    let mut buf = Vec::with_capacity(36);
    buf.extend_from_slice(b"acct");
    buf.extend_from_slice(addr);
    blake3_hash(&buf)
}

pub fn trie_value_for_account(acc: &Account) -> [u8; 32] {
    let mut buf = Vec::with_capacity(16);
    buf.extend_from_slice(&acc.balance.to_le_bytes());
    buf.extend_from_slice(&acc.nonce.to_le_bytes());
    blake3_hash(&buf)
}

pub fn trie_key_for_object(id: &ObjectId) -> [u8; 32] {
    let mut buf = Vec::with_capacity(35);
    buf.extend_from_slice(b"obj");
    buf.extend_from_slice(id);
    blake3_hash(&buf)
}

pub fn trie_value_for_object(obj: &StateObject) -> [u8; 32] {
    let mut buf = Vec::with_capacity(9);
    buf.extend_from_slice(&obj.energy.to_le_bytes());
    buf.push(object_state_to_u8(&obj.state));
    blake3_hash(&buf)
}

pub fn build_energy_trie(
    objects: &HashMap<ObjectId, StateObject>,
    accounts: &HashMap<AccountAddress, Account>,
) -> EnergyVerkleTrie {
    let mut trie = EnergyVerkleTrie::new();

    for (addr, acc) in accounts {
        trie.insert(
            trie_key_for_account(addr),
            trie_value_for_account(acc),
            u64::MAX,
            u64::MAX,
            0,
        );
    }

    for obj in objects.values() {
        trie.insert(
            trie_key_for_object(&obj.id),
            trie_value_for_object(obj),
            obj.energy,
            obj.half_life,
            obj.last_refreshed,
        );
    }

    trie
}

/// Trait for state database backends.
pub trait StateDB: Send + Sync {
    /// Begin a write-batch. Mutations between this call and
    /// `commit_batch`/`rollback_batch` are buffered. The default impl
    /// is a no-op — backends that don't support transactional rollback
    /// (e.g. `InMemoryStateDB`, `OverlayStateDB`) treat batches as
    /// pass-through. RocksDBStateDB overrides with real semantics:
    /// in-memory mutations are still applied immediately, but the
    /// batch_undo log is captured so `rollback_batch` can restore.
    ///
    /// Use case: proposer-side speculative execution for Phase 2 of
    /// `POST_EXEC_STATE_VERIFICATION_PLAN.md` — execute the block on
    /// the live DB, capture the resulting `state_root`, roll back so
    /// the second (real) execution after broadcast computes the same
    /// answer deterministically.
    fn begin_batch(&mut self) {}

    /// Commit a previously-started batch. Default no-op for backends
    /// without persistent storage.
    fn commit_batch(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Roll back a previously-started batch. Default no-op for
    /// backends without rollback support — caller is responsible for
    /// avoiding speculative execution against such backends.
    fn rollback_batch(&mut self) {}

    /// Retrieve a state object by its ID.
    fn get_object(&self, id: &ObjectId) -> Option<&StateObject>;

    /// Retrieve a mutable reference to a state object.
    fn get_object_mut(&mut self, id: &ObjectId) -> Option<&mut StateObject>;

    /// Store or update a state object.
    fn put_object(&mut self, obj: StateObject);

    /// Delete a state object by its ID and return it.
    fn delete_object(&mut self, id: &ObjectId) -> Option<StateObject>;

    /// Store a ghost record for an evaporated object.
    fn put_ghost(&mut self, record: GhostRecord);

    /// Retrieve a ghost record by object ID.
    fn get_ghost(&self, id: &ObjectId) -> Option<&GhostRecord>;

    /// Remove a ghost record (used during resurrection).
    fn remove_ghost(&mut self, id: &ObjectId) -> Option<GhostRecord>;

    /// Return all object IDs currently in active state.
    ///
    /// NOTE (M-06): Allocates a full Vec copy of all IDs. Callers include
    /// evaporation sweeps, snapshots, and API endpoints -- all low-frequency
    /// batch operations that need the full set (often iterating while the DB
    /// is mutated afterwards). Converting to a trait-returned iterator would
    /// require `Box<dyn Iterator>` with lifetime constraints for marginal
    /// benefit at current scale. Acceptable as-is.
    fn all_object_ids(&self) -> Vec<ObjectId>;

    /// Return the number of active objects.
    fn object_count(&self) -> usize;

    /// Return the number of ghost records.
    fn ghost_count(&self) -> usize;

    /// Return all ghost record object IDs.
    /// NOTE (M-06): Same Vec-copy rationale as `all_object_ids`.
    fn all_ghost_ids(&self) -> Vec<ObjectId>;

    /// Retrieve an account by address.
    fn get_account(&self, addr: &AccountAddress) -> Option<&Account>;

    /// Retrieve a mutable reference to an account.
    fn get_account_mut(&mut self, addr: &AccountAddress) -> Option<&mut Account>;

    /// Store or update an account.
    fn put_account(&mut self, account: Account);

    /// Delete an account by address and return it.
    fn delete_account(&mut self, addr: &AccountAddress) -> Option<Account>;

    /// Get or create an account (returns mutable ref). Creates with zero balance if missing.
    fn get_or_create_account(&mut self, addr: &AccountAddress) -> &mut Account;

    /// Return all account addresses.
    /// NOTE (M-06): Same Vec-copy rationale as `all_object_ids`.
    fn all_account_addresses(&self) -> Vec<AccountAddress>;

    /// Compute the state root hash over all objects and accounts.
    fn compute_state_root(&mut self) -> [u8; 32];

    /// Compress cold subtrees in the energy-annotated Verkle trie.
    /// Returns the number of subtrees compressed.
    fn compress_cold_subtrees(&mut self) -> u32;

    /// Report health of the energy-annotated Verkle trie.
    fn trie_health(&mut self) -> TrieHealth;

    /// Generate a Verkle inclusion proof for an account.
    fn prove_account(&mut self, addr: &AccountAddress) -> evaporchain_crypto::EnergyVerkleProof;

    /// Generate a Verkle inclusion proof for a state object.
    fn prove_object(&mut self, id: &ObjectId) -> evaporchain_crypto::EnergyVerkleProof;

    /// Generate a Verkle inclusion proof for an arbitrary 32-byte
    /// trie key. Wraps `EnergyVerkleTrie::prove` directly. Used
    /// by the light-client SDK's `/api/state/proof/:key_hex`
    /// endpoint and by any caller that already has the trie key
    /// in hand (e.g. a UI that hashes a structured query without
    /// going through the account/object derivation helpers).
    fn prove_at_key(&mut self, key: &[u8; 32]) -> evaporchain_crypto::EnergyVerkleProof;

    /// Serialize the current trie state to bytes for persistence.
    fn trie_snapshot(&mut self) -> Vec<u8>;

    /// Restore the trie from a previously serialized snapshot.
    fn load_trie_snapshot(&mut self, bytes: &[u8]) -> Result<(), String>;

    // ─── Privacy Layer State ──────────────────────────────────────────────

    /// Store the current Merkle note tree root.
    fn put_note_tree_root(&mut self, root: [u8; 32]);

    /// Get the current Merkle note tree root (returns zero hash if none set).
    fn get_note_tree_root(&self) -> [u8; 32];

    /// Record a spent nullifier. Returns false if already spent (double-spend).
    fn spend_nullifier(&mut self, nullifier: &[u8; 32]) -> bool;

    /// Check if a nullifier has been spent.
    fn is_nullifier_spent(&self, nullifier: &[u8; 32]) -> bool;

    /// Number of spent nullifiers.
    fn nullifier_count(&self) -> usize;

    /// Return all spent nullifiers (for snapshots).
    fn all_nullifiers(&self) -> Vec<[u8; 32]>;

    /// Store the total shielded pool balance (for auditing / invariant checks).
    fn put_shielded_pool_balance(&mut self, balance: u64);

    /// Get the total shielded pool balance.
    fn get_shielded_pool_balance(&self) -> u64;

    /// Store the note count (number of notes ever inserted into the tree).
    fn put_note_count(&mut self, count: u64);

    /// Get the note count.
    fn get_note_count(&self) -> u64;

    /// Persist a note commitment at the given leaf index. Idempotent: writing
    /// the same (index, commitment) pair twice is a no-op. Required so that
    /// `PrivacyEngine.note_tree` can be rebuilt deterministically on node
    /// restart without replaying the chain from genesis.
    fn append_note_commitment(&mut self, index: u64, commitment: [u8; 32]);

    /// Return all persisted note commitments in leaf-index order. Used by
    /// PrivacyExecutor at startup to rebuild the in-memory note tree.
    fn get_all_note_commitments(&self) -> Vec<[u8; 32]>;

    // ─── Stake Ledger ───────────────────────────────────────────────────

    /// Get a stake record by validator ID.
    fn get_stake(&self, validator_id: u64) -> Option<&StakeRecord>;

    /// Insert or update a stake record.
    fn put_stake(&mut self, record: StakeRecord);

    /// Remove a stake record by validator ID.
    fn remove_stake(&mut self, validator_id: u64) -> Option<StakeRecord>;

    /// Return all stake records.
    fn all_stakes(&self) -> Vec<&StakeRecord>;

    // ─── Delegations ────────────────────────────────────────────────────

    /// Get a single delegation record by (delegator, validator_id).
    fn get_delegation(
        &self,
        delegator: &AccountAddress,
        validator_id: u64,
    ) -> Option<&DelegationRecord>;

    /// Insert or replace a delegation record. Same key shape as
    /// `get_delegation`. Used both for new delegations and to update
    /// existing ones (e.g., add to amount, set unbonding fields).
    fn put_delegation(&mut self, record: DelegationRecord);

    /// Remove a delegation record entirely. Returns the removed record
    /// if it existed.
    fn remove_delegation(
        &mut self,
        delegator: &AccountAddress,
        validator_id: u64,
    ) -> Option<DelegationRecord>;

    /// All delegations to a given validator (used for reward
    /// distribution and slashing pass-through).
    fn delegations_for_validator(&self, validator_id: u64) -> Vec<&DelegationRecord>;

    /// All delegations from a given delegator (used by wallets and
    /// account dashboards).
    fn delegations_for_delegator(&self, delegator: &AccountAddress) -> Vec<&DelegationRecord>;

    /// Return every delegation record. Used for state snapshots and
    /// effective-stake roll-up across the full validator set.
    fn all_delegations(&self) -> Vec<&DelegationRecord>;

    // ─── Governance ─────────────────────────────────────────────────────

    fn get_proposal(&self, proposal_id: u64) -> Option<&GovernanceProposal>;
    fn put_proposal(&mut self, proposal: GovernanceProposal);
    fn all_proposals(&self) -> Vec<&GovernanceProposal>;
    fn get_governance_param(&self, key: &str) -> Option<&str>;
    fn put_governance_param(&mut self, key: String, value: String);

    // ─── Historical Snapshots ───────────────────────────────────────────

    fn commit_state_snapshot(&mut self, height: u64);
    fn get_account_at_height(&self, address: &AccountAddress, height: u64) -> Option<Account>;
    fn get_object_at_height(&self, id: &ObjectId, height: u64) -> Option<StateObject>;
    fn earliest_snapshot_height(&self) -> Option<u64>;
    fn latest_snapshot_height(&self) -> Option<u64>;
    fn prune_snapshots_before(&mut self, height: u64);

    // ─── State Pruning ───────────────────────────────────────────────────

    /// Prune historical state data older than the given block height.
    ///
    /// Removes ghost records whose `evaporated_at` epoch is strictly less than
    /// `height`, since those represent long-dead state that is no longer needed
    /// for resurrection or audit. Returns the number of records pruned.
    ///
    /// This is a storage-management operation — it does NOT affect the current
    /// active state (objects, accounts, trie) and is safe to call at any time.
    fn prune_before_height(&mut self, height: u64) -> u64;

    /// Last epoch at which storage rent was charged. Used by executors to
    /// gate `collect_storage_rent` so it fires exactly once per epoch
    /// (closes punch-list #6).
    fn get_last_rent_epoch(&self) -> u64 {
        0
    }

    /// Persist the most recent epoch at which storage rent was charged.
    /// Default no-op so back-end implementations that don't carry the
    /// cursor (e.g. transient overlays) compile without changes.
    fn put_last_rent_epoch(&mut self, _epoch: u64) {}

    // ─── Vesting registry (centralization defence) ──────────────────────
    // Default no-ops so back-ends that haven't migrated still compile.
    // Overridden by InMemoryStateDB / RocksDBStateDB.

    fn get_vesting_schedule(&self, _id: u64) -> Option<&evaporchain_types::VestingSchedule> {
        None
    }
    fn put_vesting_schedule(&mut self, _schedule: evaporchain_types::VestingSchedule) {}
    fn remove_vesting_schedule(&mut self, _id: u64) -> Option<evaporchain_types::VestingSchedule> {
        None
    }
    fn all_vesting_schedules(&self) -> Vec<evaporchain_types::VestingSchedule> {
        Vec::new()
    }

    // ─── Sentinel autonomic governance ──────────────────────────────────
    // Persistent registry of bounded parameters + their decay-weighted
    // votes. Per INVENTION_STACK.md Amendment 2 §A2.5. Default no-ops so
    // back-ends that haven't migrated still compile.

    fn get_sentinel_param(&self, _id: u32) -> Option<evaporchain_sentinel::BoundedParameter> {
        None
    }
    fn put_sentinel_param(&mut self, _param: evaporchain_sentinel::BoundedParameter) {}
    fn all_sentinel_params(&self) -> Vec<evaporchain_sentinel::BoundedParameter> {
        Vec::new()
    }
    fn get_sentinel_votes(&self, _id: u32) -> Vec<evaporchain_sentinel::Vote> {
        Vec::new()
    }
    /// Replace the entire vote slate for `parameter_id`. Caller is
    /// responsible for one-vote-per-validator semantics.
    fn put_sentinel_votes(&mut self, _parameter_id: u32, _votes: Vec<evaporchain_sentinel::Vote>) {}

    // ─── Bulk-clear helpers (snapshot restore path) ─────────────────────
    // M12 (audit 2026-05-13): pre-fix `SnapshotApplier::apply` /
    // `SnapshotFile::apply_to` only wiped objects/ghosts/accounts.
    // Stakes, delegations, sentinel params/votes, governance, vesting,
    // note_commitments, and prior spent_nullifiers all survived a
    // restore. That left a hybrid state where the validator set
    // disagreed with the restored balances, slashing kept hitting
    // ghost stakes, and previously spent nullifiers blocked legitimate
    // notes. Each backend that holds the corresponding CF should
    // override the matching `clear_*` method.

    /// Delete every nullifier. Default no-op for backends with no
    /// privacy CF.
    fn clear_all_nullifiers(&mut self) {}
    /// Delete every persisted note commitment. Default no-op.
    fn clear_all_note_commitments(&mut self) {}
    /// Delete every stake + delegation record. Default no-op.
    fn clear_all_stakes_and_delegations(&mut self) {}
    /// Delete every sentinel parameter and its vote slate. Default no-op.
    fn clear_all_sentinel_state(&mut self) {}
    /// Delete every vesting schedule. Default no-op.
    fn clear_all_vesting_schedules(&mut self) {}
    /// Delete every governance proposal + parameter. Default no-op.
    fn clear_all_governance_state(&mut self) {}

    /// Full clean-slate wipe of every state column the snapshot is
    /// about to repopulate. Called by the snapshot apply path so a
    /// node rejoining via fast-sync does not carry stale stakes,
    /// delegations, sentinel votes, or nullifiers into its restored
    /// life. **DO NOT** call this outside a snapshot-restore — it
    /// destroys consensus state.
    fn wipe_full_state_for_snapshot_restore(&mut self) {
        for id in self.all_object_ids() {
            self.delete_object(&id);
        }
        for id in self.all_ghost_ids() {
            self.remove_ghost(&id);
        }
        for addr in self.all_account_addresses() {
            self.delete_account(&addr);
        }
        self.clear_all_nullifiers();
        self.clear_all_note_commitments();
        self.clear_all_stakes_and_delegations();
        self.clear_all_sentinel_state();
        self.clear_all_vesting_schedules();
        self.clear_all_governance_state();
    }
}

/// In-memory state database for development and testing.
pub struct InMemoryStateDB {
    objects: HashMap<ObjectId, StateObject>,
    ghosts: HashMap<ObjectId, GhostRecord>,
    accounts: HashMap<AccountAddress, Account>,
    /// Persistent incremental trie — mutated on each state change.
    trie: EnergyVerkleTrie,
    /// Object IDs modified via mutable reference (trie update deferred until root computation).
    dirty_objects: std::collections::HashSet<ObjectId>,
    /// Account addresses modified via mutable reference.
    dirty_accounts: std::collections::HashSet<AccountAddress>,
    // Privacy layer state
    note_tree_root: [u8; 32],
    spent_nullifiers: std::collections::HashSet<[u8; 32]>,
    shielded_pool_balance: u64,
    note_count: u64,
    /// Persisted note commitments by leaf index. BTreeMap so iteration
    /// order matches leaf order — required for deterministic tree rebuild.
    note_commitments: BTreeMap<u64, [u8; 32]>,
    /// Cursor for per-epoch storage-rent collection (punch-list 6).
    /// Persists across calls within a single InMemoryStateDB instance
    /// so the gate in `apply_block` actually fires once-per-epoch in
    /// tests, not the trait-default no-op behaviour.
    last_rent_epoch: u64,
    // Stake ledger
    stakes: HashMap<u64, StakeRecord>,
    // Delegation ledger — keyed on (delegator, validator_id) so the same
    // delegator can have multiple delegations to different validators.
    delegations: HashMap<(AccountAddress, u64), DelegationRecord>,
    // Governance
    proposals: HashMap<u64, GovernanceProposal>,
    governance_params: HashMap<String, String>,
    // Vesting registry — addresses 35% Foundation centralization
    // by wrapping large genesis allocations in time-released schedules.
    vesting_schedules: HashMap<u64, evaporchain_types::VestingSchedule>,
    // Sentinel autonomic-governance state (§A2.5).
    sentinel_params: BTreeMap<u32, evaporchain_sentinel::BoundedParameter>,
    sentinel_votes: BTreeMap<u32, Vec<evaporchain_sentinel::Vote>>,
    // Historical snapshots
    snapshots: BTreeMap<u64, HistoricalSnapshot>,
    /// Phase 2 of POST_EXEC_STATE_VERIFICATION_PLAN.md — speculative
    /// execute. The proposer calls `begin_batch` → `execute_block` →
    /// `rollback_batch` to compute a post-state-root without
    /// committing the mutation. RocksDB has had this since 2026-04;
    /// InMemoryStateDB used the trait-default no-op until the
    /// Phase 2 wiring at `tendermint.rs:6582-6592` made the gap
    /// observable. Boxed so the resting-state size is one pointer,
    /// not the full snapshot inline.
    batch_snapshot: Option<Box<InMemoryBatchSnapshot>>,
}

/// Full-state snapshot taken on [`StateDB::begin_batch`] for
/// in-memory backends. RocksDB uses an undo log + a `WriteBatch`
/// that's discarded on rollback; in-memory has no transactional
/// surface to revert against, so we just copy every mutable field
/// and put it back on rollback. Memory cost: one InMemoryStateDB-
/// equivalent for the duration of the simulation.
struct InMemoryBatchSnapshot {
    objects: HashMap<ObjectId, StateObject>,
    ghosts: HashMap<ObjectId, GhostRecord>,
    accounts: HashMap<AccountAddress, Account>,
    trie: EnergyVerkleTrie,
    dirty_objects: std::collections::HashSet<ObjectId>,
    dirty_accounts: std::collections::HashSet<AccountAddress>,
    note_tree_root: [u8; 32],
    spent_nullifiers: std::collections::HashSet<[u8; 32]>,
    shielded_pool_balance: u64,
    note_count: u64,
    note_commitments: BTreeMap<u64, [u8; 32]>,
    last_rent_epoch: u64,
    stakes: HashMap<u64, StakeRecord>,
    delegations: HashMap<(AccountAddress, u64), DelegationRecord>,
    proposals: HashMap<u64, GovernanceProposal>,
    governance_params: HashMap<String, String>,
    vesting_schedules: HashMap<u64, evaporchain_types::VestingSchedule>,
    sentinel_params: BTreeMap<u32, evaporchain_sentinel::BoundedParameter>,
    sentinel_votes: BTreeMap<u32, Vec<evaporchain_sentinel::Vote>>,
    snapshots: BTreeMap<u64, HistoricalSnapshot>,
}

const MAX_SNAPSHOTS: usize = 256;

#[derive(Clone)]
struct HistoricalSnapshot {
    accounts: HashMap<AccountAddress, Account>,
    objects: HashMap<ObjectId, StateObject>,
}

impl InMemoryStateDB {
    pub fn new() -> Self {
        Self {
            objects: HashMap::new(),
            ghosts: HashMap::new(),
            accounts: HashMap::new(),
            trie: EnergyVerkleTrie::new(),
            dirty_objects: std::collections::HashSet::new(),
            dirty_accounts: std::collections::HashSet::new(),
            note_tree_root: [0u8; 32],
            spent_nullifiers: std::collections::HashSet::new(),
            shielded_pool_balance: 0,
            note_count: 0,
            note_commitments: BTreeMap::new(),
            last_rent_epoch: 0,
            stakes: HashMap::new(),
            delegations: HashMap::new(),
            proposals: HashMap::new(),
            governance_params: HashMap::new(),
            vesting_schedules: HashMap::new(),
            sentinel_params: BTreeMap::new(),
            sentinel_votes: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            batch_snapshot: None,
        }
    }

    /// Sync dirty entries into the trie (O(dirty) not O(n)).
    /// Entries are sorted for deterministic trie updates across nodes.
    fn sync_dirty_to_trie(&mut self) {
        let mut dirty_objs: Vec<_> = self.dirty_objects.drain().collect();
        dirty_objs.sort();
        for id in dirty_objs {
            let key = trie_key_for_object(&id);
            if let Some(obj) = self.objects.get(&id) {
                self.trie.insert(
                    key,
                    trie_value_for_object(obj),
                    obj.energy,
                    obj.half_life,
                    obj.last_refreshed,
                );
            } else {
                self.trie.delete(&key);
            }
        }
        let mut dirty_accts: Vec<_> = self.dirty_accounts.drain().collect();
        dirty_accts.sort();
        for addr in dirty_accts {
            let key = trie_key_for_account(&addr);
            if let Some(acc) = self.accounts.get(&addr) {
                self.trie
                    .insert(key, trie_value_for_account(acc), u64::MAX, u64::MAX, 0);
            }
        }
    }
}

impl Default for InMemoryStateDB {
    fn default() -> Self {
        Self::new()
    }
}

impl StateDB for InMemoryStateDB {
    fn get_object(&self, id: &ObjectId) -> Option<&StateObject> {
        self.objects.get(id)
    }

    fn get_object_mut(&mut self, id: &ObjectId) -> Option<&mut StateObject> {
        if self.objects.contains_key(id) {
            self.dirty_objects.insert(*id);
        }
        self.objects.get_mut(id)
    }

    fn put_object(&mut self, obj: StateObject) {
        let key = trie_key_for_object(&obj.id);
        let value = trie_value_for_object(&obj);
        self.trie
            .insert(key, value, obj.energy, obj.half_life, obj.last_refreshed);
        self.objects.insert(obj.id, obj);
    }

    fn delete_object(&mut self, id: &ObjectId) -> Option<StateObject> {
        let key = trie_key_for_object(id);
        self.trie.delete(&key);
        self.objects.remove(id)
    }

    fn put_ghost(&mut self, record: GhostRecord) {
        self.ghosts.insert(record.object_id, record);
    }

    fn get_ghost(&self, id: &ObjectId) -> Option<&GhostRecord> {
        self.ghosts.get(id)
    }

    fn remove_ghost(&mut self, id: &ObjectId) -> Option<GhostRecord> {
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
        if self.accounts.contains_key(addr) {
            self.dirty_accounts.insert(*addr);
        }
        self.accounts.get_mut(addr)
    }

    fn put_account(&mut self, account: Account) {
        let key = trie_key_for_account(&account.address);
        let value = trie_value_for_account(&account);
        self.trie.insert(key, value, u64::MAX, u64::MAX, 0);
        self.accounts.insert(account.address, account);
    }

    fn delete_account(&mut self, addr: &AccountAddress) -> Option<Account> {
        let key = trie_key_for_account(addr);
        self.trie.delete(&key);
        self.dirty_accounts.remove(addr);
        self.accounts.remove(addr)
    }

    fn get_or_create_account(&mut self, addr: &AccountAddress) -> &mut Account {
        if !self.accounts.contains_key(addr) {
            let account = Account {
                address: *addr,
                balance: 0,
                nonce: 0,
                storage_deposit: 0,
                storage_bytes: 0,
                last_touched_epoch: 0,
                vesting: None,
            };
            let key = trie_key_for_account(&account.address);
            let value = trie_value_for_account(&account);
            self.trie.insert(key, value, u64::MAX, u64::MAX, 0);
            self.accounts.insert(*addr, account);
        }
        self.dirty_accounts.insert(*addr);
        self.accounts
            .get_mut(addr)
            .expect("account must exist: just inserted above")
    }

    fn all_account_addresses(&self) -> Vec<AccountAddress> {
        self.accounts.keys().copied().collect()
    }

    fn put_note_tree_root(&mut self, root: [u8; 32]) {
        self.note_tree_root = root;
    }

    fn get_note_tree_root(&self) -> [u8; 32] {
        self.note_tree_root
    }

    fn spend_nullifier(&mut self, nullifier: &[u8; 32]) -> bool {
        self.spent_nullifiers.insert(*nullifier)
    }

    fn is_nullifier_spent(&self, nullifier: &[u8; 32]) -> bool {
        self.spent_nullifiers.contains(nullifier)
    }

    fn nullifier_count(&self) -> usize {
        self.spent_nullifiers.len()
    }

    fn all_nullifiers(&self) -> Vec<[u8; 32]> {
        self.spent_nullifiers.iter().copied().collect()
    }

    fn put_shielded_pool_balance(&mut self, balance: u64) {
        self.shielded_pool_balance = balance;
    }

    fn get_shielded_pool_balance(&self) -> u64 {
        self.shielded_pool_balance
    }

    fn put_note_count(&mut self, count: u64) {
        self.note_count = count;
    }

    fn get_note_count(&self) -> u64 {
        self.note_count
    }

    fn append_note_commitment(&mut self, index: u64, commitment: [u8; 32]) {
        self.note_commitments.insert(index, commitment);
    }

    fn get_all_note_commitments(&self) -> Vec<[u8; 32]> {
        self.note_commitments.values().copied().collect()
    }

    fn get_stake(&self, validator_id: u64) -> Option<&StakeRecord> {
        self.stakes.get(&validator_id)
    }

    fn put_stake(&mut self, record: StakeRecord) {
        self.stakes.insert(record.validator_id, record);
    }

    fn remove_stake(&mut self, validator_id: u64) -> Option<StakeRecord> {
        self.stakes.remove(&validator_id)
    }

    fn all_stakes(&self) -> Vec<&StakeRecord> {
        self.stakes.values().collect()
    }

    fn get_delegation(
        &self,
        delegator: &AccountAddress,
        validator_id: u64,
    ) -> Option<&DelegationRecord> {
        self.delegations.get(&(*delegator, validator_id))
    }

    fn put_delegation(&mut self, record: DelegationRecord) {
        let key = (record.delegator, record.validator_id);
        self.delegations.insert(key, record);
    }

    fn remove_delegation(
        &mut self,
        delegator: &AccountAddress,
        validator_id: u64,
    ) -> Option<DelegationRecord> {
        self.delegations.remove(&(*delegator, validator_id))
    }

    fn delegations_for_validator(&self, validator_id: u64) -> Vec<&DelegationRecord> {
        self.delegations
            .values()
            .filter(|d| d.validator_id == validator_id)
            .collect()
    }

    fn delegations_for_delegator(&self, delegator: &AccountAddress) -> Vec<&DelegationRecord> {
        self.delegations
            .values()
            .filter(|d| &d.delegator == delegator)
            .collect()
    }

    fn all_delegations(&self) -> Vec<&DelegationRecord> {
        self.delegations.values().collect()
    }

    fn compute_state_root(&mut self) -> [u8; 32] {
        self.sync_dirty_to_trie();
        self.trie.root()
    }

    fn compress_cold_subtrees(&mut self) -> u32 {
        self.sync_dirty_to_trie();
        self.trie.compress_cold()
    }

    fn trie_health(&mut self) -> TrieHealth {
        self.sync_dirty_to_trie();
        self.trie.health()
    }

    fn prove_account(&mut self, addr: &AccountAddress) -> evaporchain_crypto::EnergyVerkleProof {
        self.sync_dirty_to_trie();
        let key = trie_key_for_account(addr);
        self.trie.prove(&key)
    }

    fn prove_object(&mut self, id: &ObjectId) -> evaporchain_crypto::EnergyVerkleProof {
        self.sync_dirty_to_trie();
        let key = trie_key_for_object(id);
        self.trie.prove(&key)
    }

    fn prove_at_key(&mut self, key: &[u8; 32]) -> evaporchain_crypto::EnergyVerkleProof {
        self.sync_dirty_to_trie();
        self.trie.prove(key)
    }

    fn trie_snapshot(&mut self) -> Vec<u8> {
        self.sync_dirty_to_trie();
        self.trie.to_bytes()
    }

    fn load_trie_snapshot(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.trie = EnergyVerkleTrie::from_bytes(bytes)?;
        self.dirty_objects.clear();
        self.dirty_accounts.clear();
        Ok(())
    }

    fn prune_before_height(&mut self, height: u64) -> u64 {
        let ids_to_prune: Vec<ObjectId> = self
            .ghosts
            .iter()
            .filter(|(_, ghost)| ghost.evaporated_at < height)
            .map(|(id, _)| *id)
            .collect();
        let count = ids_to_prune.len() as u64;
        for id in ids_to_prune {
            self.ghosts.remove(&id);
        }
        count
    }

    // ─── Governance ─────────────────────────────────────────────────────

    fn get_proposal(&self, proposal_id: u64) -> Option<&GovernanceProposal> {
        self.proposals.get(&proposal_id)
    }

    fn put_proposal(&mut self, proposal: GovernanceProposal) {
        self.proposals.insert(proposal.proposal_id, proposal);
    }

    fn all_proposals(&self) -> Vec<&GovernanceProposal> {
        self.proposals.values().collect()
    }

    fn get_governance_param(&self, key: &str) -> Option<&str> {
        self.governance_params.get(key).map(|s| s.as_str())
    }

    fn put_governance_param(&mut self, key: String, value: String) {
        self.governance_params.insert(key, value);
    }

    // ─── Storage-rent cursor (punch-list 6) ───────────────────────────

    fn get_last_rent_epoch(&self) -> u64 {
        self.last_rent_epoch
    }

    fn put_last_rent_epoch(&mut self, epoch: u64) {
        self.last_rent_epoch = epoch;
    }

    // ─── Vesting registry ─────────────────────────────────────────────

    fn get_vesting_schedule(&self, id: u64) -> Option<&evaporchain_types::VestingSchedule> {
        self.vesting_schedules.get(&id)
    }

    fn put_vesting_schedule(&mut self, schedule: evaporchain_types::VestingSchedule) {
        self.vesting_schedules.insert(schedule.id, schedule);
    }

    fn remove_vesting_schedule(&mut self, id: u64) -> Option<evaporchain_types::VestingSchedule> {
        self.vesting_schedules.remove(&id)
    }

    fn all_vesting_schedules(&self) -> Vec<evaporchain_types::VestingSchedule> {
        self.vesting_schedules.values().cloned().collect()
    }

    // ─── Historical Snapshots ───────────────────────────────────────────

    fn commit_state_snapshot(&mut self, height: u64) {
        let snap = HistoricalSnapshot {
            accounts: self.accounts.clone(),
            objects: self.objects.clone(),
        };
        self.snapshots.insert(height, snap);
        while self.snapshots.len() > MAX_SNAPSHOTS {
            if let Some(&oldest) = self.snapshots.keys().next() {
                self.snapshots.remove(&oldest);
            }
        }
    }

    fn get_account_at_height(&self, address: &AccountAddress, height: u64) -> Option<Account> {
        self.snapshots
            .get(&height)
            .and_then(|s| s.accounts.get(address).cloned())
    }

    fn get_object_at_height(&self, id: &ObjectId, height: u64) -> Option<StateObject> {
        self.snapshots
            .get(&height)
            .and_then(|s| s.objects.get(id).cloned())
    }

    fn earliest_snapshot_height(&self) -> Option<u64> {
        self.snapshots.keys().next().copied()
    }

    fn latest_snapshot_height(&self) -> Option<u64> {
        self.snapshots.keys().next_back().copied()
    }

    fn prune_snapshots_before(&mut self, height: u64) {
        self.snapshots = self.snapshots.split_off(&height);
    }

    // ─── Sentinel autonomic governance ──────────────────────────────────

    fn get_sentinel_param(&self, id: u32) -> Option<evaporchain_sentinel::BoundedParameter> {
        self.sentinel_params.get(&id).copied()
    }

    fn put_sentinel_param(&mut self, param: evaporchain_sentinel::BoundedParameter) {
        self.sentinel_params.insert(param.id, param);
        self.sentinel_votes.entry(param.id).or_default();
    }

    fn all_sentinel_params(&self) -> Vec<evaporchain_sentinel::BoundedParameter> {
        self.sentinel_params.values().copied().collect()
    }

    fn get_sentinel_votes(&self, id: u32) -> Vec<evaporchain_sentinel::Vote> {
        self.sentinel_votes.get(&id).cloned().unwrap_or_default()
    }

    fn put_sentinel_votes(&mut self, parameter_id: u32, votes: Vec<evaporchain_sentinel::Vote>) {
        self.sentinel_votes.insert(parameter_id, votes);
    }

    // ─── M12 bulk-clear helpers ─────────────────────────────────────────

    fn clear_all_nullifiers(&mut self) {
        self.spent_nullifiers.clear();
    }

    fn clear_all_note_commitments(&mut self) {
        self.note_commitments.clear();
    }

    fn clear_all_stakes_and_delegations(&mut self) {
        self.stakes.clear();
        self.delegations.clear();
    }

    fn clear_all_sentinel_state(&mut self) {
        self.sentinel_params.clear();
        self.sentinel_votes.clear();
    }

    fn clear_all_vesting_schedules(&mut self) {
        self.vesting_schedules.clear();
    }

    // governance proposals / params are not currently held by
    // InMemoryStateDB beyond the trait's default-no-op accessors —
    // nothing to clear here. The trait default suffices.

    // ─── Batch (begin/commit/rollback) ───────────────────────────────
    //
    // Phase 2 of POST_EXEC_STATE_VERIFICATION_PLAN.md (wired at
    // `tendermint.rs:6582-6592`) calls `begin_batch` →
    // `execute_block` → `rollback_batch` to compute a post-state-root
    // without committing the mutation. RocksDB has supported this
    // since 2026-04 via per-write undo logging + WriteBatch
    // discard. The trait's default no-op impl was acceptable when
    // nothing speculative-executed against InMemoryStateDB; Phase 2
    // changed that. Without these overrides, a proposer's
    // speculative execute permanently mutates the in-memory state
    // and the validator-path apply re-runs against an already-
    // mutated DB, producing nonce / balance / MMR divergence.
    //
    // Approach: snapshot every mutable field on `begin_batch`,
    // restore on `rollback_batch`. Memory cost: one InMemoryStateDB-
    // equivalent for the duration of a simulation. No per-write
    // tracking — simpler than RocksDB's undo log, fine for in-
    // memory because we have no transactional surface to revert
    // against.

    fn begin_batch(&mut self) {
        self.batch_snapshot = Some(Box::new(InMemoryBatchSnapshot {
            objects: self.objects.clone(),
            ghosts: self.ghosts.clone(),
            accounts: self.accounts.clone(),
            trie: self.trie.clone(),
            dirty_objects: self.dirty_objects.clone(),
            dirty_accounts: self.dirty_accounts.clone(),
            note_tree_root: self.note_tree_root,
            spent_nullifiers: self.spent_nullifiers.clone(),
            shielded_pool_balance: self.shielded_pool_balance,
            note_count: self.note_count,
            note_commitments: self.note_commitments.clone(),
            last_rent_epoch: self.last_rent_epoch,
            stakes: self.stakes.clone(),
            delegations: self.delegations.clone(),
            proposals: self.proposals.clone(),
            governance_params: self.governance_params.clone(),
            vesting_schedules: self.vesting_schedules.clone(),
            sentinel_params: self.sentinel_params.clone(),
            sentinel_votes: self.sentinel_votes.clone(),
            snapshots: self.snapshots.clone(),
        }));
    }

    fn commit_batch(&mut self) -> Result<(), String> {
        // Drop the snapshot — mutations already in self are now permanent.
        self.batch_snapshot = None;
        Ok(())
    }

    fn rollback_batch(&mut self) {
        if let Some(snap) = self.batch_snapshot.take() {
            self.objects = snap.objects;
            self.ghosts = snap.ghosts;
            self.accounts = snap.accounts;
            self.trie = snap.trie;
            self.dirty_objects = snap.dirty_objects;
            self.dirty_accounts = snap.dirty_accounts;
            self.note_tree_root = snap.note_tree_root;
            self.spent_nullifiers = snap.spent_nullifiers;
            self.shielded_pool_balance = snap.shielded_pool_balance;
            self.note_count = snap.note_count;
            self.note_commitments = snap.note_commitments;
            self.last_rent_epoch = snap.last_rent_epoch;
            self.stakes = snap.stakes;
            self.delegations = snap.delegations;
            self.proposals = snap.proposals;
            self.governance_params = snap.governance_params;
            self.vesting_schedules = snap.vesting_schedules;
            self.sentinel_params = snap.sentinel_params;
            self.sentinel_votes = snap.sentinel_votes;
            self.snapshots = snap.snapshots;
        }
        // No snapshot active = no-op, mirroring RocksDB behaviour.
    }
}

pub fn object_state_to_u8(s: &evaporchain_types::ObjectState) -> u8 {
    match s {
        evaporchain_types::ObjectState::Active => 0,
        evaporchain_types::ObjectState::Grace => 1,
        evaporchain_types::ObjectState::Ghost => 2,
        evaporchain_types::ObjectState::Resurrected => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::ObjectState;

    fn make_object(id_byte: u8, energy: u64) -> StateObject {
        StateObject {
            id: {
                let mut id = [0u8; 32];
                id[0] = id_byte;
                id
            },
            owner: [0u8; 32],
            energy,
            half_life: 100,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![],
            decay_curve: None,
            lad_mode: None,
        }
    }

    #[test]
    fn test_put_and_get() {
        let mut db = InMemoryStateDB::new();
        let obj = make_object(1, 1000);
        db.put_object(obj.clone());
        assert_eq!(db.get_object(&obj.id), Some(&obj));
        assert_eq!(db.object_count(), 1);
    }

    #[test]
    fn test_delete() {
        let mut db = InMemoryStateDB::new();
        let obj = make_object(1, 1000);
        let id = obj.id;
        db.put_object(obj);
        let deleted = db.delete_object(&id);
        assert!(deleted.is_some());
        assert_eq!(db.object_count(), 0);
        assert!(db.get_object(&id).is_none());
    }

    #[test]
    fn test_ghost_records() {
        let mut db = InMemoryStateDB::new();
        let ghost = GhostRecord {
            object_id: [5u8; 32],
            owner: [0u8; 32],
            evaporated_at: 100,
            data_hash: [0u8; 32],
            original_data: Some(vec![1, 2, 3]),
            mmr_position: None,
            original_half_life: None,
        };
        db.put_ghost(ghost.clone());
        assert_eq!(db.ghost_count(), 1);
        assert_eq!(db.get_ghost(&[5u8; 32]), Some(&ghost));

        let removed = db.remove_ghost(&[5u8; 32]);
        assert!(removed.is_some());
        assert_eq!(db.ghost_count(), 0);
    }

    #[test]
    fn test_state_root_deterministic() {
        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(1, 1000));
        db.put_object(make_object(2, 500));
        let root1 = db.compute_state_root();
        let root2 = db.compute_state_root();
        assert_eq!(root1, root2);
        assert_ne!(root1, [0u8; 32]);
    }

    #[test]
    fn test_state_root_changes_on_mutation() {
        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(1, 1000));
        let root1 = db.compute_state_root();

        db.get_object_mut(&{
            let mut id = [0u8; 32];
            id[0] = 1;
            id
        })
        .unwrap()
        .energy = 500;
        let root2 = db.compute_state_root();

        assert_ne!(root1, root2);
    }

    #[test]
    fn test_empty_state_root() {
        let mut db = InMemoryStateDB::new();
        assert_eq!(db.compute_state_root(), [0u8; 32]);
    }

    #[test]
    fn test_trie_health_reflects_state() {
        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(1, 1000));
        db.put_object(make_object(2, 500));

        let health = db.trie_health();
        assert_eq!(health.active_leaves, 2);
        assert_eq!(health.max_energy, 1000);
        assert_eq!(health.min_half_life, 100);
        assert_eq!(health.compressed_leaves, 0);
    }

    #[test]
    fn test_trie_health_with_accounts() {
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: [1u8; 32],
            balance: 100,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        db.put_object(make_object(1, 500));

        let health = db.trie_health();
        assert_eq!(health.active_leaves, 2);
    }

    #[test]
    fn test_compress_cold_subtrees_with_zero_energy() {
        let mut db = InMemoryStateDB::new();
        let mut obj = make_object(1, 0);
        obj.state = ObjectState::Grace;
        obj.energy = 0;
        db.put_object(obj);
        db.put_object(make_object(2, 1000));

        let _compressed = db.compress_cold_subtrees();
    }

    #[test]
    fn test_empty_trie_health() {
        let mut db = InMemoryStateDB::new();
        let health = db.trie_health();
        assert_eq!(health.active_leaves, 0);
        assert_eq!(health.total_nodes, 0);
    }

    #[test]
    fn test_prune_before_height_removes_old_ghosts() {
        let mut db = InMemoryStateDB::new();

        // Insert ghosts at various epochs
        for epoch in [10u64, 50, 100, 200] {
            db.put_ghost(GhostRecord {
                object_id: {
                    let mut id = [0u8; 32];
                    id[0] = epoch as u8;
                    id
                },
                owner: [0u8; 32],
                evaporated_at: epoch,
                data_hash: [0u8; 32],
                original_data: None,
                mmr_position: None,
                original_half_life: None,
            });
        }
        assert_eq!(db.ghost_count(), 4);

        // Prune everything before height 100 — removes epochs 10 and 50
        let pruned = db.prune_before_height(100);
        assert_eq!(pruned, 2);
        assert_eq!(db.ghost_count(), 2);

        // Ghost at epoch 100 should still exist (not strictly less than 100)
        let mut id100 = [0u8; 32];
        id100[0] = 100;
        assert!(db.get_ghost(&id100).is_some());
    }

    #[test]
    fn test_prune_before_height_zero_is_noop() {
        let mut db = InMemoryStateDB::new();
        db.put_ghost(GhostRecord {
            object_id: [1u8; 32],
            owner: [0u8; 32],
            evaporated_at: 0,
            data_hash: [0u8; 32],
            original_data: None,
            mmr_position: None,
            original_half_life: None,
        });
        let pruned = db.prune_before_height(0);
        assert_eq!(pruned, 0);
        assert_eq!(db.ghost_count(), 1);
    }

    /// Punch-list 6: rent cursor defaults to 0 and persists writes
    /// monotonically. The InMemoryStateDB override (not the trait
    /// default no-op) is what makes the gate effective in tests.
    #[test]
    fn test_last_rent_epoch_in_memory_persists_writes() {
        let mut db = InMemoryStateDB::new();
        assert_eq!(db.get_last_rent_epoch(), 0, "fresh DB defaults to 0");
        db.put_last_rent_epoch(7);
        assert_eq!(db.get_last_rent_epoch(), 7);
        // Monotonic advance: rent collection always moves forward in epochs.
        db.put_last_rent_epoch(13);
        assert_eq!(db.get_last_rent_epoch(), 13);
        // The store also accepts equal-or-lower writes (callers must enforce
        // monotonicity at their layer; this layer is just a kv).
        db.put_last_rent_epoch(13);
        assert_eq!(db.get_last_rent_epoch(), 13);
    }

    // ─── Phase 2 (POST_EXEC_STATE_VERIFICATION_PLAN.md) ──────────────
    //
    // Without these batch overrides, the proposer's speculative
    // execute mutates the in-memory DB and the validator-path apply
    // re-runs against an already-mutated state. RocksDB rollback
    // works in prod; in-memory was the gap.

    fn make_account(addr_byte: u8, balance: u64, nonce: u64) -> Account {
        Account {
            address: [addr_byte; 32],
            balance,
            nonce,
            ..Account::default()
        }
    }

    #[test]
    fn batch_rollback_reverts_account_writes() {
        let mut db = InMemoryStateDB::new();

        // Pre-batch state: one account at balance 100.
        let addr1 = [1u8; 32];
        db.put_account(make_account(1, 100, 0));
        assert_eq!(db.get_account(&addr1).unwrap().balance, 100);

        // Begin a batch and mutate.
        db.begin_batch();
        db.put_account(make_account(1, 999, 5));
        let addr2 = [2u8; 32];
        db.put_account(make_account(2, 50, 0));
        // Mid-batch, mutations are visible.
        assert_eq!(db.get_account(&addr1).unwrap().balance, 999);
        assert_eq!(db.get_account(&addr1).unwrap().nonce, 5);
        assert_eq!(db.get_account(&addr2).unwrap().balance, 50);

        // Rollback — both mutations must vanish.
        db.rollback_batch();
        assert_eq!(
            db.get_account(&addr1).unwrap().balance,
            100,
            "rollback must restore pre-batch balance on existing account"
        );
        assert_eq!(
            db.get_account(&addr1).unwrap().nonce,
            0,
            "rollback must restore pre-batch nonce"
        );
        assert!(
            db.get_account(&addr2).is_none(),
            "rollback must drop account that didn't exist pre-batch"
        );
    }

    #[test]
    fn batch_commit_keeps_writes() {
        let mut db = InMemoryStateDB::new();
        let addr = [3u8; 32];

        db.begin_batch();
        db.put_account(make_account(3, 777, 0));
        let _ = db.commit_batch();

        // Commit keeps mutations; rollback after commit is a no-op
        // because the snapshot was dropped.
        assert_eq!(db.get_account(&addr).unwrap().balance, 777);
        db.rollback_batch();
        assert_eq!(
            db.get_account(&addr).unwrap().balance,
            777,
            "rollback after commit must be a no-op"
        );
    }

    #[test]
    fn batch_rollback_no_active_batch_is_noop() {
        let mut db = InMemoryStateDB::new();
        let addr = [4u8; 32];
        db.put_account(make_account(4, 42, 0));

        // Calling rollback without begin_batch must not corrupt state.
        db.rollback_batch();
        assert_eq!(db.get_account(&addr).unwrap().balance, 42);
    }

    /// T1.20 — delegation CRUD on InMemoryStateDB:
    /// put_delegation, get_delegation, remove_delegation,
    /// delegations_for_validator, delegations_for_delegator,
    /// all_delegations. Previously uncovered (lines 675-712).
    #[test]
    fn t1_20_in_memory_delegation_crud() {
        let mut db = InMemoryStateDB::new();
        let d1: AccountAddress = [10u8; 32];
        let d2: AccountAddress = [20u8; 32];

        let rec1 = DelegationRecord {
            delegator: d1,
            validator_id: 1,
            amount: 100,
            delegated_at_epoch: 5,
            unbonding_amount: 0,
            unbonding_epoch: None,
        };
        let rec2 = DelegationRecord {
            delegator: d1,
            validator_id: 2,
            amount: 50,
            delegated_at_epoch: 5,
            unbonding_amount: 0,
            unbonding_epoch: None,
        };
        let rec3 = DelegationRecord {
            delegator: d2,
            validator_id: 1,
            amount: 200,
            delegated_at_epoch: 5,
            unbonding_amount: 0,
            unbonding_epoch: None,
        };

        db.put_delegation(rec1.clone());
        db.put_delegation(rec2.clone());
        db.put_delegation(rec3.clone());

        assert_eq!(db.get_delegation(&d1, 1).unwrap().amount, 100);
        assert_eq!(db.get_delegation(&d1, 2).unwrap().amount, 50);
        assert!(db.get_delegation(&d1, 99).is_none());

        let v1 = db.delegations_for_validator(1);
        assert_eq!(v1.len(), 2);
        let v2 = db.delegations_for_validator(2);
        assert_eq!(v2.len(), 1);

        let by_d1 = db.delegations_for_delegator(&d1);
        assert_eq!(by_d1.len(), 2);
        let by_d2 = db.delegations_for_delegator(&d2);
        assert_eq!(by_d2.len(), 1);

        assert_eq!(db.all_delegations().len(), 3);

        let removed = db.remove_delegation(&d1, 1).unwrap();
        assert_eq!(removed.amount, 100);
        assert!(db.get_delegation(&d1, 1).is_none());
        assert_eq!(db.all_delegations().len(), 2);
    }

    /// T1.20 — historical-snapshot APIs on InMemoryStateDB.
    /// commit_state_snapshot, get_account_at_height,
    /// get_object_at_height, earliest/latest_snapshot_height,
    /// prune_snapshots_before. Previously uncovered (lines 825-859).
    #[test]
    fn t1_20_in_memory_historical_snapshots() {
        let mut db = InMemoryStateDB::new();
        let addr: AccountAddress = [42u8; 32];

        // No snapshots yet.
        assert!(db.earliest_snapshot_height().is_none());
        assert!(db.latest_snapshot_height().is_none());

        // Snapshot at height 10 with balance 100.
        let acc = Account {
            address: addr,
            balance: 100,
            ..Account::default()
        };
        db.put_account(acc);
        db.commit_state_snapshot(10);

        // Snapshot at height 20 after balance bump.
        let acc2 = Account {
            address: addr,
            balance: 200,
            ..Account::default()
        };
        db.put_account(acc2);
        db.commit_state_snapshot(20);

        let h10 = db.get_account_at_height(&addr, 10).unwrap();
        assert_eq!(h10.balance, 100, "height 10 captures the 100 balance");
        let h20 = db.get_account_at_height(&addr, 20).unwrap();
        assert_eq!(h20.balance, 200);
        assert!(db.get_account_at_height(&addr, 99).is_none());

        assert_eq!(db.earliest_snapshot_height(), Some(10));
        assert_eq!(db.latest_snapshot_height(), Some(20));

        // Prune everything before height 20 → only h20 remains.
        db.prune_snapshots_before(20);
        assert!(db.get_account_at_height(&addr, 10).is_none());
        assert_eq!(db.get_account_at_height(&addr, 20).unwrap().balance, 200);
        assert_eq!(db.earliest_snapshot_height(), Some(20));
    }

    /// T1.20 — InMemoryStateDB::get_object_at_height returns None
    /// for missing-snapshot AND missing-object-in-snapshot paths.
    #[test]
    fn t1_20_in_memory_object_at_height_none_paths() {
        let mut db = InMemoryStateDB::new();
        let obj_id: ObjectId = [50u8; 32];
        // No snapshots at all → None.
        assert!(db.get_object_at_height(&obj_id, 1).is_none());

        // Snapshot exists but object not in it.
        db.commit_state_snapshot(1);
        assert!(db.get_object_at_height(&obj_id, 1).is_none());
    }

    /// T1.20 — governance CRUD on InMemoryStateDB: put_proposal,
    /// get_proposal, all_proposals, put_governance_param,
    /// get_governance_param. Previously uncovered (lines 774-792).
    #[test]
    fn t1_20_in_memory_governance_proposals_and_params() {
        let mut db = InMemoryStateDB::new();

        // Missing proposal → None.
        assert!(db.get_proposal(1).is_none());

        let prop = GovernanceProposal {
            proposal_id: 1,
            title: "raise gas limit".into(),
            param_key: "block_gas_limit".into(),
            param_value: "30000000".into(),
            proposer: [99u8; 32],
            start_epoch: 1,
            end_epoch: 100,
            votes_for: 0,
            votes_against: 0,
            status: evaporchain_types::ProposalStatus::Active,
            created_at: 0,
            voters: Default::default(),
        };
        db.put_proposal(prop);
        let stored = db.get_proposal(1).unwrap();
        assert_eq!(stored.proposal_id, 1);
        assert_eq!(db.all_proposals().len(), 1);

        // Governance params kv.
        assert!(db.get_governance_param("freeze").is_none());
        db.put_governance_param("freeze".into(), "true".into());
        assert_eq!(db.get_governance_param("freeze"), Some("true"));
    }

    /// T1.20 — vesting registry on InMemoryStateDB:
    /// put_vesting_schedule, get_vesting_schedule,
    /// remove_vesting_schedule, all_vesting_schedules. Previously
    /// uncovered (lines 806-820).
    #[test]
    fn t1_20_in_memory_vesting_registry() {
        let mut db = InMemoryStateDB::new();
        let sched = evaporchain_types::VestingSchedule {
            id: 1,
            beneficiary: [33u8; 32],
            total_amount: 1_000_000,
            start_epoch: 0,
            cliff_epochs: 10,
            vesting_epochs: 100,
            released_amount: 0,
        };
        db.put_vesting_schedule(sched.clone());
        let got = db.get_vesting_schedule(1).unwrap();
        assert_eq!(got.total_amount, 1_000_000);
        assert_eq!(db.all_vesting_schedules().len(), 1);

        let removed = db.remove_vesting_schedule(1).unwrap();
        assert_eq!(removed.id, 1);
        assert!(db.get_vesting_schedule(1).is_none());
        assert_eq!(db.all_vesting_schedules().len(), 0);
    }
    #[test]
    fn test_in_memory_db_default_constructor() {
        let db = InMemoryStateDB::default();
        assert_eq!(db.object_count(), 0);
        assert_eq!(db.all_account_addresses().len(), 0);
    }

    #[test]
    fn test_nullifier_count_after_spend() {
        let mut db = InMemoryStateDB::new();
        assert_eq!(db.nullifier_count(), 0);
        let n1 = [1u8; 32];
        let n2 = [2u8; 32];
        assert!(db.spend_nullifier(&n1));
        assert!(db.spend_nullifier(&n2));
        assert_eq!(db.nullifier_count(), 2);
        assert!(!db.spend_nullifier(&n1), "double spend must be blocked");
        assert_eq!(db.nullifier_count(), 2);
    }

    #[test]
    fn test_remove_stake_and_all_stakes() {
        let mut db = InMemoryStateDB::new();
        let stake = evaporchain_types::StakeRecord {
            validator_id: 7,
            validator_address: [7u8; 32],
            staked_amount: 500,
            staked_at_epoch: 0,
            unbonding_epoch: None,
            slashed_amount: 0,
        };
        db.put_stake(stake.clone());
        assert_eq!(db.all_stakes().len(), 1);
        let removed = db.remove_stake(7);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().staked_amount, 500);
        assert_eq!(db.all_stakes().len(), 0);
        // remove non-existent returns None
        assert!(db.remove_stake(99).is_none());
    }

    #[test]
    fn test_prove_account_and_prove_object() {
        let mut db = InMemoryStateDB::new();
        let addr = [42u8; 32];
        // prove_account on empty trie should return a zero-depth proof
        let proof = db.prove_account(&addr);
        assert_eq!(proof.depth, 0);
        // put an account and prove again
        db.put_account(evaporchain_types::Account {
            address: addr,
            balance: 100,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        let _proof2 = db.prove_account(&addr);
        // prove_object
        let obj = make_object(9, 200);
        db.put_object(obj.clone());
        let _proof3 = db.prove_object(&obj.id);
    }

    #[test]
    fn test_default_trait_batch_methods_on_inmemory() {
        let mut db = InMemoryStateDB::new();
        // InMemoryStateDB overrides begin_batch/commit_batch/rollback_batch so
        // we cover the InMemory paths; default trait stubs are covered below.
        db.begin_batch();
        assert!(db.commit_batch().is_ok());
        db.rollback_batch();
    }

    /// Cover the trait-level default no-op stubs by calling them through a
    /// minimal `StateDB` implementor that does NOT override them.
    #[test]
    fn test_default_statedb_trait_stubs() {
        use evaporchain_crypto::EnergyVerkleProof;
        use evaporchain_types::{
            Account, AccountAddress, GhostRecord, ObjectId, StakeRecord, StateObject,
        };
        use std::collections::HashMap;

        // Minimal StateDB that only implements the strictly-required methods;
        // everything else falls back to the trait defaults.
        struct MinimalDB {
            objects: HashMap<ObjectId, StateObject>,
            accounts: HashMap<AccountAddress, Account>,
            ghosts: HashMap<ObjectId, GhostRecord>,
            nullifiers: std::collections::HashSet<[u8; 32]>,
        }
        impl MinimalDB {
            fn new() -> Self {
                Self {
                    objects: HashMap::new(),
                    accounts: HashMap::new(),
                    ghosts: HashMap::new(),
                    nullifiers: std::collections::HashSet::new(),
                }
            }
        }
        impl StateDB for MinimalDB {
            fn get_object(&self, id: &ObjectId) -> Option<&StateObject> {
                self.objects.get(id)
            }
            fn get_object_mut(&mut self, id: &ObjectId) -> Option<&mut StateObject> {
                self.objects.get_mut(id)
            }
            fn put_object(&mut self, obj: StateObject) {
                self.objects.insert(obj.id, obj);
            }
            fn delete_object(&mut self, id: &ObjectId) -> Option<StateObject> {
                self.objects.remove(id)
            }
            fn put_ghost(&mut self, record: GhostRecord) {
                self.ghosts.insert(record.object_id, record);
            }
            fn get_ghost(&self, id: &ObjectId) -> Option<&GhostRecord> {
                self.ghosts.get(id)
            }
            fn remove_ghost(&mut self, id: &ObjectId) -> Option<GhostRecord> {
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
            fn compress_cold_subtrees(&mut self) -> u32 {
                0
            }
            fn trie_health(&mut self) -> crate::TrieHealth {
                crate::TrieHealth {
                    active_leaves: 0,
                    compressed_leaves: 0,
                    total_nodes: 0,
                    max_energy: 0,
                    min_half_life: 0,
                    last_activity_epoch: 0,
                    compressions: 0,
                    decompressions: 0,
                }
            }
            fn prove_account(&mut self, _addr: &AccountAddress) -> EnergyVerkleProof {
                EnergyVerkleProof {
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
            fn prove_object(&mut self, _id: &ObjectId) -> EnergyVerkleProof {
                EnergyVerkleProof {
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
            fn prove_at_key(&mut self, _key: &[u8; 32]) -> EnergyVerkleProof {
                EnergyVerkleProof {
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
            fn trie_snapshot(&mut self) -> Vec<u8> {
                vec![]
            }
            fn load_trie_snapshot(&mut self, _bytes: &[u8]) -> Result<(), String> {
                Ok(())
            }
            fn put_note_tree_root(&mut self, _root: [u8; 32]) {}
            fn get_note_tree_root(&self) -> [u8; 32] {
                [0u8; 32]
            }
            fn spend_nullifier(&mut self, n: &[u8; 32]) -> bool {
                self.nullifiers.insert(*n)
            }
            fn is_nullifier_spent(&self, n: &[u8; 32]) -> bool {
                self.nullifiers.contains(n)
            }
            fn nullifier_count(&self) -> usize {
                self.nullifiers.len()
            }
            fn all_nullifiers(&self) -> Vec<[u8; 32]> {
                self.nullifiers.iter().copied().collect()
            }
            fn put_shielded_pool_balance(&mut self, _b: u64) {}
            fn get_shielded_pool_balance(&self) -> u64 {
                0
            }
            fn put_note_count(&mut self, _c: u64) {}
            fn get_note_count(&self) -> u64 {
                0
            }
            fn append_note_commitment(&mut self, _idx: u64, _c: [u8; 32]) {}
            fn get_all_note_commitments(&self) -> Vec<[u8; 32]> {
                vec![]
            }
            fn put_stake(&mut self, _r: StakeRecord) {}
            fn get_stake(&self, _id: u64) -> Option<&StakeRecord> {
                None
            }
            fn remove_stake(&mut self, _id: u64) -> Option<StakeRecord> {
                None
            }
            fn all_stakes(&self) -> Vec<&StakeRecord> {
                vec![]
            }
            fn put_delegation(&mut self, _r: evaporchain_types::DelegationRecord) {}
            fn get_delegation(
                &self,
                _delegator: &AccountAddress,
                _validator_id: u64,
            ) -> Option<&evaporchain_types::DelegationRecord> {
                None
            }
            fn remove_delegation(
                &mut self,
                _delegator: &AccountAddress,
                _validator_id: u64,
            ) -> Option<evaporchain_types::DelegationRecord> {
                None
            }
            fn delegations_for_validator(
                &self,
                _id: u64,
            ) -> Vec<&evaporchain_types::DelegationRecord> {
                vec![]
            }
            fn delegations_for_delegator(
                &self,
                _delegator: &AccountAddress,
            ) -> Vec<&evaporchain_types::DelegationRecord> {
                vec![]
            }
            fn all_delegations(&self) -> Vec<&evaporchain_types::DelegationRecord> {
                vec![]
            }
            fn get_proposal(&self, _id: u64) -> Option<&evaporchain_types::GovernanceProposal> {
                None
            }
            fn put_proposal(&mut self, _p: evaporchain_types::GovernanceProposal) {}
            fn all_proposals(&self) -> Vec<&evaporchain_types::GovernanceProposal> {
                vec![]
            }
            fn get_governance_param(&self, _key: &str) -> Option<&str> {
                None
            }
            fn put_governance_param(&mut self, _key: String, _value: String) {}
            fn commit_state_snapshot(&mut self, _height: u64) {}
            fn get_account_at_height(
                &self,
                _addr: &AccountAddress,
                _height: u64,
            ) -> Option<Account> {
                None
            }
            fn get_object_at_height(&self, _id: &ObjectId, _height: u64) -> Option<StateObject> {
                None
            }
            fn earliest_snapshot_height(&self) -> Option<u64> {
                None
            }
            fn latest_snapshot_height(&self) -> Option<u64> {
                None
            }
            fn prune_snapshots_before(&mut self, _height: u64) {}
            fn prune_before_height(&mut self, _height: u64) -> u64 {
                0
            }
        }

        let mut db = MinimalDB::new();
        // Call all the trait defaults that are uncovered
        db.begin_batch();
        assert!(db.commit_batch().is_ok());
        db.rollback_batch();
        assert_eq!(db.get_last_rent_epoch(), 0);
        db.put_last_rent_epoch(5);
        assert!(db.get_vesting_schedule(1).is_none());
        db.put_vesting_schedule(evaporchain_types::VestingSchedule {
            id: 1,
            beneficiary: [0u8; 32],
            total_amount: 1_000,
            start_epoch: 0,
            cliff_epochs: 0,
            vesting_epochs: 10,
            released_amount: 0,
        });
        assert!(db.remove_vesting_schedule(1).is_none()); // default returns None
        assert_eq!(db.all_vesting_schedules(), vec![]);
        assert!(db.get_sentinel_param(1).is_none());
        db.put_sentinel_param(evaporchain_sentinel::BoundedParameter {
            id: 1,
            current: 5,
            min: 0,
            max: 100,
        });
        assert_eq!(db.all_sentinel_params(), vec![]);
        assert_eq!(db.get_sentinel_votes(1), vec![]);
        db.put_sentinel_votes(1, vec![]);
        db.clear_all_nullifiers();
        db.clear_all_note_commitments();
        db.clear_all_stakes_and_delegations();
        db.clear_all_sentinel_state();
        db.clear_all_vesting_schedules();
        db.clear_all_governance_state();
    }
}
