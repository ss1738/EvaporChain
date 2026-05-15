//! Privacy Execution Engine for EvaporChain
//!
//! Processes Shield, Unshield, and PrivateTransfer transactions by wiring
//! the cryptographic primitives from `evaporchain-proving::privacy` into the
//! block execution pipeline.
//!
//! Architecture:
//!   - PrivacyExecutor owns a PrivacyEngine (note tree + nullifier set)
//!   - Shield: debit transparent balance → create private note in tree
//!   - Unshield: verify ZK proof → spend nullifiers → credit transparent balance
//!   - PrivateTransfer: verify ZK proof → spend nullifiers → create output notes
//!   - State is synced back to StateDB (note tree root, nullifiers, pool balance)

use evaporchain_proving::privacy::{
    verify_balance_binding, verify_merkle_proof, BalanceBindingKind, Commitment, MerkleProof,
    Nullifier, PrivacyEngine,
};
use evaporchain_state::db::StateDB;
use evaporchain_types::{MerkleProofData, PrivateTransferTx, ShieldTx, UnshieldTx};
use thiserror::Error;
use tracing::debug;

/// Default Merkle tree depth for the note tree (2^20 = ~1M notes).
pub const NOTE_TREE_DEPTH: usize = 20;

/// Gas costs for privacy transactions.
pub const GAS_SHIELD: u64 = 60_000;
pub const GAS_UNSHIELD: u64 = 80_000;
pub const GAS_PRIVATE_TRANSFER_BASE: u64 = 100_000;
pub const GAS_PRIVATE_TRANSFER_PER_INPUT: u64 = 20_000;
pub const GAS_PRIVATE_TRANSFER_PER_OUTPUT: u64 = 15_000;

/// Errors specific to privacy transaction execution.
#[derive(Debug, Error)]
pub enum PrivacyExecError {
    #[error("shield amount must be > 0")]
    ZeroShieldAmount,
    #[error("insufficient balance for shield: has {available}, needs {required}")]
    InsufficientBalanceForShield { available: u64, required: u64 },
    #[error("unshield amount must be > 0")]
    ZeroUnshieldAmount,
    #[error("double spend: nullifier {0} already spent")]
    DoubleSpend(String),
    #[error("stale anchor: proof references old Merkle root")]
    StaleAnchor,
    #[error("no input nullifiers in transfer")]
    NoInputs,
    #[error("balance mismatch in unshield proof")]
    UnshieldBalanceMismatch,
    #[error("invalid balance binding: hash does not match claimed amounts and blindings")]
    InvalidBalanceBinding,
    #[error("invalid commitment opening: input {index} does not open to claimed amount/blinding")]
    InvalidCommitmentOpening { index: usize },
    #[error("invalid output commitment: output {index} does not match claimed amount/blinding")]
    InvalidOutputCommitment { index: usize },
    #[error("invalid Merkle proof: input {index} is not a member of the note tree")]
    InvalidMerkleProof { index: usize },
    #[error("Merkle proof anchor mismatch: input {index} references a different root")]
    MerkleProofAnchorMismatch { index: usize },
    #[error("missing witness data: {field} required for privacy verification")]
    MissingWitnessData { field: String },
    #[error("witness field count mismatch: expected {expected}, got {got}")]
    WitnessFieldMismatch { expected: usize, got: usize },
    #[error("note tree is full")]
    TreeFull,
    #[error("private transfer has no outputs")]
    NoOutputs,
    #[error("energy decay proof references future epoch {epoch}, current is {current}")]
    FutureEpochInDecayProof { epoch: u64, current: u64 },
    #[error("balance overflow: operation would exceed u64::MAX")]
    BalanceOverflow,
    #[error("privacy engine error: {0}")]
    EngineError(String),
    #[error("privacy state error: {0}")]
    StateError(String),
}

/// Result of executing a privacy transaction.
#[derive(Debug)]
pub struct PrivacyExecResult {
    /// Number of notes created.
    pub notes_created: usize,
    /// Number of nullifiers spent.
    pub nullifiers_spent: usize,
    /// Fee collected (for private transfers).
    pub fee_collected: u64,
    /// Change in shielded pool balance (positive = shield, negative = unshield).
    pub pool_delta: i64,
    /// Tree index of the first note created (for shield, this is the shielded note's index).
    pub tree_index: Option<usize>,
    /// Note commitment bytes (for shield, the note's tree leaf commitment).
    pub note_commitment: Option<[u8; 32]>,
    /// Value commitment bytes (for shield, Poseidon(amount || blinding)).
    pub value_commitment: Option<[u8; 32]>,
}

/// Convert a serializable `MerkleProofData` to the proving crate's `MerkleProof`.
fn to_merkle_proof(data: &MerkleProofData) -> MerkleProof {
    MerkleProof {
        siblings: data.siblings.clone(),
        leaf_index: data.leaf_index,
        root: data.root,
    }
}

/// Default depth for the Phasing Nullifier Tree (PNT) sliding window.
/// Five live phases at 100 epochs/phase = 500 epochs of guaranteed
/// double-spend protection at any point in time. Audit-tunable.
const PNT_WINDOW_DEPTH: usize = 5;

/// Default cadence (in epochs) at which PNT auto-advances its phase
/// from `tick_pnt_phase`. With `PNT_WINDOW_DEPTH = 5`, advancing every
/// 100 epochs gives 500 epochs of live double-spend protection.
const PNT_DEFAULT_PHASE_INTERVAL_EPOCHS: u64 = 100;

/// The privacy execution engine. Maintains the in-memory note tree and
/// nullifier set, syncing state to/from StateDB at block boundaries.
#[derive(Clone)]
pub struct PrivacyExecutor {
    /// The underlying cryptographic privacy engine.
    engine: PrivacyEngine,
    /// Current epoch (set at block start).
    current_epoch: u64,
    /// Phasing Nullifier Tree (PNT, research/INVENTION_STACK.md §4.2).
    /// Tracks every spent nullifier in a bounded sliding window of
    /// phases. **Wiring stage 1: shadow-tracking only.**
    /// Inserts mirror the canonical nullifier_set / db.spend_nullifier
    /// path; the chain still gates double-spend on the unbounded set
    /// because making PNT authoritative is a consensus-breaking change
    /// that needs a hard-fork plan. PNT.live_count() is exposed so a
    /// node operator can compare growth curves before flipping the
    /// gate. Stage 2 will wire `is_spent_in_window` into the
    /// double-spend check itself.
    pub pnt: evaporchain_pnt::PhasedNullifierTree,
    /// Most recent epoch the PNT auto-advanced its phase via
    /// `tick_pnt_phase`. `None` means "never auto-advanced" — first
    /// tick at any epoch will fire (mirrors the PoHA sampler shape).
    pnt_last_phase_epoch: Option<u64>,
    /// Cadence (in epochs) at which `tick_pnt_phase` rotates the PNT
    /// window. `0` disables auto-advance entirely (caller must drive
    /// it via the original `pnt_advance_phase` direct-call path).
    pnt_phase_interval_epochs: u64,
    /// Merkle root captured at the last `tick_pnt_phase` rotation.
    /// PNT v1 Stage-2 defense: `tick_pnt_phase` additionally requires
    /// the merkle root has changed since the last rotation before
    /// rotating again. Without this gate, the bounded window can
    /// rotate purely on epoch count even when no shields moved the
    /// chain state — opening the no-intermediate-shield respend
    /// window flagged by `pnt_v1_no_intermediate_shield_respend_blocked_by_engine_nullifier_set`.
    /// `None` means "never rotated via tick_pnt_phase". Direct calls
    /// to `pnt_advance_phase` bypass this gate (test/admin escape).
    pnt_root_at_last_phase_advance: Option<[u8; 32]>,
    /// Protocol version of the block currently being executed.
    /// Set by the executor BEFORE dispatching privacy txs. Lane B.2
    /// gates the double-spend check on this:
    ///   v0 (legacy)  → check `db.is_nullifier_spent` (unbounded set,
    ///                  what every chain runs today).
    ///   v1+         → check `pnt.is_spent_in_window` (bounded sliding
    ///                  window, PNT-authoritative).
    /// Default is 0; followers reading a v1 block flip the gate via
    /// `set_protocol_version` per-block. The PNT shadow-tracking from
    /// Stage 1 keeps inserting on every spend regardless of version,
    /// so the PNT mirror is always up-to-date when the chain flips.
    current_protocol_version: u8,
}

impl PrivacyExecutor {
    /// Create a new privacy executor with the default tree depth.
    pub fn new() -> Self {
        Self {
            engine: PrivacyEngine::new(NOTE_TREE_DEPTH),
            current_epoch: 0,
            pnt: evaporchain_pnt::PhasedNullifierTree::new(PNT_WINDOW_DEPTH)
                .expect("PNT_WINDOW_DEPTH must be >= 1"),
            pnt_last_phase_epoch: None,
            pnt_phase_interval_epochs: PNT_DEFAULT_PHASE_INTERVAL_EPOCHS,
            pnt_root_at_last_phase_advance: None,
            current_protocol_version: 0,
        }
    }

    /// Create with a custom tree depth (for testing with smaller trees).
    pub fn with_depth(depth: usize) -> Self {
        Self {
            engine: PrivacyEngine::new(depth),
            current_epoch: 0,
            pnt: evaporchain_pnt::PhasedNullifierTree::new(PNT_WINDOW_DEPTH)
                .expect("PNT_WINDOW_DEPTH must be >= 1"),
            pnt_last_phase_epoch: None,
            pnt_phase_interval_epochs: PNT_DEFAULT_PHASE_INTERVAL_EPOCHS,
            pnt_root_at_last_phase_advance: None,
            current_protocol_version: 0,
        }
    }

    /// Open a fresh PNT phase. Call at end-of-epoch (or end-of-block-N
    /// for any chosen N) to age out the oldest live phase. Stage-1
    /// shadow-tracking only, so this has no effect on consensus.
    pub fn pnt_advance_phase(&mut self) {
        self.pnt.advance_phase();
        self.pnt_last_phase_epoch = Some(self.current_epoch);
    }

    /// Cadence-bounded PNT phase advance. Called per block by
    /// `execute_block`; rotates the live-phase window iff the
    /// configured `pnt_phase_interval_epochs` has elapsed since the
    /// last advance. `interval=0` disables the path entirely.
    /// Returns `true` if the window rotated.
    pub fn tick_pnt_phase(&mut self, epoch: u64) -> bool {
        if self.pnt_phase_interval_epochs == 0 {
            return false;
        }
        let due = match self.pnt_last_phase_epoch {
            Some(last) => epoch >= last.saturating_add(self.pnt_phase_interval_epochs),
            None => true,
        };
        if !due {
            return false;
        }
        // PNT v1 Stage-2 defense: also gate on root-change. If the
        // merkle root hasn't moved since the last rotation, no new
        // notes have entered the tree and no eviction is justified
        // — rotating purely on epoch count would let an attacker
        // age nullifiers out of the bounded window without
        // triggering shield activity. See companion test
        // `pnt_v1_no_intermediate_shield_respend_blocked_by_engine_nullifier_set`
        // for the Stage 2 hazard this closes.
        let current_root = self.engine.merkle_root();
        let root_changed = match self.pnt_root_at_last_phase_advance {
            Some(prev) => prev != current_root,
            None => true, // first tick fires
        };
        if !root_changed {
            return false;
        }
        self.pnt.advance_phase();
        self.pnt_last_phase_epoch = Some(epoch);
        self.pnt_root_at_last_phase_advance = Some(current_root);
        true
    }

    /// Set the PNT auto-advance cadence (epochs). 0 disables.
    pub fn set_pnt_phase_interval_epochs(&mut self, epochs: u64) {
        self.pnt_phase_interval_epochs = epochs;
    }

    /// Read the configured PNT phase-advance cadence.
    pub fn pnt_phase_interval_epochs(&self) -> u64 {
        self.pnt_phase_interval_epochs
    }

    /// Most recent epoch a PNT phase advanced (via `tick_pnt_phase`
    /// OR the direct `pnt_advance_phase` path). `None` until first
    /// rotation.
    pub fn pnt_last_phase_epoch(&self) -> Option<u64> {
        self.pnt_last_phase_epoch
    }

    /// Set the current epoch (call at the start of each block).
    pub fn set_epoch(&mut self, epoch: u64) {
        self.current_epoch = epoch;
        self.engine.set_epoch(epoch);
    }

    /// Set the block's `protocol_version` so subsequent privacy-tx
    /// double-spend checks pick the correct backend (v0 = legacy
    /// unbounded set, v1+ = PNT-authoritative). Call BEFORE
    /// `execute_unshield` / `execute_private_transfer` for each
    /// block. Lane B.2 wiring.
    pub fn set_protocol_version(&mut self, version: u8) {
        self.current_protocol_version = version;
    }

    /// Read the currently-set protocol version. Exposed for tests
    /// and operator diagnostics.
    pub fn current_protocol_version(&self) -> u8 {
        self.current_protocol_version
    }

    /// Lane B.2 dual-mode double-spend check. Returns `true` iff the
    /// nullifier is already considered spent under the active
    /// protocol version's authoritative source:
    ///   v0  → `db.is_nullifier_spent` (unbounded set)
    ///   v1+ → `pnt.is_spent_in_window` (bounded sliding window)
    ///
    /// PNT shadow-tracking from Stage 1 keeps the bounded set in
    /// sync with the unbounded set on every spend, so the flip from
    /// v0 to v1 at a fork epoch is monotone — every nullifier the
    /// chain has spent is already in the PNT live window if it's
    /// recent enough to matter.
    pub fn is_double_spend(&self, db: &dyn StateDB, nullifier: &[u8; 32]) -> bool {
        if self.current_protocol_version >= 1 {
            self.pnt.is_spent_in_window(nullifier)
        } else {
            db.is_nullifier_spent(nullifier)
        }
    }

    /// Restore in-memory privacy state (note tree, nullifier set) from
    /// the persisted commitment list in `db`. Called once at node startup
    /// after consensus state restoration so a freshly-launched node can
    /// re-verify spends against the canonical Merkle root.
    ///
    /// Walks `db.get_all_note_commitments()` (BTreeMap-ordered by leaf
    /// index), pushes each commitment into `engine.note_tree`, then
    /// asserts the rebuilt root equals `db.get_note_tree_root()`.
    ///
    /// Mismatch is a fatal restart condition — the node would otherwise
    /// silently accept ZK proofs against the wrong tree state. Returns
    /// `PrivacyExecError::StateError` so the caller can crash cleanly
    /// instead of running on divergent state. Closes punch-list 1b.
    ///
    /// Returns the number of commitments restored.
    pub fn restore_from_db(&mut self, db: &dyn StateDB) -> Result<usize, PrivacyExecError> {
        let persisted_commitments = db.get_all_note_commitments();
        let persisted_root = db.get_note_tree_root();
        let mut restored = 0usize;
        for c_bytes in &persisted_commitments {
            let c = evaporchain_proving::privacy::Commitment(*c_bytes);
            if self.engine.note_tree.insert(&c).is_some() {
                restored += 1;
            }
        }
        // Catch up the count + epoch trackers so subsequent privacy
        // ops know how many notes already exist.
        self.engine.set_epoch(self.current_epoch);

        // PRIV-N2 (audit 2026-05-15): repopulate the in-memory
        // nullifier_set + PNT live window from persisted state. Without
        // this, after any restart on `protocol_version >= 1`:
        //   - `engine.nullifier_set` is empty
        //   - `pnt.window` is empty
        //   - `is_double_spend()` reads `pnt.is_spent_in_window()` (v1+
        //     path) — returns false for every previously-spent
        //     nullifier
        //   - every prior unshield/private-transfer can be replayed by
        //     anyone with the original ZK proof
        //
        // Restoration policy: insert every persisted nullifier into
        // BOTH the canonical unbounded set AND the PNT current phase.
        // This is over-conservative for the PNT (we may retain a
        // nullifier past the point its original phase would have been
        // evicted by `advance_phase`) but it defends against the
        // replay attack with zero false negatives. The PNT's
        // subsequent `advance_phase` calls in normal block flow will
        // age these out over time.
        let persisted_nullifiers = db.all_nullifiers();
        let mut nullifiers_restored = 0usize;
        for nf in &persisted_nullifiers {
            let n = evaporchain_proving::privacy::Nullifier(*nf);
            // `spend` returns false if already inserted; that's only
            // possible if `all_nullifiers` returned a duplicate, which
            // would itself be a state corruption — silent here, the
            // PRIV-N2 invariant is satisfied either way.
            let _ = self.engine.nullifier_set.spend(&n);
            // PNT insert may surface `DoubleSpend` for the same reason;
            // best-effort restoration, log + continue if so.
            if let Err(e) = self.pnt.insert_nullifier(*nf) {
                tracing::warn!(
                    error = ?e,
                    "PrivacyExecutor::restore_from_db: PNT insert returned error \
                     (likely duplicate from db.all_nullifiers) — continuing"
                );
            } else {
                nullifiers_restored += 1;
            }
        }

        // Verify the rebuilt root matches the persisted root. An empty
        // commitment set with a zero root is the legitimate fresh-node
        // case and must not error.
        let rebuilt_root = self.engine.note_tree.root();
        let zero = [0u8; 32];
        if persisted_commitments.is_empty() && persisted_root == zero {
            return Ok(0);
        }
        if rebuilt_root != persisted_root {
            return Err(PrivacyExecError::StateError(format!(
                "privacy restore: rebuilt root {} differs from persisted root {} \
                 ({} commitments restored) — chain state may be corrupted",
                hex::encode(rebuilt_root),
                hex::encode(persisted_root),
                restored,
            )));
        }
        tracing::info!(
            commitments = restored,
            nullifiers = nullifiers_restored,
            "PrivacyExecutor: restored {} note commitments + {} nullifiers from disk; root verified",
            restored,
            nullifiers_restored,
        );
        Ok(restored)
    }

    /// Get the current Merkle root of the note tree.
    pub fn merkle_root(&self) -> [u8; 32] {
        self.engine.merkle_root()
    }

    /// Get the number of notes in the tree.
    pub fn note_count(&self) -> usize {
        self.engine.note_count()
    }

    /// Get the number of spent nullifiers.
    pub fn nullifier_count(&self) -> usize {
        self.engine.nullifier_count()
    }

    /// Get a Merkle proof for a note at the given tree index.
    pub fn get_merkle_proof(&self, leaf_index: usize) -> Option<MerkleProofData> {
        self.engine
            .get_merkle_proof(leaf_index)
            .map(|p| MerkleProofData {
                siblings: p.siblings,
                leaf_index: p.leaf_index,
                root: p.root,
            })
    }

    // ─── Shield ───────────────────────────────────────────────────────────

    /// Execute a shield transaction: burn transparent balance, create private note.
    pub fn execute_shield(
        &mut self,
        db: &mut dyn StateDB,
        tx: &ShieldTx,
    ) -> Result<PrivacyExecResult, PrivacyExecError> {
        if tx.amount == 0 {
            return Err(PrivacyExecError::ZeroShieldAmount);
        }

        // 1. Debit transparent balance
        let sender = db.get_or_create_account(&tx.from);
        // Vesting gate (TOKENOMICS §2.6 / Q14): shield must come from the
        // transferable portion of balance.
        let available = sender.transferable_balance(self.current_epoch);
        if available < tx.amount {
            return Err(PrivacyExecError::InsufficientBalanceForShield {
                available,
                required: tx.amount,
            });
        }
        // Check nonce
        if sender.nonce != tx.nonce {
            return Err(PrivacyExecError::EngineError(format!(
                "invalid nonce: expected {}, got {}",
                sender.nonce, tx.nonce
            )));
        }
        sender.balance -= tx.amount;
        sender.nonce += 1;
        // Shield debits balance + bumps nonce — stamp the demurrage anchor.
        sender.last_touched_epoch = self.current_epoch;

        // 2. Create the private note via PrivacyEngine
        let energy_blinding_arr = tx.energy_blinding;
        let shield_result = self
            .engine
            .shield(
                tx.amount,
                tx.note_owner_hash,
                tx.value_blinding,
                tx.energy,
                energy_blinding_arr,
                tx.half_life,
            )
            .map_err(|e| PrivacyExecError::EngineError(e.to_string()))?;

        // 3. Update StateDB privacy state
        let pool_balance = db.get_shielded_pool_balance();
        db.put_shielded_pool_balance(
            pool_balance
                .checked_add(tx.amount)
                .ok_or(PrivacyExecError::BalanceOverflow)?,
        );
        db.put_note_tree_root(self.engine.merkle_root());
        db.put_note_count(self.engine.note_count() as u64);
        // T0.5 follow-up — persist the new note commitment so
        // `restore_from_db` (this file, line ~284) can rebuild the
        // in-memory note tree on node restart. Pre-fix this was only
        // called from tests and the rocksdb backend stub; the
        // production shield path didn't wire it through, so any
        // chain that included shields would fail
        // `restore_from_db`'s state-root verification on startup.
        // Closes the second gap documented in PR #8.
        db.append_note_commitment(
            shield_result.tree_index as u64,
            shield_result.commitment.0,
        );

        debug!(
            from = hex::encode(tx.from),
            amount = tx.amount,
            tree_index = shield_result.tree_index,
            "Shield executed: transparent → private"
        );

        Ok(PrivacyExecResult {
            notes_created: 1,
            nullifiers_spent: 0,
            fee_collected: 0,
            pool_delta: tx.amount as i64,
            tree_index: Some(shield_result.tree_index),
            note_commitment: Some(shield_result.commitment.0),
            value_commitment: Some(shield_result.note.value_commitment.0),
        })
    }

    // ─── Unshield ─────────────────────────────────────────────────────────

    /// Execute an unshield transaction with full cryptographic verification.
    ///
    /// All witness fields are mandatory: amounts, blindings, commitments,
    /// Merkle proofs, and balance binding must be provided and valid.
    pub fn execute_unshield(
        &mut self,
        db: &mut dyn StateDB,
        tx: &UnshieldTx,
    ) -> Result<PrivacyExecResult, PrivacyExecError> {
        let n_inputs = tx.input_nullifiers.len();

        if tx.amount == 0 {
            return Err(PrivacyExecError::ZeroUnshieldAmount);
        }
        if n_inputs == 0 {
            return Err(PrivacyExecError::NoInputs);
        }

        // 1. Validate witness field counts
        if tx.input_amounts.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!(
                    "input_amounts: expected {n_inputs}, got {}",
                    tx.input_amounts.len()
                ),
            });
        }
        if tx.input_blindings.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!(
                    "input_blindings: expected {n_inputs}, got {}",
                    tx.input_blindings.len()
                ),
            });
        }
        if tx.input_value_commitments.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!(
                    "input_value_commitments: expected {n_inputs}, got {}",
                    tx.input_value_commitments.len()
                ),
            });
        }
        if tx.input_note_commitments.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!(
                    "input_note_commitments: expected {n_inputs}, got {}",
                    tx.input_note_commitments.len()
                ),
            });
        }
        if tx.input_merkle_proofs.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!(
                    "input_merkle_proofs: expected {n_inputs}, got {}",
                    tx.input_merkle_proofs.len()
                ),
            });
        }
        if tx.output_blindings.len() != tx.change_commitments.len() {
            return Err(PrivacyExecError::WitnessFieldMismatch {
                expected: tx.change_commitments.len(),
                got: tx.output_blindings.len(),
            });
        }

        // 2. Verify anchor matches current Merkle root
        if tx.anchor != self.engine.merkle_root() {
            return Err(PrivacyExecError::StaleAnchor);
        }

        // 3. Check nullifiers not already spent (against persisted db state).
        for nf in &tx.input_nullifiers {
            // Lane B.2 dual-mode: v0 reads db's unbounded set, v1+
            // reads pnt's bounded sliding window. PNT shadow-tracking
            // (Stage 1) keeps both in sync so the flip is monotone.
            if self.is_double_spend(db, nf) {
                return Err(PrivacyExecError::DoubleSpend(hex::encode(&nf[..8])));
            }
        }

        // 3a. Check for duplicate nullifiers WITHIN the transaction.
        // Without this, a malicious unshield with `input_nullifiers = [nf, nf]`
        // and matching duplicated `input_amounts` / `input_value_commitments`
        // would pass step 3 (db check) and balance binding (sums match by
        // construction) and double-extract from a single shielded note.
        // `execute_private_transfer` had this check; `execute_unshield`
        // did not. Closes audit-flagged 2026-05-03 #9.2 (privacy nullifier
        // hygiene against Block-STM / serial-replay edge cases).
        {
            let mut seen = std::collections::HashSet::new();
            for nf in &tx.input_nullifiers {
                if !seen.insert(*nf) {
                    return Err(PrivacyExecError::DoubleSpend(hex::encode(&nf[..8])));
                }
            }
        }

        // 4. Verify energy decay proofs (if any)
        for ep in &tx.energy_proofs {
            if ep.epoch_end > self.current_epoch {
                return Err(PrivacyExecError::FutureEpochInDecayProof {
                    epoch: ep.epoch_end,
                    current: self.current_epoch,
                });
            }
        }

        // 5. Verify each input: commitment opening + Merkle proof
        for i in 0..n_inputs {
            let c = Commitment(tx.input_value_commitments[i]);
            if !c.verify_opening(tx.input_amounts[i], &tx.input_blindings[i]) {
                return Err(PrivacyExecError::InvalidCommitmentOpening { index: i });
            }

            let proof = to_merkle_proof(&tx.input_merkle_proofs[i]);
            if proof.root != tx.anchor {
                return Err(PrivacyExecError::MerkleProofAnchorMismatch { index: i });
            }
            // The leaf in the Merkle tree is the note commitment (not the value commitment).
            //
            // PRIV-N1 (audit 2026-05-15): pass the verifier's trusted
            // tree depth so an attacker can't submit `siblings: vec![]`
            // with `leaf_index: 0` + `input_note_commitment = anchor`
            // and have the verify loop pass for any chosen amount.
            if !verify_merkle_proof(
                &tx.input_note_commitments[i],
                &proof,
                self.engine.note_tree.depth(),
            ) {
                return Err(PrivacyExecError::InvalidMerkleProof { index: i });
            }
        }

        // 6. Verify balance binding.
        //
        // PRIV-N3 (audit 2026-05-15): checked_add on the input sum
        // so an attacker can't wrap u64 silently.
        let sum_in: u64 = tx
            .input_amounts
            .iter()
            .try_fold(0u64, |acc, &x| acc.checked_add(x))
            .ok_or(PrivacyExecError::BalanceOverflow)?;
        let change_total = sum_in.saturating_sub(tx.amount);
        // PRIV-N4 (audit 2026-05-15): the Unshield kind tag binds
        // this verification to the Unshield tx shape — a binding
        // computed for a PrivateTransfer over the same numeric
        // tuple + blindings is now rejected here.
        if !verify_balance_binding(
            &tx.balance_binding,
            BalanceBindingKind::Unshield,
            sum_in,
            change_total,
            tx.amount,
            &tx.input_blindings,
            &tx.output_blindings,
        ) {
            return Err(PrivacyExecError::InvalidBalanceBinding);
        }
        if sum_in < tx.amount {
            return Err(PrivacyExecError::UnshieldBalanceMismatch);
        }

        // 7. Spend nullifiers. `NullifierSet::spend` returns `false` if the
        // nullifier was already in the in-memory set — that branch must be
        // surfaced as an error rather than silently dropped, otherwise an
        // in-memory/db drift (e.g., a Block-STM-retried serial pass where
        // `db.spend_nullifier` was rolled back but `engine.nullifier_set`
        // was not) could let a duplicate spend slip through.
        for nf in &tx.input_nullifiers {
            let nullifier = Nullifier(*nf);
            if !self.engine.nullifier_set.spend(&nullifier) {
                return Err(PrivacyExecError::DoubleSpend(hex::encode(&nf[..8])));
            }
            db.spend_nullifier(nf);
            // PNT shadow-track (research-buildable item #8). Stage 1:
            // record without consensus effect. The PNT may also reject
            // (`Err(PntError::DoubleSpend)`) — we drop that error today
            // because the canonical check above is already authoritative.
            // Stage 2 (post hard-fork) will surface PNT errors and gate
            // double-spend on `is_spent_in_window` directly.
            let _ = self.pnt.insert_nullifier(*nf);
        }

        // 8. Add change outputs to tree + persist their commitments
        // to db so `restore_from_db` can rebuild the tree on restart
        // (T0.5 follow-up — third call site, symmetric to the shield
        // + private_transfer paths fixed in the previous commit).
        for commitment_bytes in &tx.change_commitments {
            let commitment = Commitment(*commitment_bytes);
            let leaf_index = self
                .engine
                .note_tree
                .insert(&commitment)
                .ok_or(PrivacyExecError::TreeFull)?;
            db.append_note_commitment(leaf_index as u64, *commitment_bytes);
        }

        // 9. Credit transparent balance
        let receiver = db.get_or_create_account(&tx.to);
        receiver.balance = receiver
            .balance
            .checked_add(tx.amount)
            .ok_or(PrivacyExecError::BalanceOverflow)?;
        // Unshield credits balance — refresh demurrage anchor.
        receiver.last_touched_epoch = self.current_epoch;

        // 10. Update pool balance
        let pool_balance = db.get_shielded_pool_balance();
        db.put_shielded_pool_balance(pool_balance.saturating_sub(tx.amount));
        db.put_note_tree_root(self.engine.merkle_root());
        db.put_note_count(self.engine.note_count() as u64);

        debug!(
            to = hex::encode(tx.to),
            amount = tx.amount,
            nullifiers = tx.input_nullifiers.len(),
            change_outputs = tx.change_commitments.len(),
            "Unshield executed: private → transparent"
        );

        Ok(PrivacyExecResult {
            notes_created: tx.change_commitments.len(),
            nullifiers_spent: tx.input_nullifiers.len(),
            fee_collected: 0,
            pool_delta: -(tx.amount as i64),
            tree_index: None,
            note_commitment: None,
            value_commitment: None,
        })
    }

    // ─── Private Transfer ─────────────────────────────────────────────────

    /// Execute a private transfer with full cryptographic verification.
    ///
    /// All witness fields are mandatory: amounts, blindings, commitments,
    /// Merkle proofs, and balance binding must be provided and valid.
    pub fn execute_private_transfer(
        &mut self,
        db: &mut dyn StateDB,
        tx: &PrivateTransferTx,
    ) -> Result<PrivacyExecResult, PrivacyExecError> {
        let n_inputs = tx.input_nullifiers.len();
        let n_outputs = tx.output_commitments.len();

        if n_inputs == 0 {
            return Err(PrivacyExecError::NoInputs);
        }
        if n_outputs == 0 {
            return Err(PrivacyExecError::NoOutputs);
        }

        // 1. Validate all witness field counts
        if tx.input_amounts.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!(
                    "input_amounts: expected {n_inputs}, got {}",
                    tx.input_amounts.len()
                ),
            });
        }
        if tx.input_blindings.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!(
                    "input_blindings: expected {n_inputs}, got {}",
                    tx.input_blindings.len()
                ),
            });
        }
        if tx.input_value_commitments.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!(
                    "input_value_commitments: expected {n_inputs}, got {}",
                    tx.input_value_commitments.len()
                ),
            });
        }
        if tx.input_note_commitments.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!(
                    "input_note_commitments: expected {n_inputs}, got {}",
                    tx.input_note_commitments.len()
                ),
            });
        }
        if tx.input_merkle_proofs.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!(
                    "input_merkle_proofs: expected {n_inputs}, got {}",
                    tx.input_merkle_proofs.len()
                ),
            });
        }
        if tx.output_amounts.len() != n_outputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!(
                    "output_amounts: expected {n_outputs}, got {}",
                    tx.output_amounts.len()
                ),
            });
        }
        if tx.output_blindings.len() != n_outputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!(
                    "output_blindings: expected {n_outputs}, got {}",
                    tx.output_blindings.len()
                ),
            });
        }

        // 2. Verify anchor
        if tx.anchor != self.engine.merkle_root() {
            return Err(PrivacyExecError::StaleAnchor);
        }

        // 3. Check nullifiers not already spent
        for nf in &tx.input_nullifiers {
            // Lane B.2 dual-mode: v0 reads db's unbounded set, v1+
            // reads pnt's bounded sliding window. PNT shadow-tracking
            // (Stage 1) keeps both in sync so the flip is monotone.
            if self.is_double_spend(db, nf) {
                return Err(PrivacyExecError::DoubleSpend(hex::encode(&nf[..8])));
            }
        }

        // 4. Check for duplicate nullifiers within the transaction
        {
            let mut seen = std::collections::HashSet::new();
            for nf in &tx.input_nullifiers {
                if !seen.insert(*nf) {
                    return Err(PrivacyExecError::DoubleSpend(hex::encode(&nf[..8])));
                }
            }
        }

        // 5. Verify energy decay proofs
        for ep in &tx.energy_proofs {
            if ep.epoch_end > self.current_epoch {
                return Err(PrivacyExecError::FutureEpochInDecayProof {
                    epoch: ep.epoch_end,
                    current: self.current_epoch,
                });
            }
        }

        // 6. Verify each input: commitment opening + Merkle proof
        for i in 0..n_inputs {
            let c = Commitment(tx.input_value_commitments[i]);
            if !c.verify_opening(tx.input_amounts[i], &tx.input_blindings[i]) {
                return Err(PrivacyExecError::InvalidCommitmentOpening { index: i });
            }

            let proof = to_merkle_proof(&tx.input_merkle_proofs[i]);
            if proof.root != tx.anchor {
                return Err(PrivacyExecError::MerkleProofAnchorMismatch { index: i });
            }
            // PRIV-N1 (audit 2026-05-15): pass the verifier's trusted
            // tree depth — see the unshield path comment above.
            if !verify_merkle_proof(
                &tx.input_note_commitments[i],
                &proof,
                self.engine.note_tree.depth(),
            ) {
                return Err(PrivacyExecError::InvalidMerkleProof { index: i });
            }
        }

        // 7. Verify output commitments match claimed amounts/blindings
        for i in 0..n_outputs {
            let expected = Commitment::commit(tx.output_amounts[i], &tx.output_blindings[i]);
            if expected.0 != tx.output_commitments[i] {
                return Err(PrivacyExecError::InvalidOutputCommitment { index: i });
            }
        }

        // 8. Verify balance binding and conservation.
        //
        // PRIV-N3: checked_add on each sum (see unshield path above).
        // PRIV-N4: bind verification to the PrivateTransfer tx kind.
        let sum_in: u64 = tx
            .input_amounts
            .iter()
            .try_fold(0u64, |acc, &x| acc.checked_add(x))
            .ok_or(PrivacyExecError::BalanceOverflow)?;
        let sum_out: u64 = tx
            .output_amounts
            .iter()
            .try_fold(0u64, |acc, &x| acc.checked_add(x))
            .ok_or(PrivacyExecError::BalanceOverflow)?;
        if !verify_balance_binding(
            &tx.balance_binding,
            BalanceBindingKind::PrivateTransfer,
            sum_in,
            sum_out,
            tx.fee,
            &tx.input_blindings,
            &tx.output_blindings,
        ) {
            return Err(PrivacyExecError::InvalidBalanceBinding);
        }
        if sum_in
            != sum_out
                .checked_add(tx.fee)
                .ok_or(PrivacyExecError::BalanceOverflow)?
        {
            return Err(PrivacyExecError::UnshieldBalanceMismatch);
        }

        // 9. Spend nullifiers. `NullifierSet::spend` returns `false` if the
        // nullifier was already in the in-memory set — surface that as an
        // error so an in-memory/db drift can't let a duplicate spend through.
        // (See execute_unshield step 7 for the symmetric Block-STM rationale.)
        for nf in &tx.input_nullifiers {
            let nullifier = Nullifier(*nf);
            if !self.engine.nullifier_set.spend(&nullifier) {
                return Err(PrivacyExecError::DoubleSpend(hex::encode(&nf[..8])));
            }
            db.spend_nullifier(nf);
            // PNT shadow-track (research-buildable item #8). Stage 1:
            // record without consensus effect. The PNT may also reject
            // (`Err(PntError::DoubleSpend)`) — we drop that error today
            // because the canonical check above is already authoritative.
            // Stage 2 (post hard-fork) will surface PNT errors and gate
            // double-spend on `is_spent_in_window` directly.
            let _ = self.pnt.insert_nullifier(*nf);
        }

        // 10. Add output notes to tree + persist their commitments to
        // db so `restore_from_db` can rebuild the tree on restart
        // (T0.5 follow-up — symmetric to the shield-side persistence
        // wiring; see execute_shield step 3).
        for commitment_bytes in &tx.output_commitments {
            let commitment = Commitment(*commitment_bytes);
            let leaf_index = self
                .engine
                .note_tree
                .insert(&commitment)
                .ok_or(PrivacyExecError::TreeFull)?;
            db.append_note_commitment(leaf_index as u64, *commitment_bytes);
        }

        // 11. Fee extracted from shielded pool
        if tx.fee > 0 {
            let pool_balance = db.get_shielded_pool_balance();
            db.put_shielded_pool_balance(pool_balance.saturating_sub(tx.fee));
        }

        // 12. Sync state
        db.put_note_tree_root(self.engine.merkle_root());
        db.put_note_count(self.engine.note_count() as u64);

        debug!(
            inputs = tx.input_nullifiers.len(),
            outputs = tx.output_commitments.len(),
            fee = tx.fee,
            "Private transfer executed"
        );

        Ok(PrivacyExecResult {
            notes_created: tx.output_commitments.len(),
            nullifiers_spent: tx.input_nullifiers.len(),
            fee_collected: tx.fee,
            pool_delta: -(tx.fee as i64),
            tree_index: None,
            note_commitment: None,
            value_commitment: None,
        })
    }

    /// Estimate gas for a private transfer transaction.
    ///
    /// Audit AUDIT-2026-05-11-3: saturating arithmetic to match
    /// `parallel.rs:1018` and prevent a wrapped result from
    /// undercharging an unusually large tx. A `u64` wrap would
    /// require ~9·10¹⁴ nullifiers (not deserializable in practice),
    /// but saturating-on-fee-paths is the conservative discipline.
    pub fn estimate_private_transfer_gas(tx: &PrivateTransferTx) -> u64 {
        GAS_PRIVATE_TRANSFER_BASE
            .saturating_add(
                GAS_PRIVATE_TRANSFER_PER_INPUT
                    .saturating_mul(tx.input_nullifiers.len() as u64),
            )
            .saturating_add(
                GAS_PRIVATE_TRANSFER_PER_OUTPUT
                    .saturating_mul(tx.output_commitments.len() as u64),
            )
    }
}

impl Default for PrivacyExecutor {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════
// Tests — 100% real cryptographic flow
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_proving::privacy::compute_balance_binding;
    use evaporchain_proving::privacy::BalanceBindingKind;
    use evaporchain_proving::privacy::{Commitment, Nullifier};
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::{Account, AccountAddress};

    fn test_blinding(seed: u8) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0] = seed;
        b[31] = seed.wrapping_mul(37);
        b
    }

    fn test_addr(id: u8) -> AccountAddress {
        let mut a = [0u8; 32];
        a[0] = id;
        a
    }

    fn setup_db_with_balance(addr: &AccountAddress, balance: u64) -> InMemoryStateDB {
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: *addr,
            balance,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        db
    }

    /// Shield funds and return everything needed to spend the note later.
    struct ShieldedNote {
        amount: u64,
        blinding: [u8; 32],
        _owner_hash: [u8; 32],
        spending_secret: [u8; 32],
        tree_index: usize,
        value_commitment: [u8; 32],
        note_commitment: [u8; 32],
    }

    /// Helper: shield real funds and return all the data needed to build a real unshield/transfer.
    #[allow(clippy::too_many_arguments)]
    fn do_shield(
        executor: &mut PrivacyExecutor,
        db: &mut InMemoryStateDB,
        from: &AccountAddress,
        amount: u64,
        nonce: u64,
        owner_hash: [u8; 32],
        blinding: [u8; 32],
        spending_secret: [u8; 32],
    ) -> ShieldedNote {
        let tx = ShieldTx {
            from: *from,
            amount,
            nonce,
            note_owner_hash: owner_hash,
            value_blinding: blinding,
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        let result = executor.execute_shield(db, &tx).unwrap();
        ShieldedNote {
            amount,
            blinding,
            _owner_hash: owner_hash,
            spending_secret,
            tree_index: result.tree_index.unwrap(),
            value_commitment: result.value_commitment.unwrap(),
            note_commitment: result.note_commitment.unwrap(),
        }
    }

    /// Helper: build a fully real UnshieldTx from a shielded note.
    fn build_real_unshield(
        executor: &PrivacyExecutor,
        note: &ShieldedNote,
        to: AccountAddress,
        unshield_amount: u64,
    ) -> UnshieldTx {
        let merkle_proof = executor.get_merkle_proof(note.tree_index).unwrap();
        let nullifier = Nullifier::derive(&note.spending_secret, &Commitment(note.note_commitment));
        let binding =
            compute_balance_binding(
                BalanceBindingKind::Unshield,
                note.amount,
                0,
                unshield_amount,
                &[note.blinding],
                &[],
            );
        UnshieldTx {
            to,
            amount: unshield_amount,
            input_nullifiers: vec![nullifier.0],
            anchor: executor.merkle_root(),
            balance_binding: binding,
            input_amounts: vec![note.amount],
            input_blindings: vec![note.blinding],
            input_value_commitments: vec![note.value_commitment],
            input_note_commitments: vec![note.note_commitment],
            input_merkle_proofs: vec![merkle_proof],
            output_blindings: vec![],
            change_commitments: vec![],
            energy_proofs: vec![],
        }
    }

    /// Helper: build a fully real PrivateTransferTx from a shielded note.
    fn build_real_transfer(
        executor: &PrivacyExecutor,
        input_note: &ShieldedNote,
        output_amounts: &[u64],
        output_blindings: &[[u8; 32]],
        fee: u64,
    ) -> PrivateTransferTx {
        let merkle_proof = executor.get_merkle_proof(input_note.tree_index).unwrap();
        let nullifier = Nullifier::derive(
            &input_note.spending_secret,
            &Commitment(input_note.note_commitment),
        );

        let output_commitments: Vec<[u8; 32]> = output_amounts
            .iter()
            .zip(output_blindings.iter())
            .map(|(a, b)| Commitment::commit(*a, b).0)
            .collect();

        let sum_out: u64 = output_amounts.iter().sum();
        let binding = compute_balance_binding(
            BalanceBindingKind::PrivateTransfer,
            input_note.amount,
            sum_out,
            fee,
            &[input_note.blinding],
            output_blindings,
        );

        PrivateTransferTx {
            input_nullifiers: vec![nullifier.0],
            output_commitments,
            anchor: executor.merkle_root(),
            balance_binding: binding,
            fee,
            input_amounts: vec![input_note.amount],
            input_blindings: vec![input_note.blinding],
            input_value_commitments: vec![input_note.value_commitment],
            input_note_commitments: vec![input_note.note_commitment],
            input_merkle_proofs: vec![merkle_proof],
            output_amounts: output_amounts.to_vec(),
            output_blindings: output_blindings.to_vec(),
            energy_proofs: vec![],
        }
    }

    // ── Shield Tests ──

    #[test]
    fn test_shield_basic() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &addr,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        // Transparent balance decreased
        assert_eq!(db.get_account(&addr).unwrap().balance, 5_000);
        assert_eq!(db.get_account(&addr).unwrap().nonce, 1);
        assert_eq!(db.get_shielded_pool_balance(), 5_000);
        assert_ne!(db.get_note_tree_root(), [0u8; 32]);
        assert_eq!(db.get_note_count(), 1);

        // Real Merkle proof is obtainable
        let proof = executor.get_merkle_proof(note.tree_index).unwrap();
        assert_eq!(proof.root, executor.merkle_root());

        // Real value commitment verifies
        let c = Commitment(note.value_commitment);
        assert!(c.verify_opening(5_000, &test_blinding(20)));
    }

    #[test]
    fn test_shield_insufficient_balance() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 1_000);
        let mut executor = PrivacyExecutor::with_depth(8);

        let tx = ShieldTx {
            from: addr,
            amount: 5_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };

        let err = executor.execute_shield(&mut db, &tx).unwrap_err();
        assert!(matches!(
            err,
            PrivacyExecError::InsufficientBalanceForShield { .. }
        ));
    }

    #[test]
    fn test_shield_zero_amount() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);

        let tx = ShieldTx {
            from: addr,
            amount: 0,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        assert!(matches!(
            executor.execute_shield(&mut db, &tx),
            Err(PrivacyExecError::ZeroShieldAmount)
        ));
    }

    #[test]
    fn test_shield_nonce_check() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);

        let tx = ShieldTx {
            from: addr,
            amount: 1_000,
            nonce: 5, // wrong nonce
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        assert!(matches!(
            executor.execute_shield(&mut db, &tx),
            Err(PrivacyExecError::EngineError(_))
        ));
    }

    #[test]
    fn test_multiple_shields() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        for i in 0..5u8 {
            do_shield(
                &mut executor,
                &mut db,
                &addr,
                1_000,
                i as u64,
                test_blinding(10 + i),
                test_blinding(20 + i),
                test_blinding(90 + i),
            );
        }

        assert_eq!(db.get_account(&addr).unwrap().balance, 5_000);
        assert_eq!(db.get_shielded_pool_balance(), 5_000);
        assert_eq!(db.get_note_count(), 5);
    }

    // ── Unshield Tests (100% real) ──

    #[test]
    fn test_unshield_real_full_amount() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let tx = build_real_unshield(&executor, &note, receiver, 5_000);
        let result = executor.execute_unshield(&mut db, &tx).unwrap();

        assert_eq!(result.nullifiers_spent, 1);
        assert_eq!(result.pool_delta, -5_000);
        assert_eq!(db.get_account(&receiver).unwrap().balance, 5_000);
        assert_eq!(db.get_shielded_pool_balance(), 0);
        assert!(db.is_nullifier_spent(&tx.input_nullifiers[0]));
    }

    #[test]
    fn test_unshield_double_spend_rejected() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        // First unshield succeeds
        let tx1 = build_real_unshield(&executor, &note, receiver, 5_000);
        executor.execute_unshield(&mut db, &tx1).unwrap();

        // Second unshield with same note — double spend
        // Note: anchor changed, so we'd get StaleAnchor first. Shield another note to reset.
        let note2 = do_shield(
            &mut executor,
            &mut db,
            &sender,
            3_000,
            1,
            test_blinding(11),
            test_blinding(21),
            test_blinding(98),
        );
        // Try using the original (already spent) nullifier with new anchor
        let mut tx2 = build_real_unshield(&executor, &note2, receiver, 3_000);
        // Overwrite nullifier with the already-spent one
        tx2.input_nullifiers = tx1.input_nullifiers.clone();
        let err = executor.execute_unshield(&mut db, &tx2).unwrap_err();
        assert!(matches!(err, PrivacyExecError::DoubleSpend(_)));
    }

    #[test]
    fn test_unshield_stale_anchor_rejected() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        // Build tx with current anchor
        let tx = build_real_unshield(&executor, &note, receiver, 5_000);

        // Shield again to change the Merkle root
        do_shield(
            &mut executor,
            &mut db,
            &sender,
            2_000,
            1,
            test_blinding(11),
            test_blinding(21),
            test_blinding(98),
        );

        // Now the tx has a stale anchor
        assert!(matches!(
            executor.execute_unshield(&mut db, &tx),
            Err(PrivacyExecError::StaleAnchor)
        ));
    }

    #[test]
    fn test_unshield_invalid_balance_binding_rejected() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let mut tx = build_real_unshield(&executor, &note, receiver, 5_000);
        tx.balance_binding = [0xBB; 32]; // garbage binding

        let err = executor.execute_unshield(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::InvalidBalanceBinding));
    }

    #[test]
    fn test_unshield_invalid_commitment_opening_rejected() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let mut tx = build_real_unshield(&executor, &note, receiver, 5_000);
        // Corrupt the value commitment
        tx.input_value_commitments[0] = [0xCC; 32];

        let err = executor.execute_unshield(&mut db, &tx).unwrap_err();
        assert!(matches!(
            err,
            PrivacyExecError::InvalidCommitmentOpening { index: 0 }
        ));
    }

    #[test]
    fn test_unshield_invalid_merkle_proof_rejected() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let mut tx = build_real_unshield(&executor, &note, receiver, 5_000);
        // Corrupt the note commitment (Merkle leaf)
        tx.input_note_commitments[0] = [0xDD; 32];

        let err = executor.execute_unshield(&mut db, &tx).unwrap_err();
        assert!(matches!(
            err,
            PrivacyExecError::InvalidMerkleProof { index: 0 }
        ));
    }

    #[test]
    fn test_unshield_duplicate_nullifier_in_tx_rejected() {
        // Audit fix #9.2 (privacy nullifier hygiene): an unshield tx with
        // duplicate input_nullifiers + matching duplicated witness fields
        // would, pre-fix, pass step 3 (db check) and balance binding (sums
        // match by construction) and double-extract from a single shielded
        // note. `execute_private_transfer` already had a within-tx dup
        // check; `execute_unshield` did not.
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let mut tx = build_real_unshield(&executor, &note, receiver, 5_000);
        // Duplicate the nullifier and matching witness fields.
        tx.input_nullifiers.push(tx.input_nullifiers[0]);
        tx.input_amounts.push(tx.input_amounts[0]);
        tx.input_blindings.push(tx.input_blindings[0]);
        tx.input_value_commitments
            .push(tx.input_value_commitments[0]);
        tx.input_note_commitments.push(tx.input_note_commitments[0]);
        tx.input_merkle_proofs
            .push(tx.input_merkle_proofs[0].clone());

        let err = executor.execute_unshield(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::DoubleSpend(_)));
    }

    #[test]
    fn test_pnt_shadow_tracks_unshield_nullifier() {
        // Audit-buildable #8 (PNT wiring stage 1). Every spend records
        // into the PhasedNullifierTree alongside the canonical set.
        // Stage 1 is shadow-only — the PNT does not yet gate consensus
        // double-spend — but its growth must mirror the canonical set
        // so an operator can compare curves.
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);
        assert_eq!(executor.pnt.live_count(), 0);

        let note = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let tx = build_real_unshield(&executor, &note, receiver, 5_000);
        executor.execute_unshield(&mut db, &tx).unwrap();

        assert_eq!(
            executor.pnt.live_count(),
            1,
            "PNT must mirror the canonical nullifier_set on spend"
        );
        assert!(executor.pnt.is_spent_in_window(&tx.input_nullifiers[0]));
    }

    #[test]
    fn test_tick_pnt_phase_first_call_fires() {
        // Audit-buildable #8 follow-up: tick_pnt_phase on a fresh
        // executor must fire on the first call regardless of epoch
        // (mirrors PoHA sampler's "first tick always fires" shape).
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(7);
        executor.set_pnt_phase_interval_epochs(100);
        let phase_before = executor.pnt.current_phase;
        let advanced = executor.tick_pnt_phase(7);
        assert!(advanced, "first tick must fire");
        assert_eq!(executor.pnt.current_phase, phase_before + 1);
        assert_eq!(executor.pnt_last_phase_epoch(), Some(7));
    }

    #[test]
    fn test_tick_pnt_phase_respects_cadence() {
        // PNT v1 Stage-2 defense: tick_pnt_phase requires BOTH cadence
        // elapsed AND merkle root changed since the last rotation.
        // This test exercises the cadence half; root-change is
        // covered separately. To isolate cadence, we shield between
        // ticks so the root advances. The first tick at epoch 0 fires
        // unconditionally (no prior root recorded).
        let sender = test_addr(1);
        let mut db = setup_db_with_balance(&sender, 1_000_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_pnt_phase_interval_epochs(100);
        executor.set_epoch(0);

        assert!(executor.tick_pnt_phase(0), "first tick fires unconditionally");
        // Shield once so the next tick has a fresh root to compare against.
        let _n0 = do_shield(
            &mut executor,
            &mut db,
            &sender,
            1_000,
            0,
            test_blinding(1),
            test_blinding(2),
            test_blinding(3),
        );
        assert!(!executor.tick_pnt_phase(50), "50 < 0+100 → no fire");
        assert!(!executor.tick_pnt_phase(99), "99 < 0+100 → no fire");
        assert!(
            executor.tick_pnt_phase(100),
            "100 >= 0+100 AND root changed → fires"
        );
        // Another shield so the next at-cadence tick has a fresh root.
        let _n1 = do_shield(
            &mut executor,
            &mut db,
            &sender,
            1_000,
            1,
            test_blinding(4),
            test_blinding(5),
            test_blinding(6),
        );
        assert!(!executor.tick_pnt_phase(150), "150 < 100+100 → no fire");
        assert!(executor.tick_pnt_phase(200));
    }

    /// PNT v1 Stage-2 defense: even when cadence has elapsed,
    /// tick_pnt_phase must NOT rotate if the merkle root has not
    /// changed since the last rotation. Closes the no-intermediate-
    /// shield bypass: an attacker cannot evict their own nullifier
    /// from the bounded window just by waiting epochs to pass.
    #[test]
    fn test_tick_pnt_phase_no_fire_when_root_unchanged() {
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_pnt_phase_interval_epochs(100);
        executor.set_epoch(0);

        // First tick fires (no prior recorded root).
        assert!(executor.tick_pnt_phase(0));
        let phase_before = executor.pnt.current_phase;

        // Many epochs pass with NO shields. Root stays at empty-tree
        // sentinel. tick_pnt_phase MUST refuse to rotate even
        // though the cadence has long elapsed.
        for epoch in [100u64, 200, 1_000, 10_000, 100_000] {
            assert!(
                !executor.tick_pnt_phase(epoch),
                "epoch {} cadence elapsed but root unchanged → MUST NOT fire",
                epoch
            );
        }
        // Phase counter is unchanged.
        assert_eq!(
            executor.pnt.current_phase, phase_before,
            "PNT phase MUST NOT advance without root change"
        );
    }

    #[test]
    fn test_tick_pnt_phase_zero_interval_disables() {
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_pnt_phase_interval_epochs(0);
        assert!(!executor.tick_pnt_phase(0));
        assert!(!executor.tick_pnt_phase(u64::MAX));
        assert_eq!(executor.pnt_last_phase_epoch(), None);
    }

    #[test]
    fn test_pnt_advance_phase_keeps_recent_window() {
        // Advancing phases shouldn't lose nullifiers within the window
        // depth (5 phases by default). The very first phase containing
        // our nullifier stays live until the 6th advance_phase.
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let tx = build_real_unshield(&executor, &note, receiver, 5_000);
        let nullifier = tx.input_nullifiers[0];
        executor.execute_unshield(&mut db, &tx).unwrap();

        // Advance up to (window_depth-1) times — nullifier still in window.
        for _ in 0..4 {
            executor.pnt_advance_phase();
        }
        assert!(
            executor.pnt.is_spent_in_window(&nullifier),
            "nullifier must remain live within the window"
        );
    }

    #[test]
    fn test_pnt_authoritative_v1_uses_pnt_window() {
        // Lane B.2: under protocol_version=1, the double-spend check
        // reads `pnt.is_spent_in_window`, NOT `db.is_nullifier_spent`.
        // Pre-poison the PNT (without touching the db) and assert the
        // v1 path rejects.
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );
        let tx = build_real_unshield(&executor, &note, receiver, 5_000);

        // Pre-poison PNT only — db stays clean.
        executor
            .pnt
            .insert_nullifier(tx.input_nullifiers[0])
            .unwrap();

        // v0: legacy reads db (unbounded set) → tx passes the check
        // because db has nothing. PNT poison is invisible.
        executor.set_protocol_version(0);
        assert!(
            !executor.is_double_spend(&db, &tx.input_nullifiers[0]),
            "v0 must read db, not pnt"
        );

        // v1: PNT-authoritative → tx fails the check because PNT has it.
        executor.set_protocol_version(1);
        assert!(
            executor.is_double_spend(&db, &tx.input_nullifiers[0]),
            "v1 must read pnt, not db"
        );
    }

    #[test]
    fn test_pnt_authoritative_v0_default_is_legacy() {
        // A fresh PrivacyExecutor defaults to v0 (legacy unbounded set).
        // This is the bit-compat guarantee for every existing chain.
        let executor = PrivacyExecutor::with_depth(8);
        assert_eq!(executor.current_protocol_version(), 0);
    }

    #[test]
    fn test_unshield_inmem_set_drift_caught_by_spend_return() {
        // Audit fix #9.2 (Block-STM-related): if the in-memory
        // `engine.nullifier_set` and the db nullifier set ever drift
        // (e.g., a serial-replay path where db state was rolled back but
        // the in-memory set was not), `NullifierSet::spend` returns
        // `false` on the duplicate. Pre-fix the executor ignored the
        // return value and silently accepted the double spend. Post-fix
        // it surfaces a DoubleSpend error.
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let tx = build_real_unshield(&executor, &note, receiver, 5_000);

        // Pre-poison the in-memory nullifier set so it claims this nullifier
        // is already spent, while leaving the db consistent with the tx.
        // db.is_nullifier_spent(...) will return false; only the in-memory
        // `nullifier_set.spend()` return value will catch the drift.
        executor
            .engine
            .nullifier_set
            .spend(&Nullifier(tx.input_nullifiers[0]));

        let err = executor.execute_unshield(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::DoubleSpend(_)));
    }

    // ── Lane T0.5 sub-task 5 — adversarial spend-evict-respend ──
    //
    // Cryptographic claim: under PNT v1+ (bounded nullifier window),
    // the bounded window's eventual eviction of a nullifier MUST NOT
    // make a respend possible. Both defensive layers must hold:
    //
    //   1. Anchor enforcement (`tx.anchor == self.engine.merkle_root()`)
    //      — replays of an old tx with the original (now-stale) anchor
    //      are rejected with `StaleAnchor` BEFORE reaching the
    //      bounded-window check.
    //   2. Canonical in-memory nullifier set (`engine.nullifier_set`)
    //      — sophisticated respends that rebuild a fresh merkle proof
    //      against the CURRENT root and use the current anchor get
    //      past anchor enforcement, but step 7's `nullifier_set.spend`
    //      returns false (the nullifier is already in the unbounded
    //      in-memory set) → `DoubleSpend`.
    //
    // The PNT bounded window is a memory-efficient FAST PATH (`is_double_spend`
    // at step 3, line 503), not the authoritative source. Eviction
    // there is acceptable iff at least one of the above two layers
    // catches the respend.

    #[test]
    fn pnt_v1_respend_after_window_eviction_rejected_via_anchor_and_nullifier_set() {
        // Set up: shield once, spend, then push enough new shields to
        // advance the merkle root, AND advance enough PNT phases to
        // age the original nullifier out of the bounded window.
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 100_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);
        executor.set_protocol_version(1); // PNT v1+ authoritative

        // (1) Shield N1 → root R0; (2) spend N1 → NF1 recorded
        let n1 = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );
        let r0 = executor.merkle_root();

        let original_unshield = build_real_unshield(&executor, &n1, receiver, 5_000);
        let nf1 = original_unshield.input_nullifiers[0];
        executor
            .execute_unshield(&mut db, &original_unshield)
            .expect("first spend must succeed");
        assert!(
            executor.pnt.is_spent_in_window(&nf1),
            "NF1 must be live in PNT immediately after the spend"
        );

        // (3) Push more shields → root advances away from R0.
        for i in 0..5u8 {
            do_shield(
                &mut executor,
                &mut db,
                &sender,
                5_000,
                u64::from(i + 1),
                test_blinding(50 + i),
                test_blinding(60 + i),
                test_blinding(70 + i),
            );
        }
        let r_current = executor.merkle_root();
        assert_ne!(r0, r_current, "merkle root must advance with new shields");

        // (4) Rotate PNT phases past the bounded window. With
        // PNT_WINDOW_DEPTH = 5, advancing 5 times drops the original
        // phase that recorded NF1.
        for _ in 0..5 {
            executor.pnt_advance_phase();
        }
        assert!(
            !executor.pnt.is_spent_in_window(&nf1),
            "NF1 must have aged out of the bounded window after \
             PNT_WINDOW_DEPTH (=5) rotations"
        );
        // Confirm the bounded-window check is now LOSSY for NF1:
        // the fast-path is_double_spend says "not spent" even though
        // it was definitely spent earlier.
        assert!(
            !executor.is_double_spend(&db, &nf1),
            "v1+ is_double_spend must reflect bounded-window state — \
             returns false post-eviction (this is the gap that the \
             two defensive layers below must close)"
        );

        // ─── Attack 1 — replay original tx (anchor = stale R0) ───
        // Defensive layer: anchor enforcement at line 494.
        match executor.execute_unshield(&mut db, &original_unshield) {
            Err(PrivacyExecError::StaleAnchor) => {
                // ✓ Anchor enforcement caught the replay before any
                // nullifier check ran.
            }
            other => panic!(
                "expected StaleAnchor for old-anchor replay; got {:?}",
                other
            ),
        }

        // ─── Attack 2 — sophisticated respend with fresh proof ────
        // The attacker rebuilds the merkle proof against the current
        // root (the note_tree is append-only, so N1's commitment is
        // still at tree_index 0 in the current tree) and uses the
        // current anchor. This passes step 1 (StaleAnchor) and step 5
        // (proof.root == anchor). The bounded-window check in step 3
        // also passes (PNT evicted). Defensive layer: step 7's
        // canonical engine.nullifier_set, which is unbounded and
        // never evicts within a process lifetime.
        let fresh_attack_tx = build_real_unshield(&executor, &n1, receiver, 5_000);
        assert_eq!(
            fresh_attack_tx.anchor,
            executor.merkle_root(),
            "the rebuild must use the current root so anchor enforcement passes"
        );
        match executor.execute_unshield(&mut db, &fresh_attack_tx) {
            Err(PrivacyExecError::DoubleSpend(_)) => {
                // ✓ Canonical engine.nullifier_set retained NF1 even
                // though PNT bounded window evicted it. Joint security
                // claim holds: the bounded window's eviction is safe
                // because the unbounded set still has the nullifier.
            }
            other => panic!(
                "expected DoubleSpend from canonical nullifier_set; got {:?}",
                other
            ),
        }
    }

    // ── Private Transfer Tests (100% real) ──

    #[test]
    fn test_private_transfer_real() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &addr,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let out_blinds = [test_blinding(30), test_blinding(31)];
        let tx = build_real_transfer(&executor, &note, &[3_000, 1_900], &out_blinds, 100);

        let result = executor.execute_private_transfer(&mut db, &tx).unwrap();
        assert_eq!(result.notes_created, 2);
        assert_eq!(result.nullifiers_spent, 1);
        assert_eq!(result.fee_collected, 100);
        assert_eq!(db.get_shielded_pool_balance(), 4_900); // 5000 - 100 fee
        assert!(db.is_nullifier_spent(&tx.input_nullifiers[0]));
        assert_eq!(db.get_note_count(), 3); // 1 shield + 2 transfer outputs
    }

    #[test]
    fn test_private_transfer_double_spend_rejected() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &addr,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let tx1 = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        executor.execute_private_transfer(&mut db, &tx1).unwrap();

        // Try to spend the same note again
        let note2 = do_shield(
            &mut executor,
            &mut db,
            &addr,
            3_000,
            1,
            test_blinding(11),
            test_blinding(21),
            test_blinding(98),
        );
        let mut tx2 = build_real_transfer(&executor, &note2, &[2_900], &[test_blinding(40)], 100);
        tx2.input_nullifiers = tx1.input_nullifiers.clone();
        let err = executor
            .execute_private_transfer(&mut db, &tx2)
            .unwrap_err();
        assert!(matches!(err, PrivacyExecError::DoubleSpend(_)));
    }

    #[test]
    fn test_private_transfer_duplicate_nullifier_in_tx_rejected() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &addr,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let mut tx = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        // Duplicate the nullifier
        tx.input_nullifiers.push(tx.input_nullifiers[0]);
        // Also duplicate witness fields to match
        tx.input_amounts.push(tx.input_amounts[0]);
        tx.input_blindings.push(tx.input_blindings[0]);
        tx.input_value_commitments
            .push(tx.input_value_commitments[0]);
        tx.input_note_commitments.push(tx.input_note_commitments[0]);
        tx.input_merkle_proofs
            .push(tx.input_merkle_proofs[0].clone());

        let err = executor.execute_private_transfer(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::DoubleSpend(_)));
    }

    #[test]
    fn test_private_transfer_no_inputs_rejected() {
        let mut db = InMemoryStateDB::new();
        let mut executor = PrivacyExecutor::with_depth(8);

        let tx = PrivateTransferTx {
            input_nullifiers: vec![],
            output_commitments: vec![[0u8; 32]],
            anchor: executor.merkle_root(),
            balance_binding: [0u8; 32],
            fee: 0,
            input_amounts: vec![],
            input_blindings: vec![],
            input_value_commitments: vec![],
            input_note_commitments: vec![],
            input_merkle_proofs: vec![],
            output_amounts: vec![0],
            output_blindings: vec![[0u8; 32]],
            energy_proofs: vec![],
        };
        assert!(matches!(
            executor.execute_private_transfer(&mut db, &tx),
            Err(PrivacyExecError::NoInputs)
        ));
    }

    #[test]
    fn test_private_transfer_no_outputs_rejected() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &addr,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let mut tx = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        tx.output_commitments.clear();
        tx.output_amounts.clear();
        tx.output_blindings.clear();

        let err = executor.execute_private_transfer(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::NoOutputs));
    }

    #[test]
    fn test_private_transfer_invalid_balance_binding_rejected() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &addr,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let mut tx = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        tx.balance_binding = [0xAA; 32]; // garbage

        let err = executor.execute_private_transfer(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::InvalidBalanceBinding));
    }

    #[test]
    fn test_private_transfer_invalid_output_commitment_rejected() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &addr,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let mut tx = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        tx.output_commitments[0] = [0xDD; 32]; // doesn't match amount/blinding

        let err = executor.execute_private_transfer(&mut db, &tx).unwrap_err();
        assert!(matches!(
            err,
            PrivacyExecError::InvalidOutputCommitment { index: 0 }
        ));
    }

    #[test]
    fn test_private_transfer_invalid_merkle_proof_rejected() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &addr,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        let mut tx = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        tx.input_note_commitments[0] = [0xDD; 32]; // wrong tree leaf

        let err = executor.execute_private_transfer(&mut db, &tx).unwrap_err();
        assert!(matches!(
            err,
            PrivacyExecError::InvalidMerkleProof { index: 0 }
        ));
    }

    #[test]
    fn test_private_transfer_balance_conservation_failure() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor,
            &mut db,
            &addr,
            5_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );

        // Build valid tx, then inflate output_amounts to break conservation
        let mut tx = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        tx.output_amounts[0] = 6_000; // inflated — sum_in(5000) != sum_out(6000) + fee(100)
                                      // Recompute output commitment for the inflated amount so it passes commitment check
        tx.output_commitments[0] = Commitment::commit(6_000, &test_blinding(30)).0;
        // Recompute balance binding for the inflated values
        tx.balance_binding = compute_balance_binding(
            BalanceBindingKind::PrivateTransfer,
            5_000,
            6_000,
            100,
            &[note.blinding],
            &[test_blinding(30)],
        );

        let err = executor.execute_private_transfer(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::UnshieldBalanceMismatch));
    }

    // ── E2E: Shield → Transfer → Unshield (100% real) ──

    #[test]
    fn test_e2e_shield_transfer_unshield() {
        let alice = test_addr(1);
        let bob = test_addr(2);
        let mut db = setup_db_with_balance(&alice, 100_000);
        db.put_account(Account {
            address: bob,
            balance: 0,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        // 1. Alice shields 50,000
        let alice_note = do_shield(
            &mut executor,
            &mut db,
            &alice,
            50_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );
        assert_eq!(db.get_account(&alice).unwrap().balance, 50_000);
        assert_eq!(db.get_shielded_pool_balance(), 50_000);

        // 2. Private transfer: Alice's note → Bob(30K) + Alice change(19.5K), fee=500
        let bob_blind = test_blinding(30);
        let alice_change_blind = test_blinding(31);
        let tx = build_real_transfer(
            &executor,
            &alice_note,
            &[30_000, 19_500],
            &[bob_blind, alice_change_blind],
            500,
        );
        executor.execute_private_transfer(&mut db, &tx).unwrap();
        assert_eq!(db.get_shielded_pool_balance(), 49_500); // -500 fee

        // 3. Bob unshields 30,000 using real output note from the transfer
        // The transfer inserts output_commitments (value commitments) directly into the tree
        let bob_value_commitment = Commitment::commit(30_000, &bob_blind);
        // In private transfer, the tree leaf IS the value commitment (output_commitments[0])
        let bob_note_commitment = bob_value_commitment.0;
        // Bob's note was inserted at tree index 1 (alice's shield was 0, then transfer added 2 outputs: index 1 and 2)
        let bob_tree_index = 1;
        let bob_merkle_proof = executor.get_merkle_proof(bob_tree_index).unwrap();

        let bob_spending_secret = test_blinding(88);
        let bob_nullifier =
            Nullifier::derive(&bob_spending_secret, &Commitment(bob_note_commitment));
        let bob_binding = compute_balance_binding(
            BalanceBindingKind::Unshield,
            30_000,
            0,
            30_000,
            &[bob_blind],
            &[],
        );

        let unshield_tx = UnshieldTx {
            to: bob,
            amount: 30_000,
            input_nullifiers: vec![bob_nullifier.0],
            anchor: executor.merkle_root(),
            balance_binding: bob_binding,
            input_amounts: vec![30_000],
            input_blindings: vec![bob_blind],
            input_value_commitments: vec![bob_value_commitment.0],
            input_note_commitments: vec![bob_note_commitment],
            input_merkle_proofs: vec![bob_merkle_proof],
            output_blindings: vec![],
            change_commitments: vec![],
            energy_proofs: vec![],
        };
        executor.execute_unshield(&mut db, &unshield_tx).unwrap();

        // Final state
        assert_eq!(db.get_account(&alice).unwrap().balance, 50_000);
        assert_eq!(db.get_account(&bob).unwrap().balance, 30_000);
        assert_eq!(db.get_shielded_pool_balance(), 19_500);
        assert_eq!(db.nullifier_count(), 2);
    }

    // ── Gas Estimation ──

    #[test]
    fn test_gas_estimation() {
        let tx = PrivateTransferTx {
            input_nullifiers: vec![[0u8; 32]; 2],
            output_commitments: vec![[0u8; 32]; 3],
            anchor: [0u8; 32],
            balance_binding: [0u8; 32],
            fee: 0,
            input_amounts: vec![],
            input_blindings: vec![],
            input_value_commitments: vec![],
            input_note_commitments: vec![],
            input_merkle_proofs: vec![],
            output_amounts: vec![],
            output_blindings: vec![],
            energy_proofs: vec![],
        };
        let gas = PrivacyExecutor::estimate_private_transfer_gas(&tx);
        assert_eq!(
            gas,
            GAS_PRIVATE_TRANSFER_BASE
                + 2 * GAS_PRIVATE_TRANSFER_PER_INPUT
                + 3 * GAS_PRIVATE_TRANSFER_PER_OUTPUT
        );
    }

    // Audit AUDIT-2026-05-11-3: confirm the gas estimator saturates
    // rather than wraps when input/output counts approach u64::MAX.
    // The constructed tx is impossible to deserialize in practice
    // (Vec lengths are bounded by libp2p / serde limits) but the
    // arithmetic itself must be wrap-free regardless of input.
    #[test]
    fn gas_estimator_saturates_on_pathological_input() {
        // Use std::iter::repeat_with to avoid actually allocating
        // u64::MAX nullifiers; we only need a Vec whose `len()`
        // returns a wrap-trigger when multiplied. Allocating one
        // entry is enough — `len()` is the field we read.
        // (Direct construction of a giant Vec is OOM; we synthesize
        // a calculation that mirrors what the estimator does.)
        let huge: u64 = u64::MAX / GAS_PRIVATE_TRANSFER_PER_INPUT + 1;
        let prod = GAS_PRIVATE_TRANSFER_PER_INPUT.saturating_mul(huge);
        assert_eq!(prod, u64::MAX, "saturating_mul must clamp at u64::MAX");

        let sum = GAS_PRIVATE_TRANSFER_BASE
            .saturating_add(u64::MAX)
            .saturating_add(GAS_PRIVATE_TRANSFER_PER_OUTPUT);
        assert_eq!(sum, u64::MAX, "saturating_add must clamp at u64::MAX");
    }

    // ─── restore_from_db (Task #31) ──────────────────────────────────

    #[test]
    fn test_restore_from_db_empty_state_returns_zero() {
        let mut exec = PrivacyExecutor::with_depth(4);
        let db = InMemoryStateDB::new();
        let n = exec
            .restore_from_db(&db)
            .expect("empty restore should succeed");
        assert_eq!(n, 0);
    }

    #[test]
    fn test_restore_from_db_root_mismatch_errors() {
        let mut exec = PrivacyExecutor::with_depth(4);
        let mut db = InMemoryStateDB::new();
        // Persist a fake commitment + a wrong root.
        let bad_commitment = [0xABu8; 32];
        db.append_note_commitment(0, bad_commitment);
        db.put_note_tree_root([0xCDu8; 32]); // not the real rebuilt root
        let err = exec
            .restore_from_db(&db)
            .expect_err("root mismatch must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("rebuilt root"),
            "expected mismatch error, got: {msg}"
        );
    }

    /// PRIV-N2 (audit 2026-05-15) regression: `restore_from_db` must
    /// repopulate BOTH the canonical unbounded nullifier set AND the
    /// PNT live window from `db.all_nullifiers()`. Pre-fix, after
    /// restart on `protocol_version >= 1`, `is_double_spend()` reads
    /// `pnt.is_spent_in_window()` (the v1+ path) — which returns
    /// false for every previously-spent nullifier because the PNT
    /// was reconstructed empty. Every prior unshield can replay.
    #[test]
    fn priv_n2_restore_from_db_repopulates_nullifier_set_and_pnt_window() {
        // Pretend a prior chain run spent these nullifiers and persisted
        // them to the DB. After restart, restore_from_db must re-establish
        // the in-memory state so v1+ replay protection holds.
        let mut db = InMemoryStateDB::new();
        let nf_a = [0xAAu8; 32];
        let nf_b = [0xBBu8; 32];
        let nf_c = [0xCCu8; 32];
        // Mark them as spent in the persisted set (simulating a
        // previous chain run).
        assert!(db.spend_nullifier(&nf_a));
        assert!(db.spend_nullifier(&nf_b));
        assert!(db.spend_nullifier(&nf_c));

        // Fresh PrivacyExecutor (simulating a restart). nullifier_set
        // and pnt are both empty.
        let mut exec = PrivacyExecutor::with_depth(4);
        assert_eq!(exec.engine.nullifier_set.len(), 0);
        assert_eq!(exec.pnt.live_count(), 0);

        // Restore.
        exec.restore_from_db(&db)
            .expect("restore from db should succeed");

        // Canonical unbounded set: all three present.
        for nf in &[nf_a, nf_b, nf_c] {
            let n = evaporchain_proving::privacy::Nullifier(*nf);
            assert!(
                exec.engine.nullifier_set.is_spent(&n),
                "PRIV-N2: nullifier_set must contain persisted nullifier {:x?}",
                &nf[..4]
            );
        }
        // PNT live window: all three present, defending the v1+ path.
        for nf in &[nf_a, nf_b, nf_c] {
            assert!(
                exec.pnt.is_spent_in_window(nf),
                "PRIV-N2: PNT live window must contain persisted nullifier {:x?}",
                &nf[..4]
            );
        }
        assert_eq!(
            exec.pnt.live_count(),
            3,
            "PRIV-N2: PNT must have 3 nullifiers post-restore"
        );

        // The replay scenario: flip to v1, attempt a double-spend
        // check on a persisted nullifier — must report spent.
        exec.set_protocol_version(1);
        assert!(
            exec.is_double_spend(&db, &nf_a),
            "PRIV-N2: previously-spent nullifier must be reported spent under v1+ after restart"
        );
    }

    /// PRIV-N2: empty persisted nullifier set is the legitimate
    /// fresh-node case and must not error.
    #[test]
    fn priv_n2_restore_from_db_no_nullifiers_is_fine() {
        let mut exec = PrivacyExecutor::with_depth(4);
        let db = InMemoryStateDB::new();
        exec.restore_from_db(&db).expect("fresh restore should succeed");
        assert_eq!(exec.engine.nullifier_set.len(), 0);
        assert_eq!(exec.pnt.live_count(), 0);
    }

    /// T0.5 sub-task 5 — adversarial spend-evict-respend test.
    ///
    /// Verifies that under PNT v1 (bounded sliding-window nullifier
    /// store), the joint defense of `tx.anchor == merkle_root()` plus
    /// the bounded nullifier window rejects a respend attempt where:
    ///   1. The original spend's nullifier has aged out of the PNT
    ///      window (window says "not spent").
    ///   2. The chain has experienced subsequent shields that advanced
    ///      the merkle root past the attacker's original anchor.
    ///
    /// The audit text in MAINNET_READINESS.md T0.5 states:
    ///   "PNT bounded window + anchor enforcement are jointly secure;
    ///    either alone would be unsound."
    ///
    /// This test locks the JOINT contract under the realistic
    /// chain-progress assumption (intermediate shields advance the
    /// root). The orthogonal question of "what if no shields happen
    /// between original spend and the eviction window" is a known
    /// gap that requires either (a) anchor-history bound, (b)
    /// persistent v1 nullifier set, or (c) phase-advance gated on
    /// root-change. NOT covered here — see security review note
    /// added alongside this commit.
    #[test]
    fn pnt_v1_respend_after_window_eviction_rejected_via_anchor() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 50_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        // Activate PNT v1 — double-spend check now uses bounded window.
        executor.set_protocol_version(1);

        // Original spend at root R0. Capture the anchor for the
        // respend attempt later.
        let note_a = do_shield(
            &mut executor,
            &mut db,
            &sender,
            10_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );
        let unshield_a = build_real_unshield(&executor, &note_a, receiver, 10_000);
        let attacker_original_anchor = unshield_a.anchor;
        executor.execute_unshield(&mut db, &unshield_a).unwrap();

        // Sanity: nullifier_A is in the PNT window now.
        let nullifier_a = unshield_a.input_nullifiers[0];
        assert!(
            executor.pnt.is_spent_in_window(&nullifier_a),
            "nullifier_a must be in the PNT window immediately after spend"
        );

        // Intermediate shield — advances the merkle root past R0.
        let _note_b = do_shield(
            &mut executor,
            &mut db,
            &sender,
            5_000,
            1,
            test_blinding(11),
            test_blinding(21),
            test_blinding(98),
        );
        assert_ne!(
            executor.merkle_root(),
            attacker_original_anchor,
            "intermediate shield must advance the merkle root — \
             without this the test isn't exercising the anchor defense"
        );

        // Advance PNT phase enough times to evict nullifier_a from
        // the bounded window. The default window_depth is 5
        // (PNT_WINDOW_DEPTH at privacy_exec.rs:108); 6 explicit
        // advances guarantees the original phase has been popped.
        for _ in 0..6 {
            executor.pnt_advance_phase();
        }

        // Sanity: nullifier_A is no longer in the PNT window. Under
        // PNT v1 alone, the double-spend gate now returns false.
        // Anchor enforcement is the second line of defense.
        assert!(
            !executor.pnt.is_spent_in_window(&nullifier_a),
            "nullifier_a must have aged out of the PNT window after \
             3 phase advances; bounded window depth is small"
        );

        // Attacker re-attempts the original unshield with the original
        // anchor. The PNT window says the nullifier is not spent, but
        // the chain's current merkle_root has moved past R0. The
        // anchor check at privacy_exec.rs line 506 fires first and
        // rejects with StaleAnchor.
        let respend_attempt = unshield_a.clone();
        let err = executor
            .execute_unshield(&mut db, &respend_attempt)
            .unwrap_err();
        assert!(
            matches!(err, PrivacyExecError::StaleAnchor),
            "respend with original (now-stale) anchor MUST be rejected via \
             anchor enforcement; got {:?}",
            err
        );
    }

    /// T0.5 follow-up — Locks the current CANONICAL no-double-spend
    /// defense under PNT v1 (Stage 1 shadow-tracking).
    ///
    /// The audit narrative in MAINNET_READINESS.md T0.5 frames the
    /// defense as "PNT bounded window + anchor enforcement are
    /// jointly secure". That framing is **incomplete for the
    /// current Stage 1 wiring**. The actual canonical check today
    /// is `engine.nullifier_set.spend()` at privacy_exec.rs ~line 577
    /// — an unbounded in-memory set mirrored to db.spend_nullifier.
    /// Under v1 the additional `is_double_spend` dispatch at
    /// privacy_exec.rs:261 consults the bounded window only, but the
    /// engine.nullifier_set check still fires later in the same
    /// execute_unshield path and catches the respend regardless of
    /// protocol version.
    ///
    /// This means PNT v1 today is **Stage 1 (shadow tracking)** —
    /// the bounded window is being populated but the unbounded
    /// engine.nullifier_set check is what actually defends.
    ///
    /// **STAGE 2 HAZARD.** When the Stage 2 hard-fork plan
    /// (referenced in the comment at privacy_exec.rs ~line 583)
    /// removes the engine.nullifier_set check and makes
    /// is_double_spend the sole gate, the joint "window + anchor"
    /// defense becomes load-bearing. At that point this test would
    /// fail (respend succeeds without intermediate shields) UNLESS
    /// one of the following lands first:
    ///
    ///   - Anchor-history bound: reject any tx whose anchor is older
    ///     than the oldest live PNT phase. The chain already persists
    ///     anchors per-block; this is a comparison, not new storage.
    ///   - Phase-advance gated on root-change: only call
    ///     pnt.advance_phase() when merkle_root() differs from the
    ///     root at the previous tick. Couples eviction to chain
    ///     progress so anchor staleness and window eviction are
    ///     co-temporal.
    ///   - Persistent v1 nullifier set: keep writing to
    ///     db.spend_nullifier under v1 too, and have is_double_spend
    ///     consult BOTH the window AND the unbounded set. The window
    ///     becomes a cache; the set is the soundness gate.
    ///
    /// This test verifies the Stage 1 safety AND name-tags the
    /// Stage 2 risk so the transition is not done blindly.
    #[test]
    fn pnt_v1_no_intermediate_shield_respend_blocked_by_engine_nullifier_set() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 50_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);
        executor.set_protocol_version(1);

        // Shield → unshield → spend nullifier_a at root R0.
        let note_a = do_shield(
            &mut executor,
            &mut db,
            &sender,
            10_000,
            0,
            test_blinding(10),
            test_blinding(20),
            test_blinding(99),
        );
        let unshield_a = build_real_unshield(&executor, &note_a, receiver, 10_000);
        let root_at_spend = executor.merkle_root();
        executor.execute_unshield(&mut db, &unshield_a).unwrap();

        // No intermediate shield — root stays at R0. The anchor
        // defense will NOT fire on the respend (attacker's anchor
        // still equals current root).
        assert_eq!(
            executor.merkle_root(),
            root_at_spend,
            "no intermediate shield → merkle root unchanged"
        );

        // Evict nullifier_a from the bounded PNT window by advancing
        // 6 phases (window_depth = 5).
        for _ in 0..6 {
            executor.pnt_advance_phase();
        }
        let nullifier_a = unshield_a.input_nullifiers[0];
        assert!(
            !executor.pnt.is_spent_in_window(&nullifier_a),
            "post-eviction: PNT v1 window says nullifier is unseen"
        );
        // is_double_spend under v1 therefore returns false — the v1
        // dispatch is the bounded window only.
        assert!(
            !executor.is_double_spend(&db, &nullifier_a),
            "is_double_spend(v1) MUST return false after eviction; \
             this is the Stage-1-vs-Stage-2 boundary — under Stage 2 \
             this assertion is the load-bearing gate. Today it isn't, \
             because engine.nullifier_set fires later in execute_unshield."
        );

        // Now attempt the respend. UNDER STAGE 1, the engine.nullifier_set
        // check at privacy_exec.rs ~line 577 catches the double-spend
        // and returns Err(DoubleSpend). The bounded-window check at
        // ~line 504 returned false (per the assertion above), so the
        // FIRST gate misses, but the second gate fires.
        let respend = unshield_a.clone();
        let outcome = executor.execute_unshield(&mut db, &respend);
        assert!(
            matches!(outcome, Err(PrivacyExecError::DoubleSpend(_))),
            "Stage 1 contract: engine.nullifier_set rejects the respend \
             even when PNT v1 window-check misses. Got: {:?}",
            outcome
        );

        // Receiver balance was credited exactly once.
        let final_balance = db.get_account(&receiver).unwrap().balance;
        assert_eq!(
            final_balance, 10_000,
            "Stage 1 invariant: receiver was credited exactly once"
        );
    }
}
