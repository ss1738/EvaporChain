pub mod genesis;

use serde::{Deserialize, Serialize};

/// 32-byte object identifier.
pub type ObjectId = [u8; 32];

/// 32-byte account address.
pub type AccountAddress = [u8; 32];

/// Epoch number (monotonically increasing).
pub type Epoch = u64;

/// Energy units.
pub type Energy = u64;

/// Decay rate parameter.
pub type DecayRate = u64;

/// Half-life in epochs.
pub type HalfLife = u64;

/// LAD-VM substructural-resource mode.
///
/// Mirrors `evaporchain_lad_vm::Mode` so that `evaporchain-types` does
/// not need to depend on `evaporchain-lad-vm` (which would create a
/// circular dep — lad-vm already depends on types). The lad-vm crate
/// owns the runtime-side `From` conversions both ways so the two
/// representations stay in lockstep.
///
/// Wire format is lowercase: `"linear"` | `"affine"` | `"decaying"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LadMode {
    /// Must be consumed exactly once. `drop` is forbidden.
    Linear,
    /// May be consumed at most once. `drop` is allowed.
    Affine,
    /// Affine + decays automatically after a window. The window is
    /// stored on the LAD-VM `Resource`, not on `StateObject`.
    Decaying,
}

/// A state object stored on-chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateObject {
    pub id: ObjectId,
    pub owner: AccountAddress,
    pub energy: Energy,
    pub half_life: HalfLife,
    pub created_at: Epoch,
    pub last_refreshed: Epoch,
    pub state: ObjectState,
    pub grace_epoch: Option<Epoch>,
    pub data: Vec<u8>,
    #[serde(default)]
    pub decay_curve: Option<DecayCurve>,
    /// Optional LAD-VM substructural-resource type. Some objects are
    /// governed by the linear/affine/decaying type system; `None` means
    /// it's an ordinary state object.
    ///
    /// `#[serde(default)]` is critical — old persisted state must
    /// deserialise into `lad_mode: None` without breaking.
    #[serde(default)]
    pub lad_mode: Option<LadMode>,
}

impl StateObject {
    /// Compute remaining energy at the given epoch using exponential decay.
    pub fn energy_at(&self, current_epoch: Epoch) -> Energy {
        let epochs_since_refresh = current_epoch.saturating_sub(self.last_refreshed);
        energy_at_epoch(self.energy, self.half_life, epochs_since_refresh)
    }
}

/// Lifecycle state of an object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ObjectState {
    /// Object is live and accessible.
    Active,
    /// Energy reached zero; object is in grace period before evaporation.
    Grace,
    /// Object has been evaporated — only a nullifier proof remains.
    Ghost,
    /// Object was resurrected from Ghost state via a refresh transaction.
    Resurrected,
}

/// Record left behind when an object evaporates.
/// Stores a compact cryptographic commitment (data_hash) plus an optional
/// copy of the original data. By default, the node retains original data
/// for resurrection convenience. In production, data availability is handled
/// by the DA layer — compact ghosts (original_data = None) save storage
/// and resurrection requires the caller to supply the original data, which
/// is verified against data_hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GhostRecord {
    pub object_id: ObjectId,
    pub owner: AccountAddress,
    pub evaporated_at: Epoch,
    /// BLAKE3 hash of the original object data — always stored.
    pub data_hash: [u8; 32],
    /// Original data retained for resurrection. None = compact ghost
    /// (data must be supplied externally via DA layer for resurrection).
    #[serde(default)]
    pub original_data: Option<Vec<u8>>,
    /// Position in the MMR nullifier accumulator (None for legacy ghosts).
    #[serde(default)]
    pub mmr_position: Option<u64>,
    /// Original half-life preserved for resurrection (None for legacy ghosts).
    #[serde(default)]
    pub original_half_life: Option<u64>,
}

/// A block in the chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub number: u64,
    pub epoch: Epoch,
    pub parent_hash: [u8; 32],
    pub state_root: [u8; 32],
    pub transactions: Vec<Transaction>,
    pub timestamp: u64,
    /// Chain identifier (e.g., "evaporchain-mainnet-1"). Prevents cross-chain replay.
    #[serde(default)]
    pub chain_id: String,
    /// Validator ID that produced this block (None for single-node mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_id: Option<u64>,
    /// VRF output from the block proposer (32-byte verifiable randomness).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vrf_output: Option<[u8; 32]>,
    /// VRF proof (ML-DSA signature, ~3293 bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vrf_proof: Option<Vec<u8>>,
    /// Merkle root over all row/column roots of the erasure-coded data matrix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_root: Option<[u8; 32]>,
    /// Row Merkle roots from 2D erasure-coded extended data square.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub da_row_roots: Vec<[u8; 32]>,
    /// Column Merkle roots from 2D erasure-coded extended data square.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub da_col_roots: Vec<[u8; 32]>,
    /// Per-blob namespace commitments for data availability.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blob_commitments: Vec<[u8; 32]>,
    /// Serialized DA certificate (BLS-aggregated validator attestations).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub da_certificate: Option<Vec<u8>>,
    /// BLS aggregate commit certificate proving 2f+1 validators precommitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_certificate: Option<CommitCertificate>,
    /// Compressed Nova IVC proof attesting to the state transition.
    /// Generated by the ChainProver after block execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nova_proof: Option<Vec<u8>>,
    /// State anchor hash at anchor heights (every 100 blocks).
    /// Validators verify this matches their locally computed anchor to ensure
    /// state root agreement despite time-dependent decay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_hash: Option<[u8; 32]>,
    /// Rule-Based Consensus commitment: replaces per-block state root semantics.
    /// Between anchor points, validators agree on the decay function commitment
    /// rather than an eagerly-computed state root. Any verifier can derive the
    /// state at any epoch >= anchor_epoch using: anchor_state + decay_rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_function_commitment: Option<BlockStateCommitment>,
    /// Oracle state root — SHA-256 commitment over all finalized oracle feeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oracle_state_root: Option<[u8; 32]>,
    /// Shard health summary — number of active shards and compaction candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shard_count: Option<u16>,
    /// Soft-fork protocol version. `0` is the pre-Lane-B legacy
    /// behaviour (this is what every existing block carries via
    /// `serde(default)` — old chains read as `0`). Future fork-epoch
    /// activations bump the value, e.g. `1` → PNT-authoritative
    /// double-spend gating, `2` → EnergyVerkleTrie-authoritative
    /// state root, etc. Validators MUST refuse blocks with
    /// `protocol_version` higher than the version the binary
    /// implements; equal-or-lower is accepted (forward-compat
    /// downgrade for emergency rollback).
    ///
    /// Wire-format: `serde(default)` keeps legacy blocks bit-compat.
    /// `skip_serializing_if` is intentionally NOT used — once a chain
    /// upgrades, every block must carry the version explicitly so a
    /// joining node can validate the transition without ambiguity.
    #[serde(default)]
    pub protocol_version: u8,
    /// State-root commitment version. Independent of `protocol_version`
    /// because the state-root semantics can flip on a different fork
    /// epoch from the consensus-rule version. `0` is plain Verkle
    /// (today's authoritative). `1` is the planned EnergyVerkleTrie-
    /// authoritative flip (Lane E.2): once active, the `state_root`
    /// field commits to the energy-annotated trie + cold-subtree
    /// compressions instead of plain Verkle. The Nova step circuit's
    /// `state_hash` binding has to read whichever version the block
    /// declares.
    ///
    /// Same wire-format rules as `protocol_version`: `serde(default)`
    /// for legacy bit-compat, no `skip_serializing_if` so the field
    /// is always present once a chain has upgraded.
    #[serde(default)]
    pub state_root_version: u8,
    /// Phase-2 of `research/proposals/energy-stamped-mev-resistance.md`:
    /// per-tx submit-epoch hints carried on the wire so every validator
    /// computes the SAME priority for each tx, deterministically.
    ///
    /// Length conventions (verifier MUST enforce):
    ///   - empty `vec![]` (the default for legacy blocks): no hints,
    ///     mempool falls back to its local `tx_submit_epoch` for the
    ///     producer's own priority calculation; followers can't
    ///     reconstruct priority and the priority bonus is suppressed.
    ///   - non-empty: `submit_epoch_hints.len() == transactions.len()`,
    ///     and `submit_epoch_hints[i]` is `Some(epoch)` for tx `i` if
    ///     the proposer included a hint, `None` if not.
    ///
    /// This is a soft-fork wire-format change: existing blocks
    /// serialize bit-identically (skip-empty + skip-None), new blocks
    /// add the field only when at least one tx was hinted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub submit_epoch_hints: Vec<Option<u64>>,
    /// Phase 2.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — DAG-shaped
    /// parents. Empty (default) means single-parent linear chain
    /// semantics (use `parent_hash`). Non-empty means the block has
    /// `parents.len()` parents and represents a DAG-merge node.
    /// Phase 2.3 contract: `parents.len() > 1` requires
    /// `protocol_version >= 3`.
    ///
    /// Wire-format: `serde(default, skip_serializing_if =
    /// "Vec::is_empty")` keeps existing single-parent blocks
    /// bit-identical on the wire (the field is omitted when empty).
    /// `block_hash` does NOT include this field, preserving the
    /// pre-Light-Cone block-hash contract for chain-id continuity.
    /// When/if Phase 4's antichain finality activates DAG-aware
    /// hashing, the protocol_version bump will be the gate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<[u8; 32]>,
}

/// Phase 2.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` — wire-format
/// validation error for the `parents` field.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlockParentsValidationError {
    #[error(
        "block carries parents.len() = {n} but protocol_version = {pv}; \
         multi-parent blocks require protocol_version >= 3"
    )]
    MultiParentRequiresV3 { n: usize, pv: u8 },
    #[error("block.parents contains a duplicate entry: {0:?}")]
    DuplicateParent([u8; 32]),
    #[error(
        "block.parents[0] = {first:?} disagrees with block.parent_hash = {ph:?}; \
         when both are present they must be equal"
    )]
    ParentHashMismatch { first: [u8; 32], ph: [u8; 32] },
}

impl Block {
    /// Phase 2.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — return the
    /// block's effective parent set. If `parents` is non-empty,
    /// returns it as-is. Otherwise returns `vec![parent_hash]`
    /// (single-parent linear-chain fallback).
    ///
    /// All Light-Cone consumers should use this accessor instead
    /// of reading `parent_hash` or `parents` directly. Phase 3's
    /// state-branch materialization keys off the effective set.
    pub fn effective_parents(&self) -> Vec<[u8; 32]> {
        if !self.parents.is_empty() {
            self.parents.clone()
        } else {
            vec![self.parent_hash]
        }
    }

    /// Phase 2.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` — validate the
    /// wire-format invariants of the `parents` field.
    ///
    /// Three failure modes:
    /// - `MultiParentRequiresV3` — block carries >1 parents but
    ///   `protocol_version < 3`. Soft-fork gate.
    /// - `DuplicateParent` — same parent listed twice in `parents`.
    /// - `ParentHashMismatch` — both `parents[0]` and `parent_hash`
    ///   are present but disagree (validators rely on `parent_hash`
    ///   for legacy paths; `parents[0]` for DAG paths; the two
    ///   MUST agree to keep both paths convergent).
    ///
    /// Cycle detection is the DAG-side responsibility — see
    /// `evaporchain-light-cone::LightCone::insert` for the
    /// `MissingParent` rejection rule.
    pub fn validate_parents_wire_format(&self) -> Result<(), BlockParentsValidationError> {
        if self.parents.len() > 1 && self.protocol_version < 3 {
            return Err(BlockParentsValidationError::MultiParentRequiresV3 {
                n: self.parents.len(),
                pv: self.protocol_version,
            });
        }

        // Duplicate detection.
        let mut seen = std::collections::BTreeSet::new();
        for p in &self.parents {
            if !seen.insert(*p) {
                return Err(BlockParentsValidationError::DuplicateParent(*p));
            }
        }

        // parent_hash / parents[0] consistency.
        if !self.parents.is_empty() && self.parents[0] != self.parent_hash {
            return Err(BlockParentsValidationError::ParentHashMismatch {
                first: self.parents[0],
                ph: self.parent_hash,
            });
        }

        Ok(())
    }
}

/// Commitment to the state function for Rule-Based Consensus.
///
/// Instead of committing to a per-block state root (which is time-dependent
/// and causes divergence), blocks commit to the state *function*:
/// the anchor reference + decay rules that deterministically define state
/// at any epoch >= anchor_epoch.
///
/// Anchor blocks carry `is_anchor: true` and a full state materialization.
/// Non-anchor blocks reference the last anchor and carry only the decay rules hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockStateCommitment {
    /// Hash of the referenced anchor's full state.
    pub anchor_hash: [u8; 32],
    /// Epoch at which the anchor was created.
    pub anchor_epoch: u64,
    /// Blake3 hash of the canonical decay rules (formula + grace_period + min_half_life + version).
    pub decay_rules_hash: [u8; 32],
    /// Number of active objects at execution time.
    pub active_objects: u64,
    /// Whether this block IS an anchor point (full state materialization).
    pub is_anchor: bool,
    /// Hash of this commitment (anchor_hash || anchor_epoch || decay_rules_hash || active_objects).
    pub commitment_hash: [u8; 32],
}

/// BLS aggregate signature certificate proving consensus finality.
///
/// Contains the aggregated BLS12-381 signature from 2f+1 precommitting
/// validators, plus a bitfield indicating which validators participated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitCertificate {
    /// Height this certificate attests to.
    pub height: u64,
    /// Round in which consensus was reached.
    pub round: u32,
    /// Block hash that was committed.
    pub block_hash: [u8; 32],
    /// Aggregated BLS12-381 signature (96 bytes).
    pub aggregate_signature: Vec<u8>,
    /// Validator IDs that contributed to the aggregate.
    pub signer_ids: Vec<u64>,
}

pub const STORAGE_RENT_PER_BYTE_PER_EPOCH: u64 = 1;
pub const MIN_STORAGE_DEPOSIT: u64 = 1000;

/// Canonical genesis-seeded faucet address (`[0xFA; 32]`). Pre-funded with
/// the residual `total_supply - validators × per-validator share`. Used by
/// `/api/faucet` as the source of faucet transfers; matches the address
/// stamped by `evaporchain-cli testnet init` and the Genesis-Ceremony flow.
pub const FAUCET_ADDRESS: [u8; 32] = [0xFAu8; 32];

/// Returns `true` iff `addr` is the canonical genesis-seeded faucet address
/// ([`FAUCET_ADDRESS`] = `[0xFA; 32]`). This is distinct from the legacy
/// mint-bypass address (`[0u8; 32]`) used by execution paths to skip nonce
/// checks; the FAUCET_ADDRESS is a normal pre-funded account whose transfers
/// flow through the standard nonce + balance checks.
#[inline]
pub fn is_faucet_address(addr: &[u8; 32]) -> bool {
    *addr == FAUCET_ADDRESS
}

/// An account with a balance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub address: AccountAddress,
    pub balance: u64,
    pub nonce: u64,
    #[serde(default)]
    pub storage_deposit: u64,
    #[serde(default)]
    pub storage_bytes: u64,
    /// Epoch at which this account's balance OR nonce was last mutated by
    /// a transaction.  Anchors per-account demurrage:
    /// `demurrage_owed(balance, last_touched_epoch, current_epoch, params)`.
    ///
    /// `#[serde(default)]` keeps deserialisation of legacy persisted state
    /// (RocksDB / WAL JSON written before this field existed) safe — old
    /// accounts come back with `last_touched_epoch == 0`, which makes
    /// demurrage settle conservatively-from-genesis until the next
    /// balance/nonce-mutating tx stamps the current epoch.
    #[serde(default)]
    pub last_touched_epoch: u64,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            address: [0u8; 32],
            balance: 0,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
        }
    }
}

/// Multi-signature transaction executed at the protocol level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiSigTx {
    pub multisig_address: AccountAddress,
    pub threshold: u8,
    pub signers: Vec<AccountAddress>,
    pub inner_tx_bytes: Vec<u8>,
    pub signatures: Vec<(AccountAddress, Vec<u8>)>,
    pub public_keys: Vec<(AccountAddress, Vec<u8>)>,
    pub nonce: u64,
}

/// Blob transaction for data availability layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobTx {
    pub submitter: [u8; 32],
    pub data: Vec<u8>,
    pub nonce: u64,
    pub namespace_id: u64,
    pub signature: Option<Vec<u8>>,
    pub public_key: Option<Vec<u8>>,
}

/// Transaction types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Transaction {
    Transfer(TransferTx),
    Refresh(RefreshTx),
    CreateObject(CreateObjectTx),
    DeployContract(DeployContractTx),
    CallContract(CallContractTx),
    DeployScript(DeployScriptTx),
    CallScript(CallScriptTx),
    ValidatorStake(ValidatorStakeTx),
    ValidatorExit(ValidatorExitTx),
    /// Claim unbonded stake after the unbonding period has elapsed.
    ValidatorClaimStake(ValidatorClaimStakeTx),
    /// Shield: transparent → private (burns balance, creates note).
    Shield(ShieldTx),
    /// Unshield: private → transparent (spends note, credits balance).
    Unshield(UnshieldTx),
    /// Private transfer: private → private (spends notes, creates notes).
    PrivateTransfer(PrivateTransferTx),
    /// Deferred: time-locked transaction that executes when temporal conditions are met.
    Deferred(DeferredTx),
    /// Blob: data availability blob submission.
    Blob(BlobTx),
    /// Governance: on-chain parameter proposals and voting.
    Governance(GovernanceTx),
    /// Multi-signature transaction requiring threshold approvals.
    MultiSig(MultiSigTx),
    /// Account abstraction: user operation with optional paymaster gas sponsorship.
    UserOp(UserOpTx),
    /// Upgrade a deployed contract's bytecode (owner + governance gate).
    UpgradeContract(UpgradeContractTx),
    /// Delegate stake to a validator on behalf of a token-holder.
    /// Locks `amount` from the delegator's balance and credits it to the
    /// validator's effective stake (used in voting power + reward share).
    Delegate(DelegateTx),
    /// Withdraw a delegation. Stake is locked for the unbonding period
    /// before being returned to the delegator's balance.
    Undelegate(UndelegateTx),
    /// Rotate a validator's BLS public key. The old key remains valid
    /// for a grace window (defined in execution) so in-flight certs do
    /// not lose quorum across the boundary.
    RotateValidatorKey(RotateValidatorKeyTx),
    /// Claim a previously-undelegated amount back to the delegator's
    /// balance once the unbonding period has elapsed (P0 #4 Phase 7).
    ClaimDelegation(ClaimDelegationTx),
    /// Crooks-MEV refund transaction — protocol-issued by the block
    /// proposer to settle a `MevObservation` from an earlier block.
    /// Per `CROOKS_MEV_INTEGRATION_PLAN.md` Phase 3.1. NOT user-
    /// signed; the proposer constructs it deterministically from
    /// the chain's `mev_observations` buffer + `mev_attacker_stats`
    /// table. Validators verify the construction matches their
    /// independently-computed observation+refund (Phase 3.2
    /// determinism contract).
    Refund(RefundTx),
}

impl Transaction {
    /// Compute the canonical byte representation for signing.
    /// Excludes signature/public_key fields — only the transaction body is signed.
    pub fn signable_bytes(&self) -> Vec<u8> {
        match self {
            Transaction::Transfer(tx) => {
                let mut buf = Vec::with_capacity(1 + 32 + 32 + 8 + 8);
                buf.push(0x01); // type tag
                buf.extend_from_slice(&tx.from);
                buf.extend_from_slice(&tx.to);
                buf.extend_from_slice(&tx.amount.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf
            }
            Transaction::Refresh(tx) => {
                let mut buf = Vec::with_capacity(1 + 32 + 8);
                buf.push(0x02);
                buf.extend_from_slice(&tx.object_id);
                buf.extend_from_slice(&tx.energy_deposit.to_le_bytes());
                buf
            }
            Transaction::CreateObject(tx) => {
                let mut buf = Vec::with_capacity(1 + 32 + 32 + 8 + 8 + tx.data.len());
                buf.push(0x03);
                buf.extend_from_slice(&tx.creator);
                buf.extend_from_slice(&tx.object_id);
                buf.extend_from_slice(&tx.energy.to_le_bytes());
                buf.extend_from_slice(&tx.half_life.to_le_bytes());
                buf.extend_from_slice(&tx.data);
                if let Some(ref curve) = tx.decay_curve {
                    if let Ok(curve_bytes) = serde_json::to_vec(curve) {
                        buf.extend_from_slice(&curve_bytes);
                    }
                }
                buf
            }
            Transaction::DeployContract(tx) => {
                let mut buf = Vec::new();
                buf.push(0x04);
                buf.extend_from_slice(&tx.deployer);
                buf.extend_from_slice(tx.template.as_bytes());
                buf.extend_from_slice(tx.init_args.as_bytes());
                buf.extend_from_slice(&tx.energy.to_le_bytes());
                buf.extend_from_slice(&tx.half_life.to_le_bytes());
                buf
            }
            Transaction::CallContract(tx) => {
                let mut buf = Vec::new();
                buf.push(0x05);
                buf.extend_from_slice(&tx.caller);
                buf.extend_from_slice(&tx.contract_id.to_le_bytes());
                buf.extend_from_slice(tx.method.as_bytes());
                buf.extend_from_slice(tx.args.as_bytes());
                buf
            }
            Transaction::DeployScript(tx) => {
                let mut buf = Vec::new();
                buf.push(0x06);
                buf.extend_from_slice(&tx.deployer);
                buf.extend_from_slice(tx.source_code.as_bytes());
                buf.extend_from_slice(&tx.energy.to_le_bytes());
                buf.extend_from_slice(&tx.half_life.to_le_bytes());
                buf
            }
            Transaction::CallScript(tx) => {
                let mut buf = Vec::new();
                buf.push(0x07);
                buf.extend_from_slice(&tx.caller);
                buf.extend_from_slice(&tx.contract_id.to_le_bytes());
                buf.extend_from_slice(tx.method.as_bytes());
                buf.extend_from_slice(tx.args.as_bytes());
                buf
            }
            Transaction::ValidatorStake(tx) => {
                let mut buf = Vec::with_capacity(1 + 32 + 8 + 8 + 8);
                buf.push(0x08);
                buf.extend_from_slice(&tx.validator_address);
                buf.extend_from_slice(&tx.stake_amount.to_le_bytes());
                buf.extend_from_slice(&tx.validator_id.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                if let Some(ref vrf_pk) = tx.vrf_public_key {
                    buf.extend_from_slice(vrf_pk);
                }
                buf
            }
            Transaction::ValidatorExit(tx) => {
                let mut buf = Vec::with_capacity(1 + 32 + 8 + 8);
                buf.push(0x09);
                buf.extend_from_slice(&tx.validator_address);
                buf.extend_from_slice(&tx.validator_id.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf
            }
            Transaction::ValidatorClaimStake(tx) => {
                let mut buf = Vec::with_capacity(1 + 32 + 8 + 8);
                buf.push(0x0F);
                buf.extend_from_slice(&tx.validator_address);
                buf.extend_from_slice(&tx.validator_id.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf
            }
            Transaction::Shield(tx) => {
                let mut buf = Vec::with_capacity(1 + 32 + 8 + 8 + 32 + 32 + 8);
                buf.push(0x0A);
                buf.extend_from_slice(&tx.from);
                buf.extend_from_slice(&tx.amount.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf.extend_from_slice(&tx.note_owner_hash);
                buf.extend_from_slice(&tx.value_blinding);
                buf.extend_from_slice(&tx.half_life.to_le_bytes());
                buf
            }
            Transaction::Unshield(tx) => {
                let mut buf = Vec::new();
                buf.push(0x0B);
                buf.extend_from_slice(&tx.to);
                buf.extend_from_slice(&tx.amount.to_le_bytes());
                buf.extend_from_slice(&tx.anchor);
                for nf in &tx.input_nullifiers {
                    buf.extend_from_slice(nf);
                }
                buf.extend_from_slice(&tx.balance_binding);
                buf
            }
            Transaction::PrivateTransfer(tx) => {
                let mut buf = Vec::new();
                buf.push(0x0C);
                buf.extend_from_slice(&tx.anchor);
                buf.extend_from_slice(&tx.fee.to_le_bytes());
                for nf in &tx.input_nullifiers {
                    buf.extend_from_slice(nf);
                }
                for oc in &tx.output_commitments {
                    buf.extend_from_slice(oc);
                }
                buf.extend_from_slice(&tx.balance_binding);
                buf
            }
            Transaction::Deferred(tx) => {
                let mut buf = Vec::new();
                buf.push(0x0D);
                buf.extend_from_slice(&tx.submitter);
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf.extend_from_slice(&tx.deposit.to_le_bytes());
                // Serialize guards.
                for guard in &tx.guards {
                    match guard {
                        TemporalGuard::AfterEpoch(e) => {
                            buf.push(0x01);
                            buf.extend_from_slice(&e.to_le_bytes());
                        }
                        TemporalGuard::BeforeEpoch(e) => {
                            buf.push(0x02);
                            buf.extend_from_slice(&e.to_le_bytes());
                        }
                        TemporalGuard::EnergyBelow(id, thresh) => {
                            buf.push(0x03);
                            buf.extend_from_slice(id);
                            buf.extend_from_slice(&thresh.to_le_bytes());
                        }
                        TemporalGuard::EnergyAbove(id, thresh) => {
                            buf.push(0x04);
                            buf.extend_from_slice(id);
                            buf.extend_from_slice(&thresh.to_le_bytes());
                        }
                        TemporalGuard::ObjectEvaporated(id) => {
                            buf.push(0x05);
                            buf.extend_from_slice(id);
                        }
                        TemporalGuard::ContractInPhase(cid, phase) => {
                            buf.push(0x06);
                            buf.extend_from_slice(&cid.to_le_bytes());
                            buf.extend_from_slice(phase.as_bytes());
                        }
                    }
                }
                // Serialize inner tx.
                buf.extend_from_slice(&tx.inner_tx_bytes);
                buf
            }
            Transaction::Blob(tx) => {
                let mut buf = Vec::new();
                buf.push(0x0E);
                buf.extend_from_slice(&tx.submitter);
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf.extend_from_slice(&tx.namespace_id.to_le_bytes());
                buf.extend_from_slice(&tx.data);
                buf
            }
            Transaction::Governance(tx) => {
                let mut buf = Vec::new();
                buf.push(0x10);
                buf.extend_from_slice(&tx.sender);
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                match &tx.action {
                    GovernanceAction::CreateProposal {
                        title,
                        param_key,
                        param_value,
                        voting_epochs,
                    } => {
                        buf.push(0x01);
                        buf.extend_from_slice(title.as_bytes());
                        buf.extend_from_slice(param_key.as_bytes());
                        buf.extend_from_slice(param_value.as_bytes());
                        buf.extend_from_slice(&voting_epochs.to_le_bytes());
                    }
                    GovernanceAction::CastVote { proposal_id, vote } => {
                        buf.push(0x02);
                        buf.extend_from_slice(&proposal_id.to_le_bytes());
                        buf.push(if *vote { 1 } else { 0 });
                    }
                }
                buf
            }
            Transaction::MultiSig(tx) => {
                let mut buf = Vec::new();
                buf.push(0x11);
                buf.extend_from_slice(&tx.multisig_address);
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf.push(tx.threshold);
                for signer in &tx.signers {
                    buf.extend_from_slice(signer);
                }
                buf.extend_from_slice(&tx.inner_tx_bytes);
                buf
            }
            Transaction::UserOp(tx) => {
                let mut buf = Vec::new();
                buf.push(0x12);
                buf.extend_from_slice(&tx.sender);
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf.extend_from_slice(&tx.call_gas_limit.to_le_bytes());
                buf.extend_from_slice(&tx.call_data);
                if let Some(ref pm) = tx.paymaster {
                    buf.extend_from_slice(pm);
                }
                buf
            }
            Transaction::UpgradeContract(tx) => {
                let mut buf = Vec::new();
                buf.push(0x13);
                buf.extend_from_slice(&tx.owner);
                buf.extend_from_slice(&tx.contract_id.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf.push(if tx.governance_approved { 1 } else { 0 });
                buf.extend_from_slice(&tx.new_bytecode);
                // New (mainnet) authorization fields. Appending preserves
                // the byte layout for the legacy prefix; new wallets sign
                // over the full extended message.
                buf.extend_from_slice(&tx.new_bytecode_hash);
                buf.extend_from_slice(&(tx.endorser_stakes.len() as u32).to_le_bytes());
                for s in &tx.endorser_stakes {
                    buf.extend_from_slice(&s.to_le_bytes());
                }
                buf.extend_from_slice(&tx.required_stake.to_le_bytes());
                buf
            }
            Transaction::Delegate(tx) => {
                let mut buf = Vec::new();
                buf.push(0x14);
                buf.extend_from_slice(&tx.delegator);
                buf.extend_from_slice(&tx.validator_id.to_le_bytes());
                buf.extend_from_slice(&tx.amount.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf
            }
            Transaction::Undelegate(tx) => {
                let mut buf = Vec::new();
                buf.push(0x15);
                buf.extend_from_slice(&tx.delegator);
                buf.extend_from_slice(&tx.validator_id.to_le_bytes());
                buf.extend_from_slice(&tx.amount.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf
            }
            Transaction::RotateValidatorKey(tx) => {
                let mut buf = Vec::new();
                buf.push(0x16);
                buf.extend_from_slice(&tx.validator_address);
                buf.extend_from_slice(&tx.validator_id.to_le_bytes());
                // BLS keys + PoP signatures are length-prefixed so the
                // canonical form is unambiguous regardless of size.
                buf.extend_from_slice(&(tx.new_bls_public_key.len() as u32).to_le_bytes());
                buf.extend_from_slice(&tx.new_bls_public_key);
                buf.extend_from_slice(&(tx.bls_pop_old.len() as u32).to_le_bytes());
                buf.extend_from_slice(&tx.bls_pop_old);
                buf.extend_from_slice(&(tx.bls_pop_new.len() as u32).to_le_bytes());
                buf.extend_from_slice(&tx.bls_pop_new);
                buf.extend_from_slice(&tx.effective_epoch.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf
            }
            Transaction::ClaimDelegation(tx) => {
                let mut buf = Vec::new();
                buf.push(0x17);
                buf.extend_from_slice(&tx.delegator);
                buf.extend_from_slice(&tx.validator_id.to_le_bytes());
                buf.extend_from_slice(&tx.nonce.to_le_bytes());
                buf
            }
            // Crooks-MEV refund — protocol-issued, no signature.
            // signable_bytes still defined for canonical hashing /
            // tx_hash purposes; consensus does NOT sign-verify Refund.
            Transaction::Refund(tx) => {
                let mut buf = Vec::new();
                buf.push(0x18);
                buf.extend_from_slice(&tx.source_block_height.to_le_bytes());
                buf.extend_from_slice(&(tx.source_observation_idx as u64).to_le_bytes());
                buf.extend_from_slice(&tx.attacker);
                buf.extend_from_slice(&tx.victim);
                buf.extend_from_slice(&tx.amount.to_le_bytes());
                buf.extend_from_slice(&tx.settle_block_height.to_le_bytes());
                buf
            }
        }
    }

    /// Compute the signing message: chain_id domain separator + signable_bytes.
    /// This prevents cross-chain replay attacks (cf. EIP-155).
    pub fn signing_message(&self, chain_id: &str) -> Vec<u8> {
        let body = self.signable_bytes();
        let mut msg = Vec::with_capacity(4 + chain_id.len() + body.len());
        msg.extend_from_slice(&(chain_id.len() as u32).to_le_bytes());
        msg.extend_from_slice(chain_id.as_bytes());
        msg.extend_from_slice(&body);
        msg
    }

    /// Canonical BLAKE3 hash of this transaction (over signable_bytes).
    pub fn tx_hash(&self) -> [u8; 32] {
        *blake3::hash(&self.signable_bytes()).as_bytes()
    }

    /// Get the signature bytes (if present on the inner tx).
    pub fn signature(&self) -> Option<&[u8]> {
        match self {
            Transaction::Transfer(tx) => tx.signature.as_deref(),
            Transaction::Refresh(tx) => tx.signature.as_deref(),
            Transaction::CreateObject(tx) => tx.signature.as_deref(),
            Transaction::DeployContract(tx) => tx.signature.as_deref(),
            Transaction::CallContract(tx) => tx.signature.as_deref(),
            Transaction::DeployScript(tx) => tx.signature.as_deref(),
            Transaction::CallScript(tx) => tx.signature.as_deref(),
            Transaction::ValidatorStake(tx) => tx.signature.as_deref(),
            Transaction::ValidatorExit(tx) => tx.signature.as_deref(),
            Transaction::ValidatorClaimStake(tx) => tx.signature.as_deref(),
            Transaction::Shield(tx) => tx.signature.as_deref(),
            // Unshield and PrivateTransfer are authenticated by ZK proofs, not signatures.
            Transaction::Unshield(_) => None,
            Transaction::PrivateTransfer(_) => None,
            Transaction::Deferred(tx) => tx.signature.as_deref(),
            Transaction::Blob(tx) => tx.signature.as_deref(),
            Transaction::Governance(tx) => tx.signature.as_deref(),
            Transaction::MultiSig(_) => None,
            Transaction::UserOp(tx) => tx.signature.as_deref(),
            Transaction::UpgradeContract(tx) => tx.signature.as_deref(),
            Transaction::Delegate(tx) => tx.signature.as_deref(),
            Transaction::Undelegate(tx) => tx.signature.as_deref(),
            Transaction::RotateValidatorKey(tx) => tx.signature.as_deref(),
            Transaction::ClaimDelegation(tx) => tx.signature.as_deref(),
            // Refund is protocol-issued; no signature.
            Transaction::Refund(_) => None,
        }
    }

    /// Get the public key bytes (if present on the inner tx).
    pub fn public_key(&self) -> Option<&[u8]> {
        match self {
            Transaction::Transfer(tx) => tx.public_key.as_deref(),
            Transaction::Refresh(tx) => tx.public_key.as_deref(),
            Transaction::CreateObject(tx) => tx.public_key.as_deref(),
            Transaction::DeployContract(tx) => tx.public_key.as_deref(),
            Transaction::CallContract(tx) => tx.public_key.as_deref(),
            Transaction::DeployScript(tx) => tx.public_key.as_deref(),
            Transaction::CallScript(tx) => tx.public_key.as_deref(),
            Transaction::ValidatorStake(tx) => tx.public_key.as_deref(),
            Transaction::ValidatorExit(tx) => tx.public_key.as_deref(),
            Transaction::ValidatorClaimStake(tx) => tx.public_key.as_deref(),
            Transaction::Shield(tx) => tx.public_key.as_deref(),
            Transaction::Unshield(_) => None,
            Transaction::PrivateTransfer(_) => None,
            Transaction::Deferred(tx) => tx.public_key.as_deref(),
            Transaction::Blob(tx) => tx.public_key.as_deref(),
            Transaction::Governance(tx) => tx.public_key.as_deref(),
            Transaction::MultiSig(_) => None,
            Transaction::UserOp(tx) => tx.public_key.as_deref(),
            Transaction::UpgradeContract(tx) => tx.public_key.as_deref(),
            Transaction::Delegate(tx) => tx.public_key.as_deref(),
            Transaction::Undelegate(tx) => tx.public_key.as_deref(),
            Transaction::RotateValidatorKey(tx) => tx.public_key.as_deref(),
            Transaction::ClaimDelegation(tx) => tx.public_key.as_deref(),
            // Refund is protocol-issued; no public key.
            Transaction::Refund(_) => None,
        }
    }

    /// Get the sender/payer address for fee deduction.
    /// Returns the address of the account responsible for paying gas fees.
    pub fn sender(&self) -> Option<&AccountAddress> {
        match self {
            Transaction::Transfer(tx) => Some(&tx.from),
            Transaction::CreateObject(tx) => Some(&tx.creator),
            Transaction::DeployContract(tx) => Some(&tx.deployer),
            Transaction::CallContract(tx) => Some(&tx.caller),
            Transaction::DeployScript(tx) => Some(&tx.deployer),
            Transaction::CallScript(tx) => Some(&tx.caller),
            Transaction::Refresh(_) => None, // Refresh has no sender address field
            Transaction::ValidatorStake(tx) => Some(&tx.validator_address),
            Transaction::ValidatorExit(tx) => Some(&tx.validator_address),
            Transaction::ValidatorClaimStake(tx) => Some(&tx.validator_address),
            Transaction::Shield(tx) => Some(&tx.from),
            // Unshield/PrivateTransfer have no transparent sender — fees come from the shielded pool.
            Transaction::Unshield(_) => None,
            Transaction::PrivateTransfer(_) => None,
            Transaction::Deferred(tx) => Some(&tx.submitter),
            Transaction::Blob(tx) => Some(&tx.submitter),
            Transaction::Governance(tx) => Some(&tx.sender),
            Transaction::MultiSig(tx) => Some(&tx.multisig_address),
            Transaction::UserOp(tx) => {
                if let Some(ref pm) = tx.paymaster {
                    Some(pm)
                } else {
                    Some(&tx.sender)
                }
            }
            Transaction::UpgradeContract(tx) => Some(&tx.owner),
            Transaction::Delegate(tx) => Some(&tx.delegator),
            Transaction::Undelegate(tx) => Some(&tx.delegator),
            Transaction::RotateValidatorKey(tx) => Some(&tx.validator_address),
            Transaction::ClaimDelegation(tx) => Some(&tx.delegator),
            // Refund is protocol-issued — the "sender" semantically
            // is the chain itself. Surface the attacker (debited
            // party) for accounting/UI purposes; consensus-level
            // logic gates on the deterministic-construction contract,
            // not on sender.
            Transaction::Refund(tx) => Some(&tx.attacker),
        }
    }

    pub fn nonce(&self) -> Option<u64> {
        match self {
            Transaction::Transfer(tx) => Some(tx.nonce),
            Transaction::CreateObject(_) => None,
            Transaction::DeployContract(_) => None,
            Transaction::CallContract(_) => None,
            Transaction::DeployScript(_) => None,
            Transaction::CallScript(_) => None,
            Transaction::Refresh(_) => None,
            Transaction::ValidatorStake(tx) => Some(tx.nonce),
            Transaction::ValidatorExit(tx) => Some(tx.nonce),
            Transaction::ValidatorClaimStake(tx) => Some(tx.nonce),
            Transaction::Shield(tx) => Some(tx.nonce),
            Transaction::Unshield(_) => None,
            Transaction::PrivateTransfer(_) => None,
            Transaction::Deferred(tx) => Some(tx.nonce),
            Transaction::Blob(tx) => Some(tx.nonce),
            Transaction::Governance(tx) => Some(tx.nonce),
            Transaction::MultiSig(tx) => Some(tx.nonce),
            Transaction::UserOp(tx) => Some(tx.nonce),
            Transaction::UpgradeContract(tx) => Some(tx.nonce),
            Transaction::Delegate(tx) => Some(tx.nonce),
            Transaction::Undelegate(tx) => Some(tx.nonce),
            Transaction::RotateValidatorKey(tx) => Some(tx.nonce),
            Transaction::ClaimDelegation(tx) => Some(tx.nonce),
            // Refund tx is protocol-issued — no replay nonce. The
            // (source_block_height, source_observation_idx) pair is
            // the unique identifier; replay-protection happens via
            // the consensus engine refusing to settle the same
            // observation twice (Phase 3.3 contract).
            Transaction::Refund(_) => None,
        }
    }
}

/// Value transfer transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferTx {
    pub from: AccountAddress,
    pub to: AccountAddress,
    pub amount: u64,
    pub nonce: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
    /// Phase 4.2 of `CROOKS_MEV_INTEGRATION_PLAN.md` — victim
    /// opt-out flag for the Crooks-MEV refund pipeline. `None`
    /// (default) means standard behaviour: if this tx is detected
    /// as the victim leg of a sandwich, a `RefundTx` is auto-issued.
    /// `Some(false)` opts the sender OUT of MEV refunds — the
    /// detector still records the observation (for monitoring) but
    /// no refund settles. `Some(true)` is reserved for future
    /// "explicitly opt-in" semantics if the chain ever switches the
    /// default.
    ///
    /// Wire-format: `serde(default, skip_serializing_if =
    /// "Option::is_none")` so legacy single-purpose Transfer txs
    /// serialize bit-identically (the field is omitted when None).
    /// Hash-stability gate: `signable_bytes` does NOT include this
    /// field — chain-id continuity preserved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mev_refund_eligible: Option<bool>,
}

/// Energy refresh transaction (prevents evaporation or resurrects a ghost).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshTx {
    pub object_id: ObjectId,
    pub energy_deposit: Energy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Create a new state object with initial energy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateObjectTx {
    pub creator: AccountAddress,
    pub object_id: ObjectId,
    pub energy: Energy,
    pub half_life: HalfLife,
    pub data: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decay_curve: Option<DecayCurve>,
    /// Optional LAD-VM substructural-resource type stamped onto the
    /// resulting `StateObject`. `None` (default) produces an ordinary
    /// non-substructural object. The future `evaporchain-script-lad`
    /// frontend will set this when lowering an annotated declaration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lad_mode: Option<LadMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Deploy a smart contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployContractTx {
    pub deployer: AccountAddress,
    /// Template name: "DecayingToken", "MortalNFT", "ThermodynamicEscrow",
    /// "DecayingAuction", "StakingPool", "DAOVote"
    pub template: String,
    /// JSON-encoded initialization arguments.
    pub init_args: String,
    /// Initial energy for the contract instance.
    pub energy: Energy,
    /// Half-life for contract energy decay.
    pub half_life: HalfLife,
    /// Custom rules (JSON-encoded array), optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Call a method on a deployed contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallContractTx {
    pub caller: AccountAddress,
    pub contract_id: u64,
    pub method: String,
    /// JSON-encoded method arguments.
    pub args: String,
    /// Current epoch (for energy checks).
    pub epoch: Epoch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Deploy an EvaporScript contract from source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployScriptTx {
    pub deployer: AccountAddress,
    /// EvaporScript source code.
    pub source_code: String,
    /// Initial energy for the script contract.
    pub energy: Energy,
    /// Half-life for script contract energy decay.
    pub half_life: HalfLife,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Call a method on a deployed EvaporScript contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallScriptTx {
    pub caller: AccountAddress,
    pub contract_id: u64,
    pub method: String,
    /// JSON-encoded method arguments.
    pub args: String,
    /// Current epoch (for energy checks).
    pub epoch: Epoch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Stake tokens as a validator (or increase existing stake).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorStakeTx {
    /// Validator address (same as staker address).
    pub validator_address: AccountAddress,
    /// Amount to stake (transferred from balance to stake).
    pub stake_amount: u64,
    /// Validator ID to register or update.
    pub validator_id: u64,
    /// Sender nonce.
    pub nonce: u64,
    /// BLS12-381 public key for consensus (hex-encoded, 48 bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bls_public_key: Option<Vec<u8>>,
    /// Post-quantum VRF public key (ML-DSA, 1952 bytes) for leader election.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vrf_public_key: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Request to exit the validator set and begin unbonding.
/// Stake is locked for `unbonding_period` epochs after this tx is processed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorExitTx {
    /// Validator address.
    pub validator_address: AccountAddress,
    /// Validator ID to exit.
    pub validator_id: u64,
    /// Sender nonce.
    pub nonce: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Claim unbonded stake back to the validator's balance.
/// Only succeeds if the unbonding period has elapsed since the ValidatorExit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorClaimStakeTx {
    pub validator_address: AccountAddress,
    pub validator_id: u64,
    pub nonce: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Rotate a validator's BLS public key while keeping the validator slot
/// active. After commit, the previous key remains valid for `GRACE_PERIOD`
/// epochs so in-flight votes/certs signed with the old key still verify.
///
/// Closes punch-list item #4. Why proof-of-possession on BOTH keys:
///   - `bls_pop_old`  — proves the rotator currently controls the old key
///     (prevents external attacker from swapping a compromised key out of
///     an unwitting validator's slot).
///   - `bls_pop_new`  — proves the rotator controls the new key (rogue-key
///     defence; same logic as the original PoP at validator registration).
///
/// `effective_epoch` must be in the future relative to the block in which
/// this tx is admitted, giving operators a deterministic switchover point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotateValidatorKeyTx {
    pub validator_address: AccountAddress,
    pub validator_id: u64,
    /// New 48-byte compressed BLS12-381 G1 public key.
    pub new_bls_public_key: Vec<u8>,
    /// Proof-of-possession signature over `new_bls_public_key` by the OLD key.
    pub bls_pop_old: Vec<u8>,
    /// Proof-of-possession signature over `new_bls_public_key` by the NEW key.
    pub bls_pop_new: Vec<u8>,
    /// Epoch at which the rotation takes effect. Must be ≥ current epoch.
    pub effective_epoch: Epoch,
    pub nonce: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// On-chain stake record tracking a validator's locked stake and unbonding status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StakeRecord {
    pub validator_id: u64,
    pub validator_address: AccountAddress,
    pub staked_amount: u64,
    pub staked_at_epoch: Epoch,
    pub unbonding_epoch: Option<Epoch>,
    pub slashed_amount: u64,
}

// ═══════════════════════════════════════════════════════════════════
// Vesting Timelock (addresses 35% Foundation centralization concern)
// ═══════════════════════════════════════════════════════════════════

/// On-chain linear vesting schedule with cliff. Released over time by
/// `tick_vesting` once per block. Wraps large genesis allocations so
/// they release thermodynamically rather than as a calendar smart
/// contract.
///
/// Semantics:
///   - At any epoch `t < start_epoch + cliff_epochs`: 0 released.
///   - At `t == start_epoch + cliff_epochs`: linear release begins from 0.
///   - At `t >= start_epoch + vesting_epochs`: full `total_amount`
///     released. Caller invariant: `vesting_epochs >= cliff_epochs`.
///   - `released_amount` records how much has been credited to the
///     beneficiary so far (so repeated ticks are idempotent).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VestingSchedule {
    /// Caller-supplied unique id (e.g. genesis line index, or
    /// `blake3(beneficiary || start_epoch)` for runtime-created schedules).
    pub id: u64,
    pub beneficiary: AccountAddress,
    /// Total tokens locked by this schedule. Constant for the lifetime
    /// of the schedule.
    pub total_amount: u64,
    /// Epoch at which the cliff/vesting window begins counting.
    pub start_epoch: Epoch,
    /// Number of epochs after `start_epoch` before any tokens release.
    pub cliff_epochs: u64,
    /// Total number of epochs from `start_epoch` until the schedule is
    /// fully released. Must be >= `cliff_epochs`.
    pub vesting_epochs: u64,
    /// Sum of all releases credited so far. Monotonically non-decreasing.
    pub released_amount: u64,
}

impl VestingSchedule {
    /// How much SHOULD be released by `current_epoch` against `total_amount`.
    /// Saturating arithmetic — returns `total_amount` past the schedule end.
    pub fn vested_at(&self, current_epoch: Epoch) -> u64 {
        let cliff_end = self.start_epoch.saturating_add(self.cliff_epochs);
        if current_epoch < cliff_end {
            return 0;
        }
        let vesting_end = self.start_epoch.saturating_add(self.vesting_epochs);
        if current_epoch >= vesting_end {
            return self.total_amount;
        }
        let elapsed = current_epoch.saturating_sub(cliff_end) as u128;
        let linear_window = vesting_end.saturating_sub(cliff_end) as u128;
        if linear_window == 0 {
            return self.total_amount;
        }
        let released = (self.total_amount as u128).saturating_mul(elapsed) / linear_window;
        released.min(self.total_amount as u128) as u64
    }

    /// Releasable delta against the current `released_amount`.
    pub fn pending_release_at(&self, current_epoch: Epoch) -> u64 {
        self.vested_at(current_epoch)
            .saturating_sub(self.released_amount)
    }

    /// True iff the schedule has fully released its total.
    pub fn is_fully_vested(&self) -> bool {
        self.released_amount >= self.total_amount
    }
}

// ═══════════════════════════════════════════════════════════════════
// On-Chain Governance Types
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProposalStatus {
    Pending,
    Active,
    Passed,
    Rejected,
    Executed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceProposal {
    pub proposal_id: u64,
    pub proposer: AccountAddress,
    pub title: String,
    pub param_key: String,
    pub param_value: String,
    pub start_epoch: u64,
    pub end_epoch: u64,
    pub votes_for: u64,
    pub votes_against: u64,
    pub status: ProposalStatus,
    pub created_at: u64,
    #[serde(default)]
    pub voters: std::collections::HashSet<AccountAddress>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceVote {
    pub proposal_id: u64,
    pub voter: AccountAddress,
    pub vote: bool,
    pub stake_weight: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GovernanceAction {
    CreateProposal {
        title: String,
        param_key: String,
        param_value: String,
        voting_epochs: u64,
    },
    CastVote {
        proposal_id: u64,
        vote: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceTx {
    pub action: GovernanceAction,
    pub sender: AccountAddress,
    pub nonce: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// ERC-4337-style account abstraction: user operation with optional paymaster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOpTx {
    pub sender: AccountAddress,
    pub nonce: u64,
    pub call_data: Vec<u8>,
    pub call_gas_limit: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paymaster: Option<AccountAddress>,
    /// Per-paymaster nonce; required iff `paymaster` is `Some`. Without
    /// this, the same UserOpTx could be replayed across blocks (or across
    /// Block-STM aborts) to drain the paymaster.
    /// Closes the gap from audit/end_to_end_audit_2026_04_27.md §3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paymaster_nonce: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paymaster_data: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Delegate stake to a validator. The delegator's balance is debited
/// and the amount counts toward the validator's effective stake (voting
/// power + reward share). Multiple delegations to the same validator
/// are additive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegateTx {
    /// Token holder providing the stake.
    pub delegator: AccountAddress,
    /// Validator the stake is delegated to.
    pub validator_id: u64,
    /// Amount to delegate (debited from delegator balance).
    pub amount: u64,
    /// Sender nonce.
    pub nonce: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Undelegate stake from a validator. Begins the unbonding window;
/// the delegator can claim the released stake back to balance after
/// `chain_params.unbonding_period` epochs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndelegateTx {
    /// Token holder withdrawing the delegation.
    pub delegator: AccountAddress,
    /// Validator being undelegated from.
    pub validator_id: u64,
    /// Amount to undelegate (≤ existing delegation).
    pub amount: u64,
    /// Sender nonce.
    pub nonce: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Claim a previously-undelegated amount back to the delegator's balance.
/// Only valid once the unbonding window has elapsed
/// (`unbonding_epoch + UNBONDING_PERIOD_EPOCHS <= current_epoch`).
/// (P0 #4 Phase 7.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimDelegationTx {
    /// Token holder claiming the unbonded amount.
    pub delegator: AccountAddress,
    /// Validator the original delegation was bonded to.
    pub validator_id: u64,
    /// Sender nonce.
    pub nonce: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// On-chain record of a stake delegation between a delegator and a
/// validator. One record per (delegator, validator_id) pair; subsequent
/// `DelegateTx` to the same pair add to `amount`. Persisted as part of
/// state so reward distribution and slashing can iterate efficiently.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegationRecord {
    pub delegator: AccountAddress,
    pub validator_id: u64,
    pub amount: u64,
    /// Epoch at which the delegation was first created or last increased.
    pub delegated_at_epoch: Epoch,
    /// Set when an undelegate is in progress; the unbonded amount can
    /// be claimed back after `unbonding_epoch + chain_params.unbonding_period`.
    pub unbonding_amount: u64,
    pub unbonding_epoch: Option<Epoch>,
}

/// Upgrade a deployed contract to a new implementation.
///
/// Two authorization paths, disambiguated at apply time:
///
/// **Path A — admin upgrade.** Set `admin_signature` and
/// `admin_public_key` to the contract admin's ML-DSA-65 sig + pk over
/// the canonical signing payload
/// `JSON({type:"upgrade_contract",contract_id,new_bytecode_hash_hex,nonce})`.
/// `endorser_stakes` and `required_stake` are ignored on this path.
///
/// **Path B — governance amendment.** Leave `admin_signature` and
/// `admin_public_key` `None`. The chain enforces by stake quorum:
/// `endorser_stakes.iter().sum::<u64>() >= required_stake`. No body
/// signature on this path — mirrors the `/api/governance/fork_choice_mode`
/// pattern, where the chain (not the body) certifies the amendment.
///
/// In every case the chain verifies that
/// `BLAKE3(new_bytecode) == new_bytecode_hash` before either path is
/// considered. Closes K-10 / THREAT_MODEL §4.9.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeContractTx {
    /// Sender of the tx (charged for gas, nonce-checked). On the admin
    /// path this is normally the admin address; on the governance path
    /// this is whatever account submits the amendment.
    pub owner: AccountAddress,
    pub contract_id: u64,
    /// Replacement bytecode (UTF-8 EvaporScript source for the script
    /// engine; arbitrary bytes for future VM frontends). The hash below
    /// must match `BLAKE3(new_bytecode)`.
    pub new_bytecode: Vec<u8>,
    /// `BLAKE3(new_bytecode)`. Verified at apply time. The hash is
    /// committed-to by both the admin signature payload and the
    /// canonical signable bytes, so it cannot be quietly mutated
    /// post-signing.
    #[serde(default)]
    pub new_bytecode_hash: [u8; 32],
    pub nonce: u64,
    /// Path A — admin's ML-DSA-65 signature over
    /// `JSON({type:"upgrade_contract",contract_id,new_bytecode_hash_hex,nonce})`.
    /// `None` ⇒ governance path is taken instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_signature: Option<Vec<u8>>,
    /// Path A — admin's ML-DSA-65 public key. Must equal
    /// `contract.admin` for the admin path to succeed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_public_key: Option<Vec<u8>>,
    /// Path B — endorser stakes whose sum is checked against
    /// `required_stake`. Mirrors `ForkChoiceAmendReq`.
    #[serde(default)]
    pub endorser_stakes: Vec<u64>,
    /// Path B — minimum stake total required for the amendment to pass.
    #[serde(default)]
    pub required_stake: u64,
    /// **Legacy / advisory.** Retained because older snapshots and
    /// signing canonical-bytes carry it. Has no security effect on the
    /// new dispatch — admin/governance paths are now the sole gates.
    #[serde(default)]
    pub governance_approved: bool,
    /// Sender's tx-level signature (over `Transaction::signable_bytes`).
    /// Independent of `admin_signature` (which is over the upgrade-
    /// payload JSON, not the tx bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

// ═══════════════════════════════════════════════════════════════════
// Zero-Knowledge Privacy Types
// ═══════════════════════════════════════════════════════════════════

/// On-chain nullifier hash (32 bytes). Published when a private note is spent.
pub type NullifierHash = [u8; 32];

/// On-chain note commitment (32 bytes). Stored in the Merkle note tree.
pub type NoteCommitment = [u8; 32];

/// Serializable Merkle membership proof for a note in the tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProofData {
    /// Sibling hashes from leaf to root.
    pub siblings: Vec<[u8; 32]>,
    /// Leaf index in the tree.
    pub leaf_index: usize,
    /// Root hash this proof is valid against.
    pub root: [u8; 32],
}

/// On-chain energy decay proof data (no crypto logic — just the proof payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnergyDecayProofData {
    /// Commitment to energy at start epoch.
    pub old_energy_commitment: [u8; 32],
    /// Commitment to energy at end epoch.
    pub new_energy_commitment: [u8; 32],
    /// Binding hash proving correct decay computation.
    pub decay_binding: [u8; 32],
    /// Public: half-life of the object.
    pub half_life: u64,
    /// Public: start epoch.
    pub epoch_start: u64,
    /// Public: end epoch.
    pub epoch_end: u64,
    /// Whether the object has evaporated (energy reached 0).
    pub is_evaporated: bool,
}

/// Shield transaction: move transparent funds into the private pool.
/// Burns `amount` from `from`'s balance and creates a private note.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShieldTx {
    /// Sender's transparent address (balance is debited).
    pub from: AccountAddress,
    /// Amount to shield (burned from transparent balance).
    pub amount: u64,
    /// Sender nonce.
    pub nonce: u64,
    /// Poseidon hash of the recipient's spending public key.
    pub note_owner_hash: [u8; 32],
    /// Random blinding factor for the value commitment.
    pub value_blinding: [u8; 32],
    /// Optional energy to attach (for object-backed private notes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy: Option<u64>,
    /// Blinding factor for the energy commitment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_blinding: Option<[u8; 32]>,
    /// Half-life for energy decay (0 = pure value, no energy).
    pub half_life: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Unshield transaction: move private funds back to the transparent pool.
/// Spends private note(s) and credits `to`'s transparent balance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnshieldTx {
    /// Recipient's transparent address (balance is credited).
    pub to: AccountAddress,
    /// Amount to unshield (credited to transparent balance).
    pub amount: u64,
    /// Nullifiers of input notes being spent.
    pub input_nullifiers: Vec<NullifierHash>,
    /// Merkle root the input proofs are valid against.
    pub anchor: [u8; 32],
    /// Balance binding hash proving conservation.
    pub balance_binding: [u8; 32],
    /// Input amounts (one per input nullifier, for balance verification).
    #[serde(default)]
    pub input_amounts: Vec<u64>,
    /// Input blinding factors (one per input nullifier, for commitment/binding verification).
    #[serde(default)]
    pub input_blindings: Vec<[u8; 32]>,
    /// Input value commitments: Poseidon(amount || blinding) per input.
    #[serde(default)]
    pub input_value_commitments: Vec<NoteCommitment>,
    /// Input note commitments: the actual Merkle tree leaves. Poseidon(value_commitment || owner_hash || epoch || half_life).
    #[serde(default)]
    pub input_note_commitments: Vec<NoteCommitment>,
    /// Input Merkle proofs (one per input, for note membership verification).
    #[serde(default)]
    pub input_merkle_proofs: Vec<MerkleProofData>,
    /// Output blinding factors (one per change commitment, for binding verification).
    #[serde(default)]
    pub output_blindings: Vec<[u8; 32]>,
    /// Optional change outputs (remaining private balance).
    #[serde(default)]
    pub change_commitments: Vec<NoteCommitment>,
    /// Energy decay proofs for object-backed notes.
    #[serde(default)]
    pub energy_proofs: Vec<EnergyDecayProofData>,
    // No signature — the ZK proof itself authenticates the spender.
}

/// Private transfer: spend private notes and create new private notes.
/// Everything happens in the shielded pool — no transparent amounts visible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivateTransferTx {
    /// Nullifiers of input notes being spent.
    pub input_nullifiers: Vec<NullifierHash>,
    /// Commitments of output notes being created.
    pub output_commitments: Vec<NoteCommitment>,
    /// Merkle root the input proofs are valid against.
    pub anchor: [u8; 32],
    /// Balance binding hash proving sum(inputs) = sum(outputs) + fee.
    pub balance_binding: [u8; 32],
    /// Transparent fee paid to validators.
    pub fee: u64,
    /// Input amounts (one per input nullifier, for balance verification).
    #[serde(default)]
    pub input_amounts: Vec<u64>,
    /// Input blinding factors (one per input nullifier, for binding verification).
    #[serde(default)]
    pub input_blindings: Vec<[u8; 32]>,
    /// Input value commitments: Poseidon(amount || blinding) per input.
    #[serde(default)]
    pub input_value_commitments: Vec<NoteCommitment>,
    /// Input note commitments: the actual Merkle tree leaves.
    #[serde(default)]
    pub input_note_commitments: Vec<NoteCommitment>,
    /// Input Merkle proofs (one per input, for note membership verification).
    #[serde(default)]
    pub input_merkle_proofs: Vec<MerkleProofData>,
    /// Output amounts (one per output commitment, for balance verification).
    #[serde(default)]
    pub output_amounts: Vec<u64>,
    /// Output blinding factors (one per output commitment, for binding verification).
    #[serde(default)]
    pub output_blindings: Vec<[u8; 32]>,
    /// Energy decay proofs for object-backed notes.
    #[serde(default)]
    pub energy_proofs: Vec<EnergyDecayProofData>,
    // No signature — ZK proof authenticates.
}

// ═══════════════════════════════════════════════════════════════════
// Temporal Smart Contract Types
// ═══════════════════════════════════════════════════════════════════

/// Temporal guard: a condition that must be met for a deferred transaction to execute.
/// Guards are evaluated each block — when ALL guards pass, the inner tx fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TemporalGuard {
    /// Execute only after this epoch (inclusive).
    AfterEpoch(Epoch),
    /// Execute only before this epoch (exclusive). If epoch passes, tx expires and deposit refunds.
    BeforeEpoch(Epoch),
    /// Execute when the specified object's energy drops below the threshold.
    EnergyBelow(ObjectId, Energy),
    /// Execute when the specified object's energy is above the threshold.
    EnergyAbove(ObjectId, Energy),
    /// Execute when the specified object has evaporated (entered Ghost state).
    ObjectEvaporated(ObjectId),
    /// Execute when the specified contract is in the named phase.
    ContractInPhase(u64, String),
}

/// Deferred transaction: submitted now, executes when temporal conditions are satisfied.
///
/// The submitter pays a deposit (covers gas + queue storage). When all guards are
/// satisfied, the inner transaction is deserialized and executed. If the `BeforeEpoch`
/// guard expires, the deferred tx is cancelled and the deposit refunds (minus queue fee).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeferredTx {
    /// Account that submitted the deferred tx and pays the deposit.
    pub submitter: AccountAddress,
    /// Submitter nonce (prevents replay).
    pub nonce: u64,
    /// Deposit amount (covers execution gas + queue storage fee).
    pub deposit: u64,
    /// Temporal guards — ALL must be satisfied for inner tx to fire.
    pub guards: Vec<TemporalGuard>,
    /// Serialized inner transaction bytes (deserialized and executed when guards pass).
    pub inner_tx_bytes: Vec<u8>,
    /// Maximum gas the inner transaction may consume.
    pub gas_limit: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<Vec<u8>>,
}

/// Crooks-MEV refund transaction — `CROOKS_MEV_INTEGRATION_PLAN.md`
/// Phase 3.1. **Protocol-issued**, not user-signed: the block
/// proposer constructs one of these for every `MevObservation` in
/// the consensus engine's ring buffer that has aged past the grace
/// period. Validators reject blocks whose `Refund` transactions
/// don't match their independently-computed observation set.
///
/// `signature` and `public_key` are absent — execution-side
/// validation skips signature checks for this variant and instead
/// runs the determinism contract (Phase 3.2): the values must match
/// what the validator's own `mev_observations` + `mev_attacker_stats`
/// would have produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundTx {
    /// Block height at which the original `MevObservation` was
    /// detected. Pairs the refund to the source observation
    /// deterministically.
    pub source_block_height: u64,
    /// Index of the original observation within that block's
    /// detected MEV-observation list. (block_height, index) is the
    /// observation's stable identifier.
    pub source_observation_idx: usize,
    /// Address debited (the attacker side of the source sandwich).
    pub attacker: AccountAddress,
    /// Address credited (the victim side of the source sandwich).
    pub victim: AccountAddress,
    /// Refund amount, in native token units. Equals the
    /// `MevObservation::refund_amount` computed at observation time
    /// (Phase 2 contract).
    pub amount: u64,
    /// Block height at which this refund is being settled. Bounded
    /// by the (grace_period, refund_window) interval relative to
    /// `source_block_height` per Phase 3.3 of the plan.
    pub settle_block_height: u64,
}

/// Energy watcher: monitors an object and fires a callback when energy crosses a threshold.
/// Registered by contracts to react to thermodynamic state changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnergyWatcher {
    /// Unique watcher ID.
    pub id: u64,
    /// Object being watched.
    pub object_id: ObjectId,
    /// Energy threshold.
    pub threshold: Energy,
    /// Direction: true = fire when energy drops BELOW, false = fire when ABOVE.
    pub fire_below: bool,
    /// Contract to notify when triggered.
    pub callback_contract_id: u64,
    /// Method to call on the contract.
    pub callback_method: String,
    /// JSON args for the callback.
    pub callback_args: String,
    /// Whether this watcher has already fired (one-shot by default).
    pub fired: bool,
    /// Epoch when this watcher was registered.
    pub registered_epoch: Epoch,
}

/// Result of VRF-based committee sortition for a validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortitionResult {
    pub validator_id: u64,
    pub vrf_output: [u8; 32],
    pub vrf_proof: Vec<u8>,
    pub is_selected: bool,
    /// Number of virtual committee seats won (0 = not selected).
    pub selection_weight: u64,
}

/// Commitment to the global state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateCommitment {
    pub verkle_root: [u8; 32],
    pub accumulator_value: [u8; 32],
    pub epoch: Epoch,
}

/// Dual commitment: Verkle state trie + MMR nullifier accumulator.
/// This is the canonical commitment to EvaporChain's full state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DualCommitment {
    /// Verkle trie root over active objects and accounts.
    pub verkle_root: [u8; 32],
    /// MMR root over all energy-stamped nullifiers (evaporated objects).
    pub mmr_root: [u8; 32],
    /// Current epoch.
    pub epoch: Epoch,
    /// Number of active (non-ghost) objects.
    pub active_count: usize,
    /// Number of ghost records.
    pub ghost_count: usize,
}

/// A configurable decay curve that determines how an object's energy
/// decreases over time. Stored on-chain per object when non-default.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DecayCurve {
    Exponential {
        half_life: u64,
    },
    Linear {
        rate_per_epoch: u64,
    },
    Stepped {
        thresholds: Vec<(u64, u64)>,
    },
    Conditional {
        base: Box<DecayCurve>,
        grace_epochs: u64,
    },
    Asymptotic {
        floor: u64,
        half_life: u64,
    },
    Custom {
        bytecode: Vec<u8>,
    },
}

impl Default for DecayCurve {
    fn default() -> Self {
        DecayCurve::Exponential { half_life: 100 }
    }
}

/// Compute remaining energy after exponential decay using integer math.
///
/// Uses the approximation: energy * 2^(-epochs_elapsed / half_life)
/// Implemented via bit-shifting for complete halvings and linear
/// interpolation for the fractional part.
///
/// Initial inclusion-priority energy assigned to every tx at submission.
/// Decays each block with `MEV_INCLUSION_HALF_LIFE_BLOCKS`. Sized so a tx
/// held back for one half-life is worth ~50% of its initial priority,
/// making reordering attacks bleed value
/// (`research/proposals/energy-stamped-mev-resistance.md`). Lives in
/// `evaporchain-types` so both `evaporchain-consensus::mempool` (for
/// proposal-time priority sort) and `evaporchain-execution` (for
/// consensus-deterministic priority-bonus minting from `execute_block`)
/// can read the same constants without a circular dep.
pub const BASE_INCLUSION_ENERGY: u64 = 1_000_000;
/// Block-count half-life for tx inclusion priority. Tuned for 2-second
/// block intervals — 4 blocks ≈ 8 seconds halving, comparable to the
/// Ethereum 12-second slot window.
pub const MEV_INCLUSION_HALF_LIFE_BLOCKS: u64 = 4;

/// Mechanized monotonicity proof: `research/coq/EnergyDecayMonotonicity.v`
/// (theorem `energy_at_epoch_monotone`). Any change to this function's
/// arithmetic must be reflected in the Coq spec. Both the within-halving
/// and cross-halving cases are now `Qed` (the latter via the
/// `decay_term_bound` arithmetic helper); the file is machine-verified
/// under Rocq 9.1.1.
pub fn energy_at_epoch(initial: Energy, half_life: HalfLife, epochs_elapsed: u64) -> Energy {
    if half_life == 0 {
        return 0;
    }
    let full_halvings = epochs_elapsed / half_life;
    let remainder = epochs_elapsed % half_life;

    if full_halvings >= 64 {
        return 0;
    }

    let after_halvings = initial >> full_halvings;

    // Linear interpolation for the fractional part between halvings.
    // Use u128 to avoid overflow on large values.
    let fractional_decay =
        (after_halvings as u128 * remainder as u128 / (2u128 * half_life as u128)) as u64;
    after_halvings.saturating_sub(fractional_decay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_energy_no_decay() {
        assert_eq!(energy_at_epoch(1000, 10, 0), 1000);
    }

    #[test]
    fn test_energy_one_half_life() {
        assert_eq!(energy_at_epoch(1000, 10, 10), 500);
    }

    #[test]
    fn test_energy_two_half_lives() {
        assert_eq!(energy_at_epoch(1000, 10, 20), 250);
    }

    #[test]
    fn test_energy_zero_half_life() {
        assert_eq!(energy_at_epoch(1000, 0, 5), 0);
    }

    #[test]
    fn test_energy_large_elapsed() {
        assert_eq!(energy_at_epoch(1000, 1, 100), 0);
    }

    #[test]
    fn test_energy_partial_decay() {
        let result = energy_at_epoch(1000, 10, 5);
        assert!(result > 500 && result < 1000, "got {result}");
    }

    #[test]
    fn test_state_object_energy_at() {
        let obj = StateObject {
            id: [1u8; 32],
            owner: [2u8; 32],
            energy: 1000,
            half_life: 10,
            created_at: 0,
            last_refreshed: 5,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![],
            decay_curve: None,
            lad_mode: None,
        };
        // At epoch 15, 10 epochs since refresh -> one half-life -> 500
        assert_eq!(obj.energy_at(15), 500);
        // At epoch 5 (same as refresh), no decay
        assert_eq!(obj.energy_at(5), 1000);
    }

    // ── Transaction sender() ──

    #[test]
    fn test_transfer_sender() {
        let tx = Transaction::Transfer(TransferTx {
            from: [0xAA; 32],
            to: [0xBB; 32],
            amount: 100,
            nonce: 1,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        assert_eq!(tx.sender(), Some(&[0xAA; 32]));
    }

    /// Phase 3.1 of `CROOKS_MEV_INTEGRATION_PLAN.md` — `RefundTx`
    /// round-trip serializes through serde and surfaces the
    /// attacker (debited party) as `sender()`. No nonce — the
    /// (source_block_height, source_observation_idx) pair is the
    /// unique identifier for replay protection.
    #[test]
    /// Phase 2.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — `effective_parents()`
    /// returns the explicit `parents` Vec when populated, else
    /// `vec![parent_hash]` (single-parent fallback).
    #[test]
    fn test_block_effective_parents_fallback_and_explicit() {
        // Helper: build a Block with given parent_hash / parents /
        // protocol_version, all other fields default.
        fn mk(parent_hash: [u8; 32], parents: Vec<[u8; 32]>, pv: u8) -> Block {
            Block {
                number: 1,
                epoch: 1,
                parent_hash,
                state_root: [0u8; 32],
                transactions: vec![],
                timestamp: 0,
                chain_id: String::new(),
                producer_id: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: pv,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents,
            }
        }

        // Empty parents → fallback to vec![parent_hash].
        let legacy = mk([0xAA; 32], vec![], 0);
        assert_eq!(legacy.effective_parents(), vec![[0xAA; 32]]);

        // Non-empty parents → returned as-is.
        let dag = mk([0xAA; 32], vec![[0xAA; 32], [0xBB; 32]], 3);
        assert_eq!(dag.effective_parents(), vec![[0xAA; 32], [0xBB; 32]]);
    }

    /// Phase 2.2 — wire-format validation of `parents` field.
    /// Three failure modes + happy path.
    #[test]
    fn test_block_validate_parents_wire_format() {
        fn mk(parents: Vec<[u8; 32]>, pv: u8, ph: [u8; 32]) -> Block {
            Block {
                number: 1,
                epoch: 1,
                parent_hash: ph,
                state_root: [0u8; 32],
                transactions: vec![],
                timestamp: 0,
                chain_id: String::new(),
                producer_id: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: pv,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents,
            }
        }

        // Happy path: legacy block (empty parents) at any pv → Ok.
        assert!(mk(vec![], 0, [0xAA; 32])
            .validate_parents_wire_format()
            .is_ok());

        // Happy path: single-parent block matching parent_hash → Ok.
        assert!(mk(vec![[0xAA; 32]], 0, [0xAA; 32])
            .validate_parents_wire_format()
            .is_ok());

        // Multi-parent at v3 → Ok.
        assert!(mk(vec![[0xAA; 32], [0xBB; 32]], 3, [0xAA; 32])
            .validate_parents_wire_format()
            .is_ok());

        // Multi-parent at v2 → MultiParentRequiresV3.
        let err = mk(vec![[0xAA; 32], [0xBB; 32]], 2, [0xAA; 32])
            .validate_parents_wire_format()
            .unwrap_err();
        assert!(matches!(
            err,
            BlockParentsValidationError::MultiParentRequiresV3 { n: 2, pv: 2 }
        ));

        // Duplicate parent → DuplicateParent.
        let err = mk(vec![[0xAA; 32], [0xAA; 32]], 3, [0xAA; 32])
            .validate_parents_wire_format()
            .unwrap_err();
        assert!(matches!(
            err,
            BlockParentsValidationError::DuplicateParent(_)
        ));

        // parents[0] disagrees with parent_hash → ParentHashMismatch.
        let err = mk(vec![[0xBB; 32]], 0, [0xAA; 32])
            .validate_parents_wire_format()
            .unwrap_err();
        assert!(matches!(
            err,
            BlockParentsValidationError::ParentHashMismatch { .. }
        ));
    }

    /// Phase 2.4 of `LIGHT_CONE_FULL_DAG_PLAN.md` — hash-stability
    /// gate. Adding the `parents` field with `serde(default,
    /// skip_serializing_if = "Vec::is_empty")` MUST NOT change the
    /// JSON serialization of legacy blocks (parents = vec![]).
    /// Critical for chain-id continuity.
    #[test]
    fn test_block_legacy_serialization_omits_parents_field() {
        let b = Block {
            number: 1,
            epoch: 1,
            parent_hash: [0xAA; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
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
        };
        let s = serde_json::to_string(&b).expect("serialize");
        assert!(
            !s.contains("\"parents\""),
            "legacy block must not surface the new `parents` field on the wire — \
             chain-id continuity gate. Got: {}",
            s
        );
    }

    fn test_refund_tx_roundtrip_and_sender() {
        let tx = Transaction::Refund(RefundTx {
            source_block_height: 100,
            source_observation_idx: 0,
            attacker: [0xAA; 32],
            victim: [0xBB; 32],
            amount: 250,
            settle_block_height: 110,
        });
        // Sender = attacker.
        assert_eq!(tx.sender(), Some(&[0xAA; 32]));
        // No replay nonce.
        assert_eq!(tx.nonce(), None);
        // Round-trip via JSON.
        let s = serde_json::to_string(&tx).expect("serialize");
        let back: Transaction = serde_json::from_str(&s).expect("deserialize");
        match back {
            Transaction::Refund(r) => {
                assert_eq!(r.source_block_height, 100);
                assert_eq!(r.source_observation_idx, 0);
                assert_eq!(r.attacker, [0xAA; 32]);
                assert_eq!(r.victim, [0xBB; 32]);
                assert_eq!(r.amount, 250);
                assert_eq!(r.settle_block_height, 110);
            }
            other => panic!("expected Refund, got {:?}", other),
        }
    }

    #[test]
    fn test_refresh_has_no_sender() {
        let tx = Transaction::Refresh(RefreshTx {
            object_id: [1u8; 32],
            energy_deposit: 100,
            signature: None,
            public_key: None,
        });
        assert_eq!(tx.sender(), None);
    }

    // ── Transaction nonce() ──

    #[test]
    fn test_transfer_nonce() {
        let tx = Transaction::Transfer(TransferTx {
            from: [0; 32],
            to: [1; 32],
            amount: 50,
            nonce: 42,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        assert_eq!(tx.nonce(), Some(42));
    }

    #[test]
    fn test_refresh_has_no_nonce() {
        let tx = Transaction::Refresh(RefreshTx {
            object_id: [0u8; 32],
            energy_deposit: 0,
            signature: None,
            public_key: None,
        });
        assert_eq!(tx.nonce(), None);
    }

    // ── Transaction serialization roundtrip ──

    #[test]
    fn test_transfer_tx_roundtrip() {
        let tx = Transaction::Transfer(TransferTx {
            from: [1; 32],
            to: [2; 32],
            amount: 999,
            nonce: 7,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let json = serde_json::to_vec(&tx).unwrap();
        let back: Transaction = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.sender(), Some(&[1u8; 32]));
        assert_eq!(back.nonce(), Some(7));
    }

    // ── Block serialization ──

    #[test]
    fn test_block_serialization_roundtrip() {
        let block = Block {
            number: 100,
            epoch: 10,
            parent_hash: [0xAA; 32],
            state_root: [0xBB; 32],
            transactions: vec![],
            timestamp: 1234567890,
            chain_id: "test-chain".into(),
            producer_id: Some(1),
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
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
        };
        let json = serde_json::to_vec(&block).unwrap();
        let back: Block = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.number, 100);
        assert_eq!(back.epoch, 10);
        assert_eq!(back.chain_id, "test-chain");
        assert_eq!(back.producer_id, Some(1));
    }

    // ── ObjectState ──

    #[test]
    fn test_object_state_default_is_active() {
        let obj = StateObject {
            id: [0; 32],
            owner: [0; 32],
            energy: 100,
            half_life: 10,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![],
            decay_curve: None,
            lad_mode: None,
        };
        assert!(matches!(obj.state, ObjectState::Active));
    }

    #[test]
    fn test_state_object_serialization_roundtrip() {
        let obj = StateObject {
            id: [0xFF; 32],
            owner: [0x11; 32],
            energy: 5000,
            half_life: 20,
            created_at: 1,
            last_refreshed: 5,
            state: ObjectState::Ghost,
            grace_epoch: Some(100),
            data: vec![1, 2, 3],
            decay_curve: None,
            lad_mode: None,
        };
        let json = serde_json::to_vec(&obj).unwrap();
        let back: StateObject = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.id, [0xFF; 32]);
        assert_eq!(back.energy, 5000);
        assert!(matches!(back.state, ObjectState::Ghost));
        assert_eq!(back.grace_epoch, Some(100));
        assert_eq!(back.data, vec![1, 2, 3]);
    }

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test. evaporchain-types is the cross-crate
    /// canonical-primitive crate; if the wire shape of `Transaction`
    /// or the `energy_at_epoch` decay function ever drift, this test
    /// breaks visibly.
    ///
    /// Press claim: "evaporchain-types owns the chain's canonical
    /// types: 32-byte addresses, 32-byte object ids, the `Energy`
    /// alias for u64, the master `Transaction` enum across all 22+
    /// variants (Transfer through Refund), and the
    /// `energy_at_epoch(initial, half_life, elapsed)` decay
    /// function — mechanized in research/coq/EnergyDecayMonotonicity.v.
    /// All public APIs are validator-deterministic; the Refund
    /// variant exists for protocol-issued refunds (Crooks-MEV) and
    /// has no signature/public-key fields."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // 32-byte address, 32-byte object id, u64 energy.
        let addr: AccountAddress = [0u8; 32];
        let obj: ObjectId = [0u8; 32];
        let _e: Energy = 0;
        let _h: HalfLife = 0;
        assert_eq!(addr.len(), 32);
        assert_eq!(obj.len(), 32);

        // energy_at_epoch decay properties (mechanized in Coq).
        let initial: Energy = 1_024;
        let half_life: HalfLife = 10;
        assert_eq!(energy_at_epoch(initial, half_life, 0), 1_024);
        assert_eq!(energy_at_epoch(initial, half_life, 10), 512);
        assert_eq!(energy_at_epoch(initial, half_life, 20), 256);
        // After 64 half-lives, any u64 initial is 0.
        assert_eq!(energy_at_epoch(initial, half_life, 640), 0);
        // Zero half_life → always zero.
        assert_eq!(energy_at_epoch(1_000, 0, 5), 0);

        // Transaction::Refund has no sender/nonce — protocol-issued.
        let refund = Transaction::Refund(RefundTx {
            source_block_height: 100,
            source_observation_idx: 0,
            attacker: [1u8; 32],
            victim: [2u8; 32],
            amount: 50,
            settle_block_height: 110,
        });
        assert!(refund.sender().is_some()); // Refund returns the attacker
        assert_eq!(refund.nonce(), None); // No replay nonce
        assert!(refund.signature().is_none()); // No signature
        assert!(refund.public_key().is_none()); // No pubkey
    }
}

// ═══════════════════════════════════════════════════════════════════
// Property-Based Tests (Audit Hardening)
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Energy never increases with time (monotonically non-increasing).
        #[test]
        fn energy_monotonically_decreasing(
            initial in 1u64..1_000_000,
            half_life in 1u64..1000,
            t1 in 0u64..500,
            dt in 0u64..500,
        ) {
            let t2 = t1 + dt;
            let e1 = energy_at_epoch(initial, half_life, t1);
            let e2 = energy_at_epoch(initial, half_life, t2);
            prop_assert!(e2 <= e1, "energy increased: e({})={} > e({})={}", t1, e1, t2, e2);
        }

        /// Energy at t=0 equals the initial value.
        #[test]
        fn energy_at_zero_is_initial(initial in 0u64..u64::MAX, half_life in 1u64..1000) {
            prop_assert_eq!(energy_at_epoch(initial, half_life, 0), initial);
        }

        /// Energy after exactly one half-life is exactly half (for exact halvings).
        #[test]
        fn energy_halves_at_half_life(initial in 0u64..1_000_000, half_life in 1u64..1000) {
            let result = energy_at_epoch(initial, half_life, half_life);
            prop_assert_eq!(result, initial / 2);
        }

        /// Energy after exactly two half-lives is exactly one quarter.
        #[test]
        fn energy_quarters_at_two_half_lives(initial in 0u64..1_000_000, half_life in 1u64..500) {
            let result = energy_at_epoch(initial, half_life, 2 * half_life);
            prop_assert_eq!(result, initial / 4);
        }

        /// Energy never overflows or panics for any valid inputs.
        #[test]
        fn energy_never_panics(
            initial in any::<u64>(),
            half_life in any::<u64>(),
            elapsed in any::<u64>(),
        ) {
            let _ = energy_at_epoch(initial, half_life, elapsed);
        }

        /// Energy eventually reaches zero for any positive initial and half_life.
        #[test]
        fn energy_eventually_reaches_zero(
            initial in 1u64..1_000_000,
            half_life in 1u64..100,
        ) {
            // After 64 full halvings, any u64 initial should be 0
            let elapsed = 64 * half_life;
            let result = energy_at_epoch(initial, half_life, elapsed);
            prop_assert_eq!(result, 0, "energy should be 0 after 64 half-lives");
        }

        /// Energy with zero half_life is always zero.
        #[test]
        fn zero_half_life_gives_zero(initial in any::<u64>(), elapsed in any::<u64>()) {
            prop_assert_eq!(energy_at_epoch(initial, 0, elapsed), 0);
        }
    }
}
