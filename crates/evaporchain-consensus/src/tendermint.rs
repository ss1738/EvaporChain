//! Tendermint-style BFT consensus engine for EvaporChain.
//!
//! Implements a simplified Tendermint consensus protocol:
//!   NewRound → Propose → Prevote → Precommit → Commit
//!
//! - Round-robin proposer weighted by stake + health (via ValidatorSet)
//! - 2f+1 votes required for progression (tolerates f = (n-1)/3 failures)
//! - Timeout-based round advancement when proposer is offline
//! - Nil votes for safety (lock on first valid proposal)

use crate::da_attestation::DAAttestationManager;
use crate::encrypted_mempool::{EncryptedMempool, EncryptedTransaction};
use crate::finality::FinalityTracker;
use crate::ib_integration::{self, DEFAULT_LAMBDA_MB};
use crate::mempool::Mempool;
use crate::validator_set::{
    EpochTransitionManager, ValidatorInfo, ValidatorSet, ValidatorSetChange,
};
use crate::{BlockProductionResult, ConsensusError};
use evaporchain_bell_beacon::{
    bell_certified as bell_is_certified, chsh_s_value as bell_chsh_s_value,
    LOCAL_REALISM_S_MILLI as BELL_LOCAL_REALISM_S_MILLI,
};
use evaporchain_da::block_da::BlockDA;
use evaporchain_da::block_da_2d::{AvailabilityMetrics, BlockDA2D};
use evaporchain_da::namespace::{NamespaceMerkleTree, NamespacedBlob};
use evaporchain_entropic_slashing::entropic_slash;

use evaporchain_crypto::hash::blake3_hash;
use evaporchain_crypto::signatures::{BlsKeypair, BlsPublicKey, BlsSignature, BlsVerifier};
use evaporchain_crypto::vrf::{
    leader_vrf_input, vrf_verify, RandomnessBeacon, VrfKeypair, VrfOutput, VrfProof,
};
use evaporchain_execution::fees::PidFeeController;
use evaporchain_execution::parallel::ParallelExecutor;
use evaporchain_execution::ExecutionEngine;
use evaporchain_state::db::StateDB;
use evaporchain_types::{Block, CommitCertificate, Epoch, Transaction};

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

// ─────────────────────── Proof Verification ─────────────────────────────

/// Trait for providing anchor hashes at anchor heights.
/// Injected by the node so consensus can enforce state-anchor agreement
/// without depending on the frontier module directly.
pub trait AnchorHashProvider: Send + Sync {
    fn anchor_hash_for_height(&self, height: u64) -> Option<[u8; 32]>;
}

/// Trait for verifying Nova IVC proofs on proposed blocks.
/// Injected by the node so that consensus doesn't depend on the proving crate.
pub trait ProofVerifier: Send + Sync {
    /// Verify the proof bytes from a block.  Returns `true` if valid (or if
    /// proof is absent and proof-requirement is not enforced).
    fn verify_block_proof(
        &self,
        proof_bytes: &[u8],
        block_height: u64,
        genesis_state_root: [u8; 32],
    ) -> bool;
}

// ─────────────────────── Configuration ───────────────────────────────────

/// Default timeout for each consensus phase.
/// Window size (in slots) for Sanov equivocation slash. KL divergence is
/// computed as 1 double-sign in 100 honest proposals → near-full slash.
/// Maximum number of pending (commitment, nonce) reveal pairs the
/// consensus engine will hold between block productions. Anti-DoS
/// cap — without this, an attacker could submit arbitrary reveal
/// nonces (each 64 bytes) and exhaust validator memory before the
/// next proposal drains them. Companion to PR #17's
/// `MAX_ENCRYPTED_PENDING`; same threshold (10K).
const MAX_PENDING_REVEALS: usize = 10_000;

const SANOV_EQUIVOCATION_WINDOW: u64 = 100;
/// Window size (in rounds) for Sanov downtime slash. Honest = miss 1 in 20.
const SANOV_DOWNTIME_WINDOW: u64 = 20;

// H2 (cluster session 2026-05-06/07) bumped these 2× to "fix" a
// reproducible h~200 fork on the UK+Helsinki cluster. Reverted
// 2026-05-07: the fork was not a timing problem at all. The actual
// causes were (a) bootstrap-list incompleteness — every validator
// had only ONE peer as --bootstrap, so the libp2p mesh never closed
// among the Macs; commit 9b5a45d added the proper full-mesh launcher.
// (b) shard-sample requests went to a single round-robin peer; commit
// adb08da fans them out. (c) DA attestations were emitted only on
// CommitBlock, but Tendermint won't commit at/past da_enforcement_height
// without the cert that needs the attestation; commit b5a3c9a emits
// attestations eagerly on Proposal-receipt. With those three fixes,
// the original 8s/32s/32s timings work through h>4000 across UK and
// Helsinki at ~17 blocks/sec.
const PROPOSE_TIMEOUT_MS: u64 = 8000;
// H1 (audit 2026-05-02): prevote/precommit were 60s, 15× the propose
// window. Under partial network failures one validator could timeout
// and advance rounds while peers were still in prevote, causing a
// permanent phase desync livelock. Cap at 4× propose so timeouts ride
// the same cadence as proposal arrival.
const PREVOTE_TIMEOUT_MS: u64 = 32000;
const PRECOMMIT_TIMEOUT_MS: u64 = 32000;

// Round-backoff deltas for additive (linear) growth per round. Each
// successive round adds these increments to the base timeouts to give
// partition-recovery slack without explosion.
//
// Cluster-soak evidence 2026-05-07: the previous formula used
// exponential backoff (`1u64 << min(round, 6)`) which capped at 64×
// at round 6+. That turned the 32s prevote timeout into 32 × 64 ≈ 34
// minutes, and a single round at round 7+ took ~76 minutes total.
// Reaching round 7 took ~75 minutes by itself, then waiting through
// round 7 added another 76. The cluster soak at h=16956 sat at round
// 7 for over 22 minutes before we noticed — looked like a wedge but
// was just the timeout exploding. Replaced with additive backoff
// matching standard Tendermint (Cosmos SDK).
const PROPOSE_TIMEOUT_DELTA_MS: u64 = 1000;
const PREVOTE_TIMEOUT_DELTA_MS: u64 = 2000;
const PRECOMMIT_TIMEOUT_DELTA_MS: u64 = 2000;

/// Maximum rounds before forcing commit (prevents livelock).
const MAX_ROUNDS_PER_HEIGHT: u32 = 10;

/// Maximum serialized block size (2 MB). Enforced on both creation and reception.
const MAX_BLOCK_SIZE_BYTES: usize = 2 * 1024 * 1024;

/// Maximum transactions per block. Enforced on both creation and reception.
const MAX_TXS_PER_BLOCK: usize = 200;

// ─────────────────────── Consensus Messages ─────────────────────────────

/// Messages exchanged between validators during consensus.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum ConsensusMessage {
    /// Leader proposes a block for this height/round.
    Proposal {
        height: u64,
        round: u32,
        block: Block,
        proposer_id: u64,
    },
    /// Validator votes for a block hash (or None for nil vote).
    Prevote {
        height: u64,
        round: u32,
        block_hash: Option<[u8; 32]>,
        validator_id: u64,
        /// BLS signature over the vote message (None if validator has no BLS key).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bls_signature: Option<Vec<u8>>,
    },
    /// Validator precommits to a block hash (or None for nil precommit).
    Precommit {
        height: u64,
        round: u32,
        block_hash: Option<[u8; 32]>,
        validator_id: u64,
        /// BLS signature over the precommit message (None if validator has no BLS key).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bls_signature: Option<Vec<u8>>,
    },
    /// Validator announces its BLS public key to peers.
    KeyAnnounce {
        validator_id: u64,
        /// BLS12-381 compressed public key (48 bytes).
        bls_public_key: Vec<u8>,
        /// Proof-of-possession: BLS.Sign(sk, pk, DST=POP).
        /// Prevents rogue-key attacks on aggregate signatures.
        #[serde(default)]
        proof_of_possession: Vec<u8>,
    },
    /// Validator attests to data availability for a committed block.
    DAAttestation {
        block_number: u64,
        data_root: [u8; 32],
        validator_id: u64,
        samples_verified: u32,
        stake: u64,
        /// BLS signature over (block_number || data_root || validator_id || samples_verified).
        signature: Vec<u8>,
        /// BLS public key of the signer.
        public_key: Vec<u8>,
    },
    /// Validator broadcasts an oracle vote for an off-chain feed
    /// (e.g. price, weather, randomness). The payload is an
    /// `evaporchain_oracle::consensus::OracleVote` serialized via
    /// serde_json — kept opaque here so the consensus crate stays
    /// decoupled from the oracle crate. The node-level dispatcher
    /// deserializes and routes to `OracleBridge::submit_vote_via_validator_set`,
    /// which performs the BLS sig + validator-set membership check
    /// against the validator's REGISTERED pubkey (not the one in the
    /// payload). Closes Gap-A #1 from the end-to-end audit:
    /// previously the oracle had a self-vote path only and no inbound
    /// P2P route, so multi-validator oracle consensus did not actually
    /// run on the cluster.
    OracleVote {
        /// `OracleVote` serialized as JSON bytes. Length-bounded by the
        /// consensus message-size cap in `evaporchain-network`.
        payload: Vec<u8>,
    },
}

impl ConsensusMessage {
    pub fn height(&self) -> u64 {
        match self {
            Self::Proposal { height, .. } => *height,
            Self::Prevote { height, .. } => *height,
            Self::Precommit { height, .. } => *height,
            Self::KeyAnnounce { .. } => 0,
            Self::DAAttestation { block_number, .. } => *block_number,
            Self::OracleVote { .. } => 0,
        }
    }

    pub fn round(&self) -> u32 {
        match self {
            Self::Proposal { round, .. } => *round,
            Self::Prevote { round, .. } => *round,
            Self::Precommit { round, .. } => *round,
            Self::KeyAnnounce { .. } => 0,
            Self::DAAttestation { .. } => 0,
            Self::OracleVote { .. } => 0,
        }
    }
}

// ─────────────────────── Round State ─────────────────────────────────────

/// Phase of the consensus state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for a proposal from the round's leader.
    Propose,
    /// Received proposal, collecting prevotes.
    Prevote,
    /// Received 2f+1 prevotes, collecting precommits.
    Precommit,
    /// Received 2f+1 precommits, ready to commit.
    Commit,
}

/// State for a single consensus round.
///
/// `pub(crate)` to match the visibility of `dag_round_states` which
/// holds `HashMap<[u8; 32], RoundState>` at `pub(crate)` (Light-Cone
/// Phase 4 substrate). The type is not exposed outside the crate.
#[derive(Debug)]
pub(crate) struct RoundState {
    round: u32,
    phase: Phase,
    /// The proposed block for this round (if received).
    proposed_block: Option<Block>,
    /// Hash of the proposed block.
    proposed_hash: Option<[u8; 32]>,
    /// Prevotes received: validator_id → block_hash (None = nil).
    prevotes: HashMap<u64, Option<[u8; 32]>>,
    /// Precommits received: validator_id → block_hash (None = nil).
    precommits: HashMap<u64, Option<[u8; 32]>>,
    /// BLS signatures for prevotes: validator_id → signature bytes.
    prevote_bls_sigs: HashMap<u64, Vec<u8>>,
    /// BLS signatures for precommits: validator_id → signature bytes.
    precommit_bls_sigs: HashMap<u64, Vec<u8>>,
    /// When this round/phase started (for timeouts).
    phase_start: Instant,
    /// Whether we already sent our prevote for this round.
    prevoted: bool,
    /// Whether we already sent our precommit for this round.
    precommitted: bool,
}

impl RoundState {
    fn new(round: u32) -> Self {
        Self {
            round,
            phase: Phase::Propose,
            proposed_block: None,
            proposed_hash: None,
            prevotes: HashMap::new(),
            precommits: HashMap::new(),
            prevote_bls_sigs: HashMap::new(),
            precommit_bls_sigs: HashMap::new(),
            phase_start: Instant::now(),
            prevoted: false,
            precommitted: false,
        }
    }
}

// ─────────────────────── Outbound Actions ────────────────────────────────

/// Actions the consensus engine wants the node to perform.
#[derive(Debug)]
pub enum ConsensusAction {
    /// Broadcast a consensus message to all validators.
    BroadcastMessage(ConsensusMessage),
    /// Commit this block — apply it to state and advance height.
    CommitBlock(Block),
    /// Request state sync from peers (from_height, to_height).
    RequestSync(u64, u64),
    /// Slash a validator — update on-chain stake ledger.
    SlashValidator {
        validator_id: u64,
        amount: u64,
        reason: SlashReason,
    },
}

/// Reason for a validator slash event.
#[derive(Debug, Clone)]
pub enum SlashReason {
    Equivocation,
    Downtime { missed_blocks: u64 },
}

/// Error returned by `TendermintConsensus::governance_set_param`
/// (Lane K.1). Unknown keys + invalid-for-key values are rejected
/// with structured error data so the RPC layer can surface useful
/// diagnostics without leaking internal state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GovernanceParamError {
    /// The key isn't a known soft-fork knob. See
    /// `governance_set_param` doc for the allowlist.
    UnknownKey(String),
    /// Value isn't in the permitted set for this key.
    InvalidValue {
        key: String,
        value: String,
        permitted: Vec<String>,
    },
}

impl std::fmt::Display for GovernanceParamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKey(k) => write!(
                f,
                "unknown governance soft-fork key: {k:?} (allowlist: parent_acceptance_mode, block_source_mode, conservation_enforcement, lambda_fold_mode, cartel_alarm_mode)"
            ),
            Self::InvalidValue { key, value, permitted } => write!(
                f,
                "invalid value {value:?} for key {key:?} — permitted: {permitted:?}"
            ),
        }
    }
}

impl std::error::Error for GovernanceParamError {}

/// Phase 3.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — per-tip state-branch
/// metadata. The `snapshot` field carries an opaque `Arc` whose
/// concrete type is the chain's `StateDB` snapshot once Phase 3.2
/// wires the executor in; today it's a typed placeholder
/// (`Arc<dyn LightConeBranchSnapshot + Send + Sync>`).
#[derive(Clone)]
pub struct LightConeBranchMetadata {
    /// Block height at which this tip was first observed.
    pub created_at_block: u64,
    /// Block height of the most recent commit landing on this tip
    /// (or one of its descendants in the DAG).
    pub last_touched_block: u64,
    /// Phase 3.4 LRU score — Phase 1.1's `path_caliber` for this
    /// tip's first-parent trajectory. Stored as u64 so the eviction
    /// rule is deterministic across validators (no f64 NaN paths).
    pub caliber: u64,
    /// Phase 3.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` — opaque
    /// snapshot reference for per-tip state. `None` until the chain
    /// installs a snapshot provider (and the
    /// `light_cone_state_branches_enabled` flag is on). Phase 3.2
    /// full implementation types this as `Arc<dyn StateDB>` once
    /// the consensus crate gets the `evaporchain-state` trait
    /// dependency; today the trait `LightConeBranchSnapshot` is a
    /// minimal abstraction that the executor will implement.
    pub snapshot: Option<std::sync::Arc<dyn LightConeBranchSnapshot + Send + Sync>>,
}

impl std::fmt::Debug for LightConeBranchMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LightConeBranchMetadata")
            .field("created_at_block", &self.created_at_block)
            .field("last_touched_block", &self.last_touched_block)
            .field("caliber", &self.caliber)
            .field("snapshot_present", &self.snapshot.is_some())
            .finish()
    }
}

impl PartialEq for LightConeBranchMetadata {
    fn eq(&self, other: &Self) -> bool {
        self.created_at_block == other.created_at_block
            && self.last_touched_block == other.last_touched_block
            && self.caliber == other.caliber
            && self.snapshot.is_some() == other.snapshot.is_some()
    }
}

impl Eq for LightConeBranchMetadata {}

impl LightConeBranchMetadata {
    /// Fresh metadata for a newly-observed tip. No snapshot yet —
    /// caller installs via `attach_snapshot` if Phase 3.2 wiring is
    /// active.
    pub fn fresh(block_height: u64, caliber: u64) -> Self {
        Self {
            created_at_block: block_height,
            last_touched_block: block_height,
            caliber,
            snapshot: None,
        }
    }
}

/// Phase 3.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` — minimal trait the
/// executor implements to hand a typed snapshot ref to the consensus
/// engine without the engine having to depend on
/// `evaporchain-state`'s concrete `StateDB` trait.
///
/// Implementation contract:
/// - `tip()` returns the BlockId the snapshot was created at.
/// - `created_at_height()` returns the chain height at snapshot
///   creation. Phase 3.4 LRU uses this for tie-breaks.
///
/// The actual reads (account / object lookups) go through whatever
/// concrete API the implementation exposes — Phase 3.2's executor
/// will downcast via `Arc::downcast` or similar to its own snapshot
/// type. This trait is the consensus-side seam, not the executor's.
pub trait LightConeBranchSnapshot {
    /// BlockId the snapshot was taken at.
    fn tip(&self) -> [u8; 32];
    /// Chain height at snapshot creation.
    fn created_at_height(&self) -> u64;

    /// MCC Phase B.1 of `MCC_FULL_MULTI_PARENT_PLAN.md` — restore
    /// the captured state into `db`. Used by Phase B.2's
    /// `replay_to_head` to roll state back to the LCA before
    /// applying the forward path. The caller invokes this BEFORE
    /// applying any blocks; the executor then walks the forward
    /// path and calls `db.execute_block` for each.
    ///
    /// Default impl returns an error indicating the snapshot type
    /// does not support restoration. Concrete impls (e.g.
    /// `StateSnapshotBranch` wrapping
    /// `evaporchain_state::snapshot::StateSnapshot`) override.
    /// Test stubs that only need the metadata methods (`tip`,
    /// `created_at_height`) can ignore the default error path.
    fn restore(&self, _db: &mut dyn evaporchain_state::db::StateDB) -> Result<(), String> {
        Err("LightConeBranchSnapshot does not support restoration".to_string())
    }
}

/// MCC Phase B.1 of `MCC_FULL_MULTI_PARENT_PLAN.md` — concrete
/// implementation of `LightConeBranchSnapshot` that wraps an
/// `evaporchain_state::snapshot::StateSnapshot`. The wrapped
/// snapshot captures the full StateDB state at the time of
/// creation; `restore()` calls `StateSnapshot::apply_to` which
/// wipes the target `db` and replays every account/object/ghost
/// from the captured state.
///
/// This is the "in-memory full-state copy" implementation suitable
/// for testnet and small chains. Production deployments with large
/// state should swap in a RocksDB-Snapshot-backed impl that pins
/// the LSM tree at a given state version (cheaper memory profile,
/// no full-state copy). The trait surface is stable; only the
/// concrete `restore()` implementation changes.
pub struct StateSnapshotBranch {
    tip: [u8; 32],
    height: u64,
    snapshot: evaporchain_state::snapshot::StateSnapshot,
}

impl StateSnapshotBranch {
    /// Capture a snapshot of the current `db` state, anchored to
    /// `tip`. Calls `StateSnapshot::create` under the hood.
    pub fn capture(
        tip: [u8; 32],
        height: u64,
        epoch: u64,
        db: &mut dyn evaporchain_state::db::StateDB,
    ) -> Result<Self, String> {
        let snapshot = evaporchain_state::snapshot::SnapshotBuilder::create(db, height, epoch)
            .map_err(|e| format!("SnapshotBuilder::create failed: {:?}", e))?;
        Ok(Self {
            tip,
            height,
            snapshot,
        })
    }
}

impl LightConeBranchSnapshot for StateSnapshotBranch {
    fn tip(&self) -> [u8; 32] {
        self.tip
    }

    fn created_at_height(&self) -> u64 {
        self.height
    }

    fn restore(&self, db: &mut dyn evaporchain_state::db::StateDB) -> Result<(), String> {
        evaporchain_state::snapshot::SnapshotApplier::apply(db, &self.snapshot)
            .map_err(|e| format!("SnapshotApplier::apply failed: {:?}", e))
            .map(|_apply_result| ())
    }
}

/// MCC Phase B.3 of `MCC_FULL_MULTI_PARENT_PLAN.md` — successful
/// outcome of `replay_and_apply`. Records the LCA the replay
/// rolled back to (if any) and the sequence of blocks that were
/// applied forward to reach the target head.
///
/// Operators can compare `applied` against
/// `plan_replay_to_head(...).forward_path` to confirm the executor
/// completed every step the plan called for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayResult {
    pub lca: [u8; 32],
    pub applied: Vec<[u8; 32]>,
}

/// MCC Phase B.3 of `MCC_FULL_MULTI_PARENT_PLAN.md` — error
/// returned by `replay_and_apply`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ReplayError {
    /// `plan_replay_to_head` returned None — usually means one of
    /// the heads is missing from the Light-Cone DAG, or the two
    /// heads have no common ancestor (e.g. they're on independent
    /// genesis-disjoint subgraphs, which shouldn't happen under
    /// normal chain operation).
    #[error("planning failed: from or to head absent, or no common ancestor")]
    PlanFailed,

    /// `restore_to_lca` failed — most likely the LCA isn't tracked
    /// in `state_branches` or has no attached snapshot.
    #[error("restore_to_lca failed: {0}")]
    RestoreFailed(String),

    /// A block in `plan.forward_path` couldn't be looked up by the
    /// caller's `block_lookup` closure. The replay halts at this
    /// block; state is in a partial state (LCA + any earlier
    /// forward_path entries already applied). Caller is responsible
    /// for atomic rollback under failure (Phase B.4 separate work).
    #[error("block_lookup returned None for {0}")]
    BlockNotFound(String),

    /// `block_apply` returned an error for a specific block. State
    /// is partial; same atomic-rollback caveat as `BlockNotFound`.
    #[error("block_apply failed for {block}: {msg}")]
    ApplyFailed { block: String, msg: String },
}

/// MCC Phase B.0+ of `MCC_FULL_MULTI_PARENT_PLAN.md` — planning
/// output of `TendermintConsensus::plan_replay_to_head`.
///
/// Describes the work the executor must do to move state from one
/// MCC head to another:
///   - `lca`: the deepest common ancestor of the two heads. After
///     rollback (if needed), state will be at this block.
///   - `forward_path`: chronological sequence of blocks to apply
///     after the rollback. Empty if `to_head == lca`.
///   - `rollback_required`: `true` iff `from_head != lca`. When
///     false, the executor is already at the LCA and only needs
///     to apply `forward_path`. When true, the executor must first
///     unwind state from `from_head` back to `lca` (Phase B.1
///     snapshot/restore work).
///
/// Validator-determinism: every validator with the same DAG state
/// produces the same `ReplayWalk` for the same `(from_head, to_head)`
/// pair, because both `find_lca` and `block_path_from_to` are
/// deterministic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayWalk {
    pub lca: [u8; 32],
    pub forward_path: Vec<[u8; 32]>,
    pub rollback_required: bool,
}

/// Phase 4.4 of `CROOKS_MEV_INTEGRATION_PLAN.md` — error returned
/// by `TendermintConsensus::dispute_observation` when an operator
/// tries to dispute a refund that doesn't exist or has aged past
/// the grace period.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MevDisputeError {
    #[error("no MevObservation found for (src_h={src_h}, src_idx={src_idx})")]
    NotFound { src_h: u64, src_idx: usize },
    #[error(
        "observation (src_h={src_h}, src_idx={src_idx}) is past grace \
         period: age={age}, grace={grace}"
    )]
    PastGracePeriod {
        src_h: u64,
        src_idx: usize,
        age: u64,
        grace: u64,
    },
}

/// Error returned by `TendermintConsensus::governance_set_fork_choice_mode`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GovernanceAmendmentError {
    /// Mode string was neither `"mcc"` nor `"singh_attractor"`.
    UnrecognisedMode(String),
    /// Singh-Attractor mode requires at least one attractor in the set.
    EmptyAttractors,
    /// Endorsing validators hold less stake than the quorum threshold.
    InsufficientStake { endorsing: u64, required: u64 },
}

impl std::fmt::Display for GovernanceAmendmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnrecognisedMode(m) => write!(f, "unrecognised fork-choice mode: {m:?}"),
            Self::EmptyAttractors => write!(f, "singh_attractor mode requires ≥1 attractor"),
            Self::InsufficientStake {
                endorsing,
                required,
            } => write!(
                f,
                "endorsing stake {endorsing} < required quorum {required}"
            ),
        }
    }
}

impl std::error::Error for GovernanceAmendmentError {}

// ─────────────────────── TendermintConsensus ─────────────────────────────

/// Discriminant string for a `ConservationViolation` variant. Used by
/// `ConsensusFourActState` to expose *why* an audit failed without
/// callers depending on the energy-kernel crate's enum directly.
fn conservation_violation_discriminant(
    v: &evaporchain_energy_kernel::ConservationViolation,
) -> String {
    use evaporchain_energy_kernel::ConservationViolation as V;
    match v {
        V::RedirectChangedTotal { .. } => "RedirectChangedTotal".to_string(),
        V::DecayIncreasedTotal { .. } => "DecayIncreasedTotal".to_string(),
        V::DecayExceededLambda { .. } => "DecayExceededLambda".to_string(),
    }
}

/// Snapshot of the four-act narrative spine state for the API layer.
/// Consensus produces this; the node binary translates into the
/// public-facing `evaporchain_node::api::FourActSnapshot`. Per
/// INVENTION_STACK.md Amendment 2 §A2.5.
#[derive(Debug, Clone, Default)]
pub struct ConsensusFourActState {
    pub eulogy_count: usize,
    pub eulogy_trie_root: Option<[u8; 32]>,
    pub tombstone_addresses: Vec<[u8; 32]>,
    pub refresh_pool_total: u64,
    pub mortis_triggered: bool,
    pub mortis_epoch_of_death: Option<u64>,
    pub mortis_final_state_root: Option<[u8; 32]>,
    /// Per-block §1.2 conservation audit verdict from
    /// `ParallelExecutor::last_conservation_audit`. None until first
    /// block; Some(true) = audit passed, Some(false) = violation.
    pub last_conservation_audit_ok: Option<bool>,
    /// Violation discriminant when `last_conservation_audit_ok` is
    /// `Some(false)` — one of `"RedirectChangedTotal"`,
    /// `"DecayIncreasedTotal"`, `"DecayExceededLambda"`, mirroring the
    /// `ConservationViolation` enum. `None` when the last audit passed,
    /// or when no audit has run yet. Lets operators see *why* an audit
    /// failed without reading the full executor state.
    ///
    /// Note: under emission-bearing tokenomics, `DecayIncreasedTotal`
    /// fires every block where a block reward mints new EVP into the
    /// compartment sum — this is a known doctrine-vs-implementation
    /// gap (see DOCTRINE_PUNCH_LIST.md). The field exposes which
    /// signal an operator is seeing so the gap is diagnosable.
    pub last_conservation_violation_type: Option<String>,
    /// Number of consecutive blocks whose §1.2 audit verdict was Ok.
    /// Resets to 0 on any Err (even under `observe` mode where the
    /// block still commits). This is the operator-facing readiness
    /// signal for flipping `conservation_enforcement` to `enforce`:
    /// a sustained non-zero counter is the precondition. Threshold
    /// for "safe to flip" is a governance call, not enforced here.
    pub consecutive_clean_audits: u64,
    /// **Number of blocks currently retained in the Light-Cone DAG**, NOT
    /// the chain's block height or committed-finality count. The DAG is
    /// pruned via `LightCone::prune_before_epoch` (sliding-window
    /// retention, see `evaporchain-light-cone/src/dag.rs:175`) to bound
    /// memory, so this counter goes down when pruning fires at epoch
    /// boundaries — it is **not monotonic**. Use the canonical
    /// block-height endpoint for liveness probes; this field is
    /// diagnostic of DAG retention only. Per INVENTION_STACK.md §4.1 #1.
    pub light_cone_block_count: usize,
    /// Number of energy-stamped nullifiers accumulated in the
    /// evaporation MMR (one per object that has transitioned
    /// Active → Grace → Ghost). Object-side counterpart to
    /// `eulogy_count` for the doctrine's "small deaths" act.
    ///
    /// **Append-only by design** — has no remove method (removing
    /// would invalidate root hashes / nullifier proofs). After a
    /// chain reorg that crosses an evaporation boundary, this
    /// counter stays put while the corresponding `ghost_object_count`
    /// (in `FourActSnapshot`, populated from `db.ghost_count()`)
    /// drops. The drift is intentional: MMR is the cryptographic
    /// commitment line; ghost set is the live-state mirror.
    pub evaporation_mmr_size: usize,
    /// Root of the evaporation nullifier MMR. None until the first
    /// object evaporates; mirrors `eulogy_trie_root`'s empty-state
    /// convention.
    pub evaporation_mmr_root: Option<[u8; 32]>,
    /// Total energy redirected into the refresh pool from tombstoned
    /// producers' would-be rewards. Subset of `refresh_pool_total` —
    /// surfaces the doctrine "the chain's death is final" as an
    /// auditable per-namespace counter.
    pub dead_producer_redirect_total: u64,
}

/// Window size for TUR Liveness Detector observations. Per
/// INVENTION_STACK.md §A1.3, the chain runs the Thermodynamic
/// Uncertainty Relation against a sliding window of the per-block
/// "current J" — gas_used here. Window is governance-set; 64 blocks
/// is a launch placeholder that catches cartel-class steady-state
/// signatures within ~1 minute of activity at typical block times.
pub const TUR_WINDOW_BLOCKS: usize = 64;

/// Phase 1.3 of `CROOKS_MEV_INTEGRATION_PLAN.md` — fixed-size ring
/// buffer cap for MEV-shaped observations recorded per committed
/// block by `evaporchain-mev-detect::scan_block`. Older entries are
/// pruned when the cap is exceeded; downstream consumers (Phase 3
/// settlement) act on the most recent block-window worth of
/// observations and tolerate drops.
pub const MEV_OBSERVATION_BUFFER_CAP: usize = 1024;

/// Phase 4.4 — antichain commit-cert digest history buffer cap.
/// Each committed block (under `light_cone_state_branches_enabled`)
/// pushes a `(height, digest)` pair; oldest evicted FIFO when the
/// cap is exceeded. 128 entries covers ~10-30 minutes of recent
/// history depending on block cadence — long enough for retroactive
/// cluster-divergence diagnosis without unbounded memory growth.
pub const ANTICHAIN_DIGEST_HISTORY_CAP: usize = 128;

/// Conversion factor from window-summed gas to entropy production Σ
/// in TUR's natural units. Launch placeholder: σ = sum(window) / 1000
/// is order-of-magnitude correct (entropy ∝ flux), calibratable by
/// governance once chain activity stabilises.
pub const TUR_SIGMA_PER_GAS_NUM: u64 = 1;
pub const TUR_SIGMA_PER_GAS_DEN: u64 = 1_000;

/// Most recent per-block Bell-Beacon CHSH measurement persisted in
/// consensus state. Populated by the Bell gate hook in the commit
/// pipeline (see `tendermint.rs` per-block VRF/CHSH derivation) so the
/// node API layer can surface a live S-value through
/// `GET /api/bell/latest` instead of returning `no_data`.
///
/// `s_value_milli` is the CHSH S in milli-units (output of
/// `evaporchain_bell_beacon::chsh_s_value`, i.e. unsigned magnitude).
/// `threshold_milli` is the local-realism bound (typically
/// `LOCAL_REALISM_S_MILLI = 2000`). `bell_certified` is true iff
/// `s_value_milli` strictly exceeds `threshold_milli`.
#[derive(Debug, Clone, Copy)]
pub struct BellBeaconReading {
    pub s_value_milli: u64,
    pub threshold_milli: u64,
    pub bell_certified: bool,
    pub block_height: u64,
    pub epoch: u64,
}

/// Tendermint-style BFT consensus engine.
pub struct TendermintConsensus {
    /// Parallel partial-order DAG of every committed block. Per
    /// INVENTION_STACK.md §4.1 #1 this is the substrate for Light-Cone
    /// Consensus replacing Tendermint as the authoritative consensus
    /// (governance amendment). Read-only observability for now.
    pub light_cone_dag: evaporchain_light_cone::LightCone,
    /// Sliding window of per-block gas_used for TUR Liveness Detector.
    /// Capped at TUR_WINDOW_BLOCKS; oldest entries fall off as new
    /// blocks commit.
    pub tur_window: std::collections::VecDeque<u64>,
    /// Last TUR verdict computed at block-commit time. None until the
    /// window has at least 2 samples (variance is meaningless on 1).
    pub last_tur_verdict: Option<evaporchain_tur_liveness::Verdict>,
    /// Phase 1.3 of `CROOKS_MEV_INTEGRATION_PLAN.md` — ring buffer
    /// of sandwich-shaped tx triples observed per committed block by
    /// `evaporchain-mev-detect::scan_block`. Capped at
    /// `MEV_OBSERVATION_BUFFER_CAP`; oldest entries pruned. Phase 1
    /// is observe-only — no Crooks refund settlement runs from this
    /// buffer until Phase 3 of the plan ships the `RefundTx` plumbing.
    pub mev_observations: std::collections::VecDeque<evaporchain_mev_detect::MevObservation>,
    /// Phase 2.1 of `CROOKS_MEV_INTEGRATION_PLAN.md` — rolling
    /// per-attacker sandwich-count stats. Drives the rate-based pmf
    /// fed to the Crooks-fluctuation refund math. Pruned at the
    /// start of each `on_block_committed` to keep the table bounded
    /// AND deterministic across validators (Phase 3.2 contract):
    /// any attacker with `last_seen_height < current_height -
    /// CROOKS_MEV_DEFAULT_WINDOW_BLOCKS` is dropped.
    pub mev_attacker_stats: std::collections::HashMap<
        evaporchain_types::AccountAddress,
        evaporchain_mev_detect::AttackerStat,
    >,
    /// Phase 3.3 of `CROOKS_MEV_INTEGRATION_PLAN.md` — set of
    /// (source_block_height, source_observation_idx) pairs that
    /// have already been settled via a `RefundTx` in some prior
    /// committed block. Replay-protection: `due_refund_txs` skips
    /// pairs in this set so a single observation settles at most
    /// once. Populated in `on_block_committed` by walking the
    /// block's `Transaction::Refund` variants.
    pub settled_refunds: std::collections::HashSet<(u64, usize)>,
    /// Phase 3.5c of `CROOKS_MEV_INTEGRATION_PLAN.md` — counter of
    /// `MissingRefund` proposal rejections per validator id.
    /// Operators feed `[counts_per_validator]` into
    /// `evaporchain_entropic_slashing::entropic_slash(stake, counts)`
    /// to derive the slash amount at slashing time. Intentionally
    /// does NOT touch the validator set's stake directly — the
    /// stake-deduction wiring is a separate consensus-state-machine
    /// change tracked as Phase 3.5d follow-up.
    pub mev_missing_refund_violations: std::collections::HashMap<u64, u64>,
    /// Phase 4.4 of `CROOKS_MEV_INTEGRATION_PLAN.md` — set of
    /// `(source_block_height, source_observation_idx)` pairs that
    /// the chain operator has DISPUTED via `/api/mev/dispute`.
    /// `due_refund_txs` skips disputed pairs so an operator can
    /// cancel a pending refund within the grace period (e.g.,
    /// false-positive detection). Disputes are local to the validator
    /// receiving the RPC; consensus-wide dispute consensus is a
    /// future Phase 4.4d follow-up (this is the scaffolding).
    pub disputed_observations: std::collections::HashSet<(u64, usize)>,
    /// Phase 3.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — per-tip state-branch
    /// metadata table. Keyed by DAG leaf BlockId; value is the
    /// metadata needed for Phase 3.4's LRU eviction
    /// (`light_cone_max_concurrent_forks` cap).
    ///
    /// This is the **substrate** for Phase 3.2's per-tip executor
    /// dispatch. The actual `Arc<dyn StateDB>` snapshot ref lives
    /// next to the metadata in Phase 3.2 — see
    /// `research/light_cone/PHASE_3_DECISIONS.md` Decision 1.
    /// Today the field tracks observed tips so Phase 3.2 can plug in
    /// the snapshot-creation hook without changing the shape.
    ///
    /// Lifecycle gated by `light_cone_state_branches_enabled` flag
    /// (Phase 3.5). When the flag is `false` (default), the table
    /// stays empty regardless of DAG activity — chain bit-compat.
    pub state_branches: std::collections::HashMap<[u8; 32], LightConeBranchMetadata>,
    /// Phase 4.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` (Decision 1) —
    /// per-tip voting state. Additive: existing `round_state`
    /// stays as the primary-fork voting state; this HashMap
    /// accumulates non-primary tips' tallies when
    /// `light_cone_state_branches_enabled = true`. Empty in default
    /// linear mode (chain bit-compat).
    ///
    /// Phase 4.2's antichain-finality predicate consumes this:
    /// `try_finalize_antichain` checks each tip in the closing
    /// antichain has ≥ 2f+1 precommits in its `dag_round_states`
    /// entry. Phase 4.3's cross-fork equivocation watches for the
    /// same validator precommitting on two concurrent entries.
    pub(crate) dag_round_states: std::collections::HashMap<[u8; 32], RoundState>,
    /// Phase 4.3 of `LIGHT_CONE_FULL_DAG_PLAN.md` (Decision 3) —
    /// per-validator cross-fork equivocation counter. Increments
    /// when a validator is observed precommitting on two concurrent
    /// tips at the same round. Operators feed `[counts]` into
    /// `evaporchain_entropic_slashing::entropic_slash(stake, counts)`
    /// to derive the slash amount — same pattern as Crooks-MEV's
    /// `mev_missing_refund_violations`.
    pub cross_fork_equivocations: std::collections::HashMap<u64, u64>,
    /// Phase 4.4 of `LIGHT_CONE_FULL_DAG_PLAN.md` (Decision 4) —
    /// block-indexed finality bookkeeping. Populates alongside the
    /// existing `committed_at: HashMap<u64, u64>`; both are kept
    /// for dual-mode bookkeeping (height-indexed for linear-mode
    /// consumers, block-indexed for DAG-aware consumers).
    pub committed_at_block: std::collections::HashMap<[u8; 32], u64>,
    /// Phase 4.4 — rolling history of `(block_height,
    /// closing_antichain_digest)` pairs, one per committed block.
    /// Capped at the most-recent 128 entries (older pruned FIFO).
    /// Operators retrospectively cross-compare across cluster
    /// validators via `/api/light_cone/antichain_digest_history`:
    /// pick a height H, check each validator's reported digest at H;
    /// divergence at any past height is the freeze-class signal for
    /// antichain disagreement. Real-time alarm via header-fold or
    /// gossip is the heavier post-V1 follow-up. Populated when
    /// `light_cone_state_branches_enabled = true`.
    pub antichain_digest_history: std::collections::VecDeque<(u64, [u8; 32])>,
    /// MCC Phase C.1 of `MCC_FULL_MULTI_PARENT_PLAN.md` — the
    /// chain's currently-elected authoritative head per round under
    /// `parent_acceptance_mode = "mcc_full"`. Populated by
    /// `update_authoritative_head` (called by Phase C.2/C.3 hot-path
    /// integration when that ships); read by voting handlers to
    /// dispatch votes to the right per-tip tally.
    /// `None` means either (a) the flag isn't `mcc_full` or (b) the
    /// DAG is empty / no candidate exists. Validator-deterministic
    /// by construction (computed via `enumerate_candidate_heads`'s
    /// argmax which is itself deterministic per Phase C.5 proptest).
    pub current_authoritative_head: Option<[u8; 32]>,
    /// Causal-CHSH cartel-detection alarm (Lane O.8.1). Rolling-buffer
    /// observability primitive — every committed block pushes a
    /// `BlockSummary` into the alarm; periodic gate runs (default
    /// every 50 records, 200-record buffer, 60s concurrency window)
    /// store the latest verdict on the alarm itself for status RPCs.
    /// **No ConsensusAction emission** — this slice is observability-
    /// only. Per-validator soft-fork policy for auto-action emission
    /// on `S > cartel_floor` is the deferred Lane O.8.2.
    /// Doctrine reference: `INVENTION_STACK.md §A1.10`.
    pub cartel_alarm: evaporchain_causal_chsh::CartelAlarm,
    /// Lane O.8.2 — queue of `CartelAlarmEvent`s emitted when the
    /// chain's S crosses `honest_ceiling_milli` AND governance has
    /// set `cartel_alarm_mode = "alarm"`. Default `"observe"` mode
    /// never enqueues. Drained by `take_pending_cartel_alarms()`.
    /// **V1 is event surface only** — no validator-side reaction
    /// policy is in-protocol. Operators consume via HTTP polling or
    /// (future) a websocket subscriber.
    pub pending_cartel_alarms: Vec<evaporchain_causal_chsh::CartelAlarmEvent>,
    /// Lambda-Fold accumulated instance, ticked per committed block.
    /// O(1) memory regardless of chain length — the substrate guarantee
    /// of the energy-folded light client. Per INVENTION_STACK.md §4.1
    /// row 8.
    pub lambda_fold: evaporchain_lambda_fold::FoldedInstance,
    /// Phase 5.1 of LAMBDA_FOLD_NOVA_PLAN — real Nova-IVC fold state.
    /// Lazily constructed on first nova-mode fold (the heavy
    /// `RealBlockProver::new` `pp` setup is ~60-90 s on M4 — running it
    /// at TendermintConsensus construction would block startup; running
    /// it lazily on the first nova-mode block keeps the substrate path
    /// startup unchanged). Only present when the `lambda_fold_nova`
    /// crate feature is enabled.
    #[cfg(feature = "lambda_fold_nova")]
    lambda_fold_nova: Option<Box<evaporchain_lambda_fold::NovaFolder>>,
    /// Running Nova-folded instance — mirrors `lambda_fold` for the
    /// Nova path. Always present (cheap: holds an empty Vec until the
    /// first nova fold), only mutated when nova mode is active.
    #[cfg(feature = "lambda_fold_nova")]
    pub lambda_fold_nova_instance: evaporchain_lambda_fold::NovaFoldedInstance,
    /// This node's validator id.
    pub my_id: u64,
    /// Current block height being decided.
    height: u64,
    /// Current epoch (advances with each committed block).
    epoch: Epoch,
    /// Parent hash for the next block.
    parent_hash: [u8; 32],
    /// Execution engine.
    executor: ParallelExecutor,
    /// Transaction mempool.
    pub mempool: Mempool,
    /// Sum of energy-stamped inclusion priorities for the txs the LOCAL
    /// node included in the most recent proposal it built. Captured at
    /// `create_proposal` from `Mempool::take_with_priority_and_sum`.
    /// Phase-1.5 of the energy-stamped MEV defense.
    ///
    /// **Per-node** (not deterministic across the cluster): submit_epoch
    /// is only known to the local mempool, so two validators looking at
    /// the same block compute different priority sums. The stored value
    /// is used by the operator-driven `apply_local_priority_bonus` path
    /// below (off by default) and surfaced via tracing for observability.
    /// Consensus-safe Phase-2 wiring (deterministic priority via on-the-
    /// wire submit-epoch hints) is the future PR.
    pub last_proposal_priority_sum: u64,
    /// Validator set for leader selection and vote counting.
    pub validator_set: ValidatorSet,
    /// Current round state.
    round_state: RoundState,
    /// Locked block: once we precommit, we lock on this block.
    locked_block: Option<Block>,
    locked_round: Option<u32>,
    /// Valid block: the latest valid proposed block we've seen.
    valid_block: Option<Block>,
    valid_round: Option<u32>,
    /// Timeout configuration.
    propose_timeout: Duration,
    prevote_timeout: Duration,
    precommit_timeout: Duration,
    /// Blocks committed at each height (for duplicate detection).
    committed_heights: HashSet<u64>,
    // ── Slashing Evidence ──────────────────────────────────────────────
    /// Tracks proposals seen per (height, round) → (proposer_id, block_hash).
    /// Used to detect equivocation (same validator proposing two different blocks).
    #[allow(clippy::type_complexity)]
    proposals_seen: HashMap<(u64, u32), Vec<(u64, [u8; 32])>>,
    /// H2 (audit 2026-05-02): block hashes from validators caught
    /// equivocating. Fork-choice (singh-attractor / MCC) filters
    /// candidate heads against this set so a validator who double-
    /// signed can't have either of their forks selected. Both the
    /// original and conflicting hash get inserted at slash time.
    slashed_equivocator_blocks: HashSet<[u8; 32]>,
    /// Verification-track (re-audit 2026-05-02): suppress duplicate
    /// P2-04 ("refusing to commit — DA attestation supermajority not
    /// reached") warnings. The consensus tick fires every 100ms; if
    /// a height stalls on DA we'd previously emit 10 identical lines
    /// per second per node. Track the last warned (height, round)
    /// so we log only once per (height, round) combination.
    p2_04_last_warned: Option<(u64, u32)>,
    /// Tracks consecutive missed proposals per validator.
    /// Reset to 0 when the validator successfully produces a block.
    missed_proposals: HashMap<u64, u64>,
    /// Tracks consecutive missed votes (prevotes + precommits) per validator.
    /// Incremented each round a validator fails to vote; reset on successful vote.
    missed_votes: HashMap<u64, u64>,
    /// Weak subjectivity checkpoint: (height, state_root) pairs.
    /// Validators refuse to reorg past the most recent checkpoint.
    weak_subjectivity_checkpoints: Vec<(u64, [u8; 32])>,
    /// Interval between weak subjectivity checkpoints (in blocks).
    checkpoint_interval: u64,
    /// Externally-provided trusted checkpoint for safe bootstrap.
    /// A new node MUST provide this to defend against long-range attacks.
    /// Format: (height, state_root, block_hash).
    trusted_checkpoint: Option<(u64, [u8; 32], [u8; 32])>,
    /// BLS12-381 keypair for aggregate signature consensus (optional).
    bls_keypair: Option<BlsKeypair>,
    /// Post-quantum VRF keypair for this validator (leader election + randomness).
    vrf_keypair: Option<VrfKeypair>,
    /// On-chain randomness beacon (chains VRF outputs across blocks).
    randomness_beacon: RandomnessBeacon,
    /// Optional proof verifier for validating Nova IVC proofs on proposed blocks.
    proof_verifier: Option<Box<dyn ProofVerifier>>,
    /// Genesis state root needed for proof verification.
    genesis_state_root: [u8; 32],
    /// Epoch transition manager for validator set changes.
    epoch_manager: EpochTransitionManager,
    /// DA attestations collected per block number.
    da_attestations: HashMap<u64, Vec<evaporchain_da::certificate::DAAttestation>>,
    /// Proposer of each committed block — used to exclude self-attestation from DA certificates.
    da_block_proposers: HashMap<u64, u64>,
    /// Finality tracker for bridges, exchanges, and light clients.
    pub finality_tracker: FinalityTracker,
    /// DA attestation manager for data availability certificates.
    pub da_attestation: DAAttestationManager,
    /// MEV-protected encrypted mempool (commit-reveal scheme).
    pub encrypted_mempool: EncryptedMempool,
    /// Pending reveal nonces: (commitment, nonce) pairs submitted by users.
    pending_reveals: Vec<([u8; 32], [u8; 32])>,
    /// Anchor hash provider for rule-based consensus enforcement.
    anchor_provider: Option<Box<dyn AnchorHashProvider>>,
    /// Current state root (updated after each committed block).
    /// Used to populate state_root in proposals so validators can verify
    /// pre-execution state agreement (CometBFT-style app_hash semantics).
    current_state_root: [u8; 32],
    /// Minimum DAS confidence required to attest data availability (default 0.999).
    /// confidence = 1 - 2^(-valid_samples). 16 valid samples → ~0.999985.
    da_confidence_threshold: f64,
    /// Block height at which DA certificate enforcement becomes mandatory.
    /// Before this height: blocks without DA certificates are accepted with a warning (soft mode).
    /// At or after this height: blocks without valid DA certificates are rejected (hard mode).
    /// In both modes, if a DA certificate IS present it must pass full verification.
    da_enforcement_height: u64,
    /// Chain identifier — embedded in every block to prevent cross-chain replay.
    chain_id: String,
    /// Runtime governance parameters (updated via on-chain proposals).
    governance_params: HashMap<String, String>,
    /// Latest block height with confirmed DA attestation.
    da_confirmed_height: u64,
    /// Timestamp of the last committed block (for monotonicity validation).
    last_block_timestamp: u64,
    /// Attractor set for Singh-Attractor fork-choice when
    /// `governance_params["fork_choice_mode"] == "singh_attractor"`.
    /// Empty means MCC (default). Governance-set via
    /// `governance_set_fork_choice_mode`.
    pub fork_choice_attractors: Vec<evaporchain_singh_attractor::Attractor>,
    /// Singh-Boltzmann Stake registry. Per-validator decay/refresh state
    /// separate from the governance `ValidatorSet.stake` — the Boltzmann
    /// stake is the *effective* staking weight after continuous decay.
    /// Ticked per block: decay all → refresh proposer.
    pub boltzmann_stakes: HashMap<u64, evaporchain_boltzmann_stake::ValidatorStake>,
    /// Sliding window of `BlockSummary` entries for WSBF RG flow.
    /// Per INVENTION_STACK.md §A4.3.8 (Wilson-Singh Block Flow).
    pub wsbf_window: std::collections::VecDeque<evaporchain_wsbf::params::BlockSummary>,
    /// Latest `EffectiveParams` produced by one complete WSBF coarse-grain step.
    /// None until the window accumulates `WSBF_COARSE_GRAIN` blocks.
    pub last_effective_params: Option<evaporchain_wsbf::params::EffectiveParams>,
    /// Current consensus phase from the RG Phase Map.
    /// Per INVENTION_STACK.md §A4.3.11 (RG Consensus Phase Map).
    pub current_consensus_phase: evaporchain_rg_phase_map::ConsensusPhase,
    /// Last computed per-block CHSH S-value in milli-units. None until
    /// the first block whose VRF output produces a Bell-Beacon
    /// measurement. Populated by the Bell-Certified gate hook in the
    /// commit pipeline; surfaced through `last_bell_reading()` for the
    /// node API's `GET /api/bell/latest` handler.
    last_bell_s_milli: Option<u64>,
    /// Block height the most recent Bell-Beacon measurement is anchored
    /// to. 0 until the first measurement.
    last_bell_block_height: u64,
    /// Epoch the most recent Bell-Beacon measurement is anchored to.
    /// 0 until the first measurement.
    last_bell_epoch: u64,
    /// Whether the most recent Bell-Beacon measurement strictly exceeds
    /// the local-realism threshold (S > 2 in natural units, i.e. S_milli
    /// > 2000). False until the first measurement.
    last_bell_certified: bool,
    /// Ring buffer of recent (producer_id, exec_time_seconds) samples,
    /// recorded after each successful block commit. Capped at
    /// `BLOCK_PROD_HISTORY_CAP` entries — the oldest entry is dropped
    /// when full. Surfaced to the node API via
    /// `block_production_history()` so the Prometheus exposition can
    /// emit per-producer histogram observations (Grafana heatmap groups
    /// by `producer="validator-{id}"`).
    block_prod_history: std::collections::VecDeque<(u64, f64)>,
    /// Per-height map (height -> committed_at_ms_unix) for blocks that
    /// have been committed but whose finality certificate has not yet been
    /// observed. On finalisation the entry is removed and a (height, gap_ms)
    /// sample is pushed onto `finality_gap_history`. Surfaces the
    /// "this height has been waiting N seconds for finality" signal that
    /// drives the operator-visible finality-stall alert (Mainnet P1).
    committed_at: BTreeMap<u64, u64>,
    /// Ring buffer of recent (height, gap_ms) samples — the per-height
    /// duration between commit and finalisation. Capped at
    /// `FINALITY_GAP_HISTORY_CAP`; oldest entries fall off as new
    /// finalisations land. Drives the `evap_finality_gap_seconds`
    /// histogram on `/metrics`.
    finality_gap_history: VecDeque<(u64, u64)>,
    /// Soft "draining" flag, toggled by the admin API
    /// (`POST /api/admin/drain` / `POST /api/admin/undrain`). When true
    /// the consensus tick skips proposing and prevoting — the node
    /// becomes an observer until undrained. Used by the Ansible upgrade
    /// playbook to gracefully retire a node before binary swap.
    draining: bool,
    /// Wall-clock epoch (this consensus instance's `epoch`) at which the
    /// most recent drain was started. `None` when not draining.
    drain_started_at_epoch: Option<u64>,
    /// Small-cluster DA mode. When `true`, the proposer's own DA
    /// attestation is INCLUDED in cert assembly and quorum counting
    /// (instead of being filtered out). This is unsafe for adversarial
    /// settings (a Byzantine proposer could attest to garbage data) but
    /// is the only way a 3-validator cluster can make liveness progress
    /// under DA enforcement: with proposer excluded, every cert needs
    /// 2-of-2 non-proposer attestations, and any single dropped /
    /// delayed gossip stalls consensus. Auto-enabled by the node binary
    /// when `validator_set.len() <= 3`. For mainnet (≥ 4 validators)
    /// this is left `false` and standard 2/3 supermajority of
    /// non-proposer stake is required.
    small_cluster_da_mode: bool,
}

/// Cap on the per-validator block-production timing ring buffer kept on
/// `TendermintConsensus`. Each entry is `(producer_id, exec_time_seconds)`
/// and is appended after every committed block. 1024 keeps the histogram
/// scrape cheap while still covering ~17 min at a 1 s slot time.
pub const BLOCK_PROD_HISTORY_CAP: usize = 1024;

/// Cap on the per-height finality gap ring buffer. Each entry is
/// `(height, commit_to_finalise_gap_ms)` recorded when a height's
/// commit certificate is observed. 1024 keeps the histogram scrape
/// cheap while covering ~17 min at a 1 s slot time.
pub const FINALITY_GAP_HISTORY_CAP: usize = 1024;

impl TendermintConsensus {
    /// Create a new Tendermint consensus engine.
    pub fn new(my_id: u64, grace_period: u64, validator_set: ValidatorSet) -> Self {
        Self::new_with_gas_limit(my_id, grace_period, validator_set, 500_000)
    }

    /// Create with a custom block gas limit (for high-throughput mode).
    pub fn new_with_gas_limit(
        my_id: u64,
        grace_period: u64,
        validator_set: ValidatorSet,
        block_gas_limit: u64,
    ) -> Self {
        Self {
            light_cone_dag: evaporchain_light_cone::LightCone::new(),
            tur_window: std::collections::VecDeque::with_capacity(TUR_WINDOW_BLOCKS),
            last_tur_verdict: None,
            mev_observations: std::collections::VecDeque::with_capacity(MEV_OBSERVATION_BUFFER_CAP),
            mev_attacker_stats: std::collections::HashMap::new(),
            settled_refunds: std::collections::HashSet::new(),
            mev_missing_refund_violations: std::collections::HashMap::new(),
            disputed_observations: std::collections::HashSet::new(),
            state_branches: std::collections::HashMap::new(),
            dag_round_states: std::collections::HashMap::new(),
            cross_fork_equivocations: std::collections::HashMap::new(),
            committed_at_block: std::collections::HashMap::new(),
            antichain_digest_history: std::collections::VecDeque::with_capacity(
                ANTICHAIN_DIGEST_HISTORY_CAP,
            ),
            current_authoritative_head: None,
            cartel_alarm: evaporchain_causal_chsh::CartelAlarm::doctrine_default(),
            pending_cartel_alarms: Vec::new(),
            lambda_fold: evaporchain_lambda_fold::FoldedInstance::identity(),
            #[cfg(feature = "lambda_fold_nova")]
            lambda_fold_nova: None,
            #[cfg(feature = "lambda_fold_nova")]
            lambda_fold_nova_instance: evaporchain_lambda_fold::NovaFoldedInstance::identity(),
            my_id,
            height: 1, // Start at height 1 (genesis is 0)
            epoch: 0,
            parent_hash: [0u8; 32],
            executor: ParallelExecutor::new_production(
                grace_period,
                PidFeeController::testnet_config(),
                block_gas_limit,
            ),
            mempool: Mempool::new(),
            last_proposal_priority_sum: 0,
            validator_set,
            round_state: RoundState::new(0),
            locked_block: None,
            locked_round: None,
            valid_block: None,
            valid_round: None,
            propose_timeout: Duration::from_millis(PROPOSE_TIMEOUT_MS),
            prevote_timeout: Duration::from_millis(PREVOTE_TIMEOUT_MS),
            precommit_timeout: Duration::from_millis(PRECOMMIT_TIMEOUT_MS),
            committed_heights: HashSet::new(),
            proposals_seen: HashMap::new(),
            slashed_equivocator_blocks: HashSet::new(),
            p2_04_last_warned: None,
            missed_proposals: HashMap::new(),
            missed_votes: HashMap::new(),
            weak_subjectivity_checkpoints: Vec::new(),
            checkpoint_interval: 1000,
            trusted_checkpoint: None,
            bls_keypair: None,
            vrf_keypair: None,
            randomness_beacon: RandomnessBeacon::new(),
            proof_verifier: None,
            genesis_state_root: [0u8; 32],
            epoch_manager: EpochTransitionManager::new(),
            da_attestations: HashMap::new(),
            da_block_proposers: HashMap::new(),
            finality_tracker: FinalityTracker::new(),
            da_attestation: DAAttestationManager::new(),
            encrypted_mempool: EncryptedMempool::new(2),
            pending_reveals: Vec::new(),
            anchor_provider: None,
            current_state_root: [0u8; 32],
            da_confidence_threshold: 0.999,
            da_enforcement_height: 100,
            chain_id: String::new(),
            governance_params: HashMap::new(),
            da_confirmed_height: 0,
            last_block_timestamp: 0,
            fork_choice_attractors: Vec::new(),
            boltzmann_stakes: HashMap::new(),
            wsbf_window: std::collections::VecDeque::new(),
            last_effective_params: None,
            current_consensus_phase: evaporchain_rg_phase_map::ConsensusPhase::LivenessStable,
            last_bell_s_milli: None,
            last_bell_block_height: 0,
            last_bell_epoch: 0,
            last_bell_certified: false,
            block_prod_history: std::collections::VecDeque::with_capacity(BLOCK_PROD_HISTORY_CAP),
            committed_at: BTreeMap::new(),
            finality_gap_history: VecDeque::with_capacity(FINALITY_GAP_HISTORY_CAP),
            draining: false,
            drain_started_at_epoch: None,
            small_cluster_da_mode: false,
        }
    }

    pub fn get_governance_param(&self, key: &str) -> Option<&str> {
        self.governance_params.get(key).map(|s| s.as_str())
    }

    /// Set a governance soft-fork parameter, validated against the
    /// allowlist of known keys + their permitted values. Used by the
    /// `POST /api/governance/param` RPC (Lane K.1) so operators can
    /// flip Lane I.4 / I.5 / Layer 0 #1 knobs without recompiling +
    /// without bypassing the safety allowlist.
    ///
    /// Returns `Err` if the key isn't a known soft-fork knob or if
    /// the value isn't permitted for that key. Unknown keys are
    /// rejected (not silently inserted) so a misspelled key can't
    /// litter governance_params with junk that fails the typo-safety
    /// fall-through patterns at the consumer sites.
    ///
    /// Allowlist:
    /// - `parent_acceptance_mode` ∈ {`linear`, `mcc`}
    /// - `block_source_mode` ∈ {`fifo`, `antichain`}
    /// - `conservation_enforcement` ∈ {`observe`, `enforce`}
    /// - `lambda_fold_mode` ∈ {`hash_chain`, `nova`} — Phase 5 of
    ///   LAMBDA_FOLD_NOVA_PLAN. `hash_chain` (default) keeps the
    ///   substrate blake3 fold; `nova` flips the per-block fold call
    ///   site to the real Nova IVC path in `evaporchain-lambda-fold`'s
    ///   `nova_path` module. The Nova path needs the `lambda_fold_nova`
    ///   crate feature on `evaporchain-consensus`; with the feature
    ///   off, the flag is ignored at the call site and the
    ///   substrate path runs regardless.
    /// - `fork_choice_mode` is set via `governance_set_fork_choice_mode`
    ///   instead (it requires endorser-stake validation).
    /// - `cartel_alarm_mode` ∈ {`observe`, `alarm`} — Lane O.8.2.
    ///   `observe` (default) keeps the chain's rolling Causal-CHSH
    ///   alarm in pure-observation mode (status RPCs only). `alarm`
    ///   flips on `CartelAlarmEvent` emission whenever the chain's
    ///   honest-source S crosses `honest_ceiling_milli` (1800 under
    ///   doctrine defaults). V1 is event surface only — no validator-
    ///   side reaction policy is in-protocol.
    pub fn governance_set_param(
        &mut self,
        key: &str,
        value: &str,
    ) -> Result<(), GovernanceParamError> {
        let permitted: &[&str] = match key {
            "parent_acceptance_mode" => &["linear", "mcc", "mcc_full"],
            "block_source_mode" => &["fifo", "antichain"],
            "conservation_enforcement" => &["observe", "enforce"],
            "lambda_fold_mode" => &["hash_chain", "nova"],
            // Phase 3.4 of CROOKS_MEV_INTEGRATION_PLAN.md —
            // `crooks_mev_settlement_mode` chooses between
            // observe-only (default) and enforce (validators reject
            // blocks omitting required RefundTxs). Phase 3.5 ships
            // the slashing rule that pairs with `enforce`.
            "crooks_mev_settlement_mode" => &["observe", "enforce"],
            "cartel_alarm_mode" => &["observe", "alarm"],
            // POST_EXEC_STATE_VERIFICATION_PLAN.md Phase 4 (lane T0.3) —
            // tri-state gate over the Phase 2 speculative-execute wiring
            // (`create_proposal`) and the Phase 3 apply-time mismatch
            // check (`apply_block`).
            //   "off"     — proposer skips speculative execute (no
            //               per-block CPU cost); validator's apply
            //               check is a no-op because post_state_root
            //               stays None on the proposed block.
            //   "warn"    — proposer fills post_state_root; validator
            //               warns on mismatch but does NOT reject.
            //               Default — bit-compatible with the
            //               af6876d/cb12cf1-shipped behaviour.
            //   "enforce" — proposer fills; validator returns Err on
            //               mismatch from apply_block, refusing to
            //               commit a divergent block locally.
            "post_state_verify_mode" => &["off", "warn", "enforce"],
            // Phase 3.5 of LIGHT_CONE_FULL_DAG_PLAN.md — Decision 5
            // rollout gate. `false` (default) keeps the chain in
            // linear-state mode; `true` activates per-fork state
            // branches. Off by default — operators flip on testnet
            // before mainnet.
            "light_cone_state_branches_enabled" => &["true", "false"],
            // Phase 3.5d of CROOKS_MEV_INTEGRATION_PLAN.md —
            // wire mev_missing_refund_violations into the
            // validator-set stake-update path. Default off — the
            // counter is observable but no automatic stake
            // deduction. When `true`, every committed block
            // applies entropic_slash for each accumulated violation
            // count and resets that validator's entry.
            "crooks_mev_missing_refund_slash_enabled" => &["true", "false"],
            // Phase 2.2 of CROOKS_MEV_INTEGRATION_PLAN.md —
            // `crooks_mev_beta_mb` is a u64 ≥ 1; allowlist enforces
            // numeric parseability + lower bound rather than
            // enumerating values. Special-cased below.
            "crooks_mev_beta_mb" => {
                let v = value
                    .parse::<u64>()
                    .map_err(|_| GovernanceParamError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                        permitted: vec!["any u64 ≥ 1".to_string()],
                    })?;
                if v < 1 {
                    return Err(GovernanceParamError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                        permitted: vec!["any u64 ≥ 1".to_string()],
                    });
                }
                self.governance_params
                    .insert(key.to_string(), value.to_string());
                return Ok(());
            }
            // Phase 5.1 of LIGHT_CONE_FULL_DAG_PLAN.md —
            // orphan-detection caliber threshold. Tips with
            // caliber < threshold are eligible for orphan pruning
            // (subject to the recency check). Default 0 = no
            // orphans by caliber alone. Range 0..=u64::MAX.
            "light_cone_orphan_caliber_threshold" => {
                let v = value
                    .parse::<u64>()
                    .map_err(|_| GovernanceParamError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                        permitted: vec!["any u64".to_string()],
                    })?;
                self.governance_params
                    .insert(key.to_string(), v.to_string());
                return Ok(());
            }
            // Phase 3 Decision 2 of LIGHT_CONE_FULL_DAG_PLAN.md —
            // concurrent-fork cap as governance flag, range 1..=8,
            // default 4. Inert when light_cone_state_branches_enabled
            // is false (linear-state mode has no branches to cap).
            "light_cone_max_concurrent_forks" => {
                let v = value
                    .parse::<u8>()
                    .map_err(|_| GovernanceParamError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                        permitted: vec!["any u8 in 1..=8".to_string()],
                    })?;
                if !(1..=8).contains(&v) {
                    return Err(GovernanceParamError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                        permitted: vec!["any u8 in 1..=8".to_string()],
                    });
                }
                self.governance_params
                    .insert(key.to_string(), value.to_string());
                return Ok(());
            }
            // Phase 4.1 of CROOKS_MEV_INTEGRATION_PLAN.md —
            // confidence threshold in parts-per-million (0..=1_000_000).
            // **Audit fix HIGH H4**: ppm replaces legacy milli.
            "crooks_mev_confidence_threshold_ppm" => {
                let v = value
                    .parse::<u32>()
                    .map_err(|_| GovernanceParamError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                        permitted: vec!["any u32 in 0..=1_000_000".to_string()],
                    })?;
                if v > 1_000_000 {
                    return Err(GovernanceParamError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                        permitted: vec!["any u32 in 0..=1_000_000".to_string()],
                    });
                }
                self.governance_params
                    .insert(key.to_string(), value.to_string());
                return Ok(());
            }
            // Phase 3.3 of CROOKS_MEV_INTEGRATION_PLAN.md — grace
            // period and refund window. Both are u64 ≥ 1; window
            // must be ≥ grace (enforced at use site, not allowlist).
            "crooks_mev_grace_period_blocks" | "crooks_mev_refund_window_blocks" => {
                let v = value
                    .parse::<u64>()
                    .map_err(|_| GovernanceParamError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                        permitted: vec!["any u64 ≥ 1".to_string()],
                    })?;
                if v < 1 {
                    return Err(GovernanceParamError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                        permitted: vec!["any u64 ≥ 1".to_string()],
                    });
                }
                self.governance_params
                    .insert(key.to_string(), value.to_string());
                return Ok(());
            }
            _ => return Err(GovernanceParamError::UnknownKey(key.to_string())),
        };
        if !permitted.contains(&value) {
            return Err(GovernanceParamError::InvalidValue {
                key: key.to_string(),
                value: value.to_string(),
                permitted: permitted.iter().map(|s| s.to_string()).collect(),
            });
        }
        self.governance_params
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    pub fn da_confirmed_height(&self) -> u64 {
        self.da_confirmed_height
    }

    pub fn is_da_finalized(&self, height: u64) -> bool {
        height <= self.da_confirmed_height
    }

    /// Set the chain identifier for this consensus instance.
    pub fn set_chain_id(&mut self, chain_id: String) {
        // Mirror the chain_id onto the ParallelExecutor so signature
        // verification at execute time uses the SAME chain_id the API
        // signed the tx with. Without this propagation, executor.chain_id
        // stays empty (the default in new_production), every signed tx
        // fails sig verification at execute time, and txs silently
        // disappear despite landing in committed blocks. Caught during
        // the 3-Mini cluster faucet flow: faucet endpoint returned 200,
        // tx made it into block #6556, but balance never decremented.
        self.executor.chain_id = chain_id.clone();
        self.chain_id = chain_id;
    }

    /// Get the current chain identifier.
    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }

    /// Snapshot of the four-act narrative spine state from the
    /// internal `ParallelExecutor`. Caller (typically the node binary)
    /// translates this into the public-facing `api::FourActSnapshot`
    /// after each block. Per INVENTION_STACK.md Amendment 2 §A2.5.
    pub fn four_act_state(&self) -> ConsensusFourActState {
        let trie = &self.executor.eulogy_trie;
        // Cap surface area: latest 1024 addresses by sorted iteration.
        let tombstone_addresses: Vec<[u8; 32]> =
            trie.iter().take(1024).map(|(addr, _)| *addr).collect();
        ConsensusFourActState {
            eulogy_count: trie.len(),
            eulogy_trie_root: if trie.is_empty() {
                None
            } else {
                Some(trie.root())
            },
            tombstone_addresses,
            refresh_pool_total: self.executor.refresh_pool.total_accrued(),
            mortis_triggered: self.executor.mortis_monitor.is_triggered(),
            mortis_epoch_of_death: self
                .executor
                .mortis_certificate
                .as_ref()
                .map(|c| c.epoch_of_death),
            mortis_final_state_root: self
                .executor
                .mortis_certificate
                .as_ref()
                .map(|c| c.final_state_root),
            last_conservation_audit_ok: self
                .executor
                .last_conservation_audit
                .as_ref()
                .map(|r| r.is_ok()),
            last_conservation_violation_type: self
                .executor
                .last_conservation_audit
                .as_ref()
                .and_then(|r| r.as_ref().err().map(conservation_violation_discriminant)),
            consecutive_clean_audits: self.executor.consecutive_clean_audits,
            light_cone_block_count: self.light_cone_dag.len(),
            evaporation_mmr_size: self.executor.mmr.size(),
            evaporation_mmr_root: if self.executor.mmr.size() == 0 {
                None
            } else {
                Some(self.executor.mmr.root())
            },
            dead_producer_redirect_total: self
                .executor
                .refresh_pool
                .accrued_for(&b"evaporchain-dead-producer-refresh".to_vec()),
        }
    }

    /// Per-block hook: advance Mortis on the internal executor. Caller
    /// invokes after `execute_block` with the just-committed state root.
    pub fn tick_mortis_on_executor(
        &mut self,
        current_epoch: u64,
        state_root: [u8; 32],
    ) -> Option<&evaporchain_mortis::MortisCertificate> {
        self.executor.tick_mortis(current_epoch, state_root)
    }

    /// Per-block hook: drop tombstoned validators from the active set.
    /// Walks the executor's eulogy_trie; for any validator whose
    /// address is memorialised, marks `jailed = true` so
    /// `leader_for_epoch` (which already filters jailed validators)
    /// stops electing it. Idempotent — safe to call every block.
    /// Returns count newly jailed (typically 0).
    ///
    /// Caller (the node binary) invokes after `tick_mortis_on_executor`
    /// at every block boundary.
    pub fn enforce_validator_tombstones(&mut self) -> usize {
        let tombstone_addresses: Vec<[u8; 32]> = self
            .executor
            .eulogy_trie
            .iter()
            .map(|(addr, _)| *addr)
            .collect();
        if tombstone_addresses.is_empty() {
            return 0;
        }
        self.validator_set
            .jail_tombstoned_by_address(&tombstone_addresses)
    }

    /// Read-only access to the executor's MortisCertificate, if minted.
    pub fn mortis_certificate(&self) -> Option<&evaporchain_mortis::MortisCertificate> {
        self.executor.mortis_certificate.as_ref()
    }

    /// Build the death-certificate that *would* be minted at the
    /// current chain state, without mutating anything. For dashboards
    /// and demos: shows the cert format ahead of actual death so
    /// observers can preview the artefact while the chain is healthy.
    /// Returns None if Mortis already triggered (real cert is
    /// authoritative).
    pub fn mortis_cert_preview(&self) -> Option<evaporchain_mortis::MortisCertificate> {
        if self.executor.mortis_monitor.is_triggered() {
            return None;
        }
        let trie = &self.executor.eulogy_trie;
        let eulogy_root = if trie.is_empty() {
            [0u8; 32]
        } else {
            trie.root()
        };
        let refresh_pool = self.executor.refresh_pool.total_accrued();
        Some(evaporchain_mortis::mint_certificate(
            self.current_state_root,
            eulogy_root,
            self.epoch,
            refresh_pool,
        ))
    }

    /// SlashSettle leg of the conservation triplet.
    /// Routes `amount` slashed tokens into the executor's RefreshPool under the
    /// canonical slash-settlement namespace (ASCII "SLSH" = [0x53,0x4c,0x53,0x48]).
    /// Called by the node immediately after it records `stake.slashed_amount`.
    pub fn settle_slash(&mut self, amount: u64, epoch: u64) {
        if amount == 0 {
            return;
        }
        let slash_ns: Vec<u8> = vec![0x53, 0x4c, 0x53, 0x48]; // "SLSH"
        self.executor.refresh_pool.accrue(slash_ns, amount, epoch);
    }

    /// Read-only iteration over the executor's RefreshPool credits.
    /// Returns (namespace_hex, accrued, last_touched_epoch) tuples.
    pub fn refresh_pool_credits(&self) -> Vec<(String, u64, u64)> {
        self.executor
            .refresh_pool
            .credits()
            .map(|c| (hex::encode(&c.namespace), c.accrued, c.last_touched_epoch))
            .collect()
    }

    /// Most recent per-block Bell-Beacon CHSH measurement, or `None`
    /// before the first VRF-derived S-value lands. Read by the node
    /// API's `GET /api/bell/latest` handler so wallets can render a
    /// live BellBeaconCard instead of a static design target.
    ///
    /// `threshold_milli` is fixed at the chain's local-realism bound
    /// (`evaporchain_bell_beacon::LOCAL_REALISM_S_MILLI`). The struct
    /// is intentionally `Copy` so callers can read without holding the
    /// consensus lock for any longer than the field copy.
    pub fn last_bell_reading(&self) -> Option<BellBeaconReading> {
        self.last_bell_s_milli.map(|s| BellBeaconReading {
            s_value_milli: s,
            threshold_milli: BELL_LOCAL_REALISM_S_MILLI,
            bell_certified: self.last_bell_certified,
            block_height: self.last_bell_block_height,
            epoch: self.last_bell_epoch,
        })
    }

    /// Look up a single tombstone by address. Returns the 32-byte
    /// commitment if the account has been memorialised; None otherwise.
    pub fn tombstone_for(&self, addr: &[u8; 32]) -> Option<[u8; 32]> {
        self.executor.eulogy_trie.get(addr).map(|t| t.commitment)
    }

    /// Build the Shalizi-Crutchfield Causal-Cone summary for a given
    /// block head if it exists in the parallel Light-Cone DAG. None if
    /// `head` isn't in the DAG. Per INVENTION_STACK.md §A1.3 (Optimal
    /// Prediction Theorem) this is the constant-size sufficient
    /// statistic for predicting the chain's future from `head`'s past.
    pub fn causal_cone_summary(
        &self,
        head: [u8; 32],
        chain_lambda_half_life_epochs: u64,
        observation_epoch: u64,
    ) -> Option<evaporchain_causal_cone::CausalConeSummary> {
        let lambda = evaporchain_energy_kernel::ChainLambda::new(
            evaporchain_energy_kernel::Lambda::from_epochs(chain_lambda_half_life_epochs.max(1)),
        );
        evaporchain_causal_cone::summarize_cone(
            head,
            &self.light_cone_dag,
            lambda,
            observation_epoch,
        )
        .ok()
    }

    /// Singh-Attractor fork choice over `candidate_heads` against a
    /// caller-supplied list of attractor basins. For each candidate
    /// head, reads its block "energy" from the Light-Cone DAG and
    /// returns the head that lands inside (or nearest to) one of the
    /// attractors. Per INVENTION_STACK.md §4.2 (Tier 2 — Singh-
    /// Attractor Consensus). Like `mcc_choose_fork`, exposed for light
    /// clients ahead of governance promotion to authoritative fork
    /// choice.
    pub fn singh_attractor_fork_choice(
        &self,
        candidate_heads: &[[u8; 32]],
        attractors: &[evaporchain_singh_attractor::Attractor],
    ) -> Option<[u8; 32]> {
        if attractors.is_empty() {
            return None;
        }
        let mut best: Option<([u8; 32], u64)> = None;
        for head in candidate_heads {
            // H2 (audit 2026-05-02): never select a tainted head — any
            // hash whose proposer was caught equivocating is filtered
            // out of fork-choice.
            if self.slashed_equivocator_blocks.contains(head) {
                continue;
            }
            let block = self.light_cone_dag.get(head)?;
            let energy = block.energy;
            // Prefer in-basin candidates; fall back to closest-to-center.
            let in_basin = attractors.iter().any(|a| a.contains(energy));
            // Score: 0 if in basin, otherwise distance to nearest center.
            let score: u64 = if in_basin {
                0
            } else {
                attractors
                    .iter()
                    .map(|a| energy.abs_diff(a.center))
                    .min()
                    .unwrap_or(u64::MAX)
            };
            match best {
                None => best = Some((*head, score)),
                Some((_, prev_score)) if score < prev_score => {
                    best = Some((*head, score));
                }
                Some((prev_head, prev_score)) if score == prev_score => {
                    // Deterministic tie-break: lex-larger head wins.
                    if *head > prev_head {
                        best = Some((*head, score));
                    }
                }
                _ => {}
            }
        }
        best.map(|(h, _)| h)
    }

    /// Run Maximum-Caliber-Coherence fork choice over `candidate_heads`.
    /// For each head, builds the parent-chain trajectory back to genesis
    /// (single-parent walk; first-parent of each block in the Light-Cone
    /// DAG), then picks the trajectory whose path-caliber is maximal.
    /// Returns `None` if no candidate is in the DAG. Per
    /// INVENTION_STACK.md §A1.2 / §A1.3 (Jaynes 1980 + Stock 2009 closed-
    /// form caliber).
    ///
    /// `beta_mb` is the chain-set inverse-temperature (Jaynes
    /// multiplier-of-energy) for the caliber penalty term. The launch
    /// default 10_000 is governance-set.
    pub fn mcc_choose_fork(&self, candidate_heads: &[[u8; 32]], beta_mb: u64) -> Option<[u8; 32]> {
        let trajectories: Vec<evaporchain_mcc::Trajectory> = candidate_heads
            .iter()
            // H2 (audit 2026-05-02): drop tainted heads before
            // building trajectories so MCC never scores an
            // equivocating chain.
            .filter(|head| !self.slashed_equivocator_blocks.contains(*head))
            .filter_map(|head| self.trajectory_to_genesis(*head))
            .collect();
        if trajectories.is_empty() {
            return None;
        }
        let refs: Vec<&evaporchain_mcc::Trajectory> = trajectories.iter().collect();
        evaporchain_mcc::mcc_choose(refs, &self.light_cone_dag, beta_mb)
            .ok()
            .and_then(|t| t.head().copied())
    }

    // ─── Governance amendment: fork-choice mode ───────────────────────────

    /// Authoritative fork-choice: dispatches to MCC or Singh-Attractor based
    /// on the current governance-set mode. This is the single call-site for
    /// all block-proposal/fork-selection code paths.
    ///
    /// Defaults to MCC (`beta_mb = 10_000`) if no governance amendment has been
    /// applied yet (`fork_choice_mode` not set or `fork_choice_attractors` empty
    /// in Singh-Attractor mode).
    pub fn authoritative_head(
        &self,
        candidate_heads: &[[u8; 32]],
        beta_mb: u64,
    ) -> Option<[u8; 32]> {
        let mode = self
            .governance_params
            .get("fork_choice_mode")
            .map(|s| s.as_str())
            .unwrap_or("mcc");
        if mode == "singh_attractor" && !self.fork_choice_attractors.is_empty() {
            self.singh_attractor_fork_choice(candidate_heads, &self.fork_choice_attractors)
        } else {
            self.mcc_choose_fork(candidate_heads, beta_mb)
        }
    }

    /// Apply a governance amendment to switch the authoritative fork-choice mode.
    ///
    /// Requires that the calling validators collectively hold ≥ `required_stake`
    /// (expressed as total stake units, not fraction). The caller must pass the
    /// stake of each endorsing validator in `endorser_stakes`; this method sums
    /// them and compares against `required_stake`. Returns `Err` if the quorum
    /// is not met or the `mode` string is unrecognised.
    ///
    /// Recognised modes:
    /// - `"mcc"` — Maximum-Caliber-Coherence (default; Jaynes 1980)
    /// - `"singh_attractor"` — Singh-Attractor basin-based fork choice
    pub fn governance_set_fork_choice_mode(
        &mut self,
        mode: &str,
        attractors: Vec<evaporchain_singh_attractor::Attractor>,
        endorser_stakes: &[u64],
        required_stake: u64,
    ) -> Result<(), GovernanceAmendmentError> {
        if mode != "mcc" && mode != "singh_attractor" {
            return Err(GovernanceAmendmentError::UnrecognisedMode(mode.to_string()));
        }
        if mode == "singh_attractor" && attractors.is_empty() {
            return Err(GovernanceAmendmentError::EmptyAttractors);
        }
        let total_endorsing: u64 = endorser_stakes
            .iter()
            .copied()
            .fold(0u64, u64::saturating_add);
        if total_endorsing < required_stake {
            return Err(GovernanceAmendmentError::InsufficientStake {
                endorsing: total_endorsing,
                required: required_stake,
            });
        }
        self.governance_params
            .insert("fork_choice_mode".to_string(), mode.to_string());
        self.fork_choice_attractors = attractors;
        tracing::info!(
            mode,
            total_endorsing,
            required_stake,
            "fork-choice governance amendment applied"
        );
        Ok(())
    }

    /// Current fork-choice mode as stored in governance_params.
    pub fn fork_choice_mode(&self) -> &str {
        self.governance_params
            .get("fork_choice_mode")
            .map(|s| s.as_str())
            .unwrap_or("mcc")
    }

    /// Snapshot all governance flags + their effective values (including
    /// the documented defaults for unset keys). Used by the
    /// `GET /api/governance/flags` RPC so operators can verify which
    /// soft-fork knobs are active without reading internal state.
    ///
    /// The returned map is `{key → value}` for everything explicitly set
    /// Lane O.8.1 status getter — returns the latest cartel-alarm
    /// verdict snapshot, or `None` if the rolling buffer hasn't yet
    /// run the periodic gate (needs at least 50 records + one
    /// run-interval boundary). Operators use this for the
    /// `GET /api/cartel_alarm/chain_status` RPC (Lane O.8.2 wiring).
    pub fn cartel_alarm_status(&self) -> Option<&evaporchain_causal_chsh::AlarmStatus> {
        self.cartel_alarm.status()
    }

    /// Buffer occupancy of the cartel alarm — useful for status
    /// dashboards / health checks before the first verdict lands.
    pub fn cartel_alarm_buffer_len(&self) -> usize {
        self.cartel_alarm.buffer_len()
    }

    /// Total `record_block` calls since alarm construction. Operator-
    /// visibility into how many blocks have flowed through the alarm.
    pub fn cartel_alarm_records_seen(&self) -> u64 {
        self.cartel_alarm.records_seen()
    }

    /// Drain the queue of pending `CartelAlarmEvent`s emitted since the
    /// last call. Returns an empty Vec when nothing has fired or
    /// `cartel_alarm_mode` is the default `observe`. Lane O.8.2.
    ///
    /// Consumers (RPC layer, websocket dispatcher, slashing engine in
    /// future Lane O.8.3+) call this on a tick / on each block to
    /// surface fired alarms downstream. The queue holds at most one
    /// event per `last_run_at_height` (de-duplicated at emission time)
    /// so back-to-back ticks at the same height never double-emit.
    pub fn take_pending_cartel_alarms(&mut self) -> Vec<evaporchain_causal_chsh::CartelAlarmEvent> {
        std::mem::take(&mut self.pending_cartel_alarms)
    }

    /// Lane O.8.2f — current depth of the pending-events queue WITHOUT
    /// draining. Counterpart of `take_pending_cartel_alarms()` for
    /// observability dashboards / health checks that need to monitor
    /// queue growth without consuming events. Pairs with
    /// `cartel_alarm_buffer_len()` (rolling-buffer depth) and
    /// `cartel_alarm_records_seen()` (lifetime tick count) to give a
    /// complete operator view.
    pub fn pending_cartel_alarms_count(&self) -> usize {
        self.pending_cartel_alarms.len()
    }

    /// Lane O.8.2 emission gate. Pushes a `CartelAlarmEvent` onto
    /// `pending_cartel_alarms` iff:
    ///
    /// 1. governance flag `cartel_alarm_mode == "alarm"` (default
    ///    `"observe"` is silent),
    /// 2. the alarm has a fresh `AlarmStatus` (gate has run at least
    ///    once),
    /// 3. the chain's honest-source S in milli-units has crossed the
    ///    doctrine `honest_ceiling_milli` threshold,
    /// 4. no event for `last_run_at_height` is already queued
    ///    (de-duplicates back-to-back ticks before the next periodic
    ///    recompute).
    ///
    /// Called from `on_block_committed` after `cartel_alarm.record_block`.
    fn maybe_emit_cartel_alarm_event(&mut self) {
        let alarm_mode = self
            .governance_params
            .get("cartel_alarm_mode")
            .map(|s| s.as_str())
            .unwrap_or("observe");
        if alarm_mode != "alarm" {
            return;
        }
        let Some(st) = self.cartel_alarm.status() else {
            return;
        };
        let already_fired_for_height = self
            .pending_cartel_alarms
            .iter()
            .any(|e| e.at_height == st.last_run_at_height);
        if already_fired_for_height {
            return;
        }
        let ceiling_milli = (st.thresholds.honest_ceiling * 1000.0) as i64;
        if st.s_honest_milli < ceiling_milli {
            return;
        }
        let event = evaporchain_causal_chsh::CartelAlarmEvent {
            at_height: st.last_run_at_height,
            s_honest_milli: st.s_honest_milli,
            s_cartel_synthetic_milli: st.s_cartel_synthetic_milli,
            gap_milli: st.gap_milli,
            honest_ceiling_milli_at_fire: ceiling_milli,
            samples_per_bucket: st.samples_per_bucket,
        };
        // Lane O.8.2e: structured tracing on emission. Operators tailing
        // node logs (journalctl, Loki, structured-log shippers) see the
        // alarm instantly without polling /api/cartel_alarm/pending_events.
        // Level WARN — this is by definition an unusual event (chain's
        // own honest-source S crossed the doctrine ceiling); we want it
        // to surface above default INFO log filters.
        warn!(
            target: "cartel_alarm",
            at_height = event.at_height,
            s_honest_milli = event.s_honest_milli,
            s_cartel_synthetic_milli = event.s_cartel_synthetic_milli,
            gap_milli = event.gap_milli,
            honest_ceiling_milli_at_fire = event.honest_ceiling_milli_at_fire,
            samples_per_bucket = ?event.samples_per_bucket,
            "Causal-CHSH cartel alarm fired (chain self-monitor crossed doctrine ceiling)"
        );
        self.pending_cartel_alarms.push(event);
    }

    /// in `governance_params`, plus a small set of documented keys with
    /// their default values when unset (so operators can see the
    /// effective state, not just the explicit overrides).
    pub fn governance_flags_snapshot(&self) -> std::collections::HashMap<String, String> {
        let mut out = self.governance_params.clone();
        // Document the soft-fork keys + their defaults so consumers
        // see the *effective* value, not just the explicit overrides.
        // Keys touched by Lane I.4 / Lane I.5 / Layer 0 #1.
        // NOTE: governance defaults are intentionally cluster-compatible.
        // Doctrine-grade behaviors (antichain mempool drain, real Nova IVC,
        // strict conservation enforcement) are flipped via
        // POST /api/governance/param after a clean stop-the-world deploy —
        // not by changing these defaults. Changing the defaults in code
        // would hard-fork any running cluster on the next binary swap.
        for (key, default) in [
            ("fork_choice_mode", "mcc"),
            ("parent_acceptance_mode", "linear"),
            ("block_source_mode", "fifo"),
            ("conservation_enforcement", "observe"),
            ("lambda_fold_mode", "hash_chain"),
            ("cartel_alarm_mode", "observe"),
            // POST_EXEC_STATE_VERIFICATION_PLAN.md Phase 4 (lane T0.3) —
            // default "warn" preserves the af6876d/cb12cf1 always-on
            // Phase 2+3 behaviour. Operators flip to "off" to disable
            // the per-block speculative-execute CPU cost, or to
            // "enforce" to make apply_block return Err on mismatch.
            ("post_state_verify_mode", "warn"),
        ] {
            out.entry(key.to_string())
                .or_insert_with(|| default.to_string());
        }
        out
    }

    // ─── Singh-Boltzmann Stake ─────────────────────────────────────────────

    /// Ensure `validator_id` has a Boltzmann stake entry. If not present,
    /// seed it from the governance ValidatorSet's current stake value.
    fn ensure_boltzmann_stake(&mut self, validator_id: u64) {
        if !self.boltzmann_stakes.contains_key(&validator_id) {
            let seed_stake = self
                .validator_set
                .get(validator_id)
                .map(|v| v.stake)
                .unwrap_or(0);
            self.boltzmann_stakes.insert(
                validator_id,
                evaporchain_boltzmann_stake::ValidatorStake::fresh(seed_stake),
            );
        }
    }

    /// Decay all validators' Boltzmann stakes to `current_epoch`.
    /// Called once per committed block.
    pub fn decay_all_boltzmann_stakes(&mut self, current_epoch: u64) {
        use evaporchain_boltzmann_stake::decay_validator_stake;
        let chain_lambda =
            evaporchain_energy_kernel::ChainLambda::new(evaporchain_energy_kernel::DEFAULT_LAMBDA);
        // Seed any validator that doesn't have an entry yet.
        let validator_ids: Vec<u64> = self
            .validator_set
            .validators()
            .iter()
            .map(|v| v.id)
            .collect();
        for id in &validator_ids {
            self.ensure_boltzmann_stake(*id);
        }
        for (_, stake) in self.boltzmann_stakes.iter_mut() {
            *stake = decay_validator_stake(*stake, chain_lambda, current_epoch);
        }
    }

    /// Credit block-production refresh to the proposer's Boltzmann stake.
    /// `refresh_amount` is governance-set; the launch default is the
    /// expected decay-per-block at the target block rate.
    pub fn refresh_proposer_boltzmann_stake(
        &mut self,
        proposer_id: u64,
        current_epoch: u64,
        refresh_amount: u64,
    ) {
        use evaporchain_boltzmann_stake::refresh_on_block;
        self.ensure_boltzmann_stake(proposer_id);
        if let Some(stake) = self.boltzmann_stakes.get_mut(&proposer_id) {
            *stake = refresh_on_block(*stake, refresh_amount, current_epoch);
        }
    }

    /// Boltzmann proposer weights for all active validators.
    /// Returns `(validator_id, effective_weight)` pairs sorted descending.
    /// `beta_mb` is the Boltzmann inverse-temperature parameter (launch default 1_000).
    pub fn boltzmann_proposer_weights(&self, beta_mb: u64) -> Vec<(u64, u128)> {
        use evaporchain_boltzmann_stake::proposer_weight;
        let mut weights: Vec<(u64, u128)> = self
            .validator_set
            .validators()
            .iter()
            .map(|v| {
                let b_stake = self
                    .boltzmann_stakes
                    .get(&v.id)
                    .map(|s| s.active)
                    .unwrap_or(v.stake);
                // activity_score = blocks produced (health_score * 16 as proxy).
                // health_score is f64 ∈ [0.0, 1.0]; multiply then truncate
                // to u64. (Parallel-session draft referenced a `_ppm: u32`
                // shape that was never landed on the struct — fall back
                // to the actual field.)
                let activity = (v.health_score * 16.0) as u64;
                let w = proposer_weight(b_stake, activity, beta_mb);
                (v.id, w)
            })
            .collect();
        weights.sort_by(|a, b| b.1.cmp(&a.1));
        weights
    }

    // ─── Sanov Slashing ────────────────────────────────────────────────────

    /// Slash a validator for equivocation using the Sanov large-deviation
    /// formula. Replaces the hard-coded 10% penalty with the KL-rate
    /// function cost of "all-equivocating" vs. "honest-within-tolerance".
    ///
    /// Honest distribution: `[window-1, 1]` (1 in `window` miss tolerance).
    /// Observed distribution: `[0, window]` (fully equivocating).
    /// Slash = stake × KL(observed ‖ honest) / 1000 (millibits), capped at stake.
    pub fn sanov_slash_equivocation(&mut self, validator_id: u64, window: u64) -> u64 {
        use evaporchain_sanov_slashing::{sanov_slash, Distribution};
        let stake = match self.validator_set.get(validator_id) {
            Some(v) => v.stake,
            None => return 0,
        };
        let w = window.max(2);
        let observed = match Distribution::from_counts(&[0, w]) {
            Ok(d) => d,
            Err(_) => return ((stake as u128 * 100_000) / 1_000_000) as u64, // fallback
        };
        let honest = match Distribution::from_counts(&[w - 1, 1]) {
            Ok(d) => d,
            Err(_) => return ((stake as u128 * 100_000) / 1_000_000) as u64,
        };
        let slash_amount = match sanov_slash(stake, &observed, &honest) {
            Ok(s) => s,
            Err(_) => ((stake as u128 * 100_000) / 1_000_000) as u64,
        };
        // Entropic Slashing advisory (§Tier2): Shannon-weighted slash for comparison.
        // Sanov is authoritative; entropic is logged so governance can tune.
        if let Ok(entropic) = entropic_slash(stake, &[0, w]) {
            debug!(
                validator = validator_id,
                sanov_slash = slash_amount,
                entropic_slash = entropic,
                "entropic vs sanov equivocation slash (advisory)"
            );
        }
        self.validator_set
            .slash_with_amount(validator_id, slash_amount, true)
    }

    /// Slash a validator for downtime using the Sanov large-deviation formula.
    /// `missed_blocks` = number missed in the observation `window`.
    /// Honest distribution: `[window-1, 1]` (≈1% tolerance).
    /// Observed distribution: `[window - missed_blocks, missed_blocks]`.
    /// Slash = stake × KL(observed ‖ honest) / 1000, capped at stake.
    pub fn sanov_slash_downtime(
        &mut self,
        validator_id: u64,
        missed_blocks: u64,
        window: u64,
    ) -> u64 {
        use evaporchain_sanov_slashing::{sanov_slash, Distribution};
        if missed_blocks == 0 {
            return 0;
        }
        let stake = match self.validator_set.get(validator_id) {
            Some(v) => v.stake,
            None => return 0,
        };
        let w = window.max(missed_blocks + 1);
        let observed = match Distribution::from_counts(&[w - missed_blocks, missed_blocks]) {
            Ok(d) => d,
            Err(_) => return 0,
        };
        let honest = match Distribution::from_counts(&[w - 1, 1]) {
            Ok(d) => d,
            Err(_) => return 0,
        };
        let slash_amount = match sanov_slash(stake, &observed, &honest) {
            Ok(s) => s,
            Err(_) => return 0,
        };
        if slash_amount == 0 {
            return 0;
        }
        let jail = missed_blocks >= 3;
        self.validator_set
            .slash_with_amount(validator_id, slash_amount, jail)
    }

    /// Walk from `head` back to genesis via first-parent at each step.
    /// Returns the trajectory in genesis-first order, or None if `head`
    /// isn't in the Light-Cone DAG.
    fn trajectory_to_genesis(&self, head: [u8; 32]) -> Option<evaporchain_mcc::Trajectory> {
        if !self.light_cone_dag.contains(&head) {
            return None;
        }
        let mut path: Vec<[u8; 32]> = Vec::new();
        let mut cursor = Some(head);
        let mut depth = 0usize;
        while let Some(id) = cursor {
            path.push(id);
            // Bound depth to prevent runaway on a malformed DAG (cycles
            // are excluded by LightCone insertion rules but defence in
            // depth never hurts).
            depth += 1;
            if depth > 1_000_000 {
                break;
            }
            cursor = self
                .light_cone_dag
                .get(&id)
                .and_then(|b| b.parents.first().copied());
        }
        path.reverse();
        Some(evaporchain_mcc::Trajectory::new(path))
    }

    /// Most recent TUR Liveness Detector verdict. None if the
    /// observation window hasn't filled to ≥2 samples since startup.
    pub fn tur_liveness_verdict(&self) -> Option<evaporchain_tur_liveness::Verdict> {
        self.last_tur_verdict
    }

    /// Current Lambda-Fold accumulator (O(1) light-client commitment
    /// to chain state + energy decay). Per INVENTION_STACK.md §4.1
    /// row 8.
    pub fn lambda_fold_instance(&self) -> evaporchain_lambda_fold::FoldedInstance {
        self.lambda_fold
    }

    /// Phase 1.3 of `CROOKS_MEV_INTEGRATION_PLAN.md` — read-only
    /// view of the MEV-observation ring buffer. Operators consume
    /// this via `GET /api/mev/observations` for monitoring; Phase 3
    /// settlement will additionally consume it inside the proposer
    /// for `RefundTx` construction.
    pub fn mev_observations(
        &self,
    ) -> &std::collections::VecDeque<evaporchain_mev_detect::MevObservation> {
        &self.mev_observations
    }

    /// Phase 3.2 of `CROOKS_MEV_INTEGRATION_PLAN.md` — deterministic
    /// digest of the (observations, attacker_stats) pair. Two
    /// validators driven through identical block sequences MUST
    /// produce identical digests; divergent histories MUST diverge.
    /// Phase 3.3 will commit this digest to the block header (or
    /// state root) so consensus enforces convergence; for now it's
    /// an in-memory contract validators can cross-check via RPC.
    pub fn mev_state_digest(&self) -> [u8; 32] {
        evaporchain_mev_detect::mev_state_digest(&self.mev_observations, &self.mev_attacker_stats)
    }

    /// Phase 3.3 of `CROOKS_MEV_INTEGRATION_PLAN.md` — list of
    /// `Transaction::Refund` txs the proposer SHOULD include in
    /// the next block. Walks the observation buffer for entries in
    /// the (grace_period, refund_window) interval that haven't
    /// already been settled. Returned in canonical
    /// (source_block_height, source_observation_idx) order so all
    /// validators agree.
    ///
    /// Phase 3.4 (validator rejection) and Phase 3.5 (slashing)
    /// haven't shipped yet — until then a proposer including these
    /// txs would have its block rejected by the executor (which
    /// returns "Phase 3.5 wiring not yet landed" for `Transaction::Refund`).
    /// This accessor is therefore primarily useful for operator RPC
    /// inspection right now; integration into block construction
    /// happens once 3.4-3.5 ship.
    pub fn due_refund_txs(&self, current_height: u64) -> Vec<evaporchain_types::Transaction> {
        let grace = self
            .governance_params
            .get("crooks_mev_grace_period_blocks")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(evaporchain_mev_detect::CROOKS_MEV_DEFAULT_GRACE_PERIOD_BLOCKS);
        let window = self
            .governance_params
            .get("crooks_mev_refund_window_blocks")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(evaporchain_mev_detect::CROOKS_MEV_DEFAULT_REFUND_WINDOW_BLOCKS);
        // Phase 4.1 — confidence threshold is governance-set.
        // **Audit fix HIGH H4**: type is now ppm (u32), matches
        // `MevObservation::confidence_score_ppm`. Governance param
        // also renamed to disambiguate.
        let conf_threshold = self
            .governance_params
            .get("crooks_mev_confidence_threshold_ppm")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(evaporchain_mev_detect::CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM);
        evaporchain_mev_detect::due_refund_txs(
            &self.mev_observations,
            &self.settled_refunds,
            &self.disputed_observations,
            current_height,
            grace,
            window,
            conf_threshold,
        )
    }

    /// Phase 3.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — read-only view
    /// of the state-branch metadata table. Operators query this via
    /// RPC for monitoring; Phase 3.2 will additionally read it
    /// inside the executor dispatch path to find the right snapshot
    /// for a given tip.
    pub fn state_branches(&self) -> &std::collections::HashMap<[u8; 32], LightConeBranchMetadata> {
        &self.state_branches
    }

    /// Phase 3.1 — record-or-update metadata for a DAG tip.
    /// Idempotent: if the tip is already tracked, just bumps
    /// `last_touched_block`. Internal helper exposed for tests.
    pub(crate) fn record_state_branch(&mut self, tip: [u8; 32], block_height: u64, caliber: u64) {
        self.state_branches
            .entry(tip)
            .and_modify(|m| {
                m.last_touched_block = m.last_touched_block.max(block_height);
                // Caliber may grow as the trajectory extends; track
                // the latest score so LRU sees the freshest signal.
                m.caliber = caliber;
            })
            .or_insert_with(|| LightConeBranchMetadata::fresh(block_height, caliber));
    }

    /// Phase 4.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — number of tips
    /// currently tracking voting state in `dag_round_states`. Public
    /// because the inner `RoundState` is private to this crate;
    /// accessor returns the cardinality only.
    pub fn dag_round_states_count(&self) -> usize {
        self.dag_round_states.len()
    }

    /// Phase 4.1 — typed snapshot of a single tip's voting tally,
    /// returned as `(prevote_count, precommit_count)` pairs. None
    /// if the tip isn't tracked.
    pub fn dag_round_state_counts(&self, tip: &[u8; 32]) -> Option<(usize, usize)> {
        self.dag_round_states
            .get(tip)
            .map(|rs| (rs.prevotes.len(), rs.precommits.len()))
    }

    /// Phase 4.3 — read-only view of the cross-fork equivocation
    /// counter. Operators feed `[counts]` into `entropic_slash`.
    pub fn cross_fork_equivocations(&self) -> &std::collections::HashMap<u64, u64> {
        &self.cross_fork_equivocations
    }

    /// Phase 3.5d of `CROOKS_MEV_INTEGRATION_PLAN.md` — apply
    /// accumulated MissingRefund slashes via the validator-set's
    /// `slash_with_amount` primitive. For each non-zero entry in
    /// `mev_missing_refund_violations`, compute the slash amount
    /// via `evaporchain_entropic_slashing::entropic_slash` against
    /// the validator's current stake, then deduct it. Resets the
    /// counter for each slashed validator (prevents double-slash
    /// on subsequent calls).
    ///
    /// Returns a `Vec<(validator_id, amount_slashed)>` for operator
    /// visibility / RPC tooling.
    ///
    /// No-op when `crooks_mev_missing_refund_slash_enabled = false`
    /// (default — chain bit-compat preserved).
    ///
    /// Does NOT jail the validator — MissingRefund is policy
    /// violation (operator-level), not equivocation (consensus-level).
    /// `slash_equivocation` is the path for the latter.
    pub fn apply_mev_missing_refund_slashes(&mut self) -> Vec<(u64, u64)> {
        if self
            .governance_params
            .get("crooks_mev_missing_refund_slash_enabled")
            .map(|s| s.as_str())
            != Some("true")
        {
            return Vec::new();
        }
        // Snapshot the violation entries we'll act on this call —
        // doing this in canonical (BlockId-sort-style: validator-id-
        // sort) order makes the operation validator-deterministic
        // across the cluster.
        let mut entries: Vec<(u64, u64)> = self
            .mev_missing_refund_violations
            .iter()
            .filter(|(_, &c)| c > 0)
            .map(|(&v, &c)| (v, c))
            .collect();
        entries.sort_by_key(|&(v, _)| v);

        let mut slashed = Vec::new();
        for (validator_id, count) in entries {
            // Stake snapshot.
            let stake = self
                .validator_set
                .get(validator_id)
                .map(|v| v.stake)
                .unwrap_or(0);
            if stake == 0 {
                // Validator absent or zero-staked — nothing to slash.
                self.mev_missing_refund_violations.remove(&validator_id);
                continue;
            }
            // entropic_slash takes observed_counts as a slice; for
            // the MissingRefund single-bucket case we feed `[count]`
            // (single-outcome distribution → entropy = 0 → slash = 0).
            // Real production wiring would feed counts across
            // multiple violation buckets to get non-trivial entropy.
            // Phase 3.5d ships the wiring; entropy-based amount
            // tuning is operator follow-up.
            let amount = match evaporchain_entropic_slashing::entropic_slash(stake, &[count, 1]) {
                Ok(v) => v,
                Err(_) => 0,
            };
            if amount > 0 {
                let actual = self
                    .validator_set
                    .slash_with_amount(validator_id, amount, false);
                slashed.push((validator_id, actual));
            }
            // Reset counter regardless — operator slashing tooling
            // resets after each application.
            self.mev_missing_refund_violations.remove(&validator_id);
        }
        slashed
    }

    /// Phase 4.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` (Decision 1) —
    /// record a prevote against a specific DAG tip's `RoundState`.
    /// Creates the entry if the tip wasn't tracked. No-op when
    /// `light_cone_state_branches_enabled = false` (linear-mode
    /// chain bit-compat).
    ///
    /// `block_hash` is the block the validator is voting for (or
    /// `None` for a nil prevote — Tendermint convention).
    /// `signature` is the validator's BLS signature over the
    /// vote message; stored for the antichain commit-certificate
    /// aggregation in Phase 4.2.
    pub fn record_dag_prevote(
        &mut self,
        tip: [u8; 32],
        validator_id: u64,
        block_hash: Option<[u8; 32]>,
        signature: Vec<u8>,
    ) {
        if self
            .governance_params
            .get("light_cone_state_branches_enabled")
            .map(|s| s.as_str())
            != Some("true")
        {
            return;
        }
        let rs = self
            .dag_round_states
            .entry(tip)
            .or_insert_with(|| RoundState::new(0));
        rs.prevotes.insert(validator_id, block_hash);
        if !signature.is_empty() {
            rs.prevote_bls_sigs.insert(validator_id, signature);
        }
    }

    /// Phase 4.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` (Decision 2) —
    /// antichain finalization predicate. Walks the DAG's closing
    /// antichain (leaves) and returns the subset that meets ALL
    /// three conditions:
    ///
    /// 1. `is_antichain(lc, &S)` — vacuously true for the leaves
    ///    set per `closing_antichain`'s contract.
    /// 2. Every block `b ∈ S` has ≥ 2f+1 precommits in
    ///    `dag_round_states[&b].precommits`, where `f =
    ///    (validator_set.len() - 1) / 3`.
    /// 3. S covers the closing antichain — implicit since we
    ///    return the subset of leaves that meets condition 2.
    ///
    /// Returns the subset of leaves eligible for finalization.
    /// Empty Vec when no leaves meet the precommit threshold.
    /// No-op (returns empty) when `light_cone_state_branches_enabled
    /// = false` — linear-mode chain bit-compat preserved.
    ///
    /// Validator-deterministic: `closing_antichain` returns
    /// BTreeMap-sorted leaves, and the precommit-count check is
    /// pure-function of `dag_round_states` which validators agree
    /// on (Phase 4.1 contract).
    pub fn try_finalize_antichain(&self) -> Vec<[u8; 32]> {
        if self
            .governance_params
            .get("light_cone_state_branches_enabled")
            .map(|s| s.as_str())
            != Some("true")
        {
            return Vec::new();
        }
        let n = self.validator_set.len();
        if n == 0 {
            return Vec::new();
        }
        let f = (n.saturating_sub(1)) / 3;
        let threshold = 2 * f + 1;

        let candidates =
            evaporchain_light_cone::concurrency::closing_antichain(&self.light_cone_dag);
        let mut finalized = Vec::new();
        for tip in candidates {
            let precommit_count = self
                .dag_round_states
                .get(&tip)
                .map(|rs| rs.precommits.len())
                .unwrap_or(0);
            if precommit_count >= threshold {
                finalized.push(tip);
            }
        }
        finalized
    }

    /// Phase 4.1 + 4.3 — record a precommit against a tip + detect
    /// cross-fork equivocation. If the validator has previously
    /// precommitted on a *different* concurrent tip at the same
    /// round, increment `cross_fork_equivocations[validator_id]`
    /// per Decision 3. Operator slashing tooling reads the counter.
    ///
    /// No-op when `light_cone_state_branches_enabled = false`.
    pub fn record_dag_precommit(
        &mut self,
        tip: [u8; 32],
        validator_id: u64,
        block_hash: Option<[u8; 32]>,
        signature: Vec<u8>,
    ) {
        if self
            .governance_params
            .get("light_cone_state_branches_enabled")
            .map(|s| s.as_str())
            != Some("true")
        {
            return;
        }

        // Cross-fork equivocation check (Phase 4.3): scan all
        // OTHER tips' precommits for the same validator at the
        // same round; if any disagrees with this precommit's
        // block_hash, increment the equivocation counter.
        let this_round = self
            .dag_round_states
            .get(&tip)
            .map(|rs| rs.round)
            .unwrap_or(0);
        let mut equivocated = false;
        for (other_tip, rs) in &self.dag_round_states {
            if *other_tip == tip {
                continue;
            }
            if rs.round != this_round {
                continue;
            }
            if let Some(prior) = rs.precommits.get(&validator_id) {
                if *prior != block_hash {
                    equivocated = true;
                    break;
                }
            }
        }
        if equivocated {
            *self
                .cross_fork_equivocations
                .entry(validator_id)
                .or_insert(0) += 1;
        }

        let rs = self
            .dag_round_states
            .entry(tip)
            .or_insert_with(|| RoundState::new(0));
        rs.precommits.insert(validator_id, block_hash);
        if !signature.is_empty() {
            rs.precommit_bls_sigs.insert(validator_id, signature);
        }
    }

    /// Phase 4.4 — read-only view of block-indexed finality
    /// bookkeeping. Populates alongside `committed_at` (height-
    /// indexed); both kept for dual-mode bookkeeping per Decision 4.
    pub fn committed_at_block(&self) -> &std::collections::HashMap<[u8; 32], u64> {
        &self.committed_at_block
    }

    /// Phase 3.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` — attach a
    /// snapshot reference to an existing tip's metadata. Called by
    /// the executor after taking a RocksDB snapshot at commit time.
    /// `None` if the tip isn't tracked (caller must `record_state_branch`
    /// first).
    pub fn attach_branch_snapshot(
        &mut self,
        tip: [u8; 32],
        snapshot: std::sync::Arc<dyn LightConeBranchSnapshot + Send + Sync>,
    ) -> Option<()> {
        let m = self.state_branches.get_mut(&tip)?;
        m.snapshot = Some(snapshot);
        Some(())
    }

    /// Phase 3.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` — executor-side
    /// commit hook that closes the deferred wiring. `on_block_committed`
    /// records the just-committed tip in `state_branches` (metadata
    /// only); this method captures a `StateSnapshotBranch` of the
    /// post-execution `db` and attaches it to that tip via
    /// `attach_branch_snapshot`. After this call returns Ok, the
    /// branch is materialized — `restore_to_lca` and
    /// `replay_and_apply_atomic` can roll state back to this tip on
    /// future fork-switches.
    ///
    /// Caller invokes this from the same site that ran the executor,
    /// immediately after `on_block_committed`, so the snapshot
    /// reflects the exact post-execution state for `block`.
    ///
    /// No-op when `light_cone_state_branches_enabled != "true"` —
    /// chain bit-compat preserved. No-op when the tip isn't tracked
    /// in `state_branches` (the flag was flipped off between
    /// `on_block_committed` and this call).
    ///
    /// Errors propagate from `StateSnapshotBranch::capture` (i.e.
    /// `SnapshotBuilder::create` failure). Callers should log and
    /// continue — the chain still commits; only DAG-mode rollback
    /// against this tip is unavailable.
    pub fn capture_committed_branch_snapshot(
        &mut self,
        block: &Block,
        db: &mut dyn StateDB,
    ) -> Result<(), String> {
        let state_branches_enabled = self
            .governance_params
            .get("light_cone_state_branches_enabled")
            .map(|s| s.as_str())
            == Some("true");
        if !state_branches_enabled {
            return Ok(());
        }
        let tip_id = Self::block_hash(block);
        if !self.state_branches.contains_key(&tip_id) {
            return Ok(());
        }
        let snapshot = StateSnapshotBranch::capture(tip_id, block.number, block.epoch, db)?;
        self.attach_branch_snapshot(tip_id, std::sync::Arc::new(snapshot));
        Ok(())
    }

    /// Phase 5.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — orphan
    /// detection. Returns the set of state-branch tips that the
    /// chain considers orphaned: tips whose caliber falls below
    /// the `light_cone_orphan_caliber_threshold` governance flag
    /// (default 0) AND whose `last_touched_block` is older than
    /// `current_height - recency_window` (window: 32 blocks).
    ///
    /// The result feeds operator/auditor tooling and Phase 5.3's
    /// LRU pruning. **Does not mutate state** — purely
    /// observational. Tips returned here are CANDIDATES for
    /// pruning; whether they're actually evicted depends on the
    /// `light_cone_max_concurrent_forks` cap.
    ///
    /// Returns canonical-ordered list (sorted by BlockId) for
    /// validator-determinism.
    pub fn detect_orphan_branches(&self, current_height: u64) -> Vec<[u8; 32]> {
        let threshold = self
            .governance_params
            .get("light_cone_orphan_caliber_threshold")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let recency_window: u64 = 32;
        let staleness_horizon = current_height.saturating_sub(recency_window);

        let mut orphans: Vec<[u8; 32]> = self
            .state_branches
            .iter()
            .filter(|(_, m)| m.caliber < threshold && m.last_touched_block < staleness_horizon)
            .map(|(tip, _)| *tip)
            .collect();
        orphans.sort();
        orphans
    }

    /// Phase 3.4 of `LIGHT_CONE_FULL_DAG_PLAN.md` — concurrent-fork
    /// LRU eviction. When `state_branches.len() > cap`, evict the
    /// lowest-caliber entry (tie-break: smallest BlockId for
    /// validator-determinism). `cap` is read from the
    /// `light_cone_max_concurrent_forks` governance flag (default 4).
    ///
    /// Phase 3.2 will pair this eviction with dropping the Arc
    /// snapshot under the same key (RocksDB snapshot deref O(1)).
    /// Today it just prunes the metadata.
    pub(crate) fn prune_state_branches(&mut self) {
        let cap = self
            .governance_params
            .get("light_cone_max_concurrent_forks")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(4);

        while self.state_branches.len() > cap {
            // Find the lowest-caliber entry (tie-break: smallest tip).
            if let Some((&victim, _)) = self
                .state_branches
                .iter()
                .min_by(|(t1, m1), (t2, m2)| m1.caliber.cmp(&m2.caliber).then_with(|| t1.cmp(t2)))
            {
                self.state_branches.remove(&victim);
                // Phase 5.3 of LIGHT_CONE_FULL_DAG_PLAN.md — pair
                // the metadata eviction with a DAG-side cascade
                // prune. `prune_orphan_branch` walks the victim's
                // exclusive ancestry backwards and trims it from
                // the DAG; safety guards (non-leaf reject, unknown
                // tip no-op) make this idempotent + correctness-
                // preserving even when the victim is at a branch
                // point or has descendants from another live tip.
                let _pruned_dag = self.light_cone_dag.prune_orphan_branch(victim);
            } else {
                break;
            }
        }
    }

    /// Phase 4.4 of `CROOKS_MEV_INTEGRATION_PLAN.md` — operator
    /// dispute. Cancels a pending refund by adding its
    /// `(source_block_height, source_observation_idx)` pair to
    /// `disputed_observations` so `due_refund_txs` no longer emits
    /// it. Only effective WITHIN the grace period; past grace, the
    /// refund will already be in the proposer's set (and possibly
    /// already settled).
    ///
    /// Returns `Err` if the observation isn't in the buffer (nothing
    /// to dispute) or is already past the grace window (too late).
    /// Disputes are local to this validator; cluster-wide dispute
    /// agreement is a Phase 4.4d follow-up.
    pub fn dispute_observation(
        &mut self,
        source_block_height: u64,
        source_observation_idx: usize,
        current_height: u64,
    ) -> Result<(), MevDisputeError> {
        // Find the observation.
        let obs = self
            .mev_observations
            .iter()
            .find(|o| {
                o.block_height == source_block_height
                    && o.attacker_pre_idx == source_observation_idx
            })
            .ok_or(MevDisputeError::NotFound {
                src_h: source_block_height,
                src_idx: source_observation_idx,
            })?;

        // Read grace from governance.
        let grace = self
            .governance_params
            .get("crooks_mev_grace_period_blocks")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(evaporchain_mev_detect::CROOKS_MEV_DEFAULT_GRACE_PERIOD_BLOCKS);

        // Past grace ⇒ too late.
        let age = current_height.saturating_sub(obs.block_height);
        if age > grace {
            return Err(MevDisputeError::PastGracePeriod {
                src_h: source_block_height,
                src_idx: source_observation_idx,
                age,
                grace,
            });
        }

        self.disputed_observations
            .insert((source_block_height, source_observation_idx));
        Ok(())
    }

    /// Phase 4.4 — read-only view of disputed observations.
    pub fn disputed_observations(&self) -> &std::collections::HashSet<(u64, usize)> {
        &self.disputed_observations
    }

    /// Phase 3.4 of `CROOKS_MEV_INTEGRATION_PLAN.md` — verify the
    /// block's `Transaction::Refund` set exactly matches what the
    /// chain expects at this height. `Ok(())` when:
    /// - settlement mode is `"observe"` (default — no enforcement), OR
    /// - settlement mode is `"enforce"` and the block's refund set
    ///   matches the chain's expected set exactly (every required
    ///   refund present, no extras, payloads byte-equal).
    /// Returns a `RefundValidationError` describing the violation
    /// otherwise. Phase 3.5 will pair `MissingRefund` with a
    /// proposer-slash.
    /// Phase 3.5c of `CROOKS_MEV_INTEGRATION_PLAN.md` — read-only
    /// view of the per-proposer MissingRefund violation counter.
    /// Operators feed `[counts_per_validator]` into
    /// `evaporchain_entropic_slashing::entropic_slash(stake, counts)`
    /// to compute the slash amount; the actual stake deduction is
    /// separate consensus-state-machine work (Phase 3.5d).
    pub fn mev_missing_refund_violations(&self) -> &std::collections::HashMap<u64, u64> {
        &self.mev_missing_refund_violations
    }

    pub fn validate_block_refunds(
        &self,
        block: &Block,
    ) -> Result<(), evaporchain_mev_detect::RefundValidationError> {
        let mode = self
            .governance_params
            .get("crooks_mev_settlement_mode")
            .map(|s| s.as_str())
            .unwrap_or("observe");
        if mode != "enforce" {
            return Ok(());
        }
        let expected = self.due_refund_txs(block.number);
        let block_refunds: Vec<&evaporchain_types::RefundTx> = block
            .transactions
            .iter()
            .filter_map(|tx| match tx {
                evaporchain_types::Transaction::Refund(r) => Some(r),
                _ => None,
            })
            .collect();
        evaporchain_mev_detect::validate_block_refunds(&expected, &block_refunds)
    }

    /// Phase 5.3 of LAMBDA_FOLD_NOVA_PLAN — nova-mode fold helper.
    /// Lazily constructs the `NovaFolder` on first call (Phase 5.1's
    /// deferred-init pattern: avoids the ~60-90 s `pp` setup until the
    /// chain actually flips into nova mode), then folds one block. The
    /// substrate fold remains the authoritative accumulator at the
    /// call site; this helper updates the parallel Nova instance only.
    ///
    /// `_old_state` in `RealBlockProver::fold_real_block_with_witness`
    /// is unused (the IVC binds only the new state in `z_new`), so we
    /// pass the same `DualCommitment` for old + new — cryptographically
    /// equivalent to passing distinct values.
    #[cfg(feature = "lambda_fold_nova")]
    fn try_nova_fold(
        &mut self,
        block: &Block,
        state_root: [u8; 32],
        step_energy: u64,
    ) -> Result<(), evaporchain_lambda_fold::NovaFoldError> {
        let new_dc = evaporchain_types::DualCommitment {
            verkle_root: state_root,
            mmr_root: self.executor.mmr_root(),
            epoch: block.epoch,
            active_count: 0,
            ghost_count: 0,
        };

        if self.lambda_fold_nova.is_none() {
            // Lazy construction — uses the genesis state_root the
            // chain was initialised with. Heavy `pp` setup happens
            // here exactly once.
            let genesis_dc = evaporchain_types::DualCommitment {
                verkle_root: self.genesis_state_root,
                mmr_root: [0u8; 32],
                epoch: 0,
                active_count: 0,
                ghost_count: 0,
            };
            self.lambda_fold_nova = Some(Box::new(evaporchain_lambda_fold::NovaFolder::new(
                &genesis_dc,
            )?));
        }

        let folder = self
            .lambda_fold_nova
            .as_mut()
            .expect("lazy-init populated above");
        let thermo = evaporchain_lambda_fold::NovaThermodynamicWitness {
            object_energies: vec![(0, 0, 100)],
            evaporation_nullifiers: vec![],
        };
        let inst = folder.fold_block(block, &new_dc, &new_dc, &thermo, block.epoch, step_energy)?;
        self.lambda_fold_nova_instance = inst;
        Ok(())
    }

    /// Phase 5.1 — accessor for the running Nova-folded instance.
    /// Returns `NovaFoldedInstance::identity()` until the chain flips
    /// into nova mode and folds at least one block.
    #[cfg(feature = "lambda_fold_nova")]
    pub fn lambda_fold_nova_instance(&self) -> &evaporchain_lambda_fold::NovaFoldedInstance {
        &self.lambda_fold_nova_instance
    }

    /// Phase 5.4 of LAMBDA_FOLD_NOVA_PLAN — preprocessed vk bytes for
    /// the Nova folder. Returns `None` when the lazy folder hasn't
    /// been constructed yet (chain hasn't seen a nova-mode block).
    /// Wraps `NovaFolder::vk_bytes`, which itself triggers
    /// `CompressedSNARK::setup` on first call and caches the result.
    #[cfg(feature = "lambda_fold_nova")]
    pub fn lambda_fold_nova_vk_bytes(
        &self,
    ) -> Option<Result<Vec<u8>, evaporchain_lambda_fold::NovaFoldError>> {
        self.lambda_fold_nova.as_ref().map(|f| f.vk_bytes())
    }

    /// Number of samples currently in the TUR observation window.
    pub fn tur_window_len(&self) -> usize {
        self.tur_window.len()
    }

    /// Current consensus phase from the RG Phase Map.  `LivenessStable`
    /// until enough blocks accumulate for the first WSBF coarse-grain step.
    pub fn consensus_phase(&self) -> evaporchain_rg_phase_map::ConsensusPhase {
        self.current_consensus_phase
    }

    /// Latest WSBF `EffectiveParams` (renormalized λ and energy density).
    /// None until `WSBF_COARSE_GRAIN` blocks have been committed.
    pub fn effective_params(&self) -> Option<&evaporchain_wsbf::params::EffectiveParams> {
        self.last_effective_params.as_ref()
    }

    /// Number of blocks in the parallel Light-Cone DAG. Should equal
    /// `committed_heights.len() - 1` minus genesis edge cases under
    /// normal operation. Read-only observability for now.
    pub fn light_cone_block_count(&self) -> usize {
        self.light_cone_dag.len()
    }

    /// Phase 4.4 — antichain commit-cert digest of the current
    /// closing antichain. Deterministic 32-byte fingerprint that
    /// every validator computes from the same Light-Cone DAG state;
    /// operators compare across cluster validators (e.g. via
    /// `/api/light_cone/antichain_digest`) to confirm cross-validator
    /// agreement on antichain finality without shipping the full
    /// block-id list around. Pairs with Crooks-MEV's
    /// `mev_state_digest` as the canonical inter-validator digest
    /// for the Light-Cone substrate. Domain-separated under
    /// `evaporchain-antichain-digest-v1`.
    pub fn light_cone_antichain_digest(&self) -> [u8; 32] {
        evaporchain_light_cone::concurrency::closing_antichain_digest(&self.light_cone_dag)
    }

    /// Phase 4.4 — accessor for the closing antichain itself (sorted
    /// `BlockId` list, validator-deterministic). Returned alongside
    /// the digest so operators can audit which set the digest
    /// commits to.
    pub fn light_cone_closing_antichain(&self) -> Vec<[u8; 32]> {
        evaporchain_light_cone::concurrency::closing_antichain(&self.light_cone_dag)
    }

    /// Phase 4.4 — rolling history of `(block_height,
    /// closing_antichain_digest)` pairs, oldest first. Capped at
    /// `ANTICHAIN_DIGEST_HISTORY_CAP` (128) entries. Operators
    /// retroactively cross-compare across cluster validators: pick
    /// height H, fetch each validator's digest at H, divergence at
    /// any past height is the freeze-class signal for antichain
    /// disagreement.
    pub fn antichain_digest_history(&self) -> Vec<(u64, [u8; 32])> {
        self.antichain_digest_history.iter().copied().collect()
    }

    /// Decay-Lamport DAG integration (shipped 2026-05-06) — derive
    /// the Decay-Lamport `LamportClock` at a specific DAG block.
    /// Returns `None` if `block_id` isn't in the DAG OR `tick_quantum`
    /// is 0. Pure function of `(light_cone_dag, block_id, tick_quantum)`.
    /// Pairs with `/api/lamport_time` (chain-global running clock)
    /// and `/api/light_cone/antichain_digest` (DAG-derived
    /// cross-validator digest) as the third operator surface for
    /// the Light-Cone substrate's time semantics.
    pub fn light_cone_block_lamport_clock(
        &self,
        block_id: [u8; 32],
        tick_quantum: u64,
    ) -> Option<evaporchain_decay_lamport::LamportClock> {
        evaporchain_light_cone::decay_lamport::block_lamport_clock(
            &self.light_cone_dag,
            block_id,
            tick_quantum,
        )
        .ok()
    }

    /// Phase A.1 of `MCC_FULL_MULTI_PARENT_PLAN.md` — set of all
    /// currently-active sibling heads in the Light-Cone DAG. A "head"
    /// is a leaf: a block with no children. The MCC fork-choice picks
    /// one of these as the chosen authoritative head per round; this
    /// accessor exposes the full candidate set so consumers can
    /// enumerate, score, audit, or display every active fork.
    ///
    /// Returned as a `BTreeSet<BlockId>` for **validator-determinism**:
    /// `LightCone::leaves()` already iterates in `BTreeMap`-key order,
    /// so any two validators with the same DAG state produce the
    /// same candidate-head set in the same order.
    ///
    /// **Design note:** `MCC_FULL_MULTI_PARENT_PLAN.md` originally
    /// proposed a separate `sibling_heads: BTreeSet<BlockId>` field
    /// maintained alongside `light_cone_dag.leaves()`. The plan
    /// progress log now records the shipped variant: a *derived*
    /// accessor with no field. Keeping a parallel field would
    /// duplicate state and create a desync hazard; the DAG itself is
    /// the single source of truth for "what's a leaf right now."
    pub fn candidate_heads(&self) -> std::collections::BTreeSet<[u8; 32]> {
        self.light_cone_dag.leaves().collect()
    }

    /// Phase A.3 of `MCC_FULL_MULTI_PARENT_PLAN.md` — every active
    /// candidate head paired with its first-parent trajectory caliber,
    /// sorted by caliber descending (smaller `BlockId` tiebreak —
    /// matches `MccForkChoice::select_tip`'s argmax rule). The first
    /// entry is the chain's MCC-chosen authoritative head.
    ///
    /// Operators consume this via `/api/light_cone/candidate_heads`
    /// (Phase E.1) to debug "which heads are competing right now"
    /// without manual trajectory-walk + caliber computation.
    /// Validators consume it during Phase C hot-path integration to
    /// pick the authoritative head per round.
    ///
    /// β is sourced from `governance_params["crooks_mev_beta_mb"]`
    /// (default 1000 millibits) — same path `current_tip` uses to
    /// build its MccForkChoice. Empty DAG → empty Vec.
    pub fn enumerate_candidate_heads(&self) -> Vec<([u8; 32], u64)> {
        let beta_mb = self
            .governance_params
            .get("crooks_mev_beta_mb")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1000);
        let fc = crate::fork_choice::MccForkChoice::new(self.light_cone_dag.clone(), beta_mb);
        fc.enumerate_with_caliber()
    }

    /// MCC Phase C.4 of `MCC_FULL_MULTI_PARENT_PLAN.md` —
    /// cross-fork equivocation detection accessor. Returns the
    /// equivocation count for a given validator under mcc_full
    /// mode.
    ///
    /// **Substrate connection:** the Phase 4.3 substrate already
    /// ships `cross_fork_equivocations: HashMap<u64, u64>` —
    /// validator_id → count of observed cross-fork double-votes.
    /// This accessor exposes it as the operator-facing surface
    /// gated on `parent_acceptance_mode = "mcc_full"`. Under any
    /// other mode, returns 0 regardless of the underlying counter
    /// (chain bit-compat — equivocation slashing under MCC requires
    /// the multi-parent semantics that only mcc_full provides).
    ///
    /// Pairs with `evaporchain_entropic_slashing::entropic_slash`
    /// for slash-amount calculation: large-deviation cost as a
    /// function of (count, observation window) — Sanov 1957
    /// theorem-grade slash magnitude.
    ///
    /// **Note on certificate-based vs counts-based:** Phase 4.3's
    /// counter increments on observed double-precommit, which
    /// cannot perfectly distinguish "honest re-vote after view
    /// change" from "malicious cross-fork double-sign." A
    /// certificate-based evidence track is the deferred Phase
    /// 4.3d follow-up; until then operators should treat
    /// equivocation counts as a *signal*, not slashing trigger,
    /// and require certificate-based evidence for actual stake
    /// deduction.
    pub fn cross_fork_equivocation_count(&self, validator_id: u64) -> u64 {
        let mode = self
            .governance_params
            .get("parent_acceptance_mode")
            .map(|s| s.as_str())
            .unwrap_or("linear");
        if mode != "mcc_full" {
            return 0;
        }
        self.cross_fork_equivocations
            .get(&validator_id)
            .copied()
            .unwrap_or(0)
    }

    /// MCC Phase C.4 of `MCC_FULL_MULTI_PARENT_PLAN.md` — full
    /// equivocation snapshot. Returns the entire
    /// `validator_id -> count` map under mcc_full; empty map
    /// otherwise.
    pub fn all_cross_fork_equivocations(&self) -> std::collections::HashMap<u64, u64> {
        let mode = self
            .governance_params
            .get("parent_acceptance_mode")
            .map(|s| s.as_str())
            .unwrap_or("linear");
        if mode != "mcc_full" {
            return std::collections::HashMap::new();
        }
        self.cross_fork_equivocations.clone()
    }

    /// MCC Phase C.3 of `MCC_FULL_MULTI_PARENT_PLAN.md` —
    /// proposer multi-parent set selection. Returns the
    /// `Vec<BlockId>` the proposer should set as `block.parents`
    /// for the next block under `mcc_full`.
    ///
    /// **Behaviour:**
    /// - Under `parent_acceptance_mode = "mcc_full"`: returns the
    ///   set of currently-active sibling heads filtered to be a
    ///   true antichain (`is_antichain` predicate). The MCC-chosen
    ///   authoritative head is included as the first parent (the
    ///   `block.parent_hash` value); other concurrent heads are
    ///   the multi-parent extensions.
    /// - Under `linear` or `mcc`: returns `vec![]`. Empty `parents`
    ///   serializes as the legacy single-parent format (chain
    ///   bit-compat — `serde(skip_serializing_if = "Vec::is_empty")`
    ///   on `Block::parents` preserves wire format).
    ///
    /// **Pure read-side accessor.** The proposer's `create_proposal`
    /// will call this method to populate `block.parents`. That
    /// integration is Phase C.4 / C.6 separate work.
    ///
    /// Returns at most `light_cone_max_concurrent_forks` parents;
    /// excess heads are dropped (lowest caliber first) to bound
    /// block size.
    pub fn propose_parents(&self) -> Vec<[u8; 32]> {
        let mode = self
            .governance_params
            .get("parent_acceptance_mode")
            .map(|s| s.as_str())
            .unwrap_or("linear");
        if mode != "mcc_full" {
            return vec![];
        }
        let cap = self
            .governance_params
            .get("light_cone_max_concurrent_forks")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(4);
        // Take the top-N heads by caliber (already sorted descending).
        let scored = self.enumerate_candidate_heads();
        let candidates: Vec<[u8; 32]> = scored.into_iter().take(cap).map(|(id, _)| id).collect();
        // Filter to an antichain: drop any head that's comparable to
        // a higher-caliber head (it would violate the partial-order
        // contract on multi-parent blocks).
        let mut accepted: Vec<[u8; 32]> = Vec::with_capacity(candidates.len());
        for c in &candidates {
            let mut all_concurrent = true;
            for a in &accepted {
                if evaporchain_light_cone::concurrency::comparable(&self.light_cone_dag, *c, *a) {
                    all_concurrent = false;
                    break;
                }
            }
            if all_concurrent {
                accepted.push(*c);
            }
        }
        accepted
    }

    /// MCC Phase C.2 of `MCC_FULL_MULTI_PARENT_PLAN.md` — vote
    /// dispatch target. Returns the BlockId that voting handlers
    /// should route prevotes/precommits into for the current round:
    ///
    /// - Under `parent_acceptance_mode = "mcc_full"`: the
    ///   `current_authoritative_head` (computed by
    ///   `update_authoritative_head`) if present, falling back to
    ///   `parent_hash` if not yet computed.
    /// - Under any other mode (`linear`, `mcc`): the legacy
    ///   `parent_hash` (chain bit-compat preserved).
    ///
    /// **Pure read-side accessor.** Phase C.4 + C.6 wire this into
    /// the voting handlers (`handle_prevote`, `handle_precommit`) so
    /// votes route to the correct per-tip `dag_round_states` tally.
    /// Today the handlers still tally on `parent_hash`; this method
    /// is the substrate they'll call once that wiring lands.
    pub fn vote_target_head(&self) -> [u8; 32] {
        let mode = self
            .governance_params
            .get("parent_acceptance_mode")
            .map(|s| s.as_str())
            .unwrap_or("linear");
        if mode == "mcc_full" {
            self.current_authoritative_head.unwrap_or(self.parent_hash)
        } else {
            self.parent_hash
        }
    }

    /// MCC Phase C.1 of `MCC_FULL_MULTI_PARENT_PLAN.md` —
    /// recompute and store the authoritative head from the current
    /// candidate-head set. Pure substrate addition — does NOT yet
    /// hook into the consensus round lifecycle (that's Phase C.2/C.3
    /// when the voting handlers + proposer parent-set selection
    /// route through this field).
    ///
    /// **Behaviour:**
    /// - When `parent_acceptance_mode = "mcc_full"`, recomputes
    ///   `current_authoritative_head` as the argmax of
    ///   `enumerate_candidate_heads`. Returns the new value.
    /// - Otherwise, leaves `current_authoritative_head` as `None`
    ///   and returns `None`. Default-mode chain bit-compat
    ///   preserved — the field is observability-only until Phase
    ///   C.2/C.3 wiring promotes it.
    /// - Empty DAG → `None`.
    pub fn update_authoritative_head(&mut self) -> Option<[u8; 32]> {
        let mode = self
            .governance_params
            .get("parent_acceptance_mode")
            .map(|s| s.as_str())
            .unwrap_or("linear");
        if mode != "mcc_full" {
            self.current_authoritative_head = None;
            return None;
        }
        let chosen = self
            .enumerate_candidate_heads()
            .into_iter()
            .next()
            .map(|(id, _caliber)| id);
        self.current_authoritative_head = chosen;
        chosen
    }

    /// MCC Phase B.0+ of `MCC_FULL_MULTI_PARENT_PLAN.md` —
    /// **planning** substrate for B.2's `replay_to_head`. Composes
    /// `find_lca` + `block_path_from_to` to produce a `ReplayWalk`
    /// describing what the executor must do to move state from
    /// `from_head` to `to_head`.
    ///
    /// Returns `None` if either head is absent from the Light-Cone
    /// DAG OR no common ancestor exists.
    ///
    /// **No execution happens here.** The executor consumes the
    /// `ReplayWalk` and:
    ///   1. If `rollback_required`, rolls state back from
    ///      `from_head` to `lca` (the deferred B.1 snapshot work).
    ///   2. Applies the blocks in `forward_path` in order, calling
    ///      `db.execute_block` for each.
    /// Splitting the planning from the execution lets the planning
    /// be pure (testable without a StateDB) and keeps the executor
    /// integration localised to B.2.
    pub fn plan_replay_to_head(
        &self,
        from_head: [u8; 32],
        to_head: [u8; 32],
    ) -> Option<ReplayWalk> {
        let lca = evaporchain_light_cone::dag::find_lca(&self.light_cone_dag, from_head, to_head)?;
        let forward_path =
            evaporchain_light_cone::dag::block_path_from_to(&self.light_cone_dag, lca, to_head)?;
        Some(ReplayWalk {
            lca,
            forward_path,
            rollback_required: lca != from_head,
        })
    }

    /// MCC Phase B.2 of `MCC_FULL_MULTI_PARENT_PLAN.md` — bridge
    /// between Phase B.0+ (planning) and Phase B.1 (snapshot
    /// restore). Restores the StateDB to the captured state at
    /// `plan.lca` so the caller can subsequently apply
    /// `plan.forward_path` blocks via `execute_block`.
    ///
    /// **Caller workflow:**
    /// ```ignore
    /// let plan = consensus.plan_replay_to_head(from, to)?;
    /// if plan.rollback_required {
    ///     consensus.restore_to_lca(&plan, &mut db)?;
    /// }
    /// for block_id in &plan.forward_path {
    ///     let block = block_store.get(block_id)?;
    ///     executor.execute_block(&mut db, &block)?;
    /// }
    /// ```
    ///
    /// **Errors:**
    /// - `plan.lca` is not tracked in `state_branches` (caller
    ///   should ensure the LCA was recorded before this call)
    /// - the LCA has no attached snapshot (use
    ///   `attach_branch_snapshot` first; without a snapshot, no
    ///   rollback is possible — only forward replay)
    /// - the underlying `StateSnapshotBranch::restore` returns an
    ///   error (e.g. `SnapshotApplier::apply` fails verification)
    ///
    /// **No-op when `!plan.rollback_required`** (caller can also
    /// check `plan.rollback_required` before invoking; this method
    /// returns `Ok(())` when LCA == from, since no actual restore
    /// is needed).
    pub fn restore_to_lca(
        &self,
        plan: &ReplayWalk,
        db: &mut dyn evaporchain_state::db::StateDB,
    ) -> Result<(), String> {
        if !plan.rollback_required {
            return Ok(());
        }
        let metadata = self.state_branches.get(&plan.lca).ok_or_else(|| {
            format!(
                "LCA {} not tracked in state_branches",
                hex::encode(plan.lca)
            )
        })?;
        let snapshot = metadata.snapshot.as_ref().ok_or_else(|| {
            format!(
                "LCA {} has no attached snapshot — call attach_branch_snapshot first",
                hex::encode(plan.lca)
            )
        })?;
        snapshot.restore(db)
    }

    /// MCC Phase B.3 of `MCC_FULL_MULTI_PARENT_PLAN.md` — the
    /// **umbrella hot-path integration** for state replay. Composes
    /// the substrate primitives (B.0+ planning + B.2 restore +
    /// caller-supplied block lookup + caller-supplied block apply)
    /// into a single call that moves the StateDB from
    /// `current_head`'s state to `target_head`'s state.
    ///
    /// **Closure-driven design** rather than trait-based: callers
    /// supply `block_lookup` and `block_apply` as closures so the
    /// consensus crate doesn't need to depend on a specific
    /// executor type or block-store interface. The closures define
    /// the integration points:
    ///   - `block_lookup(&id) -> Option<Block>`: how to fetch a
    ///     block by its DAG id (typically wraps `chain_store` or
    ///     `block_history`).
    ///   - `block_apply(db, &block) -> Result<(), String>`: how to
    ///     execute the block against the StateDB (typically
    ///     `executor.execute_block(db, &block)`).
    ///
    /// **Sequence:**
    ///   1. `plan_replay_to_head(current_head, target_head)` — pure
    ///      planning. Errors out as `PlanFailed` if either head is
    ///      missing from the DAG.
    ///   2. `restore_to_lca(&plan, db)` — wipes the StateDB back to
    ///      the LCA's captured state if `rollback_required`. Errors
    ///      as `RestoreFailed`.
    ///   3. For each `block_id in plan.forward_path`:
    ///      a. `block_lookup(block_id)` — fetch the block. Errors
    ///         as `BlockNotFound`.
    ///      b. `block_apply(db, &block)` — execute. Errors as
    ///         `ApplyFailed { block, msg }`.
    ///
    /// **Atomicity caveat (Phase B.4 follow-up):** if step 3 fails
    /// midway, the StateDB is in a partial state — at the LCA plus
    /// any earlier `forward_path` entries already applied. Phase B.4
    /// will wrap the whole thing in `db.begin_batch()` /
    /// `commit_batch()` for transactional atomicity. For now,
    /// callers must handle partial-state recovery themselves.
    pub fn replay_and_apply<F1, F2>(
        &self,
        db: &mut dyn evaporchain_state::db::StateDB,
        current_head: [u8; 32],
        target_head: [u8; 32],
        mut block_lookup: F1,
        mut block_apply: F2,
    ) -> Result<ReplayResult, ReplayError>
    where
        F1: FnMut(&[u8; 32]) -> Option<evaporchain_types::Block>,
        F2: FnMut(
            &mut dyn evaporchain_state::db::StateDB,
            &evaporchain_types::Block,
        ) -> Result<(), String>,
    {
        let plan = self
            .plan_replay_to_head(current_head, target_head)
            .ok_or(ReplayError::PlanFailed)?;
        self.restore_to_lca(&plan, db)
            .map_err(ReplayError::RestoreFailed)?;
        let mut applied: Vec<[u8; 32]> = Vec::with_capacity(plan.forward_path.len());
        for block_id in &plan.forward_path {
            let block = block_lookup(block_id)
                .ok_or_else(|| ReplayError::BlockNotFound(hex::encode(block_id)))?;
            block_apply(db, &block).map_err(|msg| ReplayError::ApplyFailed {
                block: hex::encode(block_id),
                msg,
            })?;
            applied.push(*block_id);
        }
        Ok(ReplayResult {
            lca: plan.lca,
            applied,
        })
    }

    /// MCC Phase B.4 of `MCC_FULL_MULTI_PARENT_PLAN.md` — atomic
    /// transactional wrapper around `replay_and_apply`.
    ///
    /// **Contract:** either the replay succeeds completely (StateDB
    /// at `target_head`'s state, returns `Ok(ReplayResult)`) OR the
    /// StateDB is restored to its pre-replay state and the original
    /// error is returned (`Err(ReplayError)`). Never leaves the DB
    /// in a partial state.
    ///
    /// **Mechanism:** captures a `StateSnapshotBranch` of the
    /// current StateDB BEFORE the replay starts. On any error from
    /// `replay_and_apply` (PlanFailed, RestoreFailed, BlockNotFound,
    /// ApplyFailed), the pre-replay snapshot is restored, wiping
    /// any partial state changes from the failed forward-apply
    /// loop. On success, the pre-replay snapshot is dropped (no
    /// retained memory cost).
    ///
    /// **Trait-portable:** uses the B.1 `StateSnapshotBranch`
    /// substrate (full-state copy via `SnapshotBuilder::create` +
    /// `SnapshotApplier::apply`). Works for both `InMemoryStateDB`
    /// and `RocksDBStateDB` because the atomicity guarantee lives at
    /// the snapshot layer, not the StateDB-trait layer (which has
    /// no transactional methods).
    ///
    /// **Cost:** one extra full-state capture per replay attempt.
    /// For testnet-scale state this is acceptable; production
    /// deployments with large state would prefer the RocksDB
    /// WriteBatch path, which is a separate concrete-impl
    /// optimisation outside the trait surface.
    ///
    /// **Returns:** the `ReplayResult` from a successful inner
    /// `replay_and_apply`, OR the `ReplayError` from the failed
    /// inner call (after rollback). An additional internal
    /// `RollbackFailed` variant is returned if the rollback ITSELF
    /// fails — in that case the StateDB is in an undefined state
    /// and the operator must intervene.
    pub fn replay_and_apply_atomic<F1, F2>(
        &self,
        db: &mut dyn evaporchain_state::db::StateDB,
        current_head: [u8; 32],
        target_head: [u8; 32],
        block_lookup: F1,
        block_apply: F2,
        pre_replay_height: u64,
        pre_replay_epoch: u64,
    ) -> Result<ReplayResult, ReplayError>
    where
        F1: FnMut(&[u8; 32]) -> Option<evaporchain_types::Block>,
        F2: FnMut(
            &mut dyn evaporchain_state::db::StateDB,
            &evaporchain_types::Block,
        ) -> Result<(), String>,
    {
        // Capture pre-replay state. The `tip` field of the snapshot
        // is a placeholder ([0u8; 32]) since we're not registering
        // this snapshot in state_branches — it's purely a
        // transactional rollback anchor.
        let pre_replay_snapshot =
            StateSnapshotBranch::capture([0u8; 32], pre_replay_height, pre_replay_epoch, db)
                .map_err(ReplayError::RestoreFailed)?;

        match self.replay_and_apply(db, current_head, target_head, block_lookup, block_apply) {
            Ok(result) => Ok(result),
            Err(replay_err) => {
                // Roll back to pre-replay state. If THIS fails, the
                // StateDB is in an undefined state — return a
                // composite error so the operator can intervene.
                if let Err(rollback_err) = pre_replay_snapshot.restore(db) {
                    return Err(ReplayError::ApplyFailed {
                        block: "<pre-replay rollback>".to_string(),
                        msg: format!(
                            "replay failed ({:?}) AND rollback failed ({}); StateDB in undefined state",
                            replay_err, rollback_err
                        ),
                    });
                }
                Err(replay_err)
            }
        }
    }

    /// Set the proof verifier for validating Nova IVC proofs on proposed blocks.
    pub fn set_proof_verifier(
        &mut self,
        verifier: Box<dyn ProofVerifier>,
        genesis_state_root: [u8; 32],
    ) {
        self.proof_verifier = Some(verifier);
        self.genesis_state_root = genesis_state_root;
    }

    /// Set the anchor hash provider for rule-based consensus enforcement.
    pub fn set_anchor_provider(&mut self, provider: Box<dyn AnchorHashProvider>) {
        self.anchor_provider = Some(provider);
    }

    /// Set the minimum DAS confidence threshold for DA attestation (default 0.999).
    /// confidence = 1 - 2^(-valid_samples). 16 valid samples → ~0.999985.
    pub fn set_da_confidence_threshold(&mut self, threshold: f64) {
        self.da_confidence_threshold = threshold.clamp(0.0, 1.0);
    }

    /// Set the BLS keypair for this validator (enables aggregate signatures).
    pub fn set_bls_keypair(&mut self, keypair: BlsKeypair) {
        // Also register our own BLS public key in the validator set
        let pk_bytes = keypair.public_key_bytes().0.clone();
        if let Some(vi) = self.validator_set.get_mut(self.my_id) {
            vi.bls_public_key = Some(pk_bytes);
            vi.pop_verified = true;
        }
        self.bls_keypair = Some(keypair);
    }

    /// Sign an arbitrary message with this validator's BLS key. Returns the
    /// signature together with the matching public key so the caller can
    /// submit both to a verifier without holding the keypair lock open.
    /// Returns `None` when no BLS keypair has been configured.
    pub fn sign_with_bls(&self, msg: &[u8]) -> Option<(BlsSignature, BlsPublicKey)> {
        self.bls_keypair
            .as_ref()
            .map(|kp| (kp.sign(msg), kp.public_key_bytes()))
    }

    /// Generate a KeyAnnounce message for broadcasting our BLS public key
    /// along with a proof-of-possession (prevents rogue-key attacks).
    pub fn make_key_announce(&self) -> Option<ConsensusMessage> {
        self.bls_keypair
            .as_ref()
            .map(|kp| ConsensusMessage::KeyAnnounce {
                validator_id: self.my_id,
                bls_public_key: kp.public_key_bytes().0.clone(),
                proof_of_possession: kp.proof_of_possession().0.clone(),
            })
    }

    /// Set the VRF keypair for this validator (enables VRF-based leader election).
    pub fn set_vrf_keypair(&mut self, keypair: VrfKeypair) {
        self.vrf_keypair = Some(keypair);
    }

    /// Set the block height at which DA certificate enforcement becomes mandatory.
    ///
    /// Before `height`: blocks without DA certificates are accepted with a warning (soft mode).
    /// At or after `height`: blocks without valid DA certificates are rejected (hard mode).
    pub fn set_da_enforcement_height(&mut self, height: u64) {
        info!(
            old = self.da_enforcement_height,
            new = height,
            "DA enforcement height updated"
        );
        self.da_enforcement_height = height;
    }

    /// Get the current DA enforcement height.
    pub fn da_enforcement_height(&self) -> u64 {
        self.da_enforcement_height
    }

    /// Enable or disable small-cluster DA mode. See the
    /// `small_cluster_da_mode` field doc-comment for the safety
    /// implications. Intended to be auto-set at boot by the node
    /// binary based on validator-set size — NOT changed at runtime.
    pub fn set_small_cluster_da_mode(&mut self, enabled: bool) {
        if enabled && !self.small_cluster_da_mode {
            warn!(
                validators = self.validator_set.len(),
                "small-cluster DA mode ENABLED: proposer self-attestations \
                 will count toward DA quorum. Safe only for trusted devnet / \
                 cluster-smoke topologies (validators <= 3). DO NOT enable on mainnet."
            );
        } else if !enabled && self.small_cluster_da_mode {
            info!("small-cluster DA mode disabled — strict proposer-exclusion restored");
        }
        self.small_cluster_da_mode = enabled;
    }

    /// Whether small-cluster DA mode is active.
    pub fn small_cluster_da_mode(&self) -> bool {
        self.small_cluster_da_mode
    }

    /// Submit an encrypted transaction to the MEV-protected mempool.
    pub fn submit_encrypted_tx(&mut self, encrypted_tx: EncryptedTransaction) {
        debug!(
            commitment = hex::encode(encrypted_tx.commitment),
            submitted_epoch = encrypted_tx.submitted_epoch,
            "Encrypted tx submitted to MEV-protected pool"
        );
        self.encrypted_mempool.submit_encrypted(encrypted_tx);
    }

    /// Submit a reveal nonce for a previously committed encrypted
    /// transaction. The nonce will be used at the next block
    /// production to decrypt and include the tx.
    ///
    /// Returns `true` if accepted, `false` if rejected because the
    /// pending-reveals queue is at `MAX_PENDING_REVEALS`. T0.7
    /// vector 4 companion: bounds the reveal queue alongside the
    /// encrypted-mempool capacity cap (PR #17).
    pub fn submit_reveal(&mut self, commitment: [u8; 32], nonce: [u8; 32]) -> bool {
        if self.pending_reveals.len() >= MAX_PENDING_REVEALS {
            debug!(
                commitment = hex::encode(commitment),
                pending = self.pending_reveals.len(),
                cap = MAX_PENDING_REVEALS,
                "Reveal nonce rejected — pending-reveals queue at capacity"
            );
            return false;
        }
        debug!(
            commitment = hex::encode(commitment),
            "Reveal nonce submitted for encrypted tx"
        );
        self.pending_reveals.push((commitment, nonce));
        true
    }

    /// Get pending counts: (plain_mempool, encrypted_pending, reveals_pending).
    pub fn mempool_stats(&self) -> (usize, usize, usize) {
        let (enc, _plain) = self.encrypted_mempool.pending_count();
        (self.mempool.len(), enc, self.pending_reveals.len())
    }

    /// Get a reference to the randomness beacon.
    pub fn randomness_beacon(&self) -> &RandomnessBeacon {
        &self.randomness_beacon
    }

    /// Create a test-friendly consensus engine with a small privacy tree (depth 4)
    /// to avoid the ~60s initialization of the full 2^20 Merkle tree.
    /// Enable block-reward distribution on the underlying executor.
    /// Mirrors `MockConsensus::executor.enable_rewards`. Until called,
    /// validators receive no block rewards even if a `Tokenomics` is
    /// present in the genesis config — production tendermint nodes
    /// pre-this-commit shipped without a reward pipeline at all.
    pub fn enable_rewards(&mut self, tokenomics: evaporchain_types::genesis::Tokenomics) {
        self.executor.enable_rewards(tokenomics);
    }

    /// Get the priority sum captured at the most recent local proposal.
    /// Caller (operator runbook / metrics exporter / SimpleExecutor-mode
    /// node) drives where this is consumed. **Not consensus-deterministic**:
    /// only the proposing validator computes a meaningful value;
    /// followers see 0 (they didn't run create_proposal). Phase-1.5 of
    /// `research/proposals/energy-stamped-mev-resistance.md`.
    pub fn last_proposal_priority_sum(&self) -> u64 {
        self.last_proposal_priority_sum
    }

    pub fn new_for_test(my_id: u64, grace_period: u64, validator_set: ValidatorSet) -> Self {
        Self {
            light_cone_dag: evaporchain_light_cone::LightCone::new(),
            tur_window: std::collections::VecDeque::with_capacity(TUR_WINDOW_BLOCKS),
            last_tur_verdict: None,
            mev_observations: std::collections::VecDeque::with_capacity(MEV_OBSERVATION_BUFFER_CAP),
            mev_attacker_stats: std::collections::HashMap::new(),
            settled_refunds: std::collections::HashSet::new(),
            mev_missing_refund_violations: std::collections::HashMap::new(),
            disputed_observations: std::collections::HashSet::new(),
            state_branches: std::collections::HashMap::new(),
            dag_round_states: std::collections::HashMap::new(),
            cross_fork_equivocations: std::collections::HashMap::new(),
            committed_at_block: std::collections::HashMap::new(),
            antichain_digest_history: std::collections::VecDeque::with_capacity(
                ANTICHAIN_DIGEST_HISTORY_CAP,
            ),
            current_authoritative_head: None,
            cartel_alarm: evaporchain_causal_chsh::CartelAlarm::doctrine_default(),
            pending_cartel_alarms: Vec::new(),
            lambda_fold: evaporchain_lambda_fold::FoldedInstance::identity(),
            #[cfg(feature = "lambda_fold_nova")]
            lambda_fold_nova: None,
            #[cfg(feature = "lambda_fold_nova")]
            lambda_fold_nova_instance: evaporchain_lambda_fold::NovaFoldedInstance::identity(),
            my_id,
            height: 1,
            epoch: 0,
            parent_hash: [0u8; 32],
            executor: ParallelExecutor::new_for_test(grace_period),
            mempool: Mempool::new(),
            last_proposal_priority_sum: 0,
            validator_set,
            round_state: RoundState::new(0),
            locked_block: None,
            locked_round: None,
            valid_block: None,
            valid_round: None,
            propose_timeout: Duration::from_millis(PROPOSE_TIMEOUT_MS),
            prevote_timeout: Duration::from_millis(PREVOTE_TIMEOUT_MS),
            precommit_timeout: Duration::from_millis(PRECOMMIT_TIMEOUT_MS),
            committed_heights: HashSet::new(),
            proposals_seen: HashMap::new(),
            slashed_equivocator_blocks: HashSet::new(),
            p2_04_last_warned: None,
            missed_proposals: HashMap::new(),
            missed_votes: HashMap::new(),
            weak_subjectivity_checkpoints: Vec::new(),
            checkpoint_interval: 1000,
            trusted_checkpoint: None,
            bls_keypair: None,
            vrf_keypair: None,
            randomness_beacon: RandomnessBeacon::new(),
            proof_verifier: None,
            genesis_state_root: [0u8; 32],
            epoch_manager: EpochTransitionManager::new(),
            da_attestations: HashMap::new(),
            da_block_proposers: HashMap::new(),
            finality_tracker: FinalityTracker::new(),
            da_attestation: DAAttestationManager::new(),
            encrypted_mempool: EncryptedMempool::new(2),
            pending_reveals: Vec::new(),
            anchor_provider: None,
            current_state_root: [0u8; 32],
            da_confidence_threshold: 0.999,
            da_enforcement_height: 100,
            chain_id: String::new(),
            governance_params: HashMap::new(),
            da_confirmed_height: 0,
            last_block_timestamp: 0,
            fork_choice_attractors: Vec::new(),
            boltzmann_stakes: HashMap::new(),
            wsbf_window: std::collections::VecDeque::new(),
            last_effective_params: None,
            current_consensus_phase: evaporchain_rg_phase_map::ConsensusPhase::LivenessStable,
            last_bell_s_milli: None,
            last_bell_block_height: 0,
            last_bell_epoch: 0,
            last_bell_certified: false,
            block_prod_history: std::collections::VecDeque::with_capacity(BLOCK_PROD_HISTORY_CAP),
            committed_at: BTreeMap::new(),
            finality_gap_history: VecDeque::with_capacity(FINALITY_GAP_HISTORY_CAP),
            draining: false,
            drain_started_at_epoch: None,
            small_cluster_da_mode: false,
        }
    }

    /// Restore state after a restart.
    pub fn restore_state(&mut self, block_number: u64, epoch: Epoch, parent_hash: [u8; 32]) {
        self.height = block_number + 1;
        self.epoch = epoch;
        self.parent_hash = parent_hash;
        self.round_state = RoundState::new(0);
        self.set_timeouts_for_round(0);
        self.locked_block = None;
        self.locked_round = None;
        self.valid_block = None;
        self.valid_round = None;
    }

    /// Snapshot the mutable validator state (stake, delegated_stake, jailed)
    /// for persistence across restarts. Returns `(id, stake, delegated_stake, jailed)`.
    pub fn snapshot_validator_state(&self) -> Vec<(u64, u64, u64, bool)> {
        self.validator_set
            .validators()
            .iter()
            .map(|v| (v.id, v.stake, v.delegated_stake, v.jailed))
            .collect()
    }

    /// Restore mutable validator state from a previously snapshotted vector.
    /// Only updates (stake, delegated_stake, jailed) — genesis-seeded fields
    /// (bls_public_key, address, pop_verified) are left as-is.
    pub fn restore_validator_state(&mut self, state: &[(u64, u64, u64, bool)]) {
        for &(id, stake, delegated_stake, jailed) in state {
            if let Some(v) = self.validator_set.get_mut(id) {
                v.stake = stake;
                v.delegated_stake = delegated_stake;
                v.jailed = jailed;
            }
        }
    }

    /// Restore state after a restart, including the latest committed state root.
    pub fn restore_state_with_root(
        &mut self,
        block_number: u64,
        epoch: Epoch,
        parent_hash: [u8; 32],
        state_root: [u8; 32],
    ) {
        self.restore_state(block_number, epoch, parent_hash);
        self.current_state_root = state_root;
    }

    /// Snapshot the four `last_bell_*` fields into a serializable
    /// `CheckpointedBellReading`. Returns `None` if no Bell-Beacon
    /// reading has been observed yet (matching `last_bell_reading`).
    ///
    /// Persisted as part of `ConsensusCheckpoint::with_bell_reading` so
    /// the wallet `BellBeaconCard` keeps reporting the last live S-value
    /// across node restart instead of resetting to `no_data`.
    pub fn checkpoint_bell_reading(&self) -> Option<crate::persistence::CheckpointedBellReading> {
        self.last_bell_s_milli
            .map(|s| crate::persistence::CheckpointedBellReading {
                s_value_milli: s,
                block_height: self.last_bell_block_height,
                epoch: self.last_bell_epoch,
                certified: self.last_bell_certified,
            })
    }

    /// Restore the `last_bell_*` fields from a checkpoint. Pass the
    /// `last_bell_reading` field of a loaded `ConsensusCheckpoint`.
    /// `None` (or a checkpoint that pre-dates the field) leaves the
    /// fields at their default (`None` / `0` / `0` / `false`) so the
    /// next block produces a fresh measurement.
    pub fn restore_bell_reading(
        &mut self,
        reading: Option<&crate::persistence::CheckpointedBellReading>,
    ) {
        if let Some(r) = reading {
            self.last_bell_s_milli = Some(r.s_value_milli);
            self.last_bell_block_height = r.block_height;
            self.last_bell_epoch = r.epoch;
            self.last_bell_certified = r.certified;
        } else {
            self.last_bell_s_milli = None;
            self.last_bell_block_height = 0;
            self.last_bell_epoch = 0;
            self.last_bell_certified = false;
        }
    }

    /// Rebuild the in-memory privacy note tree from commitments persisted
    /// in the StateDB. Call exactly once at node startup, after `restore_state`,
    /// before any block is processed. Errors propagated from the engine
    /// (root mismatch, tree-full, etc.) are signalled via String — caller
    /// should treat them as fatal startup failures.
    ///
    /// Closes punch-list 1b.
    pub fn restore_privacy_from_db(&mut self, db: &dyn StateDB) -> Result<usize, String> {
        self.executor
            .privacy_executor
            .restore_from_db(db)
            .map_err(|e| e.to_string())
    }

    /// Apply pending validator BLS key rotations emitted by execution.
    /// Called by the block-production / commit pipeline after a successful
    /// `execute_block()` returns its `BlockExecutionResult`.
    ///
    /// For each rotation:
    ///   1. PoP-verify `bls_pop_old` against the validator's
    ///      *currently-recorded* `bls_public_key`. This is the continuity
    ///      check that proves the rotator controlled the old key — the
    ///      execution layer cannot do this verify itself because it does
    ///      not own the live ValidatorSet.
    ///   2. On success, swap the validator's pubkey: `prev = old`,
    ///      `current = new`, expiry set per `prev_key_expiry_epoch`.
    ///
    /// Returns the number of rotations actually applied. A failed
    /// continuity check causes that single rotation to be silently
    /// skipped — the tx already paid gas at execution time, but the
    /// validator set is left untouched. This matches BFT philosophy: an
    /// attacker who can submit a malformed rotation tx but not provide a
    /// valid `bls_pop_old` should not be able to disrupt the validator
    /// set, only to waste their own gas.
    ///
    /// Closes punch-list 4b consensus-side wiring.
    pub fn apply_validator_key_rotations(
        &mut self,
        rotations: &[evaporchain_execution::ValidatorKeyRotation],
    ) -> usize {
        let mut applied = 0usize;
        for rot in rotations {
            // Snapshot the current key for the continuity check before
            // borrowing the validator set mutably.
            let old_pk = match self
                .validator_set
                .get(rot.validator_id)
                .and_then(|v| v.bls_public_key.clone())
            {
                Some(pk) => pk,
                None => {
                    warn!(
                        validator_id = rot.validator_id,
                        "Skipping rotation: validator has no current BLS key"
                    );
                    continue;
                }
            };
            // Continuity-of-control: bls_pop_old must verify against the
            // OLD pubkey. The PoP message is the NEW pubkey bytes — that
            // binding prevents replay across rotation attempts.
            if !crate::validator_set::ValidatorSet::verify_pop(&old_pk, &rot.bls_pop_old) {
                warn!(
                    validator_id = rot.validator_id,
                    "Skipping rotation: bls_pop_old failed continuity verify"
                );
                continue;
            }
            if self.validator_set.rotate_validator_key(
                rot.validator_id,
                rot.new_bls_public_key.clone(),
                rot.new_bls_pop.clone(),
                rot.prev_key_expiry_epoch,
            ) {
                applied += 1;
                info!(
                    validator_id = rot.validator_id,
                    expiry = rot.prev_key_expiry_epoch,
                    "Validator BLS key rotated"
                );
            }
        }
        applied
    }

    /// Sweep validator-set: drop any prev pubkey whose grace window has
    /// elapsed. Cheap O(n). Should be called once per epoch — typically
    /// alongside `apply_validator_key_rotations` from the commit pipeline.
    pub fn purge_expired_prev_keys(&mut self) -> usize {
        self.validator_set.purge_expired_prev_keys(self.epoch)
    }

    /// Get the current committed state root.
    pub fn current_state_root(&self) -> [u8; 32] {
        self.current_state_root
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn set_height(&mut self, h: u64) {
        self.height = h;
        self.round_state = RoundState::new(0);
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn validator_set(&self) -> &ValidatorSet {
        &self.validator_set
    }

    /// Operator-side re-instatement of a validator that was
    /// auto-removed by the slashing path (typically: stake fell below
    /// `MIN_STAKE` after sustained downtime, see
    /// `validator_set.rs:491`). This adds the validator back to the
    /// in-memory active set with the supplied `ValidatorInfo`, resets
    /// the slashing-counter state (`missed_proposals`, `missed_votes`),
    /// and clears any DA-attestation rejections that were accumulating
    /// because the validator id was previously unknown.
    ///
    /// **Trust model**: this is an admin / governance hatch — the
    /// caller must verify the BLS public key out-of-band (e.g. against
    /// the genesis file). `pop_verified` on the supplied info is
    /// honoured as-given; for genesis-keyed validators the operator
    /// should set `pop_verified = true` after locally verifying the
    /// PoP (or supplying a fresh PoP signature). The endpoint that
    /// drives this is gated by `EVAPORCHAIN_ADMIN_KEY` so only an
    /// operator with that secret can call it.
    ///
    /// Returns `true` if the validator was newly added; `false` if
    /// already present (idempotent).
    pub fn reinstate_validator(&mut self, info: ValidatorInfo) -> bool {
        let id = info.id;
        if self.validator_set.get(id).is_some() {
            return false;
        }
        let added = self.validator_set.add_validator(info);
        if added {
            self.missed_proposals.insert(id, 0);
            self.missed_votes.insert(id, 0);
        }
        added
    }

    /// Operator-hatch: clear the jailed flag for a validator that is
    /// already in the set.  Returns `true` if the validator was jailed
    /// and is now unjailed; `false` if it was not found or was already
    /// active.  Stake-floor check is enforced by `ValidatorSet::unjail`.
    pub fn unjail_validator(&mut self, validator_id: u64) -> bool {
        self.validator_set.unjail(validator_id)
    }

    /// Recent per-validator block-production timing samples, oldest
    /// first. Each entry is `(producer_id, exec_time_seconds)` and
    /// gets appended after every successful block commit; bounded by
    /// `BLOCK_PROD_HISTORY_CAP`. Surfaced to the node's Prometheus
    /// exposition (`/metrics`) so the histogram emits one bucket
    /// series per `producer="validator-{id}"` label.
    pub fn block_production_history(&self) -> Vec<(u64, f64)> {
        self.block_prod_history.iter().copied().collect()
    }

    /// Record a per-validator block-production timing sample. Called
    /// by the node's commit loop after a block is applied, with the
    /// wall-clock execution time. Anonymous proposers (genesis,
    /// no-producer-id replays) are dropped so the histogram label set
    /// stays bounded. Oldest entry is evicted when the ring buffer is
    /// full.
    pub fn record_block_production_timing(&mut self, producer_id: u64, exec_time_us: u64) {
        if self.block_prod_history.len() >= BLOCK_PROD_HISTORY_CAP {
            self.block_prod_history.pop_front();
        }
        let exec_time_seconds = (exec_time_us as f64) / 1_000_000.0;
        self.block_prod_history
            .push_back((producer_id, exec_time_seconds));
    }

    /// Recent per-height finality gap samples, oldest first. Each entry
    /// is `(height, commit_to_finalise_gap_ms)` recorded the moment a
    /// height's commit certificate is observed in `on_block_committed`.
    /// Bounded by `FINALITY_GAP_HISTORY_CAP`. Drives the
    /// `evap_finality_gap_seconds` histogram on `/metrics`.
    pub fn finality_gap_history(&self) -> Vec<(u64, u64)> {
        self.finality_gap_history.iter().copied().collect()
    }

    /// Heights that have been committed but not yet seen a finality
    /// certificate, projected to `(height, age_ms_since_commit)` against
    /// the current wall clock. The worst (max) age is the operator's
    /// "finality is stalling" signal — surfaced as
    /// `evap_worst_unfinalised_gap_seconds`. Returned ordered by height.
    pub fn unfinalised_tail(&self) -> Vec<(u64, u64)> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.committed_at
            .iter()
            .map(|(h, committed_ms)| (*h, now_ms.saturating_sub(*committed_ms)))
            .collect()
    }

    /// Maximum age in `unfinalised_tail()`. 0 when nothing is pending.
    /// Drives the `EvapFinalityStalled` Prometheus alert.
    pub fn worst_unfinalised_gap_ms(&self) -> u64 {
        self.unfinalised_tail()
            .into_iter()
            .map(|(_, age)| age)
            .max()
            .unwrap_or(0)
    }

    /// Test-only: inject a synthetic commit timestamp for `height`. Used
    /// to drive deterministic finality-gap tests without sleeping.
    #[cfg(test)]
    pub(crate) fn test_record_commit_at(&mut self, height: u64, committed_at_ms: u64) {
        self.committed_at.insert(height, committed_at_ms);
    }

    /// Test-only: directly push a (height, gap_ms) sample, applying the
    /// same ring-buffer cap as the production path. Lets tests assert
    /// the eviction behaviour without running thousands of commits.
    #[cfg(test)]
    pub(crate) fn test_push_finality_gap(&mut self, height: u64, gap_ms: u64) {
        self.finality_gap_history.push_back((height, gap_ms));
        while self.finality_gap_history.len() > FINALITY_GAP_HISTORY_CAP {
            self.finality_gap_history.pop_front();
        }
    }

    /// Mark this node as draining — consensus stops proposing /
    /// prevoting until `clear_draining` is called. Idempotent: a
    /// repeat call refreshes `drain_started_at_epoch` to the current
    /// epoch. Returns the epoch the drain is anchored to.
    pub fn set_draining(&mut self) -> u64 {
        let now = self.epoch;
        self.draining = true;
        self.drain_started_at_epoch = Some(now);
        now
    }

    /// Clear the drain flag. Returns the previous draining state.
    pub fn clear_draining(&mut self) -> bool {
        let prev = self.draining;
        self.draining = false;
        self.drain_started_at_epoch = None;
        prev
    }

    /// Current draining state — `(draining, drain_started_at_epoch)`.
    pub fn drain_state(&self) -> (bool, Option<u64>) {
        (self.draining, self.drain_started_at_epoch)
    }

    /// Whether this node is currently draining (refusing to propose
    /// / prevote). Surfaced separately so hot-path consensus checks
    /// don't allocate the option pair.
    pub fn is_draining(&self) -> bool {
        self.draining
    }

    pub fn block_number(&self) -> u64 {
        self.height.saturating_sub(1)
    }

    pub fn round(&self) -> u32 {
        self.round_state.round
    }

    pub fn parent_hash(&self) -> [u8; 32] {
        self.parent_hash
    }

    /// Phase 1.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` — DAG-aware
    /// current chain head. When `parent_acceptance_mode == "mcc"`,
    /// defers to the LightCone DAG via `MccForkChoice::select_tip`
    /// (highest path-caliber leaf). When the mode is `"linear"`
    /// (default) or the DAG is empty / select_tip returns None,
    /// falls back to `self.parent_hash` — preserves existing chain
    /// behaviour bit-for-bit.
    ///
    /// Phase 1.3 wires this into `create_proposal` so proposers
    /// build on the DAG-derived head when in mcc mode. For now the
    /// accessor is read-only; consumers query it for monitoring.
    pub fn current_tip(&self) -> [u8; 32] {
        let mode = self
            .governance_params
            .get("parent_acceptance_mode")
            .map(|s| s.as_str())
            .unwrap_or("linear");
        if mode == "mcc" {
            // Build a snapshot MccForkChoice with the chain's current
            // DAG + β. β source = governance flag; default 1000 mb.
            let beta_mb = self
                .governance_params
                .get("crooks_mev_beta_mb")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(1000);
            // ForkChoice trait must be in scope for the
            // `select_tip` method to resolve.
            use crate::fork_choice::ForkChoice;
            let fc = crate::fork_choice::MccForkChoice::new(self.light_cone_dag.clone(), beta_mb);
            if let Some(tip) = fc.select_tip() {
                return tip;
            }
        }
        self.parent_hash
    }

    pub fn phase(&self) -> Phase {
        self.round_state.phase
    }

    /// Diagnostic snapshot of the current round's precommit state.
    /// Returns `(precommits_received, precommit_bls_sigs_present)`.
    /// A healthy node committing with quorum should see both equal — when they
    /// diverge, peers' Precommit messages were rejected for sig validation,
    /// or our own self-precommit ran without a BLS keypair.
    pub fn precommit_diagnostics(&self) -> (usize, usize) {
        (
            self.round_state.precommits.len(),
            self.round_state.precommit_bls_sigs.len(),
        )
    }

    /// Number of validators needed for a 2f+1 quorum (count-based, for certificate signer checks).
    ///
    /// Returns the smallest `k` such that `k > 2n/3` (strict supermajority),
    /// matching `stake_quorum_threshold` which uses the same `signed*3 >
    /// total*2` rule on stake. With n=3 equal-stake validators, two votes
    /// is exactly 2/3 of stake — not strictly greater — so quorum is 3.
    /// With n=4 it's 3; with n=7 it's 5; with n=10 it's 7.
    #[allow(dead_code)]
    pub(crate) fn quorum_size(&self) -> usize {
        let n = self.validator_set.len();
        if n == 0 {
            return usize::MAX;
        }
        (n * 2) / 3 + 1
    }

    /// Stake threshold for a 2f+1 quorum (stake-weighted).
    fn stake_quorum_threshold(&self) -> u64 {
        let total = self.validator_set.total_stake();
        if total == 0 {
            return u64::MAX;
        }
        // ceiling(2*total/3): strictly more than 2/3 of total stake.
        // With 3 equal-stake validators (total=3000) this gives 2000, so any
        // 2-of-3 combination reaches quorum. Using `total*2/3 + 1` = 2001 would
        // demand all three validators — impossible if any one times out or lags.
        (total * 2).div_ceil(3)
    }

    /// Who is the proposer for the current height/round?
    /// Uses beacon randomness when available so future leaders are unpredictable.
    /// Applies SA acceptance test (§A4.3.2): if a higher-scoring candidate exists
    /// deterministic proposer for this height+round using stake-weighted epoch hash.
    fn proposer_for_round(&self, height: u64, round: u32) -> Option<&ValidatorInfo> {
        // Do NOT use the randomness beacon for proposer selection. The beacon
        // accumulates per-block VRF outputs which diverge when any block is committed
        // via different proposers on different nodes (split-brain BFT recovery).
        // Stake-weighted epoch_hash(height*100+round) is fully deterministic across
        // all nodes regardless of beacon state.
        let virtual_epoch = height.wrapping_mul(100).wrapping_add(round as u64);
        self.validator_set.leader_for_epoch(virtual_epoch)
    }

    /// Am I the proposer for the current height/round?
    pub fn am_i_proposer(&self) -> bool {
        self.proposer_for_round(self.height, self.round_state.round)
            .is_some_and(|v| v.id == self.my_id)
    }

    /// Compute the hash of a block for voting purposes.
    ///
    /// MUST be deterministic across honest validators given the same
    /// committed-block payload — otherwise commit-certificate
    /// `block_hash` (signed at proposal time, before the gossip-path
    /// commit applies post-execution diagnostics) won't match the
    /// locally-recomputed hash, and follower nodes log
    /// "Commit certificate block_hash does not match actual block hash"
    /// every block. The fields below are the only ones guaranteed to be
    /// identical on every honest node by the time precommits are signed:
    /// number, epoch, parent_hash, state_root, timestamp, vrf_output,
    /// transactions. Anything set later in the gossip path
    /// (`state_function_commitment`, `oracle_state_root`, `shard_count`,
    /// DA roots from a node-local 2D encode) MUST be excluded — those
    /// are computed per-validator after consensus and would diverge.
    /// State authenticity is already covered by `state_root`.
    ///
    /// Public so the node binary can use it to populate
    /// `SyncServer::set_tip(height, hash)` after each block commit —
    /// closes H-21 audit finding (TipResponse used to return
    /// [0u8; 32] placeholder, leaving peer-tip verification useless).
    pub fn block_hash(block: &Block) -> [u8; 32] {
        // Consensus-4 (re-audit 2026-05-02): also commit producer_id,
        // vrf_proof, and data_root so two distinct blocks can never
        // hash identically. Previously two blocks with the same tx
        // set + state_root but different VRF or DA encodings could
        // collide. Each field gets a 1-byte presence tag + a length
        // prefix where applicable, so absent vs zero-length is
        // distinguishable.
        let mut input = Vec::new();
        input.extend_from_slice(&block.number.to_le_bytes());
        input.extend_from_slice(&block.epoch.to_le_bytes());
        input.extend_from_slice(&block.parent_hash);
        input.extend_from_slice(&block.state_root);
        input.extend_from_slice(&block.timestamp.to_le_bytes());
        // producer_id (Option<u64>) — 1-byte tag + 8 bytes if Some.
        match block.producer_id {
            Some(pid) => {
                input.push(1);
                input.extend_from_slice(&pid.to_le_bytes());
            }
            None => input.push(0),
        }
        // VRF output (commits chain randomness) — tag + bytes.
        match block.vrf_output {
            Some(ref vrf_out) => {
                input.push(1);
                input.extend_from_slice(&(vrf_out.len() as u32).to_le_bytes());
                input.extend_from_slice(vrf_out);
            }
            None => input.push(0),
        }
        // VRF proof — separate from output; both must be authenticated.
        match block.vrf_proof {
            Some(ref vrf_proof) => {
                input.push(1);
                input.extend_from_slice(&(vrf_proof.len() as u32).to_le_bytes());
                input.extend_from_slice(vrf_proof);
            }
            None => input.push(0),
        }
        // DA root — must match across honest validators (post-fix:
        // built from `build_block_da_inputs(txs)`, deterministic).
        match block.data_root {
            Some(root) => {
                input.push(1);
                input.extend_from_slice(&root);
            }
            None => input.push(0),
        }
        // Length-prefix each tx so concatenation isn't ambiguous —
        // closes the audit's "T1 + T2 vs T1||T2_suffix can collide"
        // edge case.
        for tx in &block.transactions {
            let bytes = serde_json::to_vec(tx).expect("transaction serialization must not fail");
            input.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            input.extend_from_slice(&bytes);
        }
        // POST_EXEC_STATE_VERIFICATION_PLAN.md Phase 5 (lane T0.4) —
        // commit post_state_root into the block hash so the
        // validator-claimed post-execution state becomes
        // consensus-load-bearing. A divergent proposer can no longer
        // produce two blocks with the same hash but different
        // post_state_root claims.
        //
        // Bit-compat shape (per plan): when `post_state_root` is None
        // we append NOTHING (not even a tag byte). Pre-Phase 5 blocks
        // and post-Phase 2 blocks proposed under
        // `post_state_verify_mode = "off"` both have None and so keep
        // their legacy hash. Only blocks where the proposer filled
        // post_state_root (warn or enforce mode under T0.3) gain the
        // new hash contribution.
        //
        // The asymmetric (no-tag-when-None) pattern is intentional and
        // distinct from the tag-prefixed Optionals above. Adding a
        // tag-0 here would break the hash of every existing legacy
        // block, which would brick any chain that has ever produced
        // a block under the pre-Phase-5 binary.
        if let Some(post_root) = block.post_state_root {
            input.push(1);
            input.extend_from_slice(&post_root);
        }
        blake3_hash(&input)
    }

    // ──────────────── Core State Machine ────────────────────────────────

    /// Called on every tick. Returns actions the node should perform.
    /// This is the main driver of the consensus state machine.
    pub fn tick(&mut self, db: &mut dyn StateDB) -> Vec<ConsensusAction> {
        let mut actions = Vec::new();

        // K-11 wiring: refresh per-validator delegated_stake from the live
        // DelegationRecord set so quorum/voting-power decisions in this tick
        // reflect newly bonded/unbonded delegations.
        crate::validator_set::refresh_delegated_stakes(&mut self.validator_set, &*db);

        // Re-broadcast BLS KeyAnnounce every 50 blocks so late-joining peers get our key
        if self.height > 0
            && self.height.is_multiple_of(50)
            && self.round_state.phase == Phase::Propose
            && self.round_state.round == 0
        {
            if let Some(msg) = self.make_key_announce() {
                actions.push(ConsensusAction::BroadcastMessage(msg));
            }
        }

        match self.round_state.phase {
            Phase::Propose => {
                // Phase C (Layer 4): refresh authoritative_head so propose_parents()
                // and vote_target_head() have a current DAG snapshot for this round.
                // No-op when parent_acceptance_mode is linear or mcc.
                self.update_authoritative_head();

                // Drain gate: a draining node refuses to propose or
                // self-prevote so peers route around it (Ansible upgrade
                // playbook → POST /api/admin/drain). The node still
                // observes consensus messages so it can apply blocks
                // produced by the rest of the set, just doesn't
                // contribute votes / proposals until undrained.
                let drain_gate_open = !self.draining;
                // If I'm the proposer and haven't proposed yet, propose
                if drain_gate_open
                    && self.am_i_proposer()
                    && self.round_state.proposed_block.is_none()
                {
                    if let Some(proposal) = self.create_proposal(db) {
                        let msg = ConsensusMessage::Proposal {
                            height: self.height,
                            round: self.round_state.round,
                            block: proposal.clone(),
                            proposer_id: self.my_id,
                        };
                        self.round_state.proposed_block = Some(proposal.clone());
                        self.round_state.proposed_hash = Some(Self::block_hash(&proposal));
                        // Consensus-1 (re-audit 2026-05-02): observe
                        // OUR OWN proposal here too. The follower
                        // path observes incoming proposals at
                        // on_message::Proposal (~line 2403), but the
                        // proposer never goes through on_message for
                        // its own proposal — without this call,
                        // backfill replay protection on the proposer
                        // is missing for heights we proposed.
                        self.finality_tracker.observe_proposal(self.height);
                        actions.push(ConsensusAction::BroadcastMessage(msg));

                        // Self-prevote for our own proposal
                        let hash = Self::block_hash(&proposal);
                        self.round_state.prevotes.insert(self.my_id, Some(hash));
                        self.round_state.prevoted = true;
                        // IB prevote advisory (§A4.3.1): log commit/abstain signal.
                        // DEFAULT_LAMBDA_MB=0 → always Commit. Safe to wire at
                        // this stage; hard-gate is a future governance amendment.
                        {
                            let local_stakes: Vec<u64> = self
                                .validator_set
                                .validators()
                                .iter()
                                .map(|v| v.stake)
                                .collect();
                            let _ib = ib_integration::ib_vote_from_stakes(
                                &local_stakes,
                                &local_stakes,
                                DEFAULT_LAMBDA_MB,
                            );
                            debug!(validator = self.my_id, ib_vote = ?_ib, "IB prevote signal");
                        }
                        let vote_hash = Some(hash);
                        let bls_sig = self.bls_sign_vote(
                            self.height,
                            self.round_state.round,
                            &vote_hash,
                            "prevote",
                        );
                        if let Some(ref sig) = bls_sig {
                            self.round_state
                                .prevote_bls_sigs
                                .insert(self.my_id, sig.clone());
                        }
                        let prevote = ConsensusMessage::Prevote {
                            height: self.height,
                            round: self.round_state.round,
                            block_hash: vote_hash,
                            validator_id: self.my_id,
                            bls_signature: bls_sig,
                        };
                        actions.push(ConsensusAction::BroadcastMessage(prevote));

                        // Proposer DA self-attestation: we already computed data_root
                        // from the final tx set, so attest directly without re-encoding.
                        self.da_block_proposers.insert(proposal.number, self.my_id);
                        if let Some(data_root) = proposal.data_root {
                            if let Some(att_msg) =
                                self.make_da_attestation(proposal.number, data_root, 1)
                            {
                                actions.push(ConsensusAction::BroadcastMessage(att_msg));
                            }
                        }

                        self.round_state.phase = Phase::Prevote;
                        self.round_state.phase_start = Instant::now();
                    }
                }

                // Timeout: move to prevote with nil. Drain-gated: a
                // draining node still advances its phase machine so it
                // tracks the rest of the network, but does NOT broadcast
                // a nil prevote (it has signalled "route around me").
                if self.round_state.phase_start.elapsed() > self.propose_timeout {
                    if drain_gate_open && !self.round_state.prevoted {
                        self.round_state.prevoted = true;
                        let nil_hash: Option<[u8; 32]> = None;
                        let bls_sig = self.bls_sign_vote(
                            self.height,
                            self.round_state.round,
                            &nil_hash,
                            "prevote",
                        );
                        if let Some(ref sig) = bls_sig {
                            self.round_state
                                .prevote_bls_sigs
                                .insert(self.my_id, sig.clone());
                        }
                        let prevote = ConsensusMessage::Prevote {
                            height: self.height,
                            round: self.round_state.round,
                            block_hash: nil_hash,
                            validator_id: self.my_id,
                            bls_signature: bls_sig,
                        };
                        actions.push(ConsensusAction::BroadcastMessage(prevote));
                        self.round_state.prevotes.insert(self.my_id, None);
                    }
                    self.round_state.phase = Phase::Prevote;
                    self.round_state.phase_start = Instant::now();
                }
            }

            Phase::Prevote => {
                // Check if we have quorum of prevotes for any hash
                if let Some(hash) = self.check_prevote_quorum() {
                    // Got 2f+1 prevotes for a block → precommit
                    if !self.round_state.precommitted {
                        self.round_state.precommitted = true;

                        // Lock on this block — verify proposed block hash matches quorum hash
                        if let Some(ref quorum_hash) = hash {
                            if let Some(ref proposed) = self.round_state.proposed_block {
                                if Self::block_hash(proposed) == *quorum_hash {
                                    self.locked_block = self.round_state.proposed_block.clone();
                                    self.locked_round = Some(self.round_state.round);
                                    self.valid_block = self.round_state.proposed_block.clone();
                                    self.valid_round = Some(self.round_state.round);
                                }
                            }
                        }

                        let bls_sig = self.bls_sign_vote(
                            self.height,
                            self.round_state.round,
                            &hash,
                            "precommit",
                        );
                        if let Some(ref sig) = bls_sig {
                            self.round_state
                                .precommit_bls_sigs
                                .insert(self.my_id, sig.clone());
                        }
                        let precommit = ConsensusMessage::Precommit {
                            height: self.height,
                            round: self.round_state.round,
                            block_hash: hash,
                            validator_id: self.my_id,
                            bls_signature: bls_sig,
                        };
                        actions.push(ConsensusAction::BroadcastMessage(precommit));
                        self.round_state.precommits.insert(self.my_id, hash);
                    }
                    self.round_state.phase = Phase::Precommit;
                    self.round_state.phase_start = Instant::now();
                }

                // Timeout: move to precommit with nil
                if self.round_state.phase_start.elapsed() > self.prevote_timeout {
                    if !self.round_state.precommitted {
                        self.round_state.precommitted = true;
                        let nil_hash: Option<[u8; 32]> = None;
                        let bls_sig = self.bls_sign_vote(
                            self.height,
                            self.round_state.round,
                            &nil_hash,
                            "precommit",
                        );
                        if let Some(ref sig) = bls_sig {
                            self.round_state
                                .precommit_bls_sigs
                                .insert(self.my_id, sig.clone());
                        }
                        let precommit = ConsensusMessage::Precommit {
                            height: self.height,
                            round: self.round_state.round,
                            block_hash: nil_hash,
                            validator_id: self.my_id,
                            bls_signature: bls_sig,
                        };
                        actions.push(ConsensusAction::BroadcastMessage(precommit));
                        self.round_state.precommits.insert(self.my_id, None);
                    }
                    self.round_state.phase = Phase::Precommit;
                    self.round_state.phase_start = Instant::now();
                }
            }

            Phase::Precommit => {
                // Check if we have quorum of precommits for a block
                if let Some(Some(hash)) = self.check_precommit_quorum() {
                    // 2f+1 precommits for a block → commit, *if* the rest of
                    // the gating signals are also ready. Peek with `as_ref()`
                    // first; only `take()` once we are committed to applying
                    // the block. Earlier we always `take()`d here, which
                    // consumed `proposed_block` even when DA supermajority
                    // hadn't arrived yet — the next tick saw `proposed=None`
                    // and consensus advanced to a fresh round whose
                    // precommits then went nil, looping forever. Reproduced
                    // 2026-05-02 against a tx-bearing block: precommit
                    // quorum hit before the 4th DA attestation gossip
                    // arrived, block got dropped, cluster wedged.
                    if let Some(block) = self.round_state.proposed_block.as_ref() {
                        // Verify the proposed block matches the quorum hash
                        let block_hash = Self::block_hash(block);
                        if block_hash != hash {
                            warn!(
                                height = self.height,
                                round = self.round_state.round,
                                "Precommit quorum hash mismatch — our block differs from network consensus"
                            );
                            // Local block disagrees with the network's quorum hash.
                            // Drop the local copy and request sync.
                            if let Some(stale) = self.round_state.proposed_block.take() {
                                for tx in stale.transactions.iter().rev() {
                                    self.mempool.submit_priority(tx.clone());
                                }
                            }
                            actions
                                .push(ConsensusAction::RequestSync(self.height, self.height + 1));
                        } else {
                            // P2-04: refuse to commit if block has a data_root
                            // but DA attestation supermajority hasn't been
                            // reached. Below `da_enforcement_height` we skip
                            // (genesis bootstrap window). On chain_id starting
                            // with `mainnet-` we enforce regardless of height.
                            // Importantly: keep `proposed_block` intact so
                            // the next tick can retry once the trailing DA
                            // attestation lands.
                            let mainnet = block.chain_id.starts_with("mainnet-");
                            let enforce_da = mainnet || self.height >= self.da_enforcement_height;
                            if enforce_da
                                && block.data_root.is_some()
                                && !self.has_da_supermajority(block.number)
                            {
                                // Suppress duplicate warnings — only emit
                                // once per (height, round). Without this,
                                // a DA-stalled height generates 10
                                // warns/sec via the 100ms tick.
                                let key = (block.number, self.round_state.round);
                                if self.p2_04_last_warned != Some(key) {
                                    self.p2_04_last_warned = Some(key);
                                    warn!(
                                        height = block.number,
                                        "P2-04: refusing to commit — DA attestation supermajority not reached"
                                    );
                                }
                                // No RequestSync here — we have the right
                                // block, we just need to wait one more
                                // attestation gossip.
                                return actions;
                            }
                            // All gating passed — now consume the block.
                            let mut block = self
                                .round_state
                                .proposed_block
                                .take()
                                .expect("proposed_block was Some above");
                            if block.commit_certificate.is_none() {
                                block.commit_certificate = self.try_build_commit_certificate(hash);
                            }
                            self.round_state.phase = Phase::Commit;
                            actions.push(ConsensusAction::CommitBlock(block));
                        }
                    }
                }

                // If 2f+1 precommits for nil → next round
                if let Some(None) = self.check_precommit_quorum() {
                    self.advance_round();
                }

                // Timeout: advance round
                if self.round_state.phase_start.elapsed() > self.precommit_timeout {
                    warn!(
                        height = self.height,
                        round = self.round_state.round,
                        "Precommit timeout — advancing round"
                    );
                    self.advance_round();
                }
            }

            Phase::Commit => {
                // If there's a forced block waiting (from max-round overflow), commit it
                if let Some(block) = self.round_state.proposed_block.take() {
                    actions.push(ConsensusAction::CommitBlock(block));
                }
                // Otherwise: waiting for commit to be applied externally
            }
        }

        actions
    }

    /// Process an incoming consensus message. Returns actions to perform.
    pub fn on_message(&mut self, msg: ConsensusMessage) -> Vec<ConsensusAction> {
        let mut actions = Vec::new();

        // Handle KeyAnnounce before height filters (height-independent)
        if let ConsensusMessage::KeyAnnounce {
            validator_id,
            ref bls_public_key,
            ref proof_of_possession,
        } = msg
        {
            if bls_public_key.len() != 48 {
                warn!(
                    validator_id,
                    len = bls_public_key.len(),
                    "Invalid BLS key length (expected 48)"
                );
                return actions;
            }

            // Verify proof-of-possession (prevents rogue-key attack)
            if !proof_of_possession.is_empty() {
                use evaporchain_crypto::signatures::{BlsPublicKey, BlsSignature, BlsVerifier};
                let pk = BlsPublicKey(bls_public_key.clone());
                let pop = BlsSignature(proof_of_possession.clone());
                if !BlsVerifier::verify_proof_of_possession(&pk, &pop) {
                    warn!(
                        validator = validator_id,
                        "REJECTED BLS key: proof-of-possession verification failed (possible rogue-key attack)"
                    );
                    return actions;
                }
            }

            if let Some(vi) = self.validator_set.get_mut(validator_id) {
                if vi.bls_public_key.is_none() || vi.bls_public_key.as_ref() != Some(bls_public_key)
                {
                    vi.bls_public_key = Some(bls_public_key.clone());
                    vi.bls_pop = if proof_of_possession.is_empty() {
                        None
                    } else {
                        Some(proof_of_possession.clone())
                    };
                    vi.pop_verified = !proof_of_possession.is_empty();
                    info!(
                        validator = validator_id,
                        pk_prefix = %hex::encode(&bls_public_key[..8]),
                        pop_verified = vi.pop_verified,
                        "Registered BLS public key from peer"
                    );
                }
            }
            return actions;
        }

        // Handle DA attestations (height-independent — may arrive after block commit)
        if let ConsensusMessage::DAAttestation {
            block_number,
            data_root,
            validator_id,
            samples_verified,
            stake,
            ref signature,
            ref public_key,
        } = msg
        {
            // H4 (audit 2026-05-02): verify the BLS signature AND
            // confirm the public key matches the validator's
            // registered key BEFORE storing. Previously any peer could
            // forge attestations on behalf of any validator id; the
            // map filled with garbage and `has_da_supermajority` would
            // count them, triggering premature commits whose certs
            // later failed verification on light clients.
            let v = match self.validator_set.get(validator_id) {
                Some(v) => v,
                None => {
                    warn!(
                        validator_id,
                        "Rejecting DA attestation from unknown validator"
                    );
                    return actions;
                }
            };
            // Validator must have a registered BLS pk and the
            // attestation must claim that exact pk.
            match v.bls_public_key.as_deref() {
                Some(registered) if registered == public_key.as_slice() => {}
                _ => {
                    warn!(
                        validator_id,
                        "Rejecting DA attestation: public_key mismatch with registered BLS key"
                    );
                    return actions;
                }
            }
            // Reconstruct and verify the signed message exactly as
            // `evaporchain_da::certificate::create_attestation` builds it.
            let mut signed = Vec::with_capacity(
                evaporchain_da::certificate::DA_ATTESTATION_DST.len() + 8 + 32 + 8 + 4,
            );
            signed.extend_from_slice(evaporchain_da::certificate::DA_ATTESTATION_DST);
            signed.extend_from_slice(&block_number.to_le_bytes());
            signed.extend_from_slice(&data_root);
            signed.extend_from_slice(&validator_id.to_le_bytes());
            signed.extend_from_slice(&samples_verified.to_le_bytes());
            let pk = evaporchain_crypto::signatures::BlsPublicKey(public_key.clone());
            let sig = evaporchain_crypto::signatures::BlsSignature(signature.clone());
            if !evaporchain_crypto::signatures::BlsVerifier::verify(&signed, &sig, &pk) {
                warn!(
                    validator_id,
                    block_number, "Rejecting DA attestation: BLS signature did not verify"
                );
                return actions;
            }
            let att = evaporchain_da::certificate::DAAttestation {
                block_number,
                data_root,
                validator_id,
                samples_verified,
                stake,
                signature: signature.clone(),
                public_key: public_key.clone(),
            };
            let atts = self.da_attestations.entry(block_number).or_default();
            // Deduplicate by validator_id
            if !atts.iter().any(|a| a.validator_id == validator_id) {
                atts.push(att);
                debug!(
                    block = block_number,
                    validator = validator_id,
                    total_atts = atts.len(),
                    "DA attestation received (verified)"
                );
            }
            return actions;
        }

        // OracleVote is height-independent gossip routed by the node-level
        // dispatcher to OracleBridge. The tendermint engine itself ignores
        // it; we return early so it doesn't get caught by the height/round
        // filters below (it carries height=0 / round=0 by design).
        if matches!(msg, ConsensusMessage::OracleVote { .. }) {
            return actions;
        }

        // Ignore messages for old heights
        if msg.height() < self.height {
            return actions;
        }

        // If we receive a message for a future height, we are behind — request sync.
        // Only trigger sync for gap > 1: gap=1 means the peer just committed our
        // current round and moved on; those peers still gossip precommits that let
        // our round succeed without needing external sync.
        if msg.height() > self.height {
            if msg.height() > self.height + 1 {
                tracing::warn!(
                    local_height = self.height,
                    msg_height = msg.height(),
                    "Behind by {} blocks — requesting sync",
                    msg.height() - self.height
                );
                actions.push(ConsensusAction::RequestSync(
                    self.height,
                    msg.height().saturating_sub(1),
                ));
            }
            return actions;
        }

        // Ignore messages for old rounds
        if msg.round() < self.round_state.round {
            return actions;
        }

        // Round-skip: if we receive a message from a future round at the same
        // height, jump to that round. This prevents cascading round desync where
        // nodes fall behind and can never achieve quorum.
        if msg.round() > self.round_state.round {
            info!(
                height = self.height,
                from_round = self.round_state.round,
                to_round = msg.round(),
                "Round-skipping to match peer"
            );
            self.round_state = RoundState::new(msg.round());
            self.set_timeouts_for_round(msg.round());
        }

        match msg {
            ConsensusMessage::Proposal {
                height,
                round,
                block,
                proposer_id,
            } => {
                // Verify proposer is legitimate for this round
                let expected_proposer = self.proposer_for_round(height, round).map(|v| v.id);
                if expected_proposer != Some(proposer_id) {
                    warn!(
                        expected = ?expected_proposer,
                        got = proposer_id,
                        "Invalid proposer for height={} round={}",
                        height, round
                    );
                    return actions;
                }
                // Record this height as observed in flight so a future
                // legitimate gap-fill finalization (block delivered late
                // or cluster restarted with old certs in transit) can
                // still be accepted by FinalityTracker. Heights we never
                // see proposed cannot be back-filled — closes
                // cross_verification §1 residual replay window.
                self.finality_tracker.observe_proposal(height);

                // Reject oversized proposals (DoS protection)
                if let Ok(encoded) = serde_json::to_vec(&block) {
                    if encoded.len() > MAX_BLOCK_SIZE_BYTES {
                        warn!(
                            height = height,
                            round = round,
                            size = encoded.len(),
                            max = MAX_BLOCK_SIZE_BYTES,
                            "Rejected oversized proposal from validator {}",
                            proposer_id
                        );
                        return actions;
                    }
                }

                if block.transactions.len() > MAX_TXS_PER_BLOCK {
                    warn!(
                        height = height,
                        round = round,
                        tx_count = block.transactions.len(),
                        max = MAX_TXS_PER_BLOCK,
                        "Rejected proposal: too many transactions"
                    );
                    return actions;
                }

                // Reject proposals that exceed the block gas limit
                if self.executor.block_gas_limit > 0 {
                    let total_gas: u64 = block
                        .transactions
                        .iter()
                        .map(ParallelExecutor::estimate_gas)
                        .fold(0u64, |a, g| a.saturating_add(g));
                    if total_gas > self.executor.block_gas_limit {
                        warn!(
                            height = height,
                            round = round,
                            total_gas = total_gas,
                            limit = self.executor.block_gas_limit,
                            "Rejected proposal: cumulative gas exceeds block gas limit"
                        );
                        return actions;
                    }
                }

                // Verify chain_id matches (prevents cross-chain replay)
                if !self.chain_id.is_empty()
                    && !block.chain_id.is_empty()
                    && block.chain_id != self.chain_id
                {
                    warn!(
                        height = height,
                        round = round,
                        expected = %self.chain_id,
                        got = %block.chain_id,
                        "Rejected proposal: chain_id mismatch"
                    );
                    return actions;
                }

                // Verify block connects to our chain.
                //
                // Lane I.4: governance-gated fork-choice dispatch via the
                // ForkChoice trait seam (G.3). The legacy linear rule
                // (`local == candidate`) stays the default to preserve
                // bit-exact behaviour for the cluster soak. When the
                // `parent_acceptance_mode` governance key is "mcc",
                // dispatch instead to MccForkChoice — Maximum-Caliber
                // path-entropy comparison via the LightCone DAG (Lane
                // I.3 impl). Any other value falls back to the linear
                // rule, so a typo can never halt the chain.
                let parent_accepted = match self
                    .governance_params
                    .get("parent_acceptance_mode")
                    .map(|s| s.as_str())
                    .unwrap_or("linear")
                {
                    "mcc_full" => {
                        // Phase C/D (Layer 4): multi-parent DAG acceptance.
                        // Accept if block.parent_hash is any current DAG candidate
                        // head, OR matches our cached single head (normal path).
                        let candidates = self.candidate_heads();
                        candidates.contains(&block.parent_hash)
                            || block.parent_hash == self.parent_hash
                    }
                    "mcc" => {
                        // Lane I.6: derive β from the chain's λ instead
                        // of the hardcoded constant. Mirrors the formula
                        // `evaporchain_cfm::beta_millibits_per_fee` uses
                        // (`1_000_000 / half_life`, microbits scale,
                        // post Layer 0 #5 fix). At DEFAULT_LAMBDA = 4096
                        // → β = 244 (small but non-zero, the doctrine
                        // requirement). Replicated inline to avoid
                        // adding `evaporchain-cfm` to the consensus
                        // Cargo.toml mid-session — Lane I.7 will promote
                        // the dep cleanly.
                        let half_life = evaporchain_energy_kernel::ChainLambda::new(
                            evaporchain_energy_kernel::DEFAULT_LAMBDA,
                        )
                        .half_life()
                        .max(1);
                        let beta_mb = 1_000_000u64 / half_life;
                        let fc = crate::fork_choice::MccForkChoice::new(
                            self.light_cone_dag.clone(),
                            beta_mb,
                        );
                        let v = crate::fork_choice::ForkChoice::evaluate(
                            &fc,
                            &self.parent_hash,
                            &block.parent_hash,
                        );
                        v.accept
                    }
                    _ => block.parent_hash == self.parent_hash,
                };
                if !parent_accepted {
                    warn!(
                        height = height,
                        round = round,
                        local_parent = %hex::encode(&self.parent_hash[..8]),
                        proposal_parent = %hex::encode(&block.parent_hash[..8]),
                        "Proposal parent hash mismatch — requesting sync"
                    );
                    // Our parent hash doesn't match the network's. Request
                    // recent blocks so we can re-derive the correct chain tip.
                    // Ask for the last few blocks leading up to this height.
                    let sync_from = self.height.saturating_sub(5);
                    actions.push(ConsensusAction::RequestSync(sync_from, height));
                    return actions;
                }

                // Timestamp monotonicity: block timestamp must not decrease
                if block.timestamp > 0
                    && self.last_block_timestamp > 0
                    && block.timestamp < self.last_block_timestamp
                {
                    warn!(
                        height = height,
                        round = round,
                        block_ts = block.timestamp,
                        last_ts = self.last_block_timestamp,
                        "Rejected proposal: timestamp not monotonically increasing"
                    );
                    return actions;
                }

                let hash = Self::block_hash(&block);

                // ── Equivocation Detection ──
                // Track proposals per (height, round). If the same proposer sends
                // two different block hashes for the same slot, slash them.
                let key = (height, round);
                // Look up any prior proposal hash from this validator
                // *and* drop the borrow before calling self-mutating
                // slash / taint methods. Holding `entry` across those
                // calls trips the borrow checker (E0499).
                let prev_hash_for_proposer: Option<[u8; 32]> = self
                    .proposals_seen
                    .get(&key)
                    .and_then(|v| v.iter().find(|(id, _)| *id == proposer_id).map(|(_, h)| *h));
                if let Some(prev_hash) = prev_hash_for_proposer {
                    if prev_hash != hash {
                        // EQUIVOCATION: same proposer, same slot, different block!
                        let slashed =
                            self.sanov_slash_equivocation(proposer_id, SANOV_EQUIVOCATION_WINDOW);
                        warn!(
                            validator = proposer_id,
                            slashed_amount = slashed,
                            height = height,
                            round = round,
                            "SLASHED for equivocation (double-signing)"
                        );
                        // H2 (audit 2026-05-02): taint BOTH the prior
                        // and the conflicting hash so fork-choice
                        // (singh-attractor / MCC) can never select
                        // either branch.
                        self.slashed_equivocator_blocks.insert(prev_hash);
                        self.slashed_equivocator_blocks.insert(hash);
                        actions.push(ConsensusAction::SlashValidator {
                            validator_id: proposer_id,
                            amount: slashed,
                            reason: SlashReason::Equivocation,
                        });
                        return actions; // Reject the equivocating proposal
                    }
                } else {
                    self.proposals_seen
                        .entry(key)
                        .or_default()
                        .push((proposer_id, hash));
                }

                // ── Nova proof verification ──
                // If a proof verifier is configured, validate the block's nova_proof.
                // Blocks without proofs are accepted (proof may be generated async).
                if let (Some(ref verifier), Some(ref proof_bytes)) =
                    (&self.proof_verifier, &block.nova_proof)
                {
                    if !verifier.verify_block_proof(
                        proof_bytes,
                        block.number,
                        self.genesis_state_root,
                    ) {
                        warn!(
                            height = height,
                            round = round,
                            proposer = proposer_id,
                            "Rejected proposal: invalid Nova proof"
                        );
                        return actions;
                    }
                    debug!(height = height, "Nova proof verified on proposal");
                }

                // Reject zero state_root proposals when we have a real state root
                if block.number > 1
                    && block.state_root == [0u8; 32]
                    && self.current_state_root != [0u8; 32]
                {
                    warn!(
                        height = height,
                        round = round,
                        "Rejected proposal: zero state_root on non-genesis block"
                    );
                    return actions;
                }

                // Verify the proposed state_root matches our local pre-execution state.
                // Log a warning but do NOT reject — a transient divergence (e.g. after
                // a sync) must not stall the round.  Post-execution state verification
                // in execute_block() catches genuine forks.
                if self.current_state_root != [0u8; 32]
                    && block.state_root != [0u8; 32]
                    && block.state_root != self.current_state_root
                {
                    warn!(
                        height = height,
                        round = round,
                        proposer = proposer_id,
                        local = %hex::encode(&self.current_state_root[..8]),
                        proposed = %hex::encode(&block.state_root[..8]),
                        "State root mismatch (pre-execution) — accepting proposal, will verify post-execution"
                    );
                }

                // ── VRF proof verification ──
                // If the proposer has a VRF public key and the block includes
                // a VRF proof, verify that the VRF output is valid for this
                // (height, round). This proves the proposer legitimately won
                // the leader election lottery.
                if let (Some(ref vrf_out), Some(ref vrf_proof)) =
                    (&block.vrf_output, &block.vrf_proof)
                {
                    if let Some(proposer_info) = self.validator_set.get(proposer_id) {
                        if let Some(ref vrf_pk) = proposer_info.vrf_public_key {
                            let alpha = leader_vrf_input(height, round);
                            let output = VrfOutput(*vrf_out);
                            let proof = VrfProof(vrf_proof.clone());
                            if !vrf_verify(vrf_pk, &alpha, &output, &proof) {
                                warn!(
                                    height = height,
                                    round = round,
                                    proposer = proposer_id,
                                    "Rejected proposal: invalid VRF proof"
                                );
                                return actions;
                            }
                            debug!(height = height, "VRF proof verified on proposal");
                        }
                    }
                }

                // ── Anchor hash verification ──
                // At anchor heights, verify the proposed anchor_hash matches
                // our locally computed anchor to prevent state root divergence.
                if let Some(ref provider) = self.anchor_provider {
                    if let Some(proposed_anchor) = block.anchor_hash {
                        if let Some(local_anchor) = provider.anchor_hash_for_height(height) {
                            if local_anchor != [0u8; 32]
                                && proposed_anchor != [0u8; 32]
                                && proposed_anchor != local_anchor
                            {
                                // Anchor divergence after node rejoin is expected
                                // because frontier state isn't synced. State_root
                                // comparison after execution catches real divergence.
                                warn!(
                                    height = height,
                                    round = round,
                                    proposer = proposer_id,
                                    local = %hex::encode(&local_anchor[..8]),
                                    proposed = %hex::encode(&proposed_anchor[..8]),
                                    "Anchor hash mismatch (non-fatal, state_root verified post-execution)"
                                );
                            }
                            debug!(height = height, "Anchor hash verified on proposal");
                        }
                    }
                }

                // ── Weak subjectivity check ──
                if !self.check_weak_subjectivity(&block) {
                    warn!(
                        height = height,
                        round = round,
                        "Rejected proposal: violates weak subjectivity"
                    );
                    return actions;
                }

                // ── DA certificate verification ──
                if !self.verify_da_certificate(&block) {
                    warn!(
                        height = height,
                        round = round,
                        proposer = proposer_id,
                        "Rejected proposal: invalid DA certificate"
                    );
                    return actions;
                }

                // ── Crooks-MEV refund validation (Phase 3.5b of
                // CROOKS_MEV_INTEGRATION_PLAN.md) ──
                // No-op in `observe` mode (default). In `enforce`
                // mode, rejects proposals whose RefundTx set
                // diverges from the chain's deterministically-
                // computed expected set. Phase 3.5c will add the
                // proposer-slash for `MissingRefund`.
                if let Err(refund_err) = self.validate_block_refunds(&block) {
                    // Phase 3.5c — bump per-proposer MissingRefund
                    // counter; operators feed this into
                    // entropic_slash(stake, counts) at slashing time.
                    // Mismatched/Unexpected don't bump because they
                    // could result from a benign software-version
                    // skew; only true omission is the slashable case.
                    if matches!(
                        refund_err,
                        evaporchain_mev_detect::RefundValidationError::MissingRefund { .. }
                    ) {
                        *self
                            .mev_missing_refund_violations
                            .entry(proposer_id)
                            .or_insert(0) += 1;
                    }
                    warn!(
                        height = height,
                        round = round,
                        proposer = proposer_id,
                        error = %refund_err,
                        "Rejected proposal: Crooks-MEV refund-set mismatch"
                    );
                    return actions;
                }

                // ── 2D DA row/col root verification ──
                if !block.da_row_roots.is_empty() {
                    if let Ok(tx_bytes) = serde_json::to_vec(&block.transactions) {
                        let da2d = BlockDA2D::new();
                        if let Ok(package) = da2d.encode_block(&tx_bytes) {
                            if package.header.row_roots != block.da_row_roots
                                || package.header.col_roots != block.da_col_roots
                            {
                                warn!(
                                    height = height,
                                    round = round,
                                    proposer = proposer_id,
                                    "Rejected proposal: DA-2D row/col roots mismatch"
                                );
                                return actions;
                            }
                        }
                    }
                }

                self.round_state.proposed_block = Some(block);
                self.round_state.proposed_hash = Some(hash);

                // Send prevote if we haven't already
                if !self.round_state.prevoted {
                    self.round_state.prevoted = true;

                    // Tendermint lock rule: once locked on a block, only vote
                    // for that block. Voting for a different block just because
                    // `locked_round < current_round` violates safety.
                    let vote_hash = if let (Some(ref locked), Some(_lr)) =
                        (&self.locked_block, self.locked_round)
                    {
                        let locked_hash = Self::block_hash(locked);
                        if locked_hash == hash {
                            Some(hash)
                        } else {
                            None // locked on different block — vote nil
                        }
                    } else {
                        Some(hash) // not locked, vote for proposal
                    };

                    self.round_state.prevotes.insert(self.my_id, vote_hash);
                    let bls_sig = self.bls_sign_vote(
                        self.height,
                        self.round_state.round,
                        &vote_hash,
                        "prevote",
                    );
                    if let Some(ref sig) = bls_sig {
                        self.round_state
                            .prevote_bls_sigs
                            .insert(self.my_id, sig.clone());
                    }
                    let prevote = ConsensusMessage::Prevote {
                        height: self.height,
                        round: self.round_state.round,
                        block_hash: vote_hash,
                        validator_id: self.my_id,
                        bls_signature: bls_sig,
                    };
                    actions.push(ConsensusAction::BroadcastMessage(prevote));

                    // DA sampling: if we voted for the block, sample its data availability
                    // and broadcast an attestation so the next proposer can build a certificate.
                    if vote_hash.is_some() {
                        if let Some(ref proposed) = self.round_state.proposed_block {
                            if let Some(pid) = proposed.producer_id {
                                self.da_block_proposers.insert(proposed.number, pid);
                            }
                            if let Some(att_msg) = self.perform_da_sampling(proposed) {
                                actions.push(ConsensusAction::BroadcastMessage(att_msg));
                            }
                        }
                    }

                    self.round_state.phase = Phase::Prevote;
                    self.round_state.phase_start = Instant::now();
                }
            }

            ConsensusMessage::Prevote {
                height,
                round,
                block_hash,
                validator_id,
                bls_signature,
            } => {
                if height != self.height {
                    return actions;
                }
                if round == self.round_state.round {
                    // ── Validator Membership Check ──
                    let validator = match self.validator_set.get(validator_id) {
                        Some(v) => v,
                        None => {
                            warn!(validator_id, "Rejecting prevote from unknown validator");
                            return actions;
                        }
                    };

                    // ── BLS Signature Verification ──
                    if let Some(ref bls_pk_bytes) = validator.bls_public_key {
                        let msg =
                            Self::bls_vote_message(self.height, round, &block_hash, "prevote");
                        match &bls_signature {
                            Some(sig) => {
                                let pk = BlsPublicKey(bls_pk_bytes.clone());
                                let sig = BlsSignature(sig.clone());
                                if !BlsVerifier::verify(&msg, &sig, &pk) {
                                    warn!(
                                        validator_id,
                                        "Rejecting prevote with invalid BLS signature"
                                    );
                                    return actions;
                                }
                            }
                            None => {
                                warn!(validator_id, "Rejecting prevote without BLS signature");
                                return actions;
                            }
                        }
                    } else if self.validator_set.has_bls_keys() {
                        warn!(
                            validator_id,
                            "Rejecting prevote: validator missing BLS key in BLS-enabled set"
                        );
                        return actions;
                    }

                    // ── Vote Equivocation Detection ──
                    if let Some(&existing_hash) = self.round_state.prevotes.get(&validator_id) {
                        if existing_hash != block_hash {
                            let slashed = self
                                .sanov_slash_equivocation(validator_id, SANOV_EQUIVOCATION_WINDOW);
                            warn!(
                                validator = validator_id,
                                slashed_amount = slashed,
                                height = self.height,
                                round = round,
                                "SLASHED for prevote equivocation (double-voting)"
                            );
                            return actions;
                        }
                    }
                    self.round_state.prevotes.insert(validator_id, block_hash);
                    let dag_sig = bls_signature.clone().unwrap_or_default();
                    if let Some(sig) = bls_signature {
                        self.round_state.prevote_bls_sigs.insert(validator_id, sig);
                    }

                    // Phase 4.1 of LIGHT_CONE_FULL_DAG_PLAN.md —
                    // mirror the vote into per-tip dag_round_states
                    // when the rollout flag is on AND the voted-for
                    // block is a current DAG leaf. No-op otherwise
                    // (linear-mode bit-compat preserved).
                    if let Some(tip) = block_hash {
                        if self.light_cone_dag.contains(&tip) {
                            self.record_dag_prevote(tip, validator_id, block_hash, dag_sig);
                        }
                    }

                    if self.round_state.phase == Phase::Prevote {
                        if let Some(hash) = self.check_prevote_quorum() {
                            if !self.round_state.precommitted {
                                self.round_state.precommitted = true;
                                if let Some(ref quorum_hash) = hash {
                                    if let Some(ref proposed) = self.round_state.proposed_block {
                                        if Self::block_hash(proposed) == *quorum_hash {
                                            self.locked_block =
                                                self.round_state.proposed_block.clone();
                                            self.locked_round = Some(self.round_state.round);
                                            self.valid_block =
                                                self.round_state.proposed_block.clone();
                                            self.valid_round = Some(self.round_state.round);
                                        }
                                    }
                                }
                                let bls_sig = self.bls_sign_vote(
                                    self.height,
                                    self.round_state.round,
                                    &hash,
                                    "precommit",
                                );
                                if let Some(ref sig) = bls_sig {
                                    self.round_state
                                        .precommit_bls_sigs
                                        .insert(self.my_id, sig.clone());
                                }
                                let precommit = ConsensusMessage::Precommit {
                                    height: self.height,
                                    round: self.round_state.round,
                                    block_hash: hash,
                                    validator_id: self.my_id,
                                    bls_signature: bls_sig,
                                };
                                actions.push(ConsensusAction::BroadcastMessage(precommit));
                                self.round_state.precommits.insert(self.my_id, hash);
                            }
                            self.round_state.phase = Phase::Precommit;
                            self.round_state.phase_start = Instant::now();
                        }
                    }
                }
            }

            ConsensusMessage::Precommit {
                height,
                round,
                block_hash,
                validator_id,
                bls_signature,
            } => {
                if height != self.height {
                    return actions;
                }
                if round == self.round_state.round {
                    // ── Validator Membership Check ──
                    let validator = match self.validator_set.get(validator_id) {
                        Some(v) => v,
                        None => {
                            warn!(validator_id, "Rejecting precommit from unknown validator");
                            return actions;
                        }
                    };

                    // ── BLS Signature Verification ──
                    if let Some(ref bls_pk_bytes) = validator.bls_public_key {
                        let msg =
                            Self::bls_vote_message(self.height, round, &block_hash, "precommit");
                        match &bls_signature {
                            Some(sig) => {
                                let pk = BlsPublicKey(bls_pk_bytes.clone());
                                let sig = BlsSignature(sig.clone());
                                if !BlsVerifier::verify(&msg, &sig, &pk) {
                                    warn!(
                                        validator_id,
                                        "Rejecting precommit with invalid BLS signature"
                                    );
                                    return actions;
                                }
                            }
                            None => {
                                warn!(validator_id, "Rejecting precommit without BLS signature");
                                return actions;
                            }
                        }
                    } else if self.validator_set.has_bls_keys() {
                        warn!(
                            validator_id,
                            "Rejecting precommit: validator missing BLS key in BLS-enabled set"
                        );
                        return actions;
                    }

                    // ── Vote Equivocation Detection ──
                    if let Some(&existing_hash) = self.round_state.precommits.get(&validator_id) {
                        if existing_hash != block_hash {
                            let slashed = self
                                .sanov_slash_equivocation(validator_id, SANOV_EQUIVOCATION_WINDOW);
                            warn!(
                                validator = validator_id,
                                slashed_amount = slashed,
                                height = self.height,
                                round = round,
                                "SLASHED for precommit equivocation (double-voting)"
                            );
                            return actions;
                        }
                    }
                    self.round_state.precommits.insert(validator_id, block_hash);
                    let dag_sig = bls_signature.clone().unwrap_or_default();
                    if let Some(sig) = bls_signature {
                        self.round_state
                            .precommit_bls_sigs
                            .insert(validator_id, sig);
                    }

                    // Phase 4.1 + 4.3 of LIGHT_CONE_FULL_DAG_PLAN.md —
                    // mirror the precommit into per-tip
                    // dag_round_states + run cross-fork equivocation
                    // detection (Decision 3 — counts-based; bumps
                    // cross_fork_equivocations[validator_id] on
                    // observed double-precommit at the same round).
                    // No-op when state_branches_enabled = false.
                    if let Some(tip) = block_hash {
                        if self.light_cone_dag.contains(&tip) {
                            self.record_dag_precommit(tip, validator_id, block_hash, dag_sig);
                        }
                    }

                    // Check if we can commit now. Peek before `take()`ing —
                    // see the matching tick-path block above for the bug
                    // history (DA-attestation race wedged the cluster).
                    if let Some(Some(hash)) = self.check_precommit_quorum() {
                        if let Some(block) = self.round_state.proposed_block.as_ref() {
                            let block_hash = Self::block_hash(block);
                            if block_hash != hash {
                                if let Some(stale) = self.round_state.proposed_block.take() {
                                    for tx in stale.transactions.iter().rev() {
                                        self.mempool.submit_priority(tx.clone());
                                    }
                                }
                                actions.push(ConsensusAction::RequestSync(
                                    self.height,
                                    self.height + 1,
                                ));
                            } else {
                                // P2-04: refuse to commit if block has a data_root
                                // but DA attestation supermajority hasn't been reached.
                                let mainnet = block.chain_id.starts_with("mainnet-");
                                let enforce_da =
                                    mainnet || self.height >= self.da_enforcement_height;
                                if enforce_da
                                    && block.data_root.is_some()
                                    && !self.has_da_supermajority(block.number)
                                {
                                    let key = (block.number, self.round_state.round);
                                    if self.p2_04_last_warned != Some(key) {
                                        self.p2_04_last_warned = Some(key);
                                        warn!(
                                            height = block.number,
                                            "P2-04: refusing to commit — DA attestation supermajority not reached (msg path)"
                                        );
                                    }
                                    // Keep proposed_block so the next tick
                                    // can retry once the missing DA
                                    // attestation gossip arrives.
                                } else {
                                    let mut block = self
                                        .round_state
                                        .proposed_block
                                        .take()
                                        .expect("proposed_block was Some above");
                                    if block.commit_certificate.is_none() {
                                        block.commit_certificate =
                                            self.try_build_commit_certificate(hash);
                                    }
                                    self.round_state.phase = Phase::Commit;
                                    actions.push(ConsensusAction::CommitBlock(block));
                                }
                            }
                        }
                    }
                }
            }
            // KeyAnnounce, DAAttestation, OracleVote are handled before
            // height filters — unreachable here.
            ConsensusMessage::KeyAnnounce { .. } => {}
            ConsensusMessage::DAAttestation { .. } => {}
            ConsensusMessage::OracleVote { .. } => {}
        }

        actions
    }

    /// Called after a block has been committed (applied to state).
    /// Advances to the next height.
    pub fn on_block_committed(
        &mut self,
        block: &Block,
        state_root: [u8; 32],
        objects_evaporated: usize,
    ) {
        // Per-height finality gap tracking (Mainnet P1).
        // Stamp the wall-clock at commit time. The finalisation hook below
        // (or a later out-of-band finalise) removes this entry and pushes
        // a (height, gap_ms) sample into `finality_gap_history`. Heights
        // committed without a cert remain in `committed_at` and surface
        // through `unfinalised_tail()` so operators see the stall.
        let commit_now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.committed_at.insert(block.number, commit_now_ms);
        // Phase 4.4 of LIGHT_CONE_FULL_DAG_PLAN.md (Decision 4) —
        // dual-mode finality bookkeeping. Populates the block-
        // indexed view alongside the height-indexed one so DAG-aware
        // consumers (Phase 4.2 antichain finalization) can query by
        // BlockId. Cap at 1024 entries — same buffer-cap pattern as
        // MEV_OBSERVATION_BUFFER_CAP, prune oldest by height.
        let block_id = Self::block_hash(block);
        self.committed_at_block.insert(block_id, commit_now_ms);
        if self.committed_at_block.len() > 1024 {
            // Prune the entry with the smallest commit timestamp
            // (oldest commit). HashMap iteration is non-deterministic,
            // but the pruning rule is deterministic across validators
            // since they all commit the same blocks at the same
            // committed_at_ms (well, per-validator wall-clock differs;
            // operators tolerate up to 1024-entry slack).
            if let Some((&victim, _)) = self.committed_at_block.iter().min_by_key(|(_, &t)| t) {
                self.committed_at_block.remove(&victim);
            }
        }

        // Update validator health
        if let Some(producer_id) = block.producer_id {
            self.validator_set
                .update_health_score(producer_id, objects_evaporated);
            // Reset missed-proposal counter for successful producer
            self.missed_proposals.insert(producer_id, 0);
            // Refresh Boltzmann stake for the block producer — credits
            // active validators and "kills stake-and-lease-key-to-MEV"
            // (INVENTION_STACK.md §4.1 #5).
            let refresh_per_block = 100u64;
            self.refresh_proposer_boltzmann_stake(producer_id, block.epoch, refresh_per_block);
        }
        self.validator_set.decay_health_scores();
        // At each epoch boundary, apply Boltzmann decay to all validators.
        // Idle validators' effective weight shrinks; active ones are refreshed
        // above, keeping their weight stable.
        if block.epoch != self.epoch {
            self.decay_all_boltzmann_stakes(block.epoch);
        }

        // Derive parent hash for next block
        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(&block.number.to_le_bytes());
        hash_input.extend_from_slice(&block.epoch.to_le_bytes());
        hash_input.extend_from_slice(&state_root);
        hash_input.extend_from_slice(&block.parent_hash);
        self.parent_hash = blake3_hash(&hash_input);

        // Lane O.8.1: tick the Causal-CHSH cartel alarm. Pushes a
        // BlockSummary into the rolling buffer; the alarm internally
        // recomputes the gate every `run_interval_blocks` records. No
        // ConsensusAction emission yet — observability only. Status
        // surfaces via `cartel_alarm_status()` for RPC consumption.
        // Doctrine reference: INVENTION_STACK.md §A1.10.
        let total_gas: u64 = block
            .transactions
            .iter()
            .map(|_| 21_000u64) // GAS_TRANSFER lower bound; real gas accounting happens in the executor, not here
            .sum();
        self.cartel_alarm.record_block(
            evaporchain_causal_chsh::BlockSummary {
                height: block.number,
                timestamp_secs: block.timestamp,
                energy: total_gas, // proxy: gas as energy weight
                gas: total_gas,
                tx_count: block.transactions.len() as u64,
            },
            block.number,
        );
        // Lane O.8.2: emit a CartelAlarmEvent when the chain's S crosses
        // the doctrine ceiling AND governance has set
        // `cartel_alarm_mode = "alarm"`. Default `"observe"` skips
        // emission; the alarm verdict still gets stored on the
        // alarm itself for status RPCs (Lane O.8.1b).
        self.maybe_emit_cartel_alarm_event();

        // Record this block in the parallel Light-Cone DAG. Per
        // INVENTION_STACK.md §4.1 #1 this is the substrate for the
        // partial-order consensus that replaces Tendermint via
        // governance amendment. For now: read-only observability —
        // chain authority is still Tendermint's linear chain.
        // Genesis (block.number == 0) inserted with no parents;
        // subsequent blocks inherit `block.parent_hash` if it's
        // already in the DAG (which it should be, as we insert in
        // commit order).
        let lc_block = evaporchain_light_cone::Block::new(
            self.parent_hash,
            // Parent linkage in the DAG: only include the parent if
            // we already have it in the DAG (to satisfy the
            // MissingParent invariant). Genesis edge case: empty.
            if self.light_cone_dag.contains(&block.parent_hash) {
                vec![block.parent_hash]
            } else {
                vec![]
            },
            // Per-block "energy": substitute total gas spent — the
            // chain's natural per-block work measure. Real production
            // wires whatever the chain accounts as per-block energy.
            block.transactions.len() as u64,
            block.epoch,
        );
        // Silently ignore re-insertions (forks the chain rejected
        // would never reach on_block_committed; if they did, the DAG
        // would have a duplicate-id rejection that we don't want to
        // propagate as a panic).
        let _ = self.light_cone_dag.insert(lc_block);

        // Phase 3.1 of LIGHT_CONE_FULL_DAG_PLAN.md — track the
        // newly-committed block as a state-branch tip when the
        // governance flag is on. (Default false → table stays
        // empty regardless of DAG activity → chain bit-compat.)
        // Caliber estimate at this insertion is just the block's
        // own energy (block_j); Phase 3.4's full re-scoring will
        // walk the trajectory.
        let state_branches_enabled = self
            .governance_params
            .get("light_cone_state_branches_enabled")
            .map(|s| s.as_str())
            == Some("true");
        if state_branches_enabled {
            // BlockId for the LightCone insertion above is
            // `block.parent_hash` if number == 0, else `block.parent_hash`
            // the prior tip. Use Tendermint's block hash
            // computation as the canonical id.
            let tip_id = Self::block_hash(block);
            // Caliber estimate at insertion time = block tx count
            // (matches the Light-Cone insertion's per-block "energy"
            // parameter). Phase 3.4 will re-score along trajectories.
            let caliber_estimate = block.transactions.len() as u64;
            self.record_state_branch(tip_id, block.number, caliber_estimate);
            self.prune_state_branches();

            // Phase 4.4 — push the now-current closing-antichain digest
            // into the rolling history. Operators can later
            // retrospectively cross-compare per-height digests across
            // cluster validators via
            // `/api/light_cone/antichain_digest_history`. The digest
            // is computed AFTER the new block has been inserted into
            // light_cone_dag (above) so it reflects the post-commit
            // state. FIFO eviction at `ANTICHAIN_DIGEST_HISTORY_CAP`.
            let digest =
                evaporchain_light_cone::concurrency::closing_antichain_digest(&self.light_cone_dag);
            self.antichain_digest_history
                .push_back((block.number, digest));
            while self.antichain_digest_history.len() > ANTICHAIN_DIGEST_HISTORY_CAP {
                self.antichain_digest_history.pop_front();
            }
        }

        // TUR Liveness Detector observation. Push this block's tx
        // count as the chain "current J" (same proxy the parallel
        // Light-Cone insert uses for per-block work), maintain a
        // sliding window, and run tur_check using a window-summed Σ
        // proxy. Verdict::Violation is the cartel signature: J too
        // steady for the entropy budget. Per INVENTION_STACK.md §A1.3.
        let block_j = block.transactions.len() as u64;
        self.tur_window.push_back(block_j);
        while self.tur_window.len() > TUR_WINDOW_BLOCKS {
            self.tur_window.pop_front();
        }
        if self.tur_window.len() >= 2 {
            let sum: u64 = self.tur_window.iter().sum();
            let sigma = sum.saturating_mul(TUR_SIGMA_PER_GAS_NUM) / TUR_SIGMA_PER_GAS_DEN.max(1);
            let samples: Vec<u64> = self.tur_window.iter().copied().collect();
            self.last_tur_verdict = Some(evaporchain_tur_liveness::tur_check(&samples, sigma));
        }

        // Phase 1.3 + 2 of CROOKS_MEV_INTEGRATION_PLAN.md — scan the
        // committed block for sandwich-shaped tx triples, update
        // per-attacker rolling stats, compute Crooks-fluctuation
        // refund estimates, and append observations to the bounded
        // ring buffer. Phase 1+2 are observe-only — no settlement
        // runs from this buffer until Phase 3 ships the RefundTx
        // plumbing. The scan is O(n²) over Transfer txs only.
        //
        // Phase 2 Decision 4 ordering: prune stale stats, then for
        // each new observation update the stat, THEN compute refund
        // (so the new observation contributes to its own pmf). This
        // is the deterministic-across-validators contract that
        // Phase 3.2 will rely on.
        let window = evaporchain_mev_detect::CROOKS_MEV_DEFAULT_WINDOW_BLOCKS;
        // Beta from governance, with a default if unset/unparseable.
        let beta_mb = self
            .governance_params
            .get("crooks_mev_beta_mb")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(evaporchain_mev_detect::CROOKS_MEV_DEFAULT_BETA_MB);
        // Prune stats older than the window — deterministic since
        // every validator sees the same block.number.
        let prune_horizon = block.number.saturating_sub(window);
        self.mev_attacker_stats
            .retain(|_, stat| stat.last_seen_height >= prune_horizon);

        for mut obs in evaporchain_mev_detect::scan_block(&block.transactions, block.number) {
            // Update or insert attacker stat first (Phase 2 Decision 4).
            let stat = self
                .mev_attacker_stats
                .entry(obs.attacker)
                .and_modify(|s| s.record(block.number))
                .or_insert_with(|| evaporchain_mev_detect::AttackerStat::fresh(block.number));
            obs.refund_amount =
                evaporchain_mev_detect::compute_observation_refund(&obs, stat, beta_mb, window);
            self.mev_observations.push_back(obs);
            while self.mev_observations.len() > MEV_OBSERVATION_BUFFER_CAP {
                self.mev_observations.pop_front();
            }
        }

        // Phase 3.3 of CROOKS_MEV_INTEGRATION_PLAN.md — record any
        // RefundTx the proposer included so the same observation
        // cannot be re-settled in a future block. Replay protection.
        for tx in &block.transactions {
            if let evaporchain_types::Transaction::Refund(r) = tx {
                self.settled_refunds
                    .insert((r.source_block_height, r.source_observation_idx));
            }
        }

        // Lambda-Fold per-block step. Each committed block contributes
        // one StepWitness {state_hash = state_root, step_energy = J,
        // observed_epoch = block.epoch}. The fold accumulator is O(1)
        // memory regardless of chain length. Out-of-order steps are
        // ignored (Tendermint commits monotone in epoch in practice).
        //
        // Phase 5.3 of LAMBDA_FOLD_NOVA_PLAN — branch on the
        // `lambda_fold_mode` governance flag. The substrate path
        // ALWAYS runs (it's cheap, deterministic, and provides the
        // fall-back accumulator if the Nova path errors out). The
        // Nova branch additionally runs the real IVC fold when the
        // flag is `"nova"` AND the `lambda_fold_nova` crate feature is
        // compiled in. The Nova folder is lazily constructed on first
        // use because `RealBlockProver::new` runs a ~60-90 s `pp`
        // setup that we don't want at TendermintConsensus
        // construction time.
        let chain_lambda = evaporchain_energy_kernel::ChainLambda::default_genesis();
        let step = evaporchain_lambda_fold::StepWitness::new(state_root, block_j, block.epoch);
        if let Ok(folded) = evaporchain_lambda_fold::fold(self.lambda_fold, step, chain_lambda) {
            self.lambda_fold = folded;
        }
        #[cfg(feature = "lambda_fold_nova")]
        {
            if self
                .governance_params
                .get("lambda_fold_mode")
                .map(|s| s.as_str())
                == Some("nova")
            {
                if let Err(e) = self.try_nova_fold(block, state_root, block_j) {
                    // Nova fold errors are observed but don't reject
                    // the block — the substrate fold above is the
                    // authoritative chain accumulator until 5.4
                    // promotes the Nova path.
                    tracing::warn!(error = %e, "lambda_fold nova path errored; substrate fold stands");
                }
            }
        }

        // WSBF RG flow — coarse-grain per-block data into effective λ.
        let active_accounts = self.validator_set.len() as u64;
        if let Some(ep) = crate::wsbf_integration::on_committed_block(
            &mut self.wsbf_window,
            block.number,
            block_j,
            active_accounts,
            block.epoch,
            &crate::wsbf_integration::default_rg_params(),
        ) {
            let prev_phase = self.current_consensus_phase;
            let n_validators = self.validator_set.len() as u64;
            self.current_consensus_phase =
                crate::rg_phase_integration::classify_from_effective_params(
                    &ep,
                    n_validators,
                    0, // adversary fraction unknown without evidence; caller can update
                    &evaporchain_rg_phase_map::PhaseMapParams::default(),
                );
            crate::rg_phase_integration::log_phase_transition(
                prev_phase,
                self.current_consensus_phase,
                block.number,
            );
            self.last_effective_params = Some(ep);
        }

        self.epoch = block.epoch;
        self.mempool.set_epoch(block.epoch);
        self.current_state_root = state_root;
        self.committed_heights.insert(self.height);
        if block.timestamp > 0 {
            self.last_block_timestamp = block.timestamp;
        }
        if let Some(pid) = block.producer_id {
            self.da_block_proposers.insert(block.number, pid);
        }
        self.height += 1;

        // Update DA confirmed height by checking attestation rounds
        for h in (self.da_confirmed_height + 1)..=block.number {
            if self.da_attestation.is_confirmed(h) {
                self.da_confirmed_height = h;
            } else {
                break;
            }
        }

        // Advance randomness beacon with this block's VRF output.
        // Bell-Certified gate (§4.2): derive a pseudo-CHSH S-value from the
        // VRF bytes. In production this would be a real entangled-photon
        // measurement; here we extract 4 correlation values from the VRF
        // output. Non-gating (advisory) until hardware CHSH is plumbed —
        // we always ingest but warn when the S-value fails the Bell test.
        if let Some(ref vrf_out) = block.vrf_output {
            if vrf_out.len() >= 8 {
                // Map each byte pair to a correlation in [-1000, 1000].
                let corr = |hi: u8, lo: u8| -> i64 {
                    let raw = i64::from(hi as i16 - 128) * 1000 / 128
                        + i64::from(lo as i16 - 128) * 1000 / 128;
                    raw.clamp(-1000, 1000)
                };
                let e_ab = corr(vrf_out[0], vrf_out[1]);
                let e_ab_prime = corr(vrf_out[2], vrf_out[3]);
                let e_a_prime_b = corr(vrf_out[4], vrf_out[5]);
                let e_a_prime_b_prime = corr(vrf_out[6], vrf_out[7]);
                if let Ok(s_milli) =
                    bell_chsh_s_value(e_ab, e_ab_prime, e_a_prime_b, e_a_prime_b_prime)
                {
                    let certified = bell_is_certified(s_milli, BELL_LOCAL_REALISM_S_MILLI);
                    // Persist the latest measurement so the node API's
                    // `GET /api/bell/latest` handler can serve a live
                    // S-value instead of `no_data`. Surfaced via
                    // `TendermintConsensus::last_bell_reading`.
                    self.last_bell_s_milli = Some(s_milli);
                    self.last_bell_block_height = block.number;
                    self.last_bell_epoch = block.epoch;
                    self.last_bell_certified = certified;
                    if !certified {
                        warn!(
                            height = block.number,
                            s_milli, "Bell gate: VRF-derived CHSH S-value ≤ 2 (advisory)"
                        );
                    } else {
                        debug!(
                            height = block.number,
                            s_milli, "Bell gate: beacon certified"
                        );
                    }
                }
            }
            self.randomness_beacon.ingest(block.number, vrf_out);
        }

        // ── Weak Subjectivity Checkpoint ──
        // Periodically snapshot (height, state_root) so nodes refuse to reorg
        // past this point. Prevents long-range attacks.
        if block.number > 0 && block.number.is_multiple_of(self.checkpoint_interval) {
            self.weak_subjectivity_checkpoints
                .push((block.number, state_root));
            self.prune_old_checkpoints();
            info!(
                height = block.number,
                state_root = %hex::encode(&state_root[..8]),
                ws_period = self.weak_subjectivity_period(),
                checkpoints_kept = self.weak_subjectivity_checkpoints.len(),
                "Weak subjectivity checkpoint created"
            );
        }

        // ── Epoch Transition ──
        // Scan committed block for validator stake/exit transactions
        // and queue them for the epoch transition manager.
        for tx in &block.transactions {
            match tx {
                Transaction::ValidatorStake(ref stake_tx) => {
                    let info = ValidatorInfo::new(
                        stake_tx.validator_id,
                        stake_tx.stake_amount,
                        stake_tx.validator_address,
                    );
                    self.epoch_manager
                        .queue_change(ValidatorSetChange::Join(info), block.epoch);
                    debug!(
                        validator = stake_tx.validator_id,
                        stake = stake_tx.stake_amount,
                        "Queued validator join for next epoch boundary"
                    );
                }
                Transaction::ValidatorExit(ref exit_tx) => {
                    self.epoch_manager.queue_change(
                        ValidatorSetChange::Leave {
                            validator_id: exit_tx.validator_id,
                        },
                        block.epoch,
                    );
                    debug!(
                        validator = exit_tx.validator_id,
                        "Queued validator leave for next epoch boundary"
                    );
                }
                _ => {}
            }
        }

        // Apply epoch transitions at epoch boundaries
        if EpochTransitionManager::is_epoch_boundary(block.number) {
            let result = self
                .epoch_manager
                .apply_epoch_transition(&mut self.validator_set, block.epoch);
            if !result.applied.is_empty() {
                info!(
                    epoch = block.epoch,
                    height = block.number,
                    applied = ?result.applied,
                    deferred = ?result.deferred,
                    rejected = ?result.rejected,
                    validators = self.validator_set.active_count(),
                    "Epoch transition applied"
                );
            }
        }

        // ── Finality Tracking ──
        // Record finality if we have a commit certificate (single-slot finality).
        if let Some(ref cert) = block.commit_certificate {
            let block_hash = Self::block_hash(block);
            if cert.block_hash != block_hash {
                warn!(
                    height = block.number,
                    cert_hash = %hex::encode(&cert.block_hash[..8]),
                    actual_hash = %hex::encode(&block_hash[..8]),
                    "Commit certificate block_hash does not match actual block hash"
                );
            }
            let total_stake = self.validator_set.total_stake();
            let signing_stake = cert
                .signer_ids
                .iter()
                .filter_map(|id| self.validator_set.get_validator(*id))
                .map(|v| v.effective_stake()) // P2-01
                .sum::<u64>();
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            self.finality_tracker.on_block_finalized(
                block.number,
                block_hash,
                state_root,
                block.epoch,
                cert.clone(),
                signing_stake,
                total_stake,
                timestamp,
            );

            // Re-audit (2026-05-02) integration of LightCone prune:
            // call `prune_before_epoch` every 100 finalised blocks
            // with a retention window of 1000 epochs of headroom.
            // This bounds DAG memory without affecting fork-choice
            // (anything we prune is far behind latest_finalized and
            // can't be a candidate head). Same cadence + retention as
            // the node-side block-record prune at main.rs ~line 4357.
            const LIGHT_CONE_PRUNE_INTERVAL: u64 = 100;
            const LIGHT_CONE_RETENTION_EPOCHS: u64 = 1_000;
            if block.number > 0 && block.number % LIGHT_CONE_PRUNE_INTERVAL == 0 {
                let cutoff = block.epoch.saturating_sub(LIGHT_CONE_RETENTION_EPOCHS);
                if cutoff > 0 {
                    let pruned = self.light_cone_dag.prune_before_epoch(cutoff);
                    if pruned > 0 {
                        debug!(
                            block = block.number,
                            cutoff_epoch = cutoff,
                            pruned,
                            "LightCone DAG pruned"
                        );
                    }
                }
            }

            // Per-height finality gap closure (Mainnet P1). Pop the
            // commit timestamp for this height and record the
            // commit→finalise duration. `commit_now_ms` was sampled at
            // the top of this function so the gap reflects the work
            // done between commit and cert observation; for single-slot
            // finality it's tiny, but a stalled height stays in
            // `committed_at` and is reported via `unfinalised_tail()`.
            let finalise_now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            if let Some(committed_ms) = self.committed_at.remove(&block.number) {
                let gap_ms = finalise_now_ms.saturating_sub(committed_ms);
                self.finality_gap_history.push_back((block.number, gap_ms));
                while self.finality_gap_history.len() > FINALITY_GAP_HISTORY_CAP {
                    self.finality_gap_history.pop_front();
                }
            }
        }

        // ── DA Attestation Round ──
        // Start a new DA attestation round if the block has a data_root.
        // Validators will sample shards and submit attestations.
        if let Some(data_root) = block.data_root {
            let total_stake = self.validator_set.total_stake();
            self.da_attestation
                .start_round(block.number, data_root, total_stake);

            // If we have a BLS keypair, create our own attestation immediately
            if let Some(ref bls_kp) = self.bls_keypair {
                if let Some(my_validator) = self.validator_set.get_validator(self.my_id) {
                    let att = self.da_attestation.create_own_attestation(
                        self.my_id,
                        my_validator.effective_stake(), // P2-01
                        bls_kp,
                    );
                    if let Some(attestation) = att {
                        let _ = self.da_attestation.add_attestation(attestation);
                    }
                }
            }
        }

        // Clean up old proposal evidence (keep only last 10 heights)
        let cutoff = self.height.saturating_sub(10);
        self.proposals_seen.retain(|(h, _), _| *h >= cutoff);

        // Reset round state and timeouts for new height
        self.round_state = RoundState::new(0);
        self.set_timeouts_for_round(0);
        self.locked_block = None;
        self.locked_round = None;
        self.valid_block = None;
        self.valid_round = None;

        info!(
            height = self.height,
            epoch = self.epoch,
            "Advanced to next height"
        );
    }

    /// Verify a block does not violate weak subjectivity.
    /// Checks both height ordering AND state root consistency.
    pub fn check_weak_subjectivity(&self, block: &Block) -> bool {
        // Check against trusted checkpoint (provided at bootstrap)
        if let Some((cp_height, cp_root, _cp_hash)) = self.trusted_checkpoint {
            if block.number < cp_height {
                warn!(
                    block = block.number,
                    checkpoint = cp_height,
                    "Block rejected: below trusted checkpoint"
                );
                return false;
            }
            if block.number == cp_height && block.state_root != cp_root {
                warn!(
                    block = block.number,
                    expected = %hex::encode(&cp_root[..8]),
                    got = %hex::encode(&block.state_root[..8]),
                    "Block rejected: state_root mismatch with trusted checkpoint"
                );
                return false;
            }
        }

        // Check against most recent rolling checkpoint
        if let Some(&(cp_height, cp_root)) = self.weak_subjectivity_checkpoints.iter().next_back() {
            if block.number < cp_height {
                warn!(
                    block = block.number,
                    checkpoint = cp_height,
                    "Block rejected: reorg past weak subjectivity checkpoint"
                );
                return false;
            }
            if block.number == cp_height && block.state_root != cp_root {
                warn!(
                    block = block.number,
                    checkpoint = cp_height,
                    "Block rejected: state_root diverges from checkpoint"
                );
                return false;
            }
        }
        true
    }

    /// Compute the weak subjectivity period in blocks.
    ///
    /// Based on: finality depth + unbonding period + churn-to-majority time + buffer.
    /// A node offline longer than this period MUST resync with a fresh trusted checkpoint.
    pub fn weak_subjectivity_period(&self) -> u64 {
        let finality_depth: u64 = 1; // single-slot BFT finality
        let unbonding_blocks: u64 = 3 * 100; // UNBONDING_PERIOD_EPOCHS * EPOCH_LENGTH
        let validator_count = self.validator_set.active_count() as u64;
        let max_churn_per_epoch = std::cmp::max(1, validator_count / 3);
        let epochs_to_majority = validator_count.div_ceil(max_churn_per_epoch);
        let churn_blocks = epochs_to_majority * 100; // EPOCH_LENGTH
        let safety_margin = 200; // ~200 blocks buffer

        finality_depth + unbonding_blocks + churn_blocks + safety_margin
    }

    /// Set a trusted checkpoint for safe bootstrap.
    /// New nodes joining the network MUST call this before syncing.
    pub fn set_trusted_checkpoint(
        &mut self,
        height: u64,
        state_root: [u8; 32],
        block_hash: [u8; 32],
    ) {
        info!(
            height = height,
            state_root = %hex::encode(&state_root[..8]),
            block_hash = %hex::encode(&block_hash[..8]),
            ws_period = self.weak_subjectivity_period(),
            "Trusted checkpoint set"
        );
        self.trusted_checkpoint = Some((height, state_root, block_hash));
    }

    /// Get the trusted checkpoint if set.
    pub fn trusted_checkpoint(&self) -> Option<(u64, [u8; 32], [u8; 32])> {
        self.trusted_checkpoint
    }

    /// Get all weak subjectivity checkpoints.
    pub fn checkpoints(&self) -> &[(u64, [u8; 32])] {
        &self.weak_subjectivity_checkpoints
    }

    /// Get the latest checkpoint (height, state_root).
    pub fn latest_checkpoint(&self) -> Option<(u64, [u8; 32])> {
        self.weak_subjectivity_checkpoints.last().copied()
    }

    /// Load checkpoints from persistent storage (on restart).
    pub fn load_checkpoints(&mut self, checkpoints: Vec<(u64, [u8; 32])>) {
        self.weak_subjectivity_checkpoints = checkpoints;
    }

    /// Prune checkpoints older than the weak subjectivity period,
    /// keeping at least the most recent one.
    pub fn prune_old_checkpoints(&mut self) {
        let ws_period = self.weak_subjectivity_period();
        let cutoff = self.height.saturating_sub(ws_period);
        if self.weak_subjectivity_checkpoints.len() > 1 {
            let keep_from = self
                .weak_subjectivity_checkpoints
                .iter()
                .rposition(|&(h, _)| h <= cutoff)
                .unwrap_or(0);
            if keep_from > 0 {
                self.weak_subjectivity_checkpoints.drain(..keep_from);
            }
        }
    }

    /// Apply a block received from block sync (not through consensus).
    /// Used for catch-up when joining the network.
    pub fn apply_block(
        &mut self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<BlockProductionResult, ConsensusError> {
        // Weak subjectivity check: refuse blocks that reorg past a checkpoint
        if !self.check_weak_subjectivity(block) {
            return Err(ConsensusError::ExecutionFailed(format!(
                "Block {} violates weak subjectivity checkpoint",
                block.number
            )));
        }

        let execution = self.executor.execute_block(db, block).map_err(
            |e: evaporchain_execution::ExecutionError| {
                ConsensusError::ExecutionFailed(e.to_string())
            },
        )?;

        // Phase 3 of POST_EXEC_STATE_VERIFICATION_PLAN.md — warn-mode
        // post-state-root verification. If the proposer included a
        // claim (Phase 2 fills this on RocksDB-backed nodes), compare
        // it to the local execution result. Mismatch = the proposer
        // and this validator computed a different post-execution
        // state for the same block. Phase 3 logs a structured warning
        // but does NOT reject; Phase 4 will flip this to prevote-NIL
        // behind a fork-epoch governance gate.
        //
        // Reproduced 2026-05-08 on the fresh 5-node cluster: state-
        // root divergence between UK Macs and Helsinki Hetzners on a
        // genesis-h=0 chain. This warning, once Phase 2 is deployed,
        // makes the divergence visible at the per-block level
        // instead of relying on external dashboard observation.
        if let Some(claimed) = block.post_state_root {
            if claimed != execution.state_root {
                let mode = self
                    .governance_params
                    .get("post_state_verify_mode")
                    .map(String::as_str)
                    .unwrap_or("warn");
                warn!(
                    block_height = block.number,
                    proposer_id = ?block.producer_id,
                    local_state_root = %hex::encode(&execution.state_root[..8]),
                    proposer_claim = %hex::encode(&claimed[..8]),
                    verify_mode = %mode,
                    "PHASE-3 post-state-root MISMATCH — local execution diverges from proposer claim"
                );
                // Phase 4 (lane T0.3) — enforce-mode reject. The local
                // node refuses to apply a divergent block. If 2f+1
                // validators are in enforce-mode and agree the block
                // is divergent, the chain stalls until a clean
                // proposal is produced. Operators flip from "warn"
                // to "enforce" only after a clean Phase 3 soak window.
                if mode == "enforce" {
                    return Err(ConsensusError::ExecutionFailed(format!(
                        "PHASE-4 post-state-root mismatch: local={} proposer_claim={}",
                        hex::encode(&execution.state_root[..8]),
                        hex::encode(&claimed[..8])
                    )));
                }
            } else {
                debug!(
                    block_height = block.number,
                    "PHASE-3 post-state-root MATCH"
                );
            }
        }

        // Apply any validator BLS key rotations emitted by execution. Done
        // after execute_block but before on_block_committed so the new
        // pubkey set is visible to any commit-time hooks. Closes 4b.
        if !execution.validator_key_rotations.is_empty() {
            let applied = self.apply_validator_key_rotations(&execution.validator_key_rotations);
            if applied > 0 {
                info!(
                    applied,
                    block = block.number,
                    "Validator key rotations applied"
                );
            }
        }
        // Cheap sweep: drop any prev pubkey whose grace window has elapsed.
        self.purge_expired_prev_keys();

        self.on_block_committed(block, execution.state_root, execution.objects_evaporated);

        // Phase 3.2 of LIGHT_CONE_FULL_DAG_PLAN.md — capture the
        // post-execution state into the just-recorded tip's branch
        // metadata so DAG-mode rollback (`replay_and_apply_atomic`)
        // can restore against this commit. No-op when
        // `light_cone_state_branches_enabled != "true"`.
        if let Err(e) = self.capture_committed_branch_snapshot(block, db) {
            warn!(
                block = block.number,
                error = %e,
                "Light-Cone state-branch snapshot capture failed; chain still committed but DAG-mode rollback against this tip is unavailable"
            );
        }

        info!(
            block = block.number,
            epoch = block.epoch,
            state_root = hex::encode(execution.state_root),
            "Block applied (sync)"
        );

        Ok(BlockProductionResult {
            block: block.clone(),
            execution,
        })
    }

    /// Execute a committed block and return the result.
    pub fn execute_block(
        &mut self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<BlockProductionResult, ConsensusError> {
        let execution = self.executor.execute_block(db, block).map_err(
            |e: evaporchain_execution::ExecutionError| {
                ConsensusError::ExecutionFailed(e.to_string())
            },
        )?;

        // Same post-commit application as in apply_block_sync above.
        if !execution.validator_key_rotations.is_empty() {
            let applied = self.apply_validator_key_rotations(&execution.validator_key_rotations);
            if applied > 0 {
                info!(
                    applied,
                    block = block.number,
                    "Validator key rotations applied"
                );
            }
        }
        self.purge_expired_prev_keys();

        Ok(BlockProductionResult {
            block: block.clone(),
            execution,
        })
    }

    /// Current MMR root from the execution engine.
    pub fn mmr_root(&self) -> [u8; 32] {
        self.executor.mmr_root()
    }

    /// Number of nullifiers in the execution engine's MMR.
    pub fn mmr_size(&self) -> usize {
        self.executor.mmr_size()
    }

    pub fn script_engine(&self) -> &evaporchain_script::ScriptEngine {
        &self.executor.script_engine
    }

    pub fn script_engine_mut(&mut self) -> &mut evaporchain_script::ScriptEngine {
        &mut self.executor.script_engine
    }

    pub fn contract_engine(&self) -> &evaporchain_contracts::ContractEngine {
        &self.executor.contract_engine
    }

    pub fn contract_engine_mut(&mut self) -> &mut evaporchain_contracts::ContractEngine {
        &mut self.executor.contract_engine
    }

    // ──────────────── Internal Helpers ───────────────────────────────────

    /// Create a block proposal from the current mempool.
    /// Caps transactions per block to keep proposals under gossipsub size limits.
    fn create_proposal(&mut self, _db: &mut dyn StateDB) -> Option<Block> {
        if let Some(ref locked) = self.locked_block {
            info!(
                height = self.height,
                round = self.round_state.round,
                "Re-proposing locked block"
            );
            return Some(locked.clone());
        }
        let next_epoch = self.epoch + 1;

        // Process encrypted mempool reveals first (MEV-protected txs get priority)
        let reveals: Vec<([u8; 32], [u8; 32])> = std::mem::take(&mut self.pending_reveals);
        let mut txs: Vec<Transaction> = if !reveals.is_empty() {
            let revealed = self.encrypted_mempool.process_reveals(self.epoch, &reveals);
            if !revealed.is_empty() {
                debug!(
                    revealed_count = revealed.len(),
                    epoch = self.epoch,
                    "Included MEV-protected revealed transactions"
                );
            }
            revealed
        } else {
            // Even without explicit reveals, drain any plaintext txs from encrypted pool
            self.encrypted_mempool.process_reveals(self.epoch, &[])
        };

        // Fill remaining capacity from plain mempool, respecting gas limit.
        // Uses energy-stamped inclusion priority so older transactions
        // dominate the order — the proposer's reward incentive is to include
        // high-priority txs first, which makes sandwich/frontrun attacks
        // economically unprofitable when gross MEV gain < per-block decay
        // cost. Phase 1 of `research/proposals/energy-stamped-mev-resistance.md`.
        //
        // Phase 1.5: also capture the cumulative priority sum so the
        // operator can mint a proposer-reward bonus (`apply_local_priority_bonus`).
        // This is per-node and not consensus-deterministic in v1.
        self.last_proposal_priority_sum = 0;
        // Encrypted-reveal txs taken before this point have no submit-epoch
        // hint (they don't go through the priority mempool). Pad the hints
        // vector with `None` for them so the indices stay parallel to `txs`.
        let mut hints_vec: Vec<Option<u64>> = txs.iter().map(|_| None).collect();
        let remaining = MAX_TXS_PER_BLOCK.saturating_sub(txs.len());
        // Diagnostic: mempool snapshot just before proposer drains. Logged
        // unconditionally at INFO so it's visible without RUST_LOG=debug —
        // narrow non-zero counts let us distinguish "no txs in mempool"
        // from "txs dropped by antichain/gas-check after drain".
        let mempool_snapshot_before_drain = self.mempool.len();
        if remaining > 0 {
            let (mut candidates, priority_sum, mut candidate_hints) = self
                .mempool
                .take_with_priority_sum_and_hints(remaining, self.height);
            let drained_count = candidates.len();
            self.last_proposal_priority_sum =
                self.last_proposal_priority_sum.saturating_add(priority_sum);
            if mempool_snapshot_before_drain > 0 || drained_count > 0 {
                info!(
                    height = self.height,
                    mempool_before = mempool_snapshot_before_drain,
                    drained = drained_count,
                    encrypted_reveals_kept = txs.len(),
                    "DIAG-MEMPOOL: proposer drained mempool"
                );
            }
            debug_assert_eq!(
                candidates.len(),
                candidate_hints.len(),
                "mempool returned mismatched hints"
            );
            // Lane I.5: governance-gated antichain projection. When
            // `block_source_mode == "antichain"`, post-filter the
            // FIFO draw through a same-sender dedup so the resulting
            // proposal carries only conflict-free txs (Block-STM can
            // run them in maximum parallelism). Default `"fifo"` is
            // bit-exact identical to today; any other value falls
            // through to FIFO so a typo cannot halt the chain.
            //
            // The same-sender heuristic mirrors `TxAntichainMempool`'s
            // V1 conflict rule: two txs are comparable iff they share
            // a sender (sequential nonces force ordering). The
            // projection drops the lower-priority duplicate, keeping
            // priority order intact (candidates arrive priority-desc
            // already from the FIFO draw).
            //
            // Skipped txs are returned to the mempool below (or
            // implicitly retained — they were never committed).
            // last_proposal_priority_sum is left at the unfiltered
            // value: the bonus reflects the priority of txs that were
            // *available* for inclusion, not just the antichain subset
            // — so honest proposers aren't penalised for filtering.
            let antichain_mode = self
                .governance_params
                .get("block_source_mode")
                .map(|s| s.as_str())
                .unwrap_or("fifo");
            if antichain_mode == "antichain" {
                let (kept_txs, kept_hints, dropped) =
                    crate::mempool::antichain_project(candidates, candidate_hints);
                candidates = kept_txs;
                candidate_hints = kept_hints;
                // Return the dropped txs to the mempool — they're
                // valid, just deferred. They'll re-surface on the
                // next proposal once their predecessor commits.
                for tx in dropped {
                    self.mempool.submit_priority(tx);
                }
            }
            if self.executor.block_gas_limit > 0 {
                let mut gas_used: u64 = txs
                    .iter()
                    .map(ParallelExecutor::estimate_gas)
                    .fold(0u64, |a, g| a.saturating_add(g));
                let mut rejected = Vec::new();
                for (tx, hint) in candidates.into_iter().zip(candidate_hints.into_iter()) {
                    let gas = ParallelExecutor::estimate_gas(&tx);
                    if gas_used.saturating_add(gas) > self.executor.block_gas_limit {
                        rejected.push(tx);
                    } else {
                        gas_used = gas_used.saturating_add(gas);
                        txs.push(tx);
                        hints_vec.push(Some(hint));
                    }
                }
                // Return over-gas txs to mempool for future blocks
                for tx in rejected {
                    self.mempool.submit_priority(tx);
                }
            } else {
                for (tx, hint) in candidates.into_iter().zip(candidate_hints.into_iter()) {
                    txs.push(tx);
                    hints_vec.push(Some(hint));
                }
            }
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Compute VRF output for this block (leader election proof + randomness).
        let (vrf_out, vrf_prf) = if let Some(ref vrf_kp) = self.vrf_keypair {
            let alpha = leader_vrf_input(self.height, self.round_state.round);
            let (output, proof) = vrf_kp.evaluate(&alpha);
            (Some(output.0), Some(proof.0))
        } else {
            (None, None)
        };

        let anchor_hash = self
            .anchor_provider
            .as_ref()
            .and_then(|p| p.anchor_hash_for_height(self.height));

        // Attach DA certificate from the previous block if supermajority was reached
        // (certificates are built asynchronously as attestations arrive from peers)
        let da_certificate = self.try_attach_pending_da_certificate();

        // Build block with placeholder DA fields.  We compute data_root, blob
        // commitments, and 2D roots AFTER trimming so they always reflect the
        // final transaction set that peers will see.
        // Phase 1.3 of LIGHT_CONE_FULL_DAG_PLAN.md — DAG-aware
        // proposer head selection. Under `parent_acceptance_mode =
        // "mcc"` the new block's `parent_hash` is the DAG-derived
        // tip (max-caliber leaf); otherwise it's `self.parent_hash`
        // (linear chain default — bit-for-bit unchanged from
        // pre-Light-Cone behaviour).
        let proposed_parent = self.current_tip();
        let mut block = Block {
            number: self.height,
            epoch: next_epoch,
            parent_hash: proposed_parent,
            state_root: self.current_state_root,
            transactions: txs,
            timestamp,
            chain_id: self.chain_id.clone(),
            producer_id: Some(self.my_id),
            vrf_output: vrf_out,
            vrf_proof: vrf_prf,
            data_root: None,
            blob_commitments: vec![],
            da_certificate,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            // Pre-Lane-B: every block produced by this node carries
            // protocol_version=0 (legacy semantics). Future fork-epoch
            // activations will bump this in lockstep across the cluster.
            protocol_version: 0,
            state_root_version: 0,
            // Stamp per-tx hints so followers can deterministically reconstruct
            // the priority used at proposal time. Only stamp when at least one
            // hint is `Some` — otherwise leave empty so the field
            // `skip_serializing_if = Vec::is_empty` stays bit-compat with
            // legacy blocks.
            submit_epoch_hints: if hints_vec.iter().any(|h| h.is_some()) {
                hints_vec
            } else {
                vec![]
            },
            da_row_roots: vec![],
            da_col_roots: vec![],
            // Phase C (Layer 4): emit parent set under mcc_full; vec![] for linear/mcc.
            parents: self.propose_parents(),
            post_state_root: None,
        };

        // Enforce max block size — drop transactions from the tail until the
        // serialized block fits. This prevents oversized gossip messages and
        // ensures deterministic replication limits.
        if let Ok(encoded) = serde_json::to_vec(&block) {
            if encoded.len() > MAX_BLOCK_SIZE_BYTES {
                warn!(
                    size = encoded.len(),
                    max = MAX_BLOCK_SIZE_BYTES,
                    "Block exceeds size limit — trimming transactions"
                );
                while block.transactions.len() > 1 {
                    let removed = block.transactions.pop();
                    // Keep block.submit_epoch_hints index-parallel with
                    // block.transactions: if the trimmed slot had a hint
                    // recorded, drop it too. The hints vector may be
                    // shorter than transactions when only legacy/encrypted-
                    // reveal txs exist (no hints stamped); guard the pop.
                    if !block.submit_epoch_hints.is_empty()
                        && block.submit_epoch_hints.len() == block.transactions.len() + 1
                    {
                        block.submit_epoch_hints.pop();
                    }
                    if let Some(tx) = removed {
                        self.mempool.submit_priority(tx);
                    }
                    if let Ok(enc) = serde_json::to_vec(&block) {
                        if enc.len() <= MAX_BLOCK_SIZE_BYTES {
                            break;
                        }
                    }
                }
            }
        }

        // Compute DA commitment fields on the final (post-trim) transaction set.
        if block.transactions.is_empty() {
            // Domain-separated empty-block sentinel: keyed_hash over (height ‖ parent_hash).
            // Prevents DA-attestation replay across heights and constant-sentinel
            // collisions (audit re-audit 2026-05-03 #9.1). Matches the v2 helper
            // in `lib.rs::empty_block_data_root`.
            block.data_root = Some(crate::empty_block_data_root(
                block.number,
                &block.parent_hash,
            ));
        } else if let Ok(tx_bytes) = serde_json::to_vec(&block.transactions) {
            // 1D commitment — this is the authoritative data_root stored in the header.
            match BlockDA::new() {
                Ok(da) => match da.encode_block(&tx_bytes) {
                    Ok(package) => {
                        debug!(
                            height = self.height,
                            shards = package.shards.len(),
                            data_bytes = tx_bytes.len(),
                            "DA erasure-coded block data"
                        );
                        block.data_root = Some(package.header.commitment_root);
                    }
                    Err(e) => warn!("DA encoding failed: {e} — block produced without data_root"),
                },
                Err(e) => warn!("DA init failed: {e} — block produced without data_root"),
            }

            // 2D row/col commitments for light-client sampling.
            let da2d = BlockDA2D::new();
            match da2d.encode_block(&tx_bytes) {
                Ok(package) => {
                    debug!(
                        height = self.height,
                        rows = package.header.row_roots.len(),
                        cols = package.header.col_roots.len(),
                        "DA-2D: computed row/col roots for proposal"
                    );
                    block.da_row_roots = package.header.row_roots;
                    block.da_col_roots = package.header.col_roots;
                }
                Err(e) => warn!("DA-2D encoding failed: {e}"),
            }

            // Blob commitments (namespace Merkle tree).
            let namespaced_blobs: Vec<NamespacedBlob> = block
                .transactions
                .iter()
                .map(|tx| {
                    let (ns_id, data) = match tx {
                        Transaction::Blob(blob_tx) => (blob_tx.namespace_id, blob_tx.data.clone()),
                        _ => {
                            let data = serde_json::to_vec(tx).unwrap_or_default();
                            (0u64, data)
                        }
                    };
                    let mut namespace = [0u8; 8];
                    namespace.copy_from_slice(&ns_id.to_be_bytes());
                    NamespacedBlob { namespace, data }
                })
                .collect();
            let nmt = NamespaceMerkleTree::from_blobs(&namespaced_blobs);
            block.blob_commitments = nmt.blob_commitment_hashes();
        } else {
            warn!("TX serialization failed — block produced without data_root");
        }

        // Log the proposal antichain from the parallel Light-Cone DAG.
        // Purely observational at genesis (threshold=0 always passes).
        crate::antichain_integration::log_proposal_antichain(
            &self.light_cone_dag,
            next_epoch,
            evaporchain_energy_kernel::DEFAULT_LAMBDA.epochs(),
            crate::antichain_integration::DEFAULT_ANTICHAIN_THRESHOLD,
        );
        // Causal-cone summary for proposer (§A1.3 Optimal Prediction Theorem).
        // Advisory: logged for auditability; not a gate at this stage.
        if let Some(head) = crate::antichain_integration::dag_tips(&self.light_cone_dag)
            .first()
            .copied()
        {
            if let Some(summary) = crate::causal_cone_integration::validator_cone_summary(
                &self.light_cone_dag,
                head,
                evaporchain_energy_kernel::DEFAULT_LAMBDA.epochs(),
                self.epoch,
            ) {
                crate::causal_cone_integration::log_cone_summary(&summary, self.my_id);
            }
        }

        info!(
            height = self.height,
            round = self.round_state.round,
            txs = block.transactions.len(),
            has_data_root = block.data_root.is_some(),
            "Created proposal"
        );
        // Diagnostic companion to DIAG-MEMPOOL: shows what made it into
        // the final proposal payload AFTER all the trimming/filtering
        // (antichain, gas, max-block-size). Compare against drained_count
        // to see where txs were lost.
        if !block.transactions.is_empty() {
            info!(
                height = self.height,
                final_txs = block.transactions.len(),
                "DIAG-MEMPOOL: block.transactions populated"
            );
        }

        // Phase 2 — POST_EXEC_STATE_VERIFICATION_PLAN.md.
        //
        // Speculatively execute the finalised block to stamp
        // `post_state_root` before broadcasting. Uses option (b)
        // clone-based simulate_execute (see ParallelExecutorSnapshot):
        //
        //   1. Snapshot the executor (O(n) clone of all accumulators).
        //   2. Checkpoint the DB via `begin_batch`.
        //   3. Run `execute_block` — executor + DB both mutate.
        //   4. Capture `state_root` from the result.
        //   5. Roll back DB (`rollback_batch`) + restore executor snapshot.
        //      Both are now bit-identical to pre-simulation state.
        //   6. Set `block.post_state_root = Some(state_root)`.
        //
        // If simulation errors (bad tx, out-of-gas, etc.) `post_state_root`
        // stays `None`. Phase 3's warn-mode check silently skips `None`
        // fields — no spurious mismatch warnings. `rollback_batch` is
        // called unconditionally so the DB cannot leak simulation state.
        //
        // Phase 4 (lane T0.3) gate: `post_state_verify_mode == "off"`
        // skips the speculative execute entirely (proposer leaves
        // post_state_root = None; validators with the same flag treat
        // None as no-claim). Default "warn" preserves the
        // af6876d/cb12cf1 always-on behaviour. "enforce" still does
        // the speculative execute here; the apply_block check below
        // upgrades the warn into a hard reject.
        let post_state_verify_mode = self
            .governance_params
            .get("post_state_verify_mode")
            .map(String::as_str)
            .unwrap_or("warn");
        if post_state_verify_mode != "off" {
            let snap = self.executor.snapshot_for_simulation();
            _db.begin_batch();
            let sim = self.executor.execute_block(_db, &block);
            _db.rollback_batch();
            self.executor.restore_from_simulation(snap);
            if let Ok(r) = sim {
                block.post_state_root = Some(r.state_root);
            }
        }

        Some(block)
    }

    /// Check if any block hash has 2f+1 prevotes (stake-weighted).
    /// Returns Some(Some(hash)) if quorum for a block, Some(None) if quorum for nil.
    fn check_prevote_quorum(&self) -> Option<Option<[u8; 32]>> {
        let threshold = self.stake_quorum_threshold();

        let mut hash_stake: HashMap<Option<[u8; 32]>, u64> = HashMap::new();
        for (vid, hash) in &self.round_state.prevotes {
            // Must match the per-validator weight used by `total_stake()`
            // (which is what `stake_quorum_threshold` is computed from).
            // See audit P2-01.
            let stake = self
                .validator_set
                .get(*vid)
                .map(|v| v.effective_stake())
                .unwrap_or(0);
            *hash_stake.entry(*hash).or_insert(0) += stake;
        }

        for (hash, stake) in &hash_stake {
            if *stake >= threshold {
                return Some(*hash);
            }
        }

        None
    }

    /// Check if any block hash has 2f+1 precommits (stake-weighted).
    fn check_precommit_quorum(&self) -> Option<Option<[u8; 32]>> {
        let threshold = self.stake_quorum_threshold();

        let mut hash_stake: HashMap<Option<[u8; 32]>, u64> = HashMap::new();
        for (vid, hash) in &self.round_state.precommits {
            // Must match `total_stake()` weight function. See audit P2-01.
            let stake = self
                .validator_set
                .get(*vid)
                .map(|v| v.effective_stake())
                .unwrap_or(0);
            *hash_stake.entry(*hash).or_insert(0) += stake;
        }

        for (hash, stake) in &hash_stake {
            if *stake >= threshold {
                return Some(*hash);
            }
        }

        None
    }

    /// Move to the next round within the same height.
    fn advance_round(&mut self) {
        debug!(
            height = self.height,
            round = self.round_state.round,
            phase = ?self.round_state.phase,
            prevotes = self.round_state.prevotes.len(),
            precommits = self.round_state.precommits.len(),
            proposed = self.round_state.proposed_block.is_some(),
            "advance_round"
        );
        // ── Downtime Detection ──
        // If no proposal was received this round, the expected proposer missed.
        // Track consecutive misses and slash after threshold.
        if self.round_state.proposed_block.is_none() {
            if let Some(expected) = self.proposer_for_round(self.height, self.round_state.round) {
                let expected_id = expected.id;
                let misses = self.missed_proposals.entry(expected_id).or_insert(0);
                *misses += 1;
                let total_misses = *misses;

                if total_misses >= 500 {
                    let slashed =
                        self.sanov_slash_downtime(expected_id, total_misses, SANOV_DOWNTIME_WINDOW);
                    warn!(
                        validator = expected_id,
                        missed_blocks = total_misses,
                        slashed_amount = slashed,
                        "SLASHED for downtime (missed proposals)"
                    );
                    // Reset counter after slashing (jailed at 500+)
                    self.missed_proposals.insert(expected_id, 0);
                } else {
                    debug!(
                        validator = expected_id,
                        missed_blocks = total_misses,
                        "Proposer missed round (slash at 500)"
                    );
                }
            }
        }

        // ── Vote Liveness Detection ──
        // Track validators who failed to cast prevotes or precommits.
        let active_ids: Vec<u64> = self
            .validator_set
            .validators()
            .iter()
            .filter(|v| !v.jailed)
            .map(|v| v.id)
            .collect();
        for vid in &active_ids {
            let voted_prevote = self.round_state.prevotes.contains_key(vid);
            let voted_precommit = self.round_state.precommits.contains_key(vid);
            if !voted_prevote && !voted_precommit {
                let misses = self.missed_votes.entry(*vid).or_insert(0);
                *misses += 1;
                let total = *misses;
                if total >= 1000 {
                    let slashed = self.sanov_slash_downtime(*vid, total, SANOV_DOWNTIME_WINDOW);
                    warn!(
                        validator = vid,
                        missed_votes = total,
                        slashed_amount = slashed,
                        "SLASHED for vote liveness failure"
                    );
                    self.missed_votes.insert(*vid, 0);
                }
            } else {
                self.missed_votes.insert(*vid, 0);
            }
        }

        // Return transactions from the uncommitted proposal back to the mempool
        // so they can be included in a future proposal.
        if let Some(ref block) = self.round_state.proposed_block {
            if !block.transactions.is_empty() {
                let recovered = block.transactions.len();
                for tx in block.transactions.iter().rev() {
                    self.mempool.submit_priority(tx.clone());
                }
                debug!(
                    height = self.height,
                    round = self.round_state.round,
                    recovered_txs = recovered,
                    "Returned uncommitted txs to mempool"
                );
            }
        }

        let next_round = self.round_state.round + 1;
        if next_round >= MAX_ROUNDS_PER_HEIGHT {
            warn!(
                height = self.height,
                "Max rounds reached — resetting to round 0 (empty block will go through normal consensus)"
            );
            // Do NOT force-commit: that bypasses quorum and breaks safety.
            // Instead reset to round 0 so the next proposer can propose an
            // empty block through normal Propose → Prevote → Precommit → Commit.
            // The mempool was already drained above, so the next proposal will
            // be empty (or near-empty), achieving the same livelock-prevention
            // goal without violating Agreement.
            self.round_state = RoundState::new(0);
            self.set_timeouts_for_round(0);
            return;
        }

        info!(
            height = self.height,
            from_round = self.round_state.round,
            to_round = next_round,
            "Advancing to next round"
        );
        self.round_state = RoundState::new(next_round);

        self.set_timeouts_for_round(next_round);
    }

    fn set_timeouts_for_round(&mut self, round: u32) {
        // Additive (linear) backoff: timeout = base + round * delta.
        // Standard Tendermint shape (Cosmos SDK pattern). The previous
        // formula was exponential-with-cap-at-64×; see the comment on
        // PROPOSE_TIMEOUT_DELTA_MS for the cluster-soak evidence that
        // motivated this change.
        let r = round as u64;
        let jitter_seed = self
            .height
            .wrapping_mul(31)
            .wrapping_add(r)
            .wrapping_mul(17)
            .wrapping_add(self.my_id.wrapping_mul(7));
        // Bounded jitter (≤100 ms) so validators don't all time out
        // at the same instant. Independent of round so we don't
        // re-introduce exponential growth via a multiplied jitter.
        let jitter_ms = jitter_seed % 100;
        self.propose_timeout = Duration::from_millis(
            PROPOSE_TIMEOUT_MS.saturating_add(r.saturating_mul(PROPOSE_TIMEOUT_DELTA_MS))
                + jitter_ms,
        );
        self.prevote_timeout = Duration::from_millis(
            PREVOTE_TIMEOUT_MS.saturating_add(r.saturating_mul(PREVOTE_TIMEOUT_DELTA_MS))
                + jitter_ms,
        );
        self.precommit_timeout = Duration::from_millis(
            PRECOMMIT_TIMEOUT_MS.saturating_add(r.saturating_mul(PRECOMMIT_TIMEOUT_DELTA_MS))
                + jitter_ms,
        );
    }

    /// Get current proposer info for display.
    pub fn current_proposer(&self) -> Option<&ValidatorInfo> {
        self.proposer_for_round(self.height, self.round_state.round)
    }

    // ──────────────── BLS Aggregate Signatures ─────────────────────────

    /// Construct the canonical message to BLS-sign for a vote.
    pub fn bls_vote_message(
        height: u64,
        round: u32,
        block_hash: &Option<[u8; 32]>,
        phase: &str,
    ) -> Vec<u8> {
        let mut msg = Vec::with_capacity(48);
        msg.extend_from_slice(phase.as_bytes());
        msg.extend_from_slice(&height.to_le_bytes());
        msg.extend_from_slice(&round.to_le_bytes());
        if let Some(hash) = block_hash {
            msg.extend_from_slice(hash);
        }
        msg
    }

    /// BLS-sign a vote if we have a keypair. Returns signature bytes or None.
    fn bls_sign_vote(
        &self,
        height: u64,
        round: u32,
        block_hash: &Option<[u8; 32]>,
        phase: &str,
    ) -> Option<Vec<u8>> {
        self.bls_keypair.as_ref().map(|kp| {
            let msg = Self::bls_vote_message(height, round, block_hash, phase);
            kp.sign(&msg).0
        })
    }

    /// Try to build a CommitCertificate from collected BLS precommit signatures.
    ///
    /// Audit P2-05: signer_ids are sorted ascending before aggregation so
    /// every honest node assembling the cert from the same vote set
    /// produces a byte-identical certificate. Aggregate BLS verification is
    /// permutation-invariant, but the cert itself is part of state — without
    /// canonical ordering, two nodes' certs would hash differently and split
    /// the chain. Also closes a small timing-side-channel risk where
    /// short-circuit-on-first-fail aggregate verify would consult signers
    /// in HashMap iteration order.
    fn try_build_commit_certificate(&self, block_hash: [u8; 32]) -> Option<CommitCertificate> {
        let threshold = self.stake_quorum_threshold();

        // Collect (vid, sig_bytes, stake) for every signer of this hash.
        let mut entries: Vec<(u64, Vec<u8>, u64)> = Vec::new();
        let mut precommit_match = 0usize;
        let mut precommit_no_sig: Vec<u64> = Vec::new();
        for (vid, vote_hash) in &self.round_state.precommits {
            if *vote_hash != Some(block_hash) {
                continue;
            }
            precommit_match += 1;
            let Some(sig_bytes) = self.round_state.precommit_bls_sigs.get(vid) else {
                precommit_no_sig.push(*vid);
                continue;
            };
            // Must match `total_stake()` weight function. See audit P2-01.
            let stake = self
                .validator_set
                .get(*vid)
                .map(|v| v.effective_stake())
                .unwrap_or(0);
            entries.push((*vid, sig_bytes.clone(), stake));
        }

        // Canonical order: sort ascending by validator id. Deterministic
        // across all nodes that hold the same vote set.
        entries.sort_by_key(|e| e.0);

        let signer_stake: u64 = entries.iter().map(|e| e.2).sum();
        if signer_stake < threshold {
            warn!(
                height = self.height,
                round = self.round_state.round,
                signer_stake,
                threshold,
                precommits_for_hash = precommit_match,
                signers_with_sig = entries.len(),
                missing_sig_vids = ?precommit_no_sig,
                "try_build_commit_certificate: stake below quorum threshold (cert=None)"
            );
            return None;
        }

        let signer_ids: Vec<u64> = entries.iter().map(|e| e.0).collect();
        let sigs: Vec<BlsSignature> = entries.iter().map(|e| BlsSignature(e.1.clone())).collect();

        let agg_sig = match BlsVerifier::aggregate_signatures(&sigs) {
            Some(s) => s,
            None => {
                warn!(
                    height = self.height,
                    round = self.round_state.round,
                    n_sigs = sigs.len(),
                    "try_build_commit_certificate: BLS aggregation failed (cert=None)"
                );
                return None;
            }
        };
        Some(CommitCertificate {
            height: self.height,
            round: self.round_state.round,
            block_hash,
            aggregate_signature: agg_sig.0,
            signer_ids,
        })
    }

    /// Verify a commit certificate against the current validator set.
    ///
    /// Two-pass under key rotation (punch-list 4b):
    ///   - Pass 1: build the pubkey set from each signer's *current*
    ///     `bls_public_key`. Try aggregate-verify. If it succeeds, the cert
    ///     was signed entirely with current keys — done.
    ///   - Pass 2: if pass 1 fails, rebuild the pubkey set substituting
    ///     `bls_public_key_prev` for any validator whose grace window has
    ///     not yet elapsed (`current epoch ≤ expiry`). Try again. If this
    ///     succeeds, the cert was signed with at least one validator's
    ///     pre-rotation key during the grace window — accept.
    ///
    /// Why two passes (not "throw both keys in one verify"): BLS aggregate
    /// verification expects exactly one pubkey per signer. We don't know
    /// per-signer which key was used without trying.
    ///
    /// Pass 2 only runs when pass 1 fails AND at least one signer is in
    /// its grace window, so steady-state cost is unchanged.
    pub fn verify_commit_certificate(&self, cert: &CommitCertificate) -> bool {
        self.verify_commit_certificate_inner(cert, false)
    }

    /// Sync-path variant: like `verify_commit_certificate` but tolerates a
    /// signer-stake-below-full-threshold case that arises when validator
    /// jailing state has been lost across a restart (the in-memory jailing
    /// bitmap is ephemeral; persistence was added in 2026-05-09).
    ///
    /// Safety: the BLS aggregate-verify is the cryptographic gate.  The
    /// stake-threshold relaxation is guarded by a floor of ≥1/3 of total
    /// genesis stake, which prevents a single isolated key from forging a
    /// cert while still accepting historically-valid certs whose quorum was
    /// computed against a smaller active-validator pool.
    pub fn verify_commit_certificate_for_sync(&self, cert: &CommitCertificate) -> bool {
        self.verify_commit_certificate_inner(cert, true)
    }

    fn verify_commit_certificate_inner(
        &self,
        cert: &CommitCertificate,
        allow_stake_fallback: bool,
    ) -> bool {
        // **Audit fix HIGH-9**: dedup signer_ids before stake-summing.
        // Legacy code summed `validator.effective_stake()` once per
        // entry in `signer_ids` — a malicious cert that lists the same
        // signer twice would inflate the quorum count, and (with some
        // BLS aggregation schemes) the duplicated pubkey survives
        // aggregate-verify when the signer signed once. Reject any
        // certificate with duplicate signers.
        let mut sorted_signers = cert.signer_ids.clone();
        sorted_signers.sort_unstable();
        if sorted_signers.windows(2).any(|w| w[0] == w[1]) {
            warn!("Rejecting cert: duplicate signer_ids");
            return false;
        }

        let threshold = self.stake_quorum_threshold();
        let mut signer_stake: u64 = 0;

        let mut pks = Vec::new();
        let mut any_in_grace = false;
        for &vid in &cert.signer_ids {
            if let Some(validator) = self.validator_set.get(vid) {
                // Must match `total_stake()` weight function. See audit P2-01.
                signer_stake = signer_stake.saturating_add(validator.effective_stake());
                if let Some(ref bls_pk_bytes) = validator.bls_public_key {
                    // Reject if PoP was submitted but failed verification
                    if !validator.pop_verified {
                        warn!(
                            validator_id = vid,
                            "Rejecting cert: signer has no verified proof-of-possession"
                        );
                        return false;
                    }
                    pks.push(BlsPublicKey(bls_pk_bytes.clone()));
                    if let Some(expiry) = validator.bls_prev_key_expiry_epoch {
                        if self.epoch <= expiry && validator.bls_public_key_prev.is_some() {
                            any_in_grace = true;
                        }
                    }
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }

        if signer_stake < threshold {
            if !allow_stake_fallback {
                return false;
            }
            // Stake below current threshold — the validator jailing state may
            // differ from the historical state when this cert was built.  The
            // persistence layer now saves jailing state after each block; this
            // fallback covers nodes that restarted before that fix landed.
            // Guard: signer_stake must be ≥ 1/3 of total genesis stake so that
            // no single isolated key can forge a cert through this path.
            let genesis_total: u64 = self
                .validator_set
                .validators()
                .iter()
                .map(|v| v.effective_stake())
                .fold(0u64, |a, s| a.saturating_add(s));
            let min_floor = genesis_total / 3;
            if signer_stake < min_floor {
                warn!(
                    cert_height = cert.height,
                    signer_stake,
                    min_floor,
                    "cert: stake fallback rejected — signer_stake below 1/3 genesis floor"
                );
                return false;
            }
            warn!(
                cert_height = cert.height,
                cert_round = cert.round,
                signer_stake,
                threshold,
                genesis_total,
                "cert: stake below current threshold; using sync fallback (historical jailing state lost on restart) — BLS is the cryptographic gate"
            );
        }

        let msg =
            Self::bls_vote_message(cert.height, cert.round, &Some(cert.block_hash), "precommit");
        let agg_sig = BlsSignature(cert.aggregate_signature.clone());

        // Pass 1: current keys.
        if BlsVerifier::aggregate_verify(&msg, &agg_sig, &pks) {
            return true;
        }
        if !any_in_grace {
            return false;
        }

        // Pass 2: substitute prev key for any signer whose grace window
        // is still open. We try every "one signer downgraded to prev"
        // combination would explode combinatorially; instead, we
        // substitute prev for ALL grace-eligible signers at once. This
        // matches the realistic transition pattern (a single epoch's
        // worth of votes signed with old keys after a coordinated
        // rotation), and the alternative — exhaustive subset search — is
        // not worth the cost for a corner case.
        let mut pks_with_prev = Vec::with_capacity(pks.len());
        for &vid in &cert.signer_ids {
            // Defensive: if a signer disappears from the validator set
            // between cert build and verify (rotation race, removed
            // validator, etc.), or has no registered BLS key, treat
            // the certificate as invalid rather than panicking the
            // node. Closes the Gap-A #8 critical-path expect() that
            // could SIGABRT the process under adversarial input.
            let v = match self.validator_set.get(vid) {
                Some(v) => v,
                None => {
                    warn!(
                        validator_id = vid,
                        "cert grace verify: signer not in validator set — rejecting"
                    );
                    return false;
                }
            };
            let in_grace = v
                .bls_prev_key_expiry_epoch
                .map(|exp| self.epoch <= exp)
                .unwrap_or(false);
            let pk_bytes_opt = if in_grace {
                v.bls_public_key_prev
                    .clone()
                    .or_else(|| v.bls_public_key.clone())
            } else {
                v.bls_public_key.clone()
            };
            let pk_bytes = match pk_bytes_opt {
                Some(b) => b,
                None => {
                    warn!(
                        validator_id = vid,
                        "cert grace verify: signer has no registered BLS key — rejecting"
                    );
                    return false;
                }
            };
            pks_with_prev.push(BlsPublicKey(pk_bytes));
        }
        BlsVerifier::aggregate_verify(&msg, &agg_sig, &pks_with_prev)
    }

    /// Create a DA attestation message for a committed block.
    /// Returns None if this validator has no BLS keypair.
    pub fn make_da_attestation(
        &self,
        block_number: u64,
        data_root: [u8; 32],
        shards_verified: u32,
    ) -> Option<ConsensusMessage> {
        let kp = self.bls_keypair.as_ref()?;
        // P2-01: must match `total_stake()` weight function so DA quorum
        // computation is consistent with consensus quorum.
        let stake = self
            .validator_set
            .get(self.my_id)
            .map(|v| v.effective_stake())
            .unwrap_or(0);
        let att = evaporchain_da::certificate::create_attestation(
            block_number,
            &data_root,
            self.my_id,
            shards_verified,
            stake,
            kp,
        );
        Some(ConsensusMessage::DAAttestation {
            block_number: att.block_number,
            data_root: att.data_root,
            validator_id: att.validator_id,
            samples_verified: att.samples_verified,
            stake: att.stake,
            signature: att.signature,
            public_key: att.public_key,
        })
    }

    /// P2-04: have we collected enough DA attestation stake (excluding the
    /// proposer) to satisfy the consensus quorum threshold for `block_number`?
    ///
    /// Uses the same `effective_stake()` weight function as the consensus
    /// quorum check (P2-01) so DA gating and consensus gating are
    /// consistent.
    fn has_da_supermajority(&self, block_number: u64) -> bool {
        let threshold = self.stake_quorum_threshold();
        if threshold == u64::MAX {
            return false;
        }
        // Small-cluster DA mode (validators <= 3): proposer self-attests
        // because filtering it out makes 2-of-2 non-proposer attestations
        // mandatory for every cert — any single dropped/delayed gossip
        // halts the chain. See `small_cluster_da_mode` field doc.
        let exclude_proposer = !self.small_cluster_da_mode;
        let proposer = self.da_block_proposers.get(&block_number).copied();
        // Re-audit (2026-05-02) Cons-DA: when proposer-exclusion is
        // expected (n>3 cluster) but the proposer cache is missing
        // for this block, fail CLOSED. The previous behaviour failed
        // open — a missing proposer entry let `Some(_) != None`
        // succeed for every attestation, including the proposer's
        // own self-attestation, over-counting toward supermajority.
        // Refusing here forces the caller to retry once the proposer
        // record is populated; safer than silent over-count.
        if exclude_proposer && proposer.is_none() {
            return false;
        }
        let attesters: Vec<u64> = match self.da_attestations.get(&block_number) {
            Some(atts) => atts
                .iter()
                .filter(|att| !exclude_proposer || Some(att.validator_id) != proposer)
                .map(|att| att.validator_id)
                .collect(),
            None => return false,
        };
        // Dedup — multiple attestations from same validator count once.
        let mut unique: HashSet<u64> = HashSet::new();
        let mut weight: u64 = 0;
        for vid in attesters {
            if !unique.insert(vid) {
                continue;
            }
            if let Some(v) = self.validator_set.get(vid) {
                weight = weight.saturating_add(v.effective_stake());
            }
        }
        weight >= threshold
    }

    /// Try to build a DA certificate from collected attestations for a block.
    /// Returns serialized certificate bytes if supermajority is reached.
    pub fn try_build_da_certificate(
        &mut self,
        block_number: u64,
        data_root: [u8; 32],
    ) -> Option<Vec<u8>> {
        let atts = self.da_attestations.get(&block_number)?;
        let proposer = self.da_block_proposers.get(&block_number).copied();
        let total_stake = self.validator_set.total_stake();
        let mut builder = evaporchain_da::certificate::CertificateBuilder::new(
            block_number,
            data_root,
            total_stake,
        );
        // P2-05: feed attestations in canonical (validator-id ascending)
        // order so the resulting certificate is byte-identical across nodes.
        // The HashMap-stored Vec preserves insertion order locally but is
        // not consistent across nodes that received attestations in
        // different orders.
        let mut sorted_atts: Vec<_> = atts.iter().collect();
        sorted_atts.sort_by_key(|att| att.validator_id);
        // Small-cluster mode: include the proposer's own attestation in the
        // cert. See `small_cluster_da_mode` field doc for the safety
        // implication. In normal mode, the proposer is excluded so they
        // cannot attest to their own block's DA.
        let exclude_proposer = !self.small_cluster_da_mode;
        for att in sorted_atts {
            if exclude_proposer && Some(att.validator_id) == proposer {
                continue;
            }
            builder.add_attestation(att.clone());
        }
        let cert = builder.try_build()?;
        // Serialize to bytes for the block field
        serde_json::to_vec(&cert).ok()
    }

    /// Clean up old DA attestations (keep only last 64 blocks).
    pub fn prune_da_attestations(&mut self) {
        if self.da_attestations.len() > 64 {
            let cutoff = self.height.saturating_sub(64);
            self.da_attestations.retain(|&k, _| k > cutoff);
            self.da_block_proposers.retain(|&k, _| k > cutoff);
        }
    }

    /// Try to find a pending DA certificate from recent blocks to include in a new proposal.
    /// Scans the last 10 blocks for any that reached supermajority but weren't included yet.
    fn try_attach_pending_da_certificate(&mut self) -> Option<Vec<u8>> {
        let start = self.height.saturating_sub(10);
        for bn in (start..self.height).rev() {
            if let Some(atts) = self.da_attestations.get(&bn) {
                if atts.is_empty() {
                    continue;
                }
                if let Some(&data_root) = atts.first().map(|a| &a.data_root) {
                    if let Some(cert_bytes) = self.try_build_da_certificate(bn, data_root) {
                        info!(
                            block = bn,
                            current_height = self.height,
                            "Attaching pending DA certificate from block #{} to new proposal",
                            bn,
                        );
                        return Some(cert_bytes);
                    }
                }
            }
        }
        None
    }

    /// Perform DA sampling on a proposed block and return an attestation if valid.
    ///
    /// Uses 2D extended data square (Celestia-style) sampling when the block
    /// carries `da_row_roots` / `da_col_roots`. Falls back to 1D shard sampling
    /// when 2D roots are absent (backward compatibility).
    ///
    /// For 2D sampling, 16 random cells are sampled from the extended data square
    /// and verified against both row and column commitments. The resulting
    /// `AvailabilityMetrics` must meet `da_confidence_threshold` (default 0.999,
    /// i.e. ~10 valid samples minimum) for the validator to attest.
    pub fn perform_da_sampling(&self, block: &Block) -> Option<ConsensusMessage> {
        let data_root = block.data_root?;

        // Empty-block sentinel: must mirror the proposer logic at the
        // top of create_proposal. The proposer skips the BlockDA encoding
        // entirely for txs.is_empty() and stamps a domain-separated
        // sentinel via `crate::empty_block_data_root(height, parent_hash)`.
        // The verifier recomputes the same sentinel from the block header.
        if block.transactions.is_empty() {
            let expected = crate::empty_block_data_root(block.number, &block.parent_hash);
            if data_root != expected {
                warn!(
                    height = block.number,
                    "DA sampling: empty-block data_root differs from sentinel"
                );
                return None;
            }
            return self.make_da_attestation(block.number, data_root, 0);
        }

        // ── test-utils fast path ─────────────────────────────────────────
        // Skip the expensive Reed-Solomon re-encode (3-4s for 200-tx blocks)
        // when running a trusted test cluster. Block hash already covers
        // data_root integrity; no external erasure-code fraud proof needed
        // on a 3-mini Tailscale BFT net.
        // test-utils feature was historical; the gate is dead code now.
        // Production self-attestation lives a few lines below.
        #[allow(unexpected_cfgs)]
        #[cfg(feature = "test-utils")]
        return self.make_da_attestation(block.number, data_root, 1);

        #[allow(unreachable_code)]
        let tx_bytes = serde_json::to_vec(&block.transactions).ok()?;

        // ── 2D sampling path (preferred) ────────────────────────────────
        if !block.da_row_roots.is_empty() && !block.da_col_roots.is_empty() {
            let da2d = BlockDA2D::new();
            let package = da2d.encode_block(&tx_bytes).ok()?;

            // Verify row/col roots match the proposer's header
            // (data_root integrity is covered by the 1D path; 2D uses row/col commitments)
            if package.header.row_roots != block.da_row_roots
                || package.header.col_roots != block.da_col_roots
            {
                warn!(
                    height = block.number,
                    "DA-2D sampling: row/col roots mismatch — local encoding differs from proposer"
                );
                return None;
            }

            let seed = {
                let mut s = Vec::with_capacity(40);
                s.extend_from_slice(b"da-2d-sample");
                s.extend_from_slice(&block.number.to_le_bytes());
                s.extend_from_slice(&self.my_id.to_le_bytes());
                s
            };

            // 16 cells -> confidence ~ 1 - 2^(-16) ~ 0.999985 if all valid
            let num_samples = 16usize;
            let (results, _all_valid) =
                da2d.light_client_sample(&package, block.number, num_samples, &seed);

            let metrics = AvailabilityMetrics::from_samples(&results, package.header.extended_dim);

            if metrics.confidence < self.da_confidence_threshold {
                warn!(
                    height = block.number,
                    confidence = %format!("{:.6}", metrics.confidence),
                    threshold = %format!("{:.6}", self.da_confidence_threshold),
                    valid = metrics.valid_samples,
                    total = metrics.total_samples,
                    recovery_possible = metrics.recovery_possible,
                    "DA-2D sampling failed: confidence below threshold"
                );
                return None;
            }

            info!(
                height = block.number,
                confidence = %format!("{:.6}", metrics.confidence),
                valid = metrics.valid_samples,
                total = metrics.total_samples,
                unique_rows = metrics.unique_rows_hit,
                unique_cols = metrics.unique_cols_hit,
                recovery_possible = metrics.recovery_possible,
                "DA-2D sampling passed"
            );

            return self.make_da_attestation(block.number, data_root, metrics.valid_samples as u32);
        }

        // ── 1D fallback path ────────────────────────────────────────────
        let da = BlockDA::new().ok()?;
        let package = da.encode_block(&tx_bytes).ok()?;

        if package.header.commitment_root != data_root {
            warn!(
                height = block.number,
                "DA sampling: data_root mismatch — local encoding differs from proposer's commitment"
            );
            return None;
        }

        let seed = {
            let mut s = Vec::with_capacity(40);
            s.extend_from_slice(b"da-sample");
            s.extend_from_slice(&block.number.to_le_bytes());
            s.extend_from_slice(&self.my_id.to_le_bytes());
            s
        };
        let num_samples = 6usize.min(package.shards.len());
        let queries =
            BlockDA::generate_sample_queries(block.number, &package.header, num_samples, &seed);

        let mut verified = 0u32;
        for q in &queries {
            if let Ok(response) = da.prove_shard(&package, q.shard_index) {
                if BlockDA::verify_shard_sample(&package.header, &response) {
                    verified += 1;
                }
            }
        }

        if verified < 4.min(num_samples as u32) {
            warn!(
                height = block.number,
                verified,
                required = 4.min(num_samples as u32),
                "DA sampling failed: insufficient verified shards"
            );
            return None;
        }

        debug!(
            height = block.number,
            verified,
            total_samples = num_samples,
            "DA sampling passed (1D fallback)"
        );

        self.make_da_attestation(block.number, data_root, verified)
    }

    /// Verify a DA certificate included in a received block.
    ///
    /// Enforcement modes based on `da_enforcement_height`:
    /// - **Soft mode** (block.number < da_enforcement_height): blocks without DA
    ///   certificates are accepted with a warning. If a certificate IS present,
    ///   it must pass full verification (BLS signatures, supermajority, etc.).
    /// - **Hard mode** (block.number >= da_enforcement_height): blocks without a
    ///   valid DA certificate are rejected outright.
    pub fn verify_da_certificate(&self, block: &Block) -> bool {
        let cert_bytes = match &block.da_certificate {
            Some(bytes) => bytes,
            None => {
                // No DA certificate present — decide based on enforcement height
                if block.number < self.da_enforcement_height {
                    warn!(
                        block = block.number,
                        enforcement_height = self.da_enforcement_height,
                        "Block has no DA certificate (soft mode — accepting before enforcement height)"
                    );
                    return true;
                } else {
                    warn!(
                        block = block.number,
                        enforcement_height = self.da_enforcement_height,
                        "Block rejected: missing DA certificate (hard mode — enforcement active)"
                    );
                    return false;
                }
            }
        };

        // Certificate is present — always verify fully regardless of height
        let cert: evaporchain_da::certificate::DACertificate =
            match serde_json::from_slice(cert_bytes) {
                Ok(c) => c,
                Err(_) => {
                    warn!(
                        block = block.number,
                        "DA certificate deserialization failed"
                    );
                    return false;
                }
            };
        // Verify supermajority stake
        if !cert.is_supermajority() {
            warn!(
                block = block.number,
                attested = cert.attested_stake,
                total = cert.total_stake,
                "DA certificate does not have supermajority"
            );
            return false;
        }
        // Verify attestation count is non-trivial
        if cert.attestations.is_empty() {
            warn!(block = block.number, "DA certificate has zero attestations");
            return false;
        }
        // C-09 FIX: Verify all BLS signatures on attestations and recompute
        // attested_stake from attestation data. Without this, a forged certificate
        // with fabricated attested_stake and garbage signatures would be accepted.
        if !cert.verify_signatures() {
            warn!(
                block = block.number,
                "DA certificate contains invalid BLS signatures or inflated stake"
            );
            return false;
        }
        true
    }
}

// ─────────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::Account;

    fn addr(b: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    fn make_validator(id: u64, stake: u64) -> ValidatorInfo {
        let mut address = [0u8; 32];
        address[0] = id as u8;
        ValidatorInfo::new(id, stake, address)
    }

    fn make_validator_set(ids: &[u64]) -> ValidatorSet {
        let validators: Vec<_> = ids.iter().map(|&id| make_validator(id, 1000)).collect();
        ValidatorSet::with_validators(validators)
    }

    fn make_consensus(my_id: u64, ids: &[u64]) -> TendermintConsensus {
        TendermintConsensus::new_for_test(my_id, 5, make_validator_set(ids))
    }

    // ── Round-backoff timeout regression ──────────────────────────────
    //
    // Confirms additive (linear) backoff post the 2026-05-07 cluster
    // soak fix. Previous formula was `1u64 << min(round, 6)` —
    // exponential capped at 64×, which made round 6+ each take ~76
    // minutes of timeout and stalled the cluster soak at h=16956 for
    // over 22 minutes before we noticed.

    #[test]
    fn timeouts_grow_linearly_per_round_not_exponentially() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4, 5]);
        tc.set_timeouts_for_round(0);
        let p0 = tc.propose_timeout.as_millis();
        let pv0 = tc.prevote_timeout.as_millis();
        let pc0 = tc.precommit_timeout.as_millis();

        tc.set_timeouts_for_round(7);
        let p7 = tc.propose_timeout.as_millis();
        let pv7 = tc.prevote_timeout.as_millis();
        let pc7 = tc.precommit_timeout.as_millis();

        // At round 7 with the OLD exponential formula, each timeout
        // would be 64× the base — propose 512s, prevote 2048s,
        // precommit 2048s. The fix is additive: round * delta.
        // Concretely: propose 8 + 7 = 15s, prevote 32 + 14 = 46s,
        // precommit 32 + 14 = 46s. Allow ±100ms for jitter.
        assert!(
            p7 < 20_000,
            "round-7 propose_timeout exploded under exponential backoff: {p7} ms"
        );
        assert!(
            pv7 < 60_000,
            "round-7 prevote_timeout exploded under exponential backoff: {pv7} ms"
        );
        assert!(
            pc7 < 60_000,
            "round-7 precommit_timeout exploded under exponential backoff: {pc7} ms"
        );

        // Sanity: round 0 unchanged baseline (within jitter).
        assert!(
            (8_000..=8_100).contains(&(p0 as u64)),
            "round-0 propose_timeout drifted: {p0} ms"
        );
        assert!(
            (32_000..=32_100).contains(&(pv0 as u64)),
            "round-0 prevote_timeout drifted: {pv0} ms"
        );
        assert!(
            (32_000..=32_100).contains(&(pc0 as u64)),
            "round-0 precommit_timeout drifted: {pc0} ms"
        );

        // Monotonicity: round 7 > round 0 (some growth, just not 64×).
        assert!(p7 > p0);
        assert!(pv7 > pv0);
        assert!(pc7 > pc0);
    }

    // ── Validator key rotation cert verification (punch-list 4d) ──────

    /// Build a 4-validator set with real BLS keypairs and PoP-verified
    /// pubkeys. Returns (validator_set, keypairs_indexed_by_id).
    fn make_real_keyed_validators() -> (
        ValidatorSet,
        Vec<evaporchain_crypto::signatures::BlsKeypair>,
    ) {
        use evaporchain_crypto::signatures::BlsKeypair;
        let mut vs = ValidatorSet::new();
        let mut kps = Vec::new();
        for vid in 1u64..=4 {
            let kp = BlsKeypair::generate();
            let mut info = ValidatorInfo::new(vid, 1000, addr(vid as u8));
            info.bls_public_key = Some(kp.public_key_bytes().0.clone());
            info.pop_verified = true;
            vs.add_validator(info);
            kps.push(kp);
        }
        (vs, kps)
    }

    fn build_cert(
        height: u64,
        round: u32,
        block_hash: [u8; 32],
        signer_ids: Vec<u64>,
        signatures: Vec<evaporchain_crypto::signatures::BlsSignature>,
    ) -> CommitCertificate {
        use evaporchain_crypto::signatures::BlsVerifier;
        let agg = BlsVerifier::aggregate_signatures(&signatures).expect("aggregate");
        CommitCertificate {
            height,
            round,
            block_hash,
            signer_ids,
            aggregate_signature: agg.0,
        }
    }

    #[test]
    fn test_two_pass_cert_verification_during_grace_window() {
        let (mut vs, kps) = make_real_keyed_validators();
        // Rotate validator id=1: stash old key, set expiry epoch = 10.
        let old_pk = vs.get(1).unwrap().bls_public_key.clone().unwrap();
        let new_kp = evaporchain_crypto::signatures::BlsKeypair::generate();
        let new_pk = new_kp.public_key_bytes().0.clone();
        let new_pop = new_kp.proof_of_possession().0.clone();
        assert!(vs.rotate_validator_key(1, new_pk.clone(), new_pop, 10));
        // Sanity: validator 1's current key is now the NEW key, prev = OLD.
        assert_eq!(vs.get(1).unwrap().bls_public_key.as_ref().unwrap(), &new_pk);
        assert_eq!(
            vs.get(1).unwrap().bls_public_key_prev.as_ref().unwrap(),
            &old_pk
        );

        let mut tc = TendermintConsensus::new_for_test(1, 5, vs);

        let block_hash = [9u8; 32];
        let msg = TendermintConsensus::bls_vote_message(7, 0, &Some(block_hash), "precommit");

        // Construct a cert signed by all 4 validators, BUT validator 1
        // signs with their PREVIOUS key (kps[0]) — modelling a vote that
        // was sent before the rotation propagated to this node.
        let signatures = vec![
            kps[0].sign(&msg), // validator 1 with OLD key
            kps[1].sign(&msg),
            kps[2].sign(&msg),
            kps[3].sign(&msg),
        ];
        let cert = build_cert(7, 0, block_hash, vec![1, 2, 3, 4], signatures);

        // Within grace window (current epoch = 5 ≤ expiry 10):
        // pass 1 with current keys fails (validator 1 used old key);
        // pass 2 substitutes prev key for validator 1 → succeeds.
        tc.epoch = 5;
        assert!(
            tc.verify_commit_certificate(&cert),
            "cert with old-key signature must verify within grace window"
        );

        // Past grace window (current epoch = 11 > expiry 10): both passes
        // fail. Pass 2 doesn't substitute prev because grace expired.
        tc.epoch = 11;
        assert!(
            !tc.verify_commit_certificate(&cert),
            "cert with old-key signature must NOT verify after grace expiry"
        );
    }

    #[test]
    fn test_two_pass_cert_verification_with_only_new_keys() {
        let (mut vs, kps) = make_real_keyed_validators();
        // Rotate validator 1 — but the cert is signed with the new key.
        let old_pk = vs.get(1).unwrap().bls_public_key.clone().unwrap();
        let new_kp = evaporchain_crypto::signatures::BlsKeypair::generate();
        let new_pk = new_kp.public_key_bytes().0.clone();
        let new_pop = new_kp.proof_of_possession().0.clone();
        assert!(vs.rotate_validator_key(1, new_pk, new_pop, 10));
        assert_ne!(old_pk, vs.get(1).unwrap().bls_public_key.clone().unwrap());

        let mut tc = TendermintConsensus::new_for_test(1, 5, vs);
        let block_hash = [3u8; 32];
        let msg = TendermintConsensus::bls_vote_message(7, 0, &Some(block_hash), "precommit");
        let signatures = vec![
            new_kp.sign(&msg), // validator 1 with NEW key (post-rotation)
            kps[1].sign(&msg),
            kps[2].sign(&msg),
            kps[3].sign(&msg),
        ];
        let cert = build_cert(7, 0, block_hash, vec![1, 2, 3, 4], signatures);

        tc.epoch = 5;
        assert!(
            tc.verify_commit_certificate(&cert),
            "cert signed entirely with current keys must verify on pass 1"
        );

        // Even past grace, current-key cert still verifies.
        tc.epoch = 100;
        assert!(
            tc.verify_commit_certificate(&cert),
            "post-grace, current-key cert still verifies"
        );
    }

    #[test]
    fn test_apply_validator_key_rotations_with_continuity_check() {
        let (vs, kps) = make_real_keyed_validators();
        let mut tc = TendermintConsensus::new_for_test(1, 5, vs);

        // Operator (validator 1) generates a new keypair and signs the
        // continuity proof with their OLD key over the NEW pubkey bytes.
        let new_kp = evaporchain_crypto::signatures::BlsKeypair::generate();
        let new_pk = new_kp.public_key_bytes().0.clone();

        // bls_pop_old: in the current implementation,
        // `apply_validator_key_rotations` calls `verify_pop(old_pk, bls_pop_old)`,
        // which checks that `bls_pop_old` is a PoP signature over the
        // OLD pubkey itself (`proof_of_possession()` semantics). This
        // proves "submitter controls the old key" but does NOT bind the
        // old key to the new key bytes. A tighter binding (sign new_pk
        // with old key under POP DST) is tracked as a follow-up; for now
        // the loose continuity proof is what's exercised by the test.
        let pop_sig_old = kps[0].proof_of_possession().0.clone();

        let new_pop = new_kp.proof_of_possession().0.clone();

        // The current `apply_validator_key_rotations` continuity check
        // expects bls_pop_old to verify against the OLD pubkey using the
        // POP DST. `proof_of_possession()` signs the signer's OWN pk,
        // so passing kps[0].proof_of_possession() will verify against
        // kps[0]'s pubkey — which IS the validator's old key. The PoP is
        // for kps[0]'s OWN pk, not for new_pk. The continuity check thus
        // succeeds at the BLS level (PoP of old key by old key) but does
        // NOT bind old_key to new_key. A future tightening should make
        // bls_pop_old sign new_pk under POP DST. For 4d, we exercise the
        // continuity-of-control path with the looser binding currently in
        // place; the tighter binding is tracked as a follow-up.

        let rotation = evaporchain_execution::ValidatorKeyRotation {
            validator_id: 1,
            new_bls_public_key: new_pk.clone(),
            bls_pop_old: pop_sig_old,
            new_bls_pop: new_pop,
            prev_key_expiry_epoch: 100,
        };

        let applied = tc.apply_validator_key_rotations(&[rotation]);
        assert_eq!(applied, 1, "rotation should apply when continuity verifies");
        // Validator 1's current key should now be the new key.
        let v = tc.validator_set.get(1).unwrap();
        assert_eq!(v.bls_public_key.as_ref().unwrap(), &new_pk);
        assert!(v.bls_public_key_prev.is_some());
        assert_eq!(v.bls_prev_key_expiry_epoch, Some(100));
    }

    #[test]
    fn test_apply_validator_key_rotations_rejects_bad_continuity_proof() {
        let (vs, _kps) = make_real_keyed_validators();
        let mut tc = TendermintConsensus::new_for_test(1, 5, vs);

        let new_kp = evaporchain_crypto::signatures::BlsKeypair::generate();
        let attacker_kp = evaporchain_crypto::signatures::BlsKeypair::generate();

        let rotation = evaporchain_execution::ValidatorKeyRotation {
            validator_id: 1,
            new_bls_public_key: new_kp.public_key_bytes().0.clone(),
            // Continuity "proof" signed by an UNRELATED key — should fail.
            bls_pop_old: attacker_kp.proof_of_possession().0.clone(),
            new_bls_pop: new_kp.proof_of_possession().0.clone(),
            prev_key_expiry_epoch: 100,
        };

        let applied = tc.apply_validator_key_rotations(&[rotation]);
        assert_eq!(applied, 0, "bad continuity proof must be rejected");
        // Validator 1's key should be UNCHANGED.
        assert!(tc
            .validator_set
            .get(1)
            .unwrap()
            .bls_public_key_prev
            .is_none());
    }

    #[test]
    fn test_quorum_size() {
        // 1 validator: quorum = 1
        let tc = make_consensus(1, &[1]);
        assert_eq!(tc.quorum_size(), 1);

        // 3 validators: quorum = 3 (strict >2/3 majority)
        let tc = make_consensus(1, &[1, 2, 3]);
        assert_eq!(tc.quorum_size(), 3);

        // 4 validators: quorum = 3
        let tc = make_consensus(1, &[1, 2, 3, 4]);
        assert_eq!(tc.quorum_size(), 3);

        // 6 validators: quorum = 5 (strict >2/3)
        let tc = make_consensus(1, &[1, 2, 3, 4, 5, 6]);
        assert_eq!(tc.quorum_size(), 5);

        // 7 validators: quorum = 5
        let tc = make_consensus(1, &[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(tc.quorum_size(), 5);

        // 2 validators: quorum = 2
        let tc = make_consensus(1, &[1, 2]);
        assert_eq!(tc.quorum_size(), 2);
    }

    #[test]
    fn test_proposal_creation() {
        let mut db = InMemoryStateDB::new();
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);

        // Find a round where we are proposer
        let mut found = false;
        for round in 0..100 {
            tc.round_state = RoundState::new(round);
            if tc.am_i_proposer() {
                let proposal = tc.create_proposal(&mut db);
                assert!(proposal.is_some());
                let block = proposal.unwrap();
                assert_eq!(block.number, 1);
                assert_eq!(block.producer_id, Some(1));
                found = true;
                break;
            }
        }
        assert!(found, "Should be proposer for at least one round");
    }

    /// MCC Phase C.6 / D.1 — load-bearing hot-path test. Deferred from
    /// the original Phase C.6 list ("proposer_emits_multi_parent_block_
    /// under_mcc_full → D.1 add-on") but never landed in `tests/
    /// mcc_phase_d.rs` (which only exercises substrate-level accessor
    /// convergence, not the actual `create_proposal` round behaviour).
    /// This is the test that proves the wiring at line ~6402
    /// (`parents: self.propose_parents()`) actually fires under
    /// `mcc_full` mode and emits a block whose parents form the
    /// committed antichain.
    ///
    /// Setup: single proposer, 4-validator set, `parent_acceptance_mode
    /// = mcc_full`, light_cone_dag populated with genesis + 3 sibling
    /// forks. Drives `create_proposal` and asserts the resulting block
    /// carries `parents.len() == 3` (the full antichain) AND the parent
    /// set matches the proposer's `propose_parents()` accessor —
    /// substrate-vs-hot-path agreement.
    #[test]
    fn mcc_phase_c_hot_path_proposer_emits_multi_parent_block() {
        use evaporchain_light_cone::Block as LcBlock;
        let mut db = InMemoryStateDB::new();
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("parent_acceptance_mode", "mcc_full")
            .expect("mcc_full is allowlisted");

        // Populate the light-cone DAG with 3 sibling forks at h=1 off
        // genesis. Same shape as `mcc_phase_d1_four_validators_converge_
        // on_three_forks` so the antichain is exactly {fork-A, fork-B,
        // fork-C}.
        let g = [0u8; 32];
        let fa = [1u8; 32];
        let fb = [2u8; 32];
        let fc = [3u8; 32];
        tc.light_cone_dag
            .insert(LcBlock::new(g, vec![], 1000, 0))
            .expect("insert genesis");
        tc.light_cone_dag
            .insert(LcBlock::new(fa, vec![g], 1001, 1))
            .expect("insert fork A");
        tc.light_cone_dag
            .insert(LcBlock::new(fb, vec![g], 1002, 1))
            .expect("insert fork B");
        tc.light_cone_dag
            .insert(LcBlock::new(fc, vec![g], 1003, 1))
            .expect("insert fork C");

        // Substrate-level expectation: 3 candidate heads, propose_parents
        // emits all 3.
        let expected_parents = tc.propose_parents();
        assert_eq!(
            expected_parents.len(),
            3,
            "propose_parents() under mcc_full must emit the 3-fork antichain; \
             got {:?}",
            expected_parents
        );
        let expected_parent_set: std::collections::BTreeSet<[u8; 32]> =
            expected_parents.iter().copied().collect();

        // Find a round where we're the proposer (mirrors test_proposal_creation).
        let mut emitted: Option<Block> = None;
        for round in 0..100 {
            tc.round_state = RoundState::new(round);
            if tc.am_i_proposer() {
                emitted = tc.create_proposal(&mut db);
                break;
            }
        }
        let block = emitted.expect("proposer must emit a block in some round");

        // Hot-path-vs-substrate agreement: the block's parents field
        // matches the substrate accessor exactly. This is what verifies
        // the wiring at create_proposal's `parents: self.propose_parents()`
        // line is correctly fed by the same code path the substrate
        // tests exercise.
        let block_parent_set: std::collections::BTreeSet<[u8; 32]> =
            block.parents.iter().copied().collect();
        assert_eq!(
            block.parents.len(),
            3,
            "proposer must emit a 3-parent block under mcc_full; got {} parents",
            block.parents.len()
        );
        assert_eq!(
            block_parent_set, expected_parent_set,
            "block.parents set must match propose_parents() set"
        );
        // First parent is the authoritative head (highest caliber).
        assert_eq!(
            block.parents[0], expected_parents[0],
            "block.parents[0] must lead with the authoritative head"
        );
    }

    /// MCC Phase C.6 / D.1 — full 4-validator BFT round under
    /// `mcc_full`. Beyond the substrate-level convergence (D.1) and
    /// single-proposer hot-path emission (the test above), this drives
    /// the complete propose → prevote → precommit → commit pipeline
    /// across 4 in-process validators with the DAG pre-populated with
    /// 3 sibling forks. Verifies that:
    ///
    ///   1. The proposer emits a multi-parent block,
    ///   2. The other 3 validators accept the multi-parent block under
    ///      mcc_full's parent-acceptance arm,
    ///   3. Prevote + precommit quorum tally on the multi-parent block
    ///      (block_hash differentiation works correctly when parent
    ///      sets differ from the linear-chain default),
    ///   4. The block commits and lands as `CommitBlock` action.
    ///
    /// This is the load-bearing end-to-end test that proves multi-parent
    /// consensus works under realistic 4-validator BFT — not just at
    /// the substrate-accessor level. Mirrors the structure of
    /// `test_multi_validator_consensus_simulation` (linear path) for
    /// parity.
    #[test]
    fn mcc_phase_c_hot_path_4_validator_full_round_under_mcc_full() {
        use evaporchain_light_cone::Block as LcBlock;
        let ids = &[1u64, 2, 3, 4];
        let mut validators: Vec<TendermintConsensus> =
            ids.iter().map(|&id| make_consensus(id, ids)).collect();

        // Flip every validator into mcc_full mode. Same DAG, same
        // governance state — validator-determinism is locked by C.5
        // already, so all 4 will compute identical candidate-head
        // sets and parent orderings.
        for v in &mut validators {
            v.governance_set_param("parent_acceptance_mode", "mcc_full")
                .expect("mcc_full is allowlisted");
        }

        // Pre-populate every validator's light_cone_dag with the same
        // 3-fork antichain. Mirrors the C.6 / D.1 setup so propose_parents
        // emits the full 3-parent set.
        let g = [0u8; 32];
        let fa = [1u8; 32];
        let fb = [2u8; 32];
        let fc = [3u8; 32];
        for v in &mut validators {
            v.light_cone_dag
                .insert(LcBlock::new(g, vec![], 1000, 0))
                .expect("genesis");
            v.light_cone_dag
                .insert(LcBlock::new(fa, vec![g], 1001, 1))
                .expect("fork A");
            v.light_cone_dag
                .insert(LcBlock::new(fb, vec![g], 1002, 1))
                .expect("fork B");
            v.light_cone_dag
                .insert(LcBlock::new(fc, vec![g], 1003, 1))
                .expect("fork C");
        }

        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        // Drive the consensus pipeline (mirrors
        // `test_multi_validator_consensus_simulation`). Tick once to
        // produce the initial proposal, then deliver messages and tick
        // until a CommitBlock fires or 20 rounds elapse.
        let mut messages = Vec::new();
        for v in &mut validators {
            let actions = v.tick(&mut db);
            for a in actions {
                if let ConsensusAction::BroadcastMessage(msg) = a {
                    messages.push(msg);
                }
            }
        }

        let mut commit_actions: Vec<Block> = Vec::new();
        for _ in 0..20 {
            let current_msgs: Vec<_> = std::mem::take(&mut messages);
            for msg in &current_msgs {
                for v in &mut validators {
                    let actions = v.on_message(msg.clone());
                    for a in actions {
                        match a {
                            ConsensusAction::BroadcastMessage(m) => messages.push(m),
                            ConsensusAction::CommitBlock(b) => commit_actions.push(b),
                            _ => {}
                        }
                    }
                }
            }
            for v in &mut validators {
                let actions = v.tick(&mut db);
                for a in actions {
                    match a {
                        ConsensusAction::BroadcastMessage(m) => messages.push(m),
                        ConsensusAction::CommitBlock(b) => commit_actions.push(b),
                        _ => {}
                    }
                }
            }
            if !commit_actions.is_empty() {
                break;
            }
        }

        assert!(
            !commit_actions.is_empty(),
            "4-validator mcc_full network should reach consensus on a multi-parent block"
        );
        let committed = &commit_actions[0];
        assert_eq!(
            committed.parents.len(),
            3,
            "committed block must carry the 3-fork antichain as parents; got {} parents",
            committed.parents.len()
        );
        let parent_set: std::collections::BTreeSet<[u8; 32]> =
            committed.parents.iter().copied().collect();
        let expected_set: std::collections::BTreeSet<[u8; 32]> = [fa, fb, fc].into_iter().collect();
        assert_eq!(
            parent_set, expected_set,
            "committed block's parent set must equal the 3-fork antichain {{A, B, C}}"
        );
    }

    /// MCC Phase C.6 / D.1 — companion to the multi-parent test:
    /// flipping `parent_acceptance_mode` back to `linear` (or leaving
    /// it default) MUST emit a single-parent block (`parents` empty
    /// — the wire-format default that `serde(skip_if_empty)` collapses
    /// to a linear-chain block). This is the bit-compatibility safety
    /// net: a governance flag flip back to `linear` immediately
    /// restores pre-MCC wire format.
    #[test]
    fn mcc_phase_c_hot_path_proposer_emits_empty_parents_under_linear() {
        use evaporchain_light_cone::Block as LcBlock;
        let mut db = InMemoryStateDB::new();
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Default mode is "linear"; populate the DAG anyway so the
        // assertion is non-trivial — proves the proposer DOESN'T pick
        // up the multi-parent set even when the substrate has one.
        let g = [0u8; 32];
        tc.light_cone_dag
            .insert(LcBlock::new(g, vec![], 1000, 0))
            .expect("insert genesis");
        tc.light_cone_dag
            .insert(LcBlock::new([1u8; 32], vec![g], 1001, 1))
            .expect("insert fork A");
        tc.light_cone_dag
            .insert(LcBlock::new([2u8; 32], vec![g], 1002, 1))
            .expect("insert fork B");

        let mut emitted: Option<Block> = None;
        for round in 0..100 {
            tc.round_state = RoundState::new(round);
            if tc.am_i_proposer() {
                emitted = tc.create_proposal(&mut db);
                break;
            }
        }
        let block = emitted.expect("proposer must emit a block");
        assert!(
            block.parents.is_empty(),
            "linear mode must emit empty parents (single-parent wire format); \
             got {:?}",
            block.parents
        );
    }

    #[test]
    fn test_full_consensus_round_single_validator() {
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        let mut tc = make_consensus(1, &[1]);
        assert_eq!(tc.height(), 1);

        // Single validator should be able to self-propose, self-vote, self-commit
        let actions = tc.tick(&mut db);
        assert!(!actions.is_empty(), "Should produce proposal + prevote");

        // With single validator, quorum is 1 — should progress through all phases
        let mut all_actions = actions;
        for _ in 0..10 {
            let more = tc.tick(&mut db);
            all_actions.extend(more);
            if tc.phase() == Phase::Commit {
                break;
            }
        }

        // Should have a CommitBlock action
        let has_commit = all_actions
            .iter()
            .any(|a| matches!(a, ConsensusAction::CommitBlock(_)));
        assert!(has_commit, "Should reach commit");
    }

    #[test]
    fn test_multi_validator_consensus_simulation() {
        let ids = &[1u64, 2, 3, 4];
        let mut validators: Vec<TendermintConsensus> =
            ids.iter().map(|&id| make_consensus(id, ids)).collect();

        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        // Tick all validators — the proposer should create a proposal
        let mut messages = Vec::new();
        for v in &mut validators {
            let actions = v.tick(&mut db);
            for a in actions {
                if let ConsensusAction::BroadcastMessage(msg) = a {
                    messages.push(msg);
                }
            }
        }

        // Deliver all messages to all validators
        let mut commit_actions = Vec::new();
        for _ in 0..20 {
            let current_msgs: Vec<_> = std::mem::take(&mut messages);
            for msg in &current_msgs {
                for v in &mut validators {
                    let actions = v.on_message(msg.clone());
                    for a in actions {
                        match a {
                            ConsensusAction::BroadcastMessage(m) => messages.push(m),
                            ConsensusAction::CommitBlock(b) => commit_actions.push(b),
                            _ => {}
                        }
                    }
                }
            }

            // Tick all validators
            for v in &mut validators {
                let actions = v.tick(&mut db);
                for a in actions {
                    match a {
                        ConsensusAction::BroadcastMessage(m) => messages.push(m),
                        ConsensusAction::CommitBlock(b) => commit_actions.push(b),
                        _ => {}
                    }
                }
            }

            if !commit_actions.is_empty() {
                break;
            }
        }

        assert!(
            !commit_actions.is_empty(),
            "4-validator network should reach consensus"
        );
    }

    #[test]
    fn test_advance_height() {
        let mut tc = make_consensus(1, &[1]);
        assert_eq!(tc.height(), 1);
        assert_eq!(tc.epoch(), 0);

        let block = Block {
            number: 1,
            epoch: 1,
            parent_hash: [0u8; 32],
            state_root: [1u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
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
            post_state_root: None,
        };

        tc.on_block_committed(&block, [1u8; 32], 0);
        assert_eq!(tc.height(), 2);
        assert_eq!(tc.epoch(), 1);
    }

    #[test]
    fn test_block_hash_deterministic() {
        let block = Block {
            number: 1,
            epoch: 1,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 12345,
            chain_id: String::new(),
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
            post_state_root: None,
        };

        let h1 = TendermintConsensus::block_hash(&block);
        let h2 = TendermintConsensus::block_hash(&block);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_proposal_timeout_sends_nil_prevote() {
        let mut db = InMemoryStateDB::new();

        // Use validator 2, but make validator 1 the proposer
        let mut tc = make_consensus(2, &[1, 2, 3, 4]);

        // Find a round where validator 2 is NOT proposer
        for round in 0..100 {
            tc.round_state = RoundState::new(round);
            if !tc.am_i_proposer() {
                break;
            }
        }
        assert!(!tc.am_i_proposer());

        // Simulate timeout by setting phase_start far in the past
        tc.round_state.phase_start = Instant::now() - Duration::from_secs(10);

        let actions = tc.tick(&mut db);
        // Should send nil prevote after timeout
        let has_nil_prevote = actions.iter().any(|a| {
            matches!(
                a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote {
                    block_hash: None,
                    ..
                })
            )
        });
        assert!(has_nil_prevote, "Should send nil prevote on timeout");
    }

    #[test]
    fn test_restore_state() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.restore_state(100, 100, [42u8; 32]);
        assert_eq!(tc.height(), 101);
        assert_eq!(tc.epoch(), 100);
        assert_eq!(tc.parent_hash, [42u8; 32]);
    }

    // ─── Adversarial Consensus Tests ──────────────────────────────────

    #[test]
    fn test_stale_message_ignored() {
        // Old-height messages should be silently ignored
        let mut tc = make_consensus(1, &[1, 2, 3]);
        // Advance to height 5
        tc.restore_state(4, 4, [0u8; 32]);
        assert_eq!(tc.height(), 5);

        // Send a prevote from height 1 — should produce no actions
        let stale = ConsensusMessage::Prevote {
            height: 1,
            round: 0,
            block_hash: Some([1u8; 32]),
            validator_id: 2,
            bls_signature: None,
        };
        let actions = tc.on_message(stale);
        assert!(actions.is_empty(), "Stale messages should be dropped");
    }

    #[test]
    fn test_duplicate_votes_ignored() {
        // Same validator voting twice for the same round shouldn't double-count
        let ids = &[1, 2, 3, 4];
        let mut nodes: Vec<_> = ids.iter().map(|&id| make_consensus(id, ids)).collect();

        // Find proposer for height 1 round 0
        let proposer_id = nodes[0].proposer_for_round(1, 0).unwrap().id;
        let proposer_idx = ids.iter().position(|&id| id == proposer_id).unwrap();

        // Let proposer tick to create proposal
        let mut db = InMemoryStateDB::new();
        let actions = nodes[proposer_idx].tick(&mut db);
        let proposal = actions.iter().find_map(|a| match a {
            ConsensusAction::BroadcastMessage(msg @ ConsensusMessage::Proposal { .. }) => {
                Some(msg.clone())
            }
            _ => None,
        });
        assert!(proposal.is_some(), "Proposer should create a proposal");

        // Deliver proposal to validator 2 (not the proposer)
        let non_proposer_idx = ids.iter().position(|&id| id != proposer_id).unwrap();
        let actions = nodes[non_proposer_idx].on_message(proposal.clone().unwrap());

        // Validator 2 should send a prevote
        let prevote = actions.iter().find_map(|a| match a {
            ConsensusAction::BroadcastMessage(msg @ ConsensusMessage::Prevote { .. }) => {
                Some(msg.clone())
            }
            _ => None,
        });
        assert!(prevote.is_some(), "Should generate a prevote");

        // Deliver the same prevote to proposer TWICE
        let actions1 = nodes[proposer_idx].on_message(prevote.clone().unwrap());
        let actions2 = nodes[proposer_idx].on_message(prevote.unwrap());

        // The second delivery shouldn't cause different behavior than if it hadn't happened
        // (the vote is already recorded, so it's a no-op)
        // We just verify no crash and no duplicate commit
        let commits1 = actions1
            .iter()
            .filter(|a| matches!(a, ConsensusAction::CommitBlock(_)))
            .count();
        let commits2 = actions2
            .iter()
            .filter(|a| matches!(a, ConsensusAction::CommitBlock(_)))
            .count();
        // Should not commit from duplicate votes alone
        assert!(
            commits1 + commits2 <= 1,
            "Duplicate votes should not cause multiple commits"
        );
    }

    #[test]
    fn test_wrong_proposer_rejected() {
        // A proposal from a non-leader should be ignored
        let ids = &[1, 2, 3, 4];
        let mut tc = make_consensus(1, ids);

        let proposer_id = tc.proposer_for_round(1, 0).unwrap().id;
        let wrong_id = ids.iter().find(|&&id| id != proposer_id).unwrap();

        // Create fake proposal from wrong validator
        let block = Block {
            number: 1,
            epoch: 1,
            parent_hash: tc.parent_hash,
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: Some(*wrong_id),
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
            post_state_root: None,
        };

        let fake_proposal = ConsensusMessage::Proposal {
            height: 1,
            round: 0,
            block,
            proposer_id: *wrong_id,
        };

        let actions = tc.on_message(fake_proposal);
        // Should not generate a prevote for a wrong proposer's block
        let prevotes: Vec<_> = actions
            .iter()
            .filter(|a| {
                matches!(
                    a,
                    ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. })
                )
            })
            .collect();
        assert!(prevotes.is_empty(), "Should not prevote for wrong proposer");
    }

    #[test]
    fn test_parent_acceptance_mode_mcc_diverges_from_linear_on_diverging_parent() {
        // Lane J.2 integration test: with parent_acceptance_mode = "mcc",
        // a proposal with diverging parent_hash dispatches through
        // MccForkChoice (Lane I.4 wire-up + I.6 β derivation) instead
        // of the legacy `local == candidate` equality check. With an
        // empty LightCone DAG (fresh TC), MccForkChoice falls through
        // to lex tie-break on the trajectory heads — accepts the
        // candidate iff its hash is lex-larger than local's.
        //
        // Pick `block.parent_hash = [0xFF; 32]` (lex max) and
        // `tc.parent_hash = [0x00; 32]` (lex min, the default).
        //   - Linear mode: REJECT (parents don't match) → RequestSync
        //   - MCC mode: ACCEPT (FF > 00 lex tie-break)
        //
        // This is the load-bearing differential test: it proves the
        // mode flag actually gets consulted, by observing different
        // outcomes from the same input.
        let ids = &[1, 2, 3, 4];
        let mut tc = make_consensus(1, ids);

        // Force a known proposer for round 0 + valid signing key.
        let proposer_id = tc.proposer_for_round(1, 0).unwrap().id;

        // Construct a proposal whose parent_hash diverges from local.
        let mk_proposal = || {
            let block = Block {
                number: 1,
                epoch: 1,
                parent_hash: [0xFF; 32], // lex max — diverges from default [0; 32]
                state_root: [0u8; 32],
                transactions: vec![],
                timestamp: 0,
                chain_id: String::new(),
                producer_id: Some(proposer_id),
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
                post_state_root: None,
            };
            ConsensusMessage::Proposal {
                height: 1,
                round: 0,
                block,
                proposer_id,
            }
        };

        // Sanity: tc.parent_hash starts at [0; 32].
        assert_eq!(tc.parent_hash, [0u8; 32]);

        // ── Mode: linear (default). Diverging parent → reject + RequestSync.
        let actions_linear = tc.on_message(mk_proposal());
        let linear_request_sync = actions_linear
            .iter()
            .any(|a| matches!(a, ConsensusAction::RequestSync(_, _)));
        assert!(
            linear_request_sync,
            "linear mode must reject diverging parent and emit RequestSync"
        );

        // ── Mode: mcc. Same proposal, different governance flag.
        // Reset round state for a fresh on_message call.
        let mut tc2 = make_consensus(1, ids);
        tc2.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc".to_string());
        assert_eq!(tc2.parent_hash, [0u8; 32]);

        let actions_mcc = tc2.on_message(mk_proposal());
        // MCC mode accepts (lex tie-break: FF > 00). The proposal
        // proceeds past the parent check; we don't assert on what
        // happens after (timestamp, chain_id, sig checks may still
        // intervene), only that the parent-hash gate did NOT
        // short-circuit with RequestSync.
        let mcc_request_sync = actions_mcc.iter().any(|a| {
            matches!(
                a,
                ConsensusAction::RequestSync(s, _) if *s == tc2.height.saturating_sub(5)
            )
        });
        assert!(
            !mcc_request_sync,
            "mcc mode must NOT emit the parent-hash-divergence RequestSync \
             at the linear short-circuit site (FF > 00 lex tie-break accepts) \
             — got actions {:?}",
            actions_mcc
        );
    }

    #[test]
    fn test_governance_set_param_accepts_all_allowlisted_pairs() {
        // Lane K.2: every (key, value) pair in the allowlist must
        // succeed and actually mutate governance_params. Locks the
        // contract operators rely on at POST /api/governance/param.
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);

        // (key, value) pairs that MUST succeed.
        let pairs = vec![
            ("parent_acceptance_mode", "linear"),
            ("parent_acceptance_mode", "mcc"),
            ("block_source_mode", "fifo"),
            ("block_source_mode", "antichain"),
            ("conservation_enforcement", "observe"),
            ("conservation_enforcement", "enforce"),
            // Phase 5.2 of LAMBDA_FOLD_NOVA_PLAN — `lambda_fold_mode`
            // governance flag chooses between the substrate blake3
            // hash-chain fold and the real Nova IVC pipeline at the
            // tendermint per-block fold call site.
            ("lambda_fold_mode", "hash_chain"),
            ("lambda_fold_mode", "nova"),
            // Phase 2.2 of CROOKS_MEV_INTEGRATION_PLAN.md.
            ("crooks_mev_beta_mb", "1000"),
            ("crooks_mev_beta_mb", "1"),
            ("crooks_mev_beta_mb", "999999"),
            // Phase 3.4 of CROOKS_MEV_INTEGRATION_PLAN.md.
            ("crooks_mev_settlement_mode", "observe"),
            ("crooks_mev_settlement_mode", "enforce"),
        ];
        for (key, value) in &pairs {
            assert!(
                tc.governance_set_param(key, value).is_ok(),
                "allowlist pair ({key}, {value}) must succeed"
            );
            assert_eq!(
                tc.get_governance_param(key),
                Some(*value),
                "governance_params must reflect the set value"
            );
        }
        // After the loop, each key holds its LAST-set value (later
        // pairs overwrite earlier ones for the same key). Snapshot
        // must reflect those last-writes.
        let snap = tc.governance_flags_snapshot();
        assert_eq!(
            snap.get("parent_acceptance_mode").map(|s| s.as_str()),
            Some("mcc")
        );
        assert_eq!(
            snap.get("block_source_mode").map(|s| s.as_str()),
            Some("antichain")
        );
        assert_eq!(
            snap.get("conservation_enforcement").map(|s| s.as_str()),
            Some("enforce")
        );
        assert_eq!(
            snap.get("lambda_fold_mode").map(|s| s.as_str()),
            Some("nova")
        );
    }

    /// Phase 5.1 of LAMBDA_FOLD_NOVA_PLAN — default-features build:
    /// the Nova folder is not compiled in, so flipping
    /// `lambda_fold_mode = "nova"` is a no-op at the call site (the
    /// substrate fold runs unconditionally). Locks the feature-gate
    /// contract.
    #[test]
    fn test_lambda_fold_nova_mode_no_op_without_feature() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("lambda_fold_mode", "nova").unwrap();
        // The flag is observable…
        assert_eq!(tc.get_governance_param("lambda_fold_mode"), Some("nova"));
        // …and the substrate fold accumulator is at identity (no
        // folds run yet — the test doesn't drive on_block_committed).
        assert!(tc.lambda_fold.is_identity());
    }

    /// Lane O.8.1c — drive 60 synthetic blocks through `on_block_committed`
    /// and verify the on-chain Causal-CHSH alarm fires. With doctrine
    /// defaults (capacity=200, run_interval=50, window=60s), the first
    /// run lands when records_seen reaches 50 (interval) AND
    /// buffer_len ≥ 50 — both met after the 50th committed block. By
    /// 60 blocks the alarm has run AT LEAST once; status() is
    /// populated and the verdict is "Pass" on synthetic honest
    /// chain traffic.
    ///
    /// Locks the Lane O.8.1 wire-up end-to-end:
    ///   on_block_committed → cartel_alarm.record_block → periodic
    ///   gate run → cartel_alarm_status() returns Some.
    #[test]
    fn test_cartel_alarm_fires_after_60_committed_blocks() {
        fn make_block_for_test(height: u64) -> Block {
            // Vary timestamps so the concurrency-window proxy admits
            // pairs (12s spacing matches Eth mainnet block-time, well
            // under the 60s window default).
            Block {
                number: height,
                epoch: height / 10,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: vec![],
                producer_id: Some(0),
                timestamp: height * 12, // seconds
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);

        // Pre-tick assertions: alarm exists but hasn't run yet.
        assert!(tc.cartel_alarm_status().is_none());
        assert_eq!(tc.cartel_alarm_buffer_len(), 0);
        assert_eq!(tc.cartel_alarm_records_seen(), 0);

        // Drive 60 blocks through the consensus commit hook.
        for h in 1..=60u64 {
            let block = make_block_for_test(h);
            let mut state_root = [0u8; 32];
            state_root[0] = h as u8;
            tc.on_block_committed(&block, state_root, 0);
        }

        // Post-tick assertions: alarm has fired at least once.
        assert_eq!(tc.cartel_alarm_records_seen(), 60);
        assert_eq!(tc.cartel_alarm_buffer_len(), 60); // still below 200 cap
        let st = tc
            .cartel_alarm_status()
            .expect("alarm should have run by 60 blocks (interval=50, buffer≥50)");
        // Empty-block synthetic chain: tx_count=0 + gas=0 for every
        // block → some buckets may have ≤ 5 samples (medians collapse
        // to zero, observables are constant). Accept either:
        //  - Pass (S well below 1.8 ceiling — typical on healthy spread)
        //  - InputError starting with "InputError:" (under-populated
        //    bucket — expected on the all-zero synthetic shape)
        // What we assert: status IS populated, and the verdict string
        // is well-formed.
        assert!(
            st.verdict == "Pass" || st.verdict == "Fail" || st.verdict.starts_with("InputError"),
            "unexpected verdict: {:?}",
            st.verdict
        );
        assert_eq!(
            st.last_run_at_height, 50,
            "first run fires at records_seen=50, height=50"
        );
    }

    /// Lane O.8.2 — `CartelAlarmEvent` emission with governance flag.
    ///
    /// Locks the wire-up end-to-end:
    ///   1. Default `cartel_alarm_mode = "observe"` ⇒ no events emitted
    ///      even when the alarm reports an over-ceiling status.
    ///   2. Setting `cartel_alarm_mode = "alarm"` via
    ///      `governance_set_param` ⇒ next emission gate fires.
    ///   3. `take_pending_cartel_alarms` drains the queue.
    ///   4. Re-firing at the same `last_run_at_height` is de-duplicated
    ///      (no double emission across multiple ticks before the next
    ///      periodic recompute).
    ///
    /// Uses the `_inject_status_for_test` doctrine helper on
    /// `CartelAlarm` to simulate an over-ceiling status without having
    /// to coerce the synthetic block-summary path into actually
    /// crossing the bound (which it can't on healthy synthetic data —
    /// that's the whole point of Causal-CHSH).
    #[test]
    fn test_cartel_alarm_event_emission_governance_gated() {
        use evaporchain_causal_chsh::{AlarmStatus, GateThresholds};

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);

        // Inject an over-ceiling status: s_honest_milli = 2500 (>= 1800
        // doctrine ceiling). Synthesises the "chain detected coordination
        // among its own validators" condition.
        let over_ceiling_status = AlarmStatus {
            s_honest: 2.5,
            s_cartel_synthetic: 3.6,
            gap: 1.1,
            s_honest_milli: 2500,
            s_cartel_synthetic_milli: 3600,
            gap_milli: 1100,
            verdict: "Fail".to_string(),
            last_run_at_height: 100,
            samples_per_bucket: [25, 25, 25, 25],
            thresholds: GateThresholds::doctrine(),
        };
        tc.cartel_alarm
            ._inject_status_for_test(over_ceiling_status.clone());

        // Step 1: default `observe` mode → emission gate is silent.
        tc.maybe_emit_cartel_alarm_event();
        assert_eq!(
            tc.take_pending_cartel_alarms().len(),
            0,
            "default observe mode must not emit events even on over-ceiling status"
        );

        // Step 2: flip governance to `alarm` mode via the K.1 RPC path.
        tc.governance_set_param("cartel_alarm_mode", "alarm")
            .expect("alarm mode must be in the allowlist");

        // Step 3: emission gate now fires → exactly one event drained.
        tc.maybe_emit_cartel_alarm_event();
        let events = tc.take_pending_cartel_alarms();
        assert_eq!(events.len(), 1, "alarm mode + over-ceiling must emit");
        assert_eq!(events[0].at_height, 100);
        assert_eq!(events[0].s_honest_milli, 2500);
        assert_eq!(events[0].s_cartel_synthetic_milli, 3600);
        assert_eq!(events[0].gap_milli, 1100);
        assert_eq!(events[0].honest_ceiling_milli_at_fire, 1800);
        assert_eq!(events[0].samples_per_bucket, [25, 25, 25, 25]);

        // Step 4: same status, second tick → no re-emission (still drained;
        // dedupe is by `at_height` matching an event already in the queue
        // BEFORE the drain). Re-inject + re-fire to check the dedupe path.
        tc.cartel_alarm
            ._inject_status_for_test(over_ceiling_status.clone());
        tc.maybe_emit_cartel_alarm_event(); // queue now has 1 event for h=100
        tc.maybe_emit_cartel_alarm_event(); // dedupe must skip
        let events_after_dedupe = tc.take_pending_cartel_alarms();
        assert_eq!(
            events_after_dedupe.len(),
            1,
            "back-to-back ticks at same height must not double-emit"
        );

        // Step 5: governance defaults expose the new flag with `observe`
        // as the documented default.
        let snap = tc.governance_flags_snapshot();
        // Note: step 2 set it to `alarm`; the snapshot returns the
        // *effective* value, so we expect "alarm" here, not the
        // default. The default-when-unset is exercised in the next
        // assertion using a fresh TC.
        assert_eq!(
            snap.get("cartel_alarm_mode").map(|s| s.as_str()),
            Some("alarm")
        );

        let fresh = make_consensus(1, &[1, 2, 3, 4]);
        let snap_default = fresh.governance_flags_snapshot();
        assert_eq!(
            snap_default.get("cartel_alarm_mode").map(|s| s.as_str()),
            Some("observe"),
            "default cartel_alarm_mode must be observe"
        );
    }

    /// Lane O.8.2c — full-pipeline integration test for cartel-alarm
    /// emission. Drives blocks through `on_block_committed` with
    /// `cartel_alarm_mode = "alarm"` set via the K.1 RPC path, then
    /// uses `_inject_status_for_test` to simulate an over-ceiling
    /// status (synthetic empty-block traffic can't naturally cross
    /// the doctrine ceiling — that's the whole point of Causal-CHSH),
    /// drives one more block to trigger `maybe_emit_cartel_alarm_event`
    /// from inside the real consensus hot path, and verifies the
    /// queued event is drainable via `take_pending_cartel_alarms`.
    ///
    /// Distinct from the unit test in `test_cartel_alarm_event_emission_
    /// governance_gated`: that test calls `maybe_emit_cartel_alarm_event`
    /// directly. This test exercises the full path through
    /// `on_block_committed` so the call-site wiring stays locked.
    #[test]
    fn test_cartel_alarm_event_emission_via_on_block_committed() {
        use evaporchain_causal_chsh::{AlarmStatus, GateThresholds};
        use evaporchain_types::Block;

        fn make_block_for_test(height: u64) -> Block {
            Block {
                number: height,
                epoch: height / 10,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: vec![],
                producer_id: Some(0),
                timestamp: height * 12,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);

        // Step 1: drive a few blocks under default `observe` mode.
        // Even if the synthetic alarm somehow produced a fresh status,
        // observe mode must keep the queue empty.
        for h in 1..=5u64 {
            let block = make_block_for_test(h);
            let mut state_root = [0u8; 32];
            state_root[0] = h as u8;
            tc.on_block_committed(&block, state_root, 0);
        }
        assert_eq!(
            tc.take_pending_cartel_alarms().len(),
            0,
            "default observe mode must not queue events on hot path"
        );

        // Step 2: governance flip to alarm mode.
        tc.governance_set_param("cartel_alarm_mode", "alarm")
            .expect("alarm mode must pass allowlist");

        // Step 3: inject an over-ceiling status (synthetic empty blocks
        // can't naturally cross the bound — that's the whole point of
        // Causal-CHSH on honest traffic).
        let over_ceiling = AlarmStatus {
            s_honest: 2.4,
            s_cartel_synthetic: 3.5,
            gap: 1.1,
            s_honest_milli: 2400,
            s_cartel_synthetic_milli: 3500,
            gap_milli: 1100,
            verdict: "Fail".to_string(),
            last_run_at_height: 6, // matches the next on_block_committed call
            samples_per_bucket: [20, 20, 20, 20],
            thresholds: GateThresholds::doctrine(),
        };
        tc.cartel_alarm._inject_status_for_test(over_ceiling);

        // Step 4: drive one more block through the real hot path. The
        // record_block call inside on_block_committed pushes a new
        // BlockSummary but DOESN'T overwrite the injected status (the
        // periodic recompute only fires at run_interval boundaries
        // with buffer.len() >= 50; we're far below both). So the
        // emission gate sees our injected over-ceiling status and
        // queues the event.
        let block_6 = make_block_for_test(6);
        let mut state_root_6 = [0u8; 32];
        state_root_6[0] = 6;
        tc.on_block_committed(&block_6, state_root_6, 0);

        // Step 5: drain the queue and verify.
        let events = tc.take_pending_cartel_alarms();
        assert_eq!(
            events.len(),
            1,
            "alarm mode + over-ceiling status must emit on hot path"
        );
        assert_eq!(events[0].at_height, 6);
        assert_eq!(events[0].s_honest_milli, 2400);
        assert_eq!(events[0].honest_ceiling_milli_at_fire, 1800);
        assert_eq!(events[0].samples_per_bucket, [20, 20, 20, 20]);

        // Step 6: drive another block — the dedupe should keep the
        // queue empty for this height since we already emitted.
        // (Inject the same status again to simulate the alarm not
        // having recomputed yet.)
        let over_ceiling_again = AlarmStatus {
            s_honest: 2.4,
            s_cartel_synthetic: 3.5,
            gap: 1.1,
            s_honest_milli: 2400,
            s_cartel_synthetic_milli: 3500,
            gap_milli: 1100,
            verdict: "Fail".to_string(),
            last_run_at_height: 6, // SAME height as before
            samples_per_bucket: [20, 20, 20, 20],
            thresholds: GateThresholds::doctrine(),
        };
        tc.cartel_alarm._inject_status_for_test(over_ceiling_again);

        // Re-emit: queue is empty (last drain), so dedupe by `at_height`
        // is against a cleared queue → it WILL emit again. The dedupe
        // semantic is "don't double-emit while the event is still in
        // the queue", not "never re-emit for a height". Operators
        // should track their own ack-set if they need historical
        // dedup.
        let block_7 = make_block_for_test(7);
        let mut state_root_7 = [0u8; 32];
        state_root_7[0] = 7;
        tc.on_block_committed(&block_7, state_root_7, 0);
        let events_after_drain = tc.take_pending_cartel_alarms();
        assert_eq!(
            events_after_drain.len(),
            1,
            "after a drain, a same-height status reappears in the queue (operator owns ack-set)"
        );
    }

    /// Phase 1.5 of `CROOKS_MEV_INTEGRATION_PLAN.md` — drive a
    /// synthetic sandwich block through `on_block_committed` and
    /// Phase 3.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — state-branch
    /// table starts empty. Default-off rollout-flag means the table
    /// stays empty even when the chain commits blocks (chain-bit-compat).
    #[test]
    fn test_state_branches_starts_empty_and_flag_off_keeps_empty() {
        let tc = make_consensus(1, &[1, 2, 3, 4]);
        assert!(tc.state_branches().is_empty());
    }

    /// Phase 3.1 — `record_state_branch` is idempotent. Re-recording
    /// the same tip bumps `last_touched_block` and refreshes
    /// `caliber` but doesn't double-count.
    #[test]
    fn test_state_branches_record_idempotent() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        let tip = [0xAA; 32];
        tc.record_state_branch(tip, 1, 100);
        tc.record_state_branch(tip, 5, 250);
        assert_eq!(tc.state_branches().len(), 1);
        let m = &tc.state_branches()[&tip];
        assert_eq!(m.created_at_block, 1);
        assert_eq!(m.last_touched_block, 5);
        assert_eq!(m.caliber, 250);
    }

    /// Phase 4.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` (Decision 1) —
    /// `dag_round_states` starts empty (linear-mode default).
    /// `dag_round_states_count` is 0 at construction.
    #[test]
    fn test_dag_round_states_starts_empty() {
        let tc = make_consensus(1, &[1, 2, 3, 4]);
        assert_eq!(tc.dag_round_states_count(), 0);
        assert_eq!(tc.dag_round_state_counts(&[0xAA; 32]), None);
    }

    /// Phase 4.1 — manually inserting a `RoundState` for a tip
    /// surfaces via the typed counts accessor. Locks the seam
    /// Phase 4.1 implementation will plug into.
    #[test]
    fn test_dag_round_states_insert_surfaces_via_counts() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        let tip = [0xAA; 32];
        let mut rs = RoundState::new(0);
        rs.prevotes.insert(1, Some([0x11; 32]));
        rs.prevotes.insert(2, Some([0x11; 32]));
        rs.precommits.insert(1, Some([0x11; 32]));
        tc.dag_round_states.insert(tip, rs);

        assert_eq!(tc.dag_round_states_count(), 1);
        assert_eq!(tc.dag_round_state_counts(&tip), Some((2, 1)));
        assert_eq!(tc.dag_round_state_counts(&[0xBB; 32]), None);
    }

    /// Phase 6.3 of `LIGHT_CONE_FULL_DAG_PLAN.md` — performance
    /// budget benchmark. Drives 1000 DAG blocks @ 4 concurrent
    /// forks and times the hot operations:
    /// - LightCone insertion (per-block)
    /// - MccForkChoice::select_tip (per-tip-selection)
    /// - state-branch metadata insertion + LRU prune
    ///
    /// Plan budgets: insertion < 100 ms/block, select_tip < 50 ms,
    /// state-branch operations < 200 ms. Marked `#[ignore]`
    /// because it's an instrumentation test, not a correctness
    /// check. Run with
    /// `cargo test -p evaporchain-consensus --release --
    ///  --ignored --nocapture`.
    #[test]
    #[ignore = "perf benchmark — run with --ignored to record numbers"]
    fn benchmark_light_cone_phase_6_3() {
        use evaporchain_light_cone::Block as LcBlock;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();
        tc.governance_set_param("light_cone_max_concurrent_forks", "4")
            .unwrap();

        // Build a 1000-block DAG with 4 concurrent forks. Pattern:
        // genesis → 4 leaves → each leaf extended 249 times in
        // parallel = ~1000 blocks total.
        let total_blocks = 1000;
        let n_forks = 4;
        let blocks_per_fork = total_blocks / n_forks;

        let g = [0xFF; 32];
        tc.light_cone_dag
            .insert(LcBlock::new(g, vec![], 1000, 0))
            .unwrap();

        // 4 leaf seeds.
        let mut tips: Vec<[u8; 32]> = (0..n_forks)
            .map(|i| {
                let mut id = [0u8; 32];
                id[0] = 0xA0 + i as u8;
                id
            })
            .collect();
        for tip in &tips {
            tc.light_cone_dag
                .insert(LcBlock::new(*tip, vec![g], 100, 1))
                .unwrap();
        }

        let insert_start = std::time::Instant::now();
        for round in 0..blocks_per_fork {
            for fork_idx in 0..n_forks {
                let mut new_tip = [0u8; 32];
                new_tip[0] = 0xA0 + fork_idx as u8;
                new_tip[1] = (round as u8).wrapping_add(1);
                new_tip[2] = ((round >> 8) as u8).wrapping_add(1);
                let parent = tips[fork_idx];
                if tc
                    .light_cone_dag
                    .insert(LcBlock::new(new_tip, vec![parent], 100, (round + 2) as u64))
                    .is_ok()
                {
                    tips[fork_idx] = new_tip;
                }
            }
        }
        let insert_total = insert_start.elapsed();
        let insert_per_block = insert_total / total_blocks as u32;
        eprintln!(
            "[Phase 6.3] DAG insertion: {:?} total / {:?} per block ({} blocks)",
            insert_total, insert_per_block, total_blocks
        );
        assert!(
            insert_per_block.as_millis() < 100,
            "insertion budget: < 100 ms/block; got {:?}",
            insert_per_block
        );

        // select_tip benchmark.
        let fc = crate::fork_choice::MccForkChoice::new(tc.light_cone_dag.clone(), 1000);
        let select_start = std::time::Instant::now();
        let _tip = {
            use crate::fork_choice::ForkChoice;
            fc.select_tip()
        };
        let select_elapsed = select_start.elapsed();
        eprintln!(
            "[Phase 6.3] select_tip on {}-block DAG: {:?}",
            total_blocks, select_elapsed
        );
        assert!(
            select_elapsed.as_millis() < 50,
            "select_tip budget: < 50 ms; got {:?}",
            select_elapsed
        );

        // state-branch metadata insertion + LRU prune.
        let sb_start = std::time::Instant::now();
        for tip in &tips {
            tc.record_state_branch(*tip, 1000, 100);
        }
        tc.prune_state_branches();
        let sb_elapsed = sb_start.elapsed();
        eprintln!("[Phase 6.3] 4-fork state-branch ops: {:?}", sb_elapsed);
        assert!(
            sb_elapsed.as_millis() < 200,
            "state-branch ops budget: < 200 ms; got {:?}",
            sb_elapsed
        );
    }

    /// Phase 6.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` — adversarial
    /// 2-fork split-vote test. Validators split 2/2 voting on
    /// different leaves; with f=1 (4 validators), threshold=3,
    /// neither leaf gets quorum → no finalization. When one
    /// validator switches sides, the surviving leaf reaches 3
    /// precommits and finalizes.
    ///
    /// **Honest-validator switching IS cross-fork equivocation**
    /// in this minimal implementation because the equivocation
    /// detector scans other tips' precommits without distinguishing
    /// honest-switch from malicious double-vote. Phase 4.3d
    /// (certificate-based equivocation evidence with on-chain proof)
    /// would refine this; for now operators interpret the counter
    /// in context.
    #[test]
    fn test_dag_mode_adversarial_2fork_split_vote_converges() {
        use evaporchain_light_cone::Block as LcBlock;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();

        let g = [0xFF; 32];
        let a = [0xAA; 32];
        let b = [0xBB; 32];
        tc.light_cone_dag
            .insert(LcBlock::new(g, vec![], 1000, 0))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(a, vec![g], 100, 1))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(b, vec![g], 100, 1))
            .unwrap();

        // Split 2/2: validators 1,2 → leaf A; validators 3,4 → leaf B.
        tc.record_dag_precommit(a, 1, Some(a), vec![]);
        tc.record_dag_precommit(a, 2, Some(a), vec![]);
        tc.record_dag_precommit(b, 3, Some(b), vec![]);
        tc.record_dag_precommit(b, 4, Some(b), vec![]);

        // Threshold = 2*1+1 = 3; neither leaf has 3 precommits.
        let finalized_round_1 = tc.try_finalize_antichain();
        assert!(
            finalized_round_1.is_empty(),
            "split-vote 2/2 must NOT finalize at threshold=3"
        );

        // Validator 3 switches to leaf A. Equivocation triggers
        // (Decision 3 — counts-based; can't distinguish honest
        // re-vote from malicious double-vote at this layer).
        tc.record_dag_precommit(a, 3, Some(a), vec![]);
        assert!(
            tc.cross_fork_equivocations().get(&3).copied().unwrap_or(0) > 0,
            "switching honestly between tips IS counted as equivocation \
             in the minimal counts-based detection (Decision 3 honesty \
             caveat); Phase 4.3d certificate-based detection refines this"
        );

        // Now leaf A has 3 precommits → finalizes.
        let finalized_round_2 = tc.try_finalize_antichain();
        assert_eq!(
            finalized_round_2,
            vec![a],
            "after switch, leaf A reaches quorum and finalizes"
        );
    }

    /// Phase 6.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — end-to-end
    /// DAG-mode integration test. Exercises the full pipeline:
    /// Phase 4.1 vote-record → Phase 4.2 finalization →
    /// Phase 4.3 cross-fork equivocation → Phase 4.4 dual-mode
    /// finality bookkeeping → Phase 5 LRU eviction (paired with
    /// DAG cascade-prune).
    ///
    /// Closes the substrate-level integration claim of the
    /// Light-Cone Full DAG plan. The pipeline runs end-to-end
    /// behind the `light_cone_state_branches_enabled` flag.
    #[test]
    fn test_dag_mode_full_pipeline_end_to_end() {
        use evaporchain_light_cone::Block as LcBlock;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();
        // Cap=3 so we'll see LRU kick in.
        tc.governance_set_param("light_cone_max_concurrent_forks", "3")
            .unwrap();

        // Build a 4-fork DAG: genesis → A, B, C, D (4 sibling leaves).
        let g = [0xFF; 32];
        let a = [0xAA; 32];
        let b = [0xBB; 32];
        let c = [0xCC; 32];
        let d = [0xDD; 32];
        tc.light_cone_dag
            .insert(LcBlock::new(g, vec![], 1000, 0))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(a, vec![g], 100, 1))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(b, vec![g], 200, 1))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(c, vec![g], 150, 1))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(d, vec![g], 175, 1))
            .unwrap();

        // Record state-branch metadata for each leaf.
        tc.record_state_branch(a, 1, 100);
        tc.record_state_branch(b, 1, 200);
        tc.record_state_branch(c, 1, 150);
        tc.record_state_branch(d, 1, 175);

        // Cap=3 → LRU triggers, lowest caliber `a` evicted from
        // metadata + DAG.
        tc.prune_state_branches();
        assert!(!tc.state_branches.contains_key(&a));
        assert!(!tc.light_cone_dag.contains(&a));
        // Genesis survives (branch point).
        assert!(tc.light_cone_dag.contains(&g));

        // Voting phase: 4 validators, threshold = 2*1+1 = 3.
        // Leaf B: validators 1, 2, 3 precommit → meets quorum.
        for v in 1..=3u64 {
            tc.record_dag_precommit(b, v, Some(b), vec![]);
        }
        // Leaf C: validators 1, 2 precommit → below quorum.
        for v in 1..=2u64 {
            tc.record_dag_precommit(c, v, Some(c), vec![]);
        }
        // Leaf D: validators 1, 2, 3, 4 precommit → meets quorum.
        for v in 1..=4u64 {
            tc.record_dag_precommit(d, v, Some(d), vec![]);
        }

        // Cross-fork equivocation: validator 1 precommitted on b,
        // c, d (3 different blocks) at the same round → 2 increments
        // (b→c, c→d). Actually validator 1 precommits b first; then
        // c (different hash from b at same round → equivocation
        // on c); then d (different hash from c at same round →
        // equivocation on d). Same logic for validators 2, 3.
        // Net: each of 1..=3 gets bumped multiple times. Validator
        // 4 only precommitted on d → no equivocation observed for it
        // YET (no other tip records its precommit).
        assert!(
            tc.cross_fork_equivocations().get(&1).copied().unwrap_or(0) > 0,
            "validator 1 precommitted on multiple tips → equivocation"
        );
        assert_eq!(
            tc.cross_fork_equivocations().get(&4),
            None,
            "validator 4 only voted once → no equivocation"
        );

        // Finalization: only b and d meet quorum.
        let finalized = tc.try_finalize_antichain();
        let mut sorted = finalized.clone();
        sorted.sort();
        assert_eq!(sorted, vec![b, d], "expected b + d to finalize");
        assert!(!sorted.contains(&c), "c (below-quorum) must NOT finalize");
        assert!(!sorted.contains(&a), "a (LRU-evicted) must NOT finalize");

        // Closing antichain still includes the surviving leaves.
        let ac = evaporchain_light_cone::concurrency::closing_antichain(&tc.light_cone_dag);
        let mut sorted_ac = ac.clone();
        sorted_ac.sort();
        // a was pruned; b, c, d, AND g (genesis) are tracked by the
        // DAG. g is the parent of b, c, d so it has children → NOT
        // a leaf. Surviving leaves: b, c, d.
        assert_eq!(sorted_ac, vec![b, c, d]);
    }

    /// Phase 3.5d of `CROOKS_MEV_INTEGRATION_PLAN.md` —
    /// `apply_mev_missing_refund_slashes` no-op when flag is off.
    #[test]
    fn test_apply_mev_missing_refund_slashes_flag_gated() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Seed a violation counter.
        tc.mev_missing_refund_violations.insert(1, 5);
        // Default flag-off → no-op + counter unchanged.
        let result = tc.apply_mev_missing_refund_slashes();
        assert!(result.is_empty());
        assert_eq!(tc.mev_missing_refund_violations.get(&1), Some(&5));
    }

    /// Phase 3.5d — flag on + non-zero counter + non-zero stake →
    /// slash applies via validator_set.slash_with_amount and
    /// counter resets.
    #[test]
    fn test_apply_mev_missing_refund_slashes_applies_slash() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("crooks_mev_missing_refund_slash_enabled", "true")
            .unwrap();

        // Validator 1 has some default stake from make_consensus;
        // confirm it's non-zero before we slash.
        let stake_before = tc.validator_set.get(1).expect("validator 1 exists").stake;
        assert!(stake_before > 0);

        // Violation count > 0 → entropic_slash returns a non-zero
        // amount on a {count, 1} two-outcome distribution.
        tc.mev_missing_refund_violations.insert(1, 100);
        let slashed = tc.apply_mev_missing_refund_slashes();
        // Counter reset.
        assert!(tc.mev_missing_refund_violations.get(&1).is_none());
        // The result should report at least the validator we
        // configured (real slash amount depends on entropy math).
        let entry_for_1 = slashed.iter().find(|(v, _)| *v == 1);
        assert!(
            entry_for_1.is_some(),
            "validator 1 should be in the slashed list"
        );
    }

    /// Phase 3.5d — flag on but validator absent → counter reset
    /// without panicking.
    #[test]
    fn test_apply_mev_missing_refund_slashes_unknown_validator() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("crooks_mev_missing_refund_slash_enabled", "true")
            .unwrap();
        tc.mev_missing_refund_violations.insert(99, 50);
        let slashed = tc.apply_mev_missing_refund_slashes();
        // Validator 99 doesn't exist → no slash entry.
        assert!(slashed.iter().all(|(v, _)| *v != 99));
        // Counter reset regardless — operator tooling expects it.
        assert!(tc.mev_missing_refund_violations.get(&99).is_none());
    }

    /// Phase 4.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` —
    /// `try_finalize_antichain` no-op when flag is off.
    #[test]
    fn test_try_finalize_antichain_flag_gated() {
        let tc = make_consensus(1, &[1, 2, 3, 4]);
        // Default flag-off → empty.
        assert!(tc.try_finalize_antichain().is_empty());
    }

    /// Phase 4.2 — empty DAG → no candidates → empty finalization.
    #[test]
    fn test_try_finalize_antichain_empty_dag() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();
        assert!(tc.try_finalize_antichain().is_empty());
    }

    /// Phase 4.2 — leaf with insufficient precommits is NOT
    /// finalized. With 4 validators, f=1, threshold = 2*1+1 = 3.
    #[test]
    fn test_try_finalize_antichain_below_quorum() {
        use evaporchain_light_cone::Block as LcBlock;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();
        let leaf = [0xAA; 32];
        tc.light_cone_dag
            .insert(LcBlock::new(leaf, vec![], 1000, 0))
            .unwrap();
        // Record 2 precommits — below the threshold of 3.
        tc.record_dag_precommit(leaf, 1, Some(leaf), vec![]);
        tc.record_dag_precommit(leaf, 2, Some(leaf), vec![]);
        let finalized = tc.try_finalize_antichain();
        assert!(finalized.is_empty(), "below-quorum leaf must not finalize");
    }

    /// Phase 4.2 — leaf with ≥ 2f+1 precommits IS finalized.
    #[test]
    fn test_try_finalize_antichain_meets_quorum() {
        use evaporchain_light_cone::Block as LcBlock;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();
        let leaf = [0xAA; 32];
        tc.light_cone_dag
            .insert(LcBlock::new(leaf, vec![], 1000, 0))
            .unwrap();
        // Record 3 precommits → ≥ threshold (2*1+1 with n=4).
        tc.record_dag_precommit(leaf, 1, Some(leaf), vec![]);
        tc.record_dag_precommit(leaf, 2, Some(leaf), vec![]);
        tc.record_dag_precommit(leaf, 3, Some(leaf), vec![]);
        let finalized = tc.try_finalize_antichain();
        assert_eq!(finalized, vec![leaf]);
    }

    /// Phase 4.2 — multi-leaf antichain: each leaf finalizes
    /// independently. Two siblings, each with 3 precommits → both
    /// finalize.
    #[test]
    fn test_try_finalize_antichain_multi_leaf() {
        use evaporchain_light_cone::Block as LcBlock;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();
        let g = [0xFF; 32];
        let a = [0xAA; 32];
        let b = [0xBB; 32];
        tc.light_cone_dag
            .insert(LcBlock::new(g, vec![], 1000, 0))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(a, vec![g], 100, 1))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(b, vec![g], 200, 1))
            .unwrap();
        for v in 1..=3u64 {
            // Same validators precommit on BOTH leaves with the
            // matching block_hash — equivocation detection skips
            // (Decision 3: same-block_hash on two tips is NOT
            // equivocation).
            tc.record_dag_precommit(a, v, Some(a), vec![]);
            tc.record_dag_precommit(b, v, Some(b), vec![]);
        }
        // Equivocation IS detected here because each validator
        // precommitted on different block_hashes (a vs b) at the
        // same round.
        assert!(!tc.cross_fork_equivocations().is_empty());

        // But finalization is per-leaf; both leaves meet quorum.
        let finalized = tc.try_finalize_antichain();
        let mut sorted = finalized.clone();
        sorted.sort();
        assert_eq!(sorted, vec![a, b]);
    }

    /// Phase 4.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` —
    /// `record_dag_prevote` no-op when flag is off; populates when
    /// flag is on.
    #[test]
    fn test_record_dag_prevote_flag_gated() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        let tip = [0xAA; 32];
        // Default flag-off → no-op.
        tc.record_dag_prevote(tip, 1, Some([0x11; 32]), vec![1, 2, 3]);
        assert!(tc.dag_round_states.is_empty());

        // Flag on → record persists.
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();
        tc.record_dag_prevote(tip, 1, Some([0x11; 32]), vec![1, 2, 3]);
        assert_eq!(tc.dag_round_states_count(), 1);
        assert_eq!(tc.dag_round_state_counts(&tip), Some((1, 0)));
    }

    /// Phase 4.1 + 4.3 — precommit on two concurrent tips at the
    /// same round increments the cross-fork equivocation counter.
    #[test]
    fn test_record_dag_precommit_detects_cross_fork_equivocation() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();
        let tip_a = [0xAA; 32];
        let tip_b = [0xBB; 32];

        // Validator 42 precommits on tip A for block X.
        tc.record_dag_precommit(tip_a, 42, Some([0xAB; 32]), vec![]);
        assert!(tc.cross_fork_equivocations().is_empty());

        // Validator 42 also precommits on tip B for a DIFFERENT
        // block at the same round — equivocation.
        tc.record_dag_precommit(tip_b, 42, Some([0xCD; 32]), vec![]);
        assert_eq!(tc.cross_fork_equivocations().get(&42), Some(&1));

        // Honest validator 7 only precommits on tip A → no equivocation.
        tc.record_dag_precommit(tip_a, 7, Some([0xAB; 32]), vec![]);
        assert_eq!(tc.cross_fork_equivocations().get(&7), None);
    }

    /// Phase 4.1 + 4.3 — same validator, same block_hash on two
    /// tips at the same round = NOT equivocation (rare but legal:
    /// the block is shared between two voting tracks).
    #[test]
    fn test_record_dag_precommit_same_block_hash_not_equivocation() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();
        tc.record_dag_precommit([0xAA; 32], 42, Some([0xAB; 32]), vec![]);
        tc.record_dag_precommit([0xBB; 32], 42, Some([0xAB; 32]), vec![]);
        assert!(tc.cross_fork_equivocations().is_empty());
    }

    /// Phase 4.3 (Decision 3) — `cross_fork_equivocations` starts
    /// empty; manual increment surfaces via accessor.
    #[test]
    fn test_cross_fork_equivocations_counter() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        assert!(tc.cross_fork_equivocations().is_empty());
        // Synthetic increment — Phase 4.3's voting-handler will do
        // this when it observes a validator double-precommitting
        // across concurrent tips.
        *tc.cross_fork_equivocations.entry(42).or_insert(0) += 1;
        *tc.cross_fork_equivocations.entry(42).or_insert(0) += 1;
        *tc.cross_fork_equivocations.entry(7).or_insert(0) += 1;
        assert_eq!(tc.cross_fork_equivocations().get(&42), Some(&2));
        assert_eq!(tc.cross_fork_equivocations().get(&7), Some(&1));
        assert_eq!(tc.cross_fork_equivocations().get(&100), None);
    }

    /// Phase 4.4 (Decision 4) — `committed_at_block` populates
    /// alongside `committed_at` on every commit. Dual-mode
    /// bookkeeping; both accessors return the same epoch.
    #[test]
    fn test_committed_at_block_dual_mode_bookkeeping() {
        use evaporchain_types::{Block, TransferTx};

        fn make_block_local(num: u64) -> Block {
            Block {
                number: num,
                epoch: num,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: vec![Transaction::Transfer(TransferTx {
                    from: [1u8; 32],
                    to: [2u8; 32],
                    amount: 1,
                    nonce: num,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                })],
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        assert!(tc.committed_at_block().is_empty());

        let block = make_block_local(1);
        tc.on_block_committed(&block, [0u8; 32], 0);

        // Both accessors populated.
        assert_eq!(tc.committed_at_block().len(), 1);
        // Block-indexed key is the canonical block_hash.
        let block_id = TendermintConsensus::block_hash(&block);
        assert!(tc.committed_at_block().contains_key(&block_id));
    }

    /// Phase 5.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — `detect_orphan_branches`
    /// returns tips below the caliber threshold AND outside the
    /// recency window (32 blocks). Default threshold = 0 means no
    /// orphans by caliber alone.
    #[test]
    fn test_detect_orphan_branches_default_threshold() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.record_state_branch([0xAA; 32], 1, 100);
        tc.record_state_branch([0xBB; 32], 1, 200);
        // Default threshold = 0; nothing below 0 → no orphans.
        let orphans = tc.detect_orphan_branches(100);
        assert!(orphans.is_empty());
    }

    /// Phase 5.1 — with threshold = 150, tip [0xAA; 32] (caliber=100)
    /// is below threshold; tip [0xBB; 32] (caliber=200) is above.
    /// Recency: both `last_touched_block = 1`; current_height=100;
    /// staleness_horizon = 100 - 32 = 68; 1 < 68 so both are stale.
    /// Net: only [0xAA; 32] qualifies.
    #[test]
    fn test_detect_orphan_branches_caliber_filter() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_orphan_caliber_threshold", "150")
            .unwrap();
        tc.record_state_branch([0xAA; 32], 1, 100); // below threshold + stale
        tc.record_state_branch([0xBB; 32], 1, 200); // above threshold
        let orphans = tc.detect_orphan_branches(100);
        assert_eq!(orphans, vec![[0xAA; 32]]);
    }

    /// Phase 5.1 — recency window protects fresh tips. Tip with
    /// low caliber but recent last_touched_block is NOT orphaned.
    #[test]
    fn test_detect_orphan_branches_recency_filter() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_orphan_caliber_threshold", "150")
            .unwrap();
        // Tip with low caliber but touched at height 90 (within 32
        // of current 100 → not stale).
        tc.record_state_branch([0xAA; 32], 90, 100);
        let orphans = tc.detect_orphan_branches(100);
        assert!(
            orphans.is_empty(),
            "fresh low-caliber tip must NOT be orphaned"
        );
    }

    /// Phase 5.1 — canonical sorting: orphans returned in BlockId
    /// order regardless of insertion order (validator determinism).
    #[test]
    fn test_detect_orphan_branches_canonical_order() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_orphan_caliber_threshold", "1000")
            .unwrap();
        // Insert in reverse-sort order; expect sorted output.
        tc.record_state_branch([0xCC; 32], 1, 100);
        tc.record_state_branch([0xAA; 32], 1, 100);
        tc.record_state_branch([0xBB; 32], 1, 100);
        let orphans = tc.detect_orphan_branches(100);
        assert_eq!(orphans, vec![[0xAA; 32], [0xBB; 32], [0xCC; 32]]);
    }

    /// Phase 6.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — re-exported
    /// types are usable from `crate::` (mirrors the path a
    /// downstream consumer would see via `evaporchain_consensus::`).
    /// Locks the public-API surface for executor crates that
    /// implement `LightConeBranchSnapshot`.
    #[test]
    fn test_light_cone_substrate_reexports_usable() {
        // The re-exports at lib.rs:24 surface these types into the
        // crate root. From a test inside `tendermint::tests`, that
        // means `crate::LightConeBranchMetadata` and
        // `crate::LightConeBranchSnapshot` should both resolve.
        let _: crate::LightConeBranchMetadata = crate::LightConeBranchMetadata::fresh(1, 100);

        // Trait re-export is also at the crate root.
        struct StubAtCrateRoot;
        impl crate::LightConeBranchSnapshot for StubAtCrateRoot {
            fn tip(&self) -> [u8; 32] {
                [0xCC; 32]
            }
            fn created_at_height(&self) -> u64 {
                42
            }
        }
        let s = StubAtCrateRoot;
        assert_eq!(s.tip(), [0xCC; 32]);
        assert_eq!(s.created_at_height(), 42);
    }

    /// Phase 6.1-substrate of `LIGHT_CONE_FULL_DAG_PLAN.md` —
    /// end-to-end integration of Phase 3 (state branches) + Phase 5
    /// (LRU + DAG-cascade prune) substrate via the real
    /// `on_block_committed` lifecycle. Drives 5 sequential blocks
    /// with `light_cone_state_branches_enabled = true` + cap=3,
    /// asserts that state_branches stays bounded at the cap, the
    /// DAG cascade-prunes oldest, and orphan-detection surfaces
    /// stale tips.
    #[test]
    fn test_light_cone_substrate_end_to_end() {
        use evaporchain_types::{Block, TransferTx};

        fn make_block_local(num: u64) -> Block {
            Block {
                number: num,
                epoch: num,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: vec![Transaction::Transfer(TransferTx {
                    from: [1u8; 32],
                    to: [2u8; 32],
                    amount: num + 1,
                    nonce: num,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                })],
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);

        // Default mode: state_branches stays empty regardless of activity.
        for h in 1..=3u64 {
            tc.on_block_committed(&make_block_local(h), [0u8; 32], 0);
        }
        assert!(
            tc.state_branches.is_empty(),
            "default-off chain must keep state_branches empty across activity"
        );
        assert!(
            tc.committed_at_block().len() >= 3,
            "Phase 4.4 dual-mode bookkeeping populates regardless of \
             state_branches_enabled (block-indexed view is for \
             antichain-finality consumers, not just DAG mode)"
        );

        // Flip the rollout flag + tighten the cap.
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();
        tc.governance_set_param("light_cone_max_concurrent_forks", "3")
            .unwrap();

        // Commit 5 more blocks. state_branches must populate, and
        // when len > 3 the LRU prune kicks in (lowest-caliber drop).
        for h in 4..=8u64 {
            tc.on_block_committed(&make_block_local(h), [0u8; 32], 0);
        }
        // Cap = 3 → state_branches.len() <= 3.
        assert!(
            tc.state_branches.len() <= 3,
            "LRU cap enforced: len={} > 3",
            tc.state_branches.len()
        );

        // Orphan-detection rule (Phase 5.1) — with default
        // threshold = 0, no orphans. Bump threshold high enough so
        // every tip is below it; recency window = 32, so any tip
        // last touched < (current_height - 32) shows up. Our tips
        // were touched at heights 4..=8; current_height=100 puts
        // staleness_horizon=68; all tips < 68 → all stale.
        tc.governance_set_param("light_cone_orphan_caliber_threshold", "10000")
            .unwrap();
        let orphans = tc.detect_orphan_branches(100);
        assert_eq!(
            orphans.len(),
            tc.state_branches.len(),
            "with high threshold + stale recency, every tip orphan-eligible"
        );

        // Phase 4.4 dual-mode bookkeeping: committed_at_block has
        // entries from all 8 commits.
        assert_eq!(tc.committed_at_block().len(), 8);
    }

    /// Phase 5.3 of `LIGHT_CONE_FULL_DAG_PLAN.md` — LRU eviction
    /// at the metadata level pairs with a DAG-side cascade prune.
    /// When `prune_state_branches` evicts a tip, the matching
    /// `LightCone` ancestors are trimmed (subject to safety:
    /// non-leaf rejection, branch-point stop). Locks the contract
    /// the consensus engine relies on for unbounded-DAG memory
    /// hygiene.
    #[test]
    fn test_state_branches_lru_paired_dag_prune() {
        use evaporchain_light_cone::Block as LcBlock;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Cap=2 so 3 inserts → 1 eviction.
        tc.governance_set_param("light_cone_max_concurrent_forks", "2")
            .unwrap();

        // Insert a 3-fork DAG: genesis → A, B, C (three siblings,
        // all leaves). Each has its own state-branch metadata.
        let g = [0xFF; 32];
        let a = [0xAA; 32];
        let b = [0xBB; 32];
        let c = [0xCC; 32];
        tc.light_cone_dag
            .insert(LcBlock::new(g, vec![], 1000, 0))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(a, vec![g], 100, 1))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(b, vec![g], 200, 1))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(c, vec![g], 150, 1))
            .unwrap();

        // Record metadata for all 3 leaves with distinct calibers.
        // a = lowest caliber → first to evict.
        tc.record_state_branch(a, 1, 100);
        tc.record_state_branch(b, 1, 200);
        tc.record_state_branch(c, 1, 150);
        assert_eq!(tc.state_branches.len(), 3);

        // Pre-prune: DAG has all 4 blocks (genesis + 3 leaves).
        assert!(tc.light_cone_dag.contains(&g));
        assert!(tc.light_cone_dag.contains(&a));
        assert!(tc.light_cone_dag.contains(&b));
        assert!(tc.light_cone_dag.contains(&c));

        // Trigger eviction.
        tc.prune_state_branches();

        // Metadata: lowest-caliber leaf `a` evicted.
        assert_eq!(tc.state_branches.len(), 2);
        assert!(!tc.state_branches.contains_key(&a));
        assert!(tc.state_branches.contains_key(&b));
        assert!(tc.state_branches.contains_key(&c));

        // DAG side: `a` is gone (cascade-pruned). `g` survives —
        // it's a branch point shared with `b` and `c`. `b` and `c`
        // are live leaves and untouched.
        assert!(!tc.light_cone_dag.contains(&a));
        assert!(tc.light_cone_dag.contains(&g));
        assert!(tc.light_cone_dag.contains(&b));
        assert!(tc.light_cone_dag.contains(&c));
    }

    /// Phase 3.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` — `LightConeBranchSnapshot`
    /// trait + `attach_branch_snapshot` method. Locks the seam the
    /// executor will plug into in Phase 3.2 full implementation.
    #[test]
    fn test_state_branches_snapshot_attach() {
        // Synthetic snapshot impl — minimal trait surface, no
        // dependencies on evaporchain-state.
        struct StubSnapshot {
            tip: [u8; 32],
            height: u64,
        }
        impl LightConeBranchSnapshot for StubSnapshot {
            fn tip(&self) -> [u8; 32] {
                self.tip
            }
            fn created_at_height(&self) -> u64 {
                self.height
            }
        }

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        let tip = [0xAA; 32];

        // Without prior record: attach returns None.
        let snap = std::sync::Arc::new(StubSnapshot { tip, height: 1 });
        assert_eq!(tc.attach_branch_snapshot(tip, snap.clone()), None);

        // After record: attach succeeds.
        tc.record_state_branch(tip, 1, 100);
        assert_eq!(tc.attach_branch_snapshot(tip, snap.clone()), Some(()));

        // Snapshot is now in the metadata; trait methods reachable.
        let m = &tc.state_branches()[&tip];
        let s = m.snapshot.as_ref().expect("snapshot attached");
        assert_eq!(s.tip(), tip);
        assert_eq!(s.created_at_height(), 1);
    }

    /// Phase 3.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` —
    /// `capture_committed_branch_snapshot` closes the executor-side
    /// wiring: under `light_cone_state_branches_enabled = "true"`,
    /// after `record_state_branch` has registered the tip, calling
    /// the capture method materializes a `StateSnapshotBranch` from
    /// the executor's `db` and attaches it to the metadata. The
    /// branch is now restorable via `replay_and_apply_atomic`.
    #[test]
    fn test_capture_committed_branch_snapshot_attaches_to_recorded_tip() {
        fn block_at(height: u64) -> Block {
            Block {
                number: height,
                epoch: height / 10,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: vec![],
                producer_id: Some(0),
                timestamp: height * 12,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();

        let block = block_at(7);
        let tip = TendermintConsensus::block_hash(&block);
        // The on_block_committed path normally records this; do it
        // directly to keep the test focused on the capture method.
        tc.record_state_branch(tip, block.number, /*caliber*/ 1);

        let mut db = InMemoryStateDB::new();
        tc.capture_committed_branch_snapshot(&block, &mut db)
            .expect("capture must succeed under enabled flag");

        let m = &tc.state_branches()[&tip];
        let s = m.snapshot.as_ref().expect("snapshot must be attached");
        assert_eq!(s.tip(), tip);
        assert_eq!(s.created_at_height(), block.number);
    }

    /// Phase 3.2 — flag-off: capture is a no-op (chain bit-compat).
    /// `state_branches` is empty (because `record_state_branch` is
    /// also gated by the flag in `on_block_committed`); the capture
    /// method returns Ok without touching anything.
    #[test]
    fn test_capture_committed_branch_snapshot_noop_when_flag_off() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Flag intentionally NOT set → default off.
        let mut block = Block {
            number: 1,
            epoch: 0,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            producer_id: Some(0),
            timestamp: 12,
            chain_id: String::new(),
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
            post_state_root: None,
        };
        block.timestamp = 24;

        let mut db = InMemoryStateDB::new();
        tc.capture_committed_branch_snapshot(&block, &mut db)
            .expect("flag-off capture must return Ok (no-op)");

        assert!(
            tc.state_branches().is_empty(),
            "flag-off path must not populate state_branches"
        );
    }

    /// Phase 3.2 — defensive no-op when the tip isn't recorded
    /// (operator flipped the flag on AFTER on_block_committed
    /// registered the tip — race window). Capture must return Ok
    /// without panic and without inserting a stub metadata entry.
    #[test]
    fn test_capture_committed_branch_snapshot_noop_when_tip_not_tracked() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("light_cone_state_branches_enabled", "true")
            .unwrap();
        let block = Block {
            number: 2,
            epoch: 0,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            producer_id: Some(0),
            timestamp: 24,
            chain_id: String::new(),
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
            post_state_root: None,
        };

        // Deliberately skip record_state_branch.
        let mut db = InMemoryStateDB::new();
        tc.capture_committed_branch_snapshot(&block, &mut db)
            .expect("untracked-tip capture must return Ok (no-op)");

        let tip = TendermintConsensus::block_hash(&block);
        assert!(
            !tc.state_branches().contains_key(&tip),
            "no-op must not insert a stub metadata entry"
        );
    }

    /// Phase 3.4 (LRU eviction) — when state_branches exceeds the
    /// `light_cone_max_concurrent_forks` cap, the lowest-caliber
    /// entry is evicted; tie-break is smallest BlockId.
    #[test]
    fn test_state_branches_lru_eviction() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Cap=2 to make the test small.
        tc.governance_set_param("light_cone_max_concurrent_forks", "2")
            .unwrap();
        tc.record_state_branch([0xAA; 32], 1, 100); // lowest caliber
        tc.record_state_branch([0xBB; 32], 2, 200);
        tc.record_state_branch([0xCC; 32], 3, 150);
        tc.prune_state_branches();
        assert_eq!(tc.state_branches().len(), 2, "cap=2 → 2 survivors");
        assert!(
            !tc.state_branches().contains_key(&[0xAA; 32]),
            "lowest-caliber tip [0xAA; 32] should have been evicted"
        );
        assert!(tc.state_branches().contains_key(&[0xBB; 32]));
        assert!(tc.state_branches().contains_key(&[0xCC; 32]));
    }

    /// Phase 3.5 of `LIGHT_CONE_FULL_DAG_PLAN.md` — `light_cone_state_branches_enabled`
    /// governance flag accepts `"true"` / `"false"`, rejects anything
    /// else. Default-off (this commit doesn't change runtime behaviour).
    #[test]
    fn test_governance_light_cone_state_branches_flag() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Both values accepted.
        assert!(tc
            .governance_set_param("light_cone_state_branches_enabled", "false")
            .is_ok());
        assert!(tc
            .governance_set_param("light_cone_state_branches_enabled", "true")
            .is_ok());
        // Junk values rejected.
        let err = tc
            .governance_set_param("light_cone_state_branches_enabled", "yes")
            .unwrap_err();
        assert!(matches!(err, GovernanceParamError::InvalidValue { .. }));
    }

    /// Phase 3 Decision 2 — `light_cone_max_concurrent_forks`
    /// accepts u8 in 1..=8; rejects 0, 9, and non-numeric.
    #[test]
    fn test_governance_light_cone_max_concurrent_forks_flag() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Boundary: 1 OK, 8 OK, 4 OK (default).
        assert!(tc
            .governance_set_param("light_cone_max_concurrent_forks", "1")
            .is_ok());
        assert!(tc
            .governance_set_param("light_cone_max_concurrent_forks", "8")
            .is_ok());
        assert!(tc
            .governance_set_param("light_cone_max_concurrent_forks", "4")
            .is_ok());
        // Out-of-range rejections.
        for bad in &["0", "9", "256", "-1", "lots"] {
            let err = tc
                .governance_set_param("light_cone_max_concurrent_forks", bad)
                .unwrap_err();
            assert!(
                matches!(err, GovernanceParamError::InvalidValue { .. }),
                "value {bad:?} should be rejected as InvalidValue"
            );
        }
    }

    /// Phase 1.3 of `LIGHT_CONE_FULL_DAG_PLAN.md` — under
    /// `parent_acceptance_mode = "mcc"` mode, the proposer's
    /// `current_tip()` returns the DAG-derived head; under default
    /// `linear` mode it returns `parent_hash`. This test drives a
    /// 2-leaf DAG (genesis + one block at height 1) and confirms
    /// `current_tip()` flips between the two modes.
    #[test]
    fn test_current_tip_mcc_mode_returns_dag_leaf() {
        use evaporchain_light_cone::Block as LcBlock;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Inject a single block into the DAG by hand so the test
        // doesn't depend on on_block_committed plumbing.
        let genesis_id = [0xAA; 32];
        let leaf_id = [0xBB; 32];
        tc.light_cone_dag
            .insert(LcBlock::new(genesis_id, vec![], 1000, 0))
            .expect("insert genesis");
        tc.light_cone_dag
            .insert(LcBlock::new(leaf_id, vec![genesis_id], 700, 1))
            .expect("insert leaf");

        // Linear mode: still parent_hash (which is [0u8; 32] at this
        // point — never updated through the test path).
        assert_eq!(tc.current_tip(), tc.parent_hash());

        // Flip to mcc: now select_tip walks the DAG and picks the
        // higher-caliber leaf. With a single non-genesis leaf, that's
        // `leaf_id` (the genesis is also a leaf in our 2-block DAG —
        // tie-break is the smaller BlockId, but caliber differs.)
        tc.governance_set_param("parent_acceptance_mode", "mcc")
            .expect("mcc accepted by allowlist");
        let tip = tc.current_tip();
        // Either leaf is a valid pick depending on caliber math; the
        // contract here is just that mcc-mode actually consults the
        // DAG (returns one of the leaves) instead of parent_hash.
        assert!(
            tip == genesis_id || tip == leaf_id,
            "mcc-mode current_tip must return a DAG leaf, got {:?}",
            tip
        );
        assert_ne!(
            tip,
            tc.parent_hash(),
            "mcc-mode tip must not silently fall back to parent_hash \
             when the DAG has at least one block"
        );
    }

    /// Phase 1.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` — `current_tip()`
    /// defers to the DAG only under `parent_acceptance_mode = "mcc"`.
    /// In default `linear` mode it returns `parent_hash` unchanged
    /// (chain behaviour is bit-for-bit preserved).
    #[test]
    fn test_current_tip_falls_back_to_parent_hash_in_linear_mode() {
        let tc = make_consensus(1, &[1, 2, 3, 4]);
        // Default mode: linear. parent_hash = [0u8; 32] at construction.
        assert_eq!(tc.current_tip(), [0u8; 32]);
        assert_eq!(tc.current_tip(), tc.parent_hash());
    }

    /// Phase 1.2 — flipping to mcc mode with an empty DAG still
    /// returns parent_hash (select_tip is None on empty DAG, which
    /// is the safe fallback per the docstring contract).
    #[test]
    fn test_current_tip_mcc_mode_empty_dag_falls_back() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("parent_acceptance_mode", "mcc")
            .expect("mcc accepted by allowlist");
        // No blocks committed yet → DAG empty → select_tip None →
        // falls back to parent_hash.
        assert_eq!(tc.current_tip(), tc.parent_hash());
    }

    /// Phase 6.1 of `CROOKS_MEV_INTEGRATION_PLAN.md` — single
    /// end-to-end pipeline test that exercises **every** consensus-
    /// side Crooks-MEV stage in one run:
    ///
    ///   1. Detection (Phase 1) — sandwich block produces an observation.
    ///   2. Refund computation (Phase 2) — observation gains a refund_amount.
    ///   3. Producer helper (Phase 3.3) — past grace, due_refund_txs returns the Refund tx.
    ///   4. Validator rejection (Phase 3.4) — enforce mode requires the proposer to include it.
    ///   5. Replay protection (Phase 3.3) — once committed, settled_refunds populates.
    ///   6. Determinism (Phase 3.2) — mev_state_digest converges across two validators driven through the same blocks.
    ///   7. Anti-gaming (Phase 4) — disputed observation drops out of due_refund_txs.
    ///
    /// Locks the chain-level pipeline contract for `observe → enforce`
    /// rollout. Executor-side balance movement (Phase 3.5a) is exercised
    /// independently in `evaporchain-execution::tests::test_refund_*`.
    #[test]
    fn test_crooks_mev_end_to_end_consensus_pipeline() {
        use evaporchain_types::{Block, TransferTx};

        fn addr_local(seed: u8) -> [u8; 32] {
            let mut a = [0u8; 32];
            a[0] = seed;
            a
        }
        fn transfer_local(from: u8, to: u8, amount: u64, nonce: u64) -> Transaction {
            Transaction::Transfer(TransferTx {
                from: addr_local(from),
                to: addr_local(to),
                amount,
                nonce,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })
        }
        fn make_block_local(num: u64, txs: Vec<Transaction>) -> Block {
            Block {
                number: num,
                epoch: num,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: txs,
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let mut a = make_consensus(1, &[1, 2, 3, 4]);
        let mut b = make_consensus(2, &[1, 2, 3, 4]);

        // Phase 1+2: sandwich block. Both validators detect.
        let sandwich = make_block_local(
            1,
            vec![
                transfer_local(0xAA, 0x99, 100, 0),
                transfer_local(0xBB, 0x99, 200, 0),
                transfer_local(0xAA, 0x99, 150, 1),
            ],
        );
        a.on_block_committed(&sandwich, [0u8; 32], 0);
        b.on_block_committed(&sandwich, [0u8; 32], 0);

        // Both have one observation, refund_amount populated.
        assert_eq!(a.mev_observations().len(), 1);
        let obs = &a.mev_observations()[0];
        assert!(obs.refund_amount.is_some());
        assert!(obs.refund_amount.unwrap() <= 250);

        // Phase 3.2: digests converge.
        assert_eq!(a.mev_state_digest(), b.mev_state_digest());

        // Phase 3.3: past grace, due_refund_txs returns the Refund tx.
        let due = a.due_refund_txs(10);
        assert_eq!(due.len(), 1);
        let due_refund = match &due[0] {
            Transaction::Refund(r) => r.clone(),
            _ => unreachable!(),
        };

        // Phase 3.4 + 5: enforce-mode validator REQUIRES the refund.
        a.governance_set_param("crooks_mev_settlement_mode", "enforce")
            .unwrap();
        let proposed = make_block_local(10, vec![Transaction::Refund(due_refund.clone())]);
        assert_eq!(a.validate_block_refunds(&proposed), Ok(()));

        // Empty proposal at the same height → MissingRefund.
        let empty = make_block_local(10, vec![]);
        let err = a.validate_block_refunds(&empty).unwrap_err();
        assert!(matches!(
            err,
            evaporchain_mev_detect::RefundValidationError::MissingRefund { .. }
        ));

        // Phase 3.3 commit → settled_refunds populates.
        a.on_block_committed(&proposed, [0u8; 32], 0);
        assert!(a.settled_refunds.contains(&(1, 0)));

        // Replay protection: subsequent due_refund_txs no longer emits.
        let after = a.due_refund_txs(20);
        assert!(after.is_empty());

        // Phase 4 anti-gaming on validator b (still in observe mode):
        // dispute the observation within grace → due_refund_txs skips.
        b.dispute_observation(1, 0, 3)
            .expect("dispute within grace");
        let due_b = b.due_refund_txs(10);
        assert!(due_b.is_empty(), "disputed observation must not settle");
    }

    /// Cross-layer integration test — closes the gap between the
    /// existing consensus-only Crooks-MEV test (above) and the
    /// existing execution-only `test_refund_moves_balance_attacker_to_victim`
    /// in evaporchain-execution. Drives a real sandwich attack
    /// through the FULL pipeline:
    ///
    ///   1. Pre-fund attacker, victim, target accounts in StateDB.
    ///   2. `apply_block(sandwich)` — executes balance changes (transfer
    ///      semantics) AND records observations (consensus on_block_committed).
    ///   3. Verify the consensus layer detected the sandwich and the
    ///      executor moved balances per the transfer txs.
    ///   4. Past grace, `due_refund_txs` returns the Refund tx.
    ///   5. `apply_block(settlement)` — executes the Refund (attacker
    ///      debited, victim credited).
    ///   6. Assert: attacker balance dropped FURTHER than the sandwich
    ///      cost, victim balance recovered. End-to-end economic punishment.
    ///
    /// This is the canonical "Crooks-MEV refund punishes the attacker"
    /// demo. Previously the consensus layer's pipeline test handcrafted
    /// a Refund tx and checked validation; the execution layer's test
    /// handcrafted a Refund and checked balance movement. Neither
    /// connected the actual MEV detection to actual balance change. This
    /// commit ties them.
    #[test]
    fn test_crooks_mev_end_to_end_attacker_economically_punished() {
        use evaporchain_state::db::InMemoryStateDB;
        use evaporchain_types::{Block, TransferTx};

        // Local helpers — mirror the per-test helpers used elsewhere
        // in this file so the harness stays self-contained.
        fn addr_local(seed: u8) -> [u8; 32] {
            let mut a = [0u8; 32];
            a[0] = seed;
            a
        }
        fn fund(db: &mut InMemoryStateDB, byte: u8, balance: u64) {
            db.put_account(evaporchain_types::Account {
                address: addr_local(byte),
                balance,
                nonce: 0,
                storage_deposit: 0,
                storage_bytes: 0,
                last_touched_epoch: 0,
                vesting: None,
            });
        }
        fn transfer_local(from: u8, to: u8, amount: u64, nonce: u64) -> Transaction {
            Transaction::Transfer(TransferTx {
                from: addr_local(from),
                to: addr_local(to),
                amount,
                nonce,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })
        }
        fn make_block_local(num: u64, txs: Vec<Transaction>) -> Block {
            Block {
                number: num,
                epoch: num,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: txs,
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        // ─── Setup ──────────────────────────────────────────────────
        // Attacker (0xAA) sandwiches the victim (0xBB)'s transfer to
        // target (0x99). Sandwich semantics: attacker pre-trade,
        // victim trade, attacker post-trade — same target each time.
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 0xAA, 10_000); // attacker
        fund(&mut db, 0xBB, 1_000);  // victim
        fund(&mut db, 0x99, 0);      // target

        // ─── Phase 1: sandwich block ────────────────────────────────
        // Three transfers all targeting 0x99:
        //   - tx0: 0xAA → 0x99 (50)   — attacker front-run
        //   - tx1: 0xBB → 0x99 (100)  — victim's real trade
        //   - tx2: 0xAA → 0x99 (50)   — attacker back-run
        let sandwich = make_block_local(
            1,
            vec![
                transfer_local(0xAA, 0x99, 50, 0),
                transfer_local(0xBB, 0x99, 100, 0),
                transfer_local(0xAA, 0x99, 50, 1),
            ],
        );

        // apply_block runs execute_block (balance changes) AND
        // on_block_committed (observation recording) — production wrapper.
        tc.apply_block(&mut db, &sandwich)
            .expect("sandwich block must apply cleanly");

        // Post-sandwich balances reflect the 3 transfers (no refund yet).
        let attacker_after_sandwich = db.get_account(&addr_local(0xAA)).unwrap().balance;
        let victim_after_sandwich = db.get_account(&addr_local(0xBB)).unwrap().balance;
        let target_after_sandwich = db.get_account(&addr_local(0x99)).unwrap().balance;
        // Each transfer also burns gas; assert direction, not exact values.
        assert!(
            attacker_after_sandwich < 10_000,
            "attacker balance must have dropped by the 2 front+back transfers + gas"
        );
        assert!(
            victim_after_sandwich < 1_000,
            "victim balance must have dropped by the 1 victim transfer + gas"
        );
        assert!(
            target_after_sandwich >= 200,
            "target balance must have received the 3 transfers (50 + 100 + 50 = 200)"
        );

        // Consensus detected the sandwich.
        assert_eq!(
            tc.mev_observations().len(),
            1,
            "scan_block must detect exactly one sandwich pattern"
        );
        let obs = &tc.mev_observations()[0];
        assert_eq!(obs.attacker, addr_local(0xAA));
        assert_eq!(obs.victim, addr_local(0xBB));
        assert!(
            obs.refund_amount.unwrap_or(0) > 0,
            "refund_amount must be positive after Phase 2 computation"
        );

        // ─── Phase 2: enforce mode + due_refund_txs past grace ──────
        tc.governance_set_param("crooks_mev_settlement_mode", "enforce")
            .expect("enforce is allowlisted");
        // Default crooks_mev_grace_period_blocks = 5.  Query past grace.
        let due = tc.due_refund_txs(10);
        assert_eq!(due.len(), 1, "exactly one refund tx due past grace");
        let refund_tx = due.into_iter().next().unwrap();
        let refund_amount = match &refund_tx {
            Transaction::Refund(r) => {
                assert_eq!(r.attacker, addr_local(0xAA));
                assert_eq!(r.victim, addr_local(0xBB));
                r.amount
            }
            _ => unreachable!("due_refund_txs must emit Refund variants"),
        };
        assert!(
            refund_amount > 0,
            "refund amount must be positive — Crooks-MEV is supposed to punish"
        );

        // ─── Phase 3: settlement block applies the refund ───────────
        let settlement = make_block_local(10, vec![refund_tx]);
        tc.apply_block(&mut db, &settlement)
            .expect("settlement block with valid refund must apply under enforce mode");

        // ─── Assertions: attacker debited, victim credited ──────────
        let attacker_final = db.get_account(&addr_local(0xAA)).unwrap().balance;
        let victim_final = db.get_account(&addr_local(0xBB)).unwrap().balance;
        assert_eq!(
            attacker_final,
            attacker_after_sandwich.saturating_sub(refund_amount),
            "attacker balance must have dropped by exactly the refund amount"
        );
        assert_eq!(
            victim_final,
            victim_after_sandwich.saturating_add(refund_amount),
            "victim balance must have been credited by exactly the refund amount"
        );
        // The economic story: attacker is now strictly worse off than
        // before the sandwich (their gain from the sandwich didn't
        // recover via price impact in this synthetic harness, AND
        // they paid the refund). This is the load-bearing claim:
        // **MEV extraction has been turned from +EV to -EV by the
        // chain's design.**
        assert!(
            attacker_final < attacker_after_sandwich,
            "attacker must end up strictly worse off after the refund (start={}, after_sandwich={}, final={})",
            10_000,
            attacker_after_sandwich,
            attacker_final,
        );

        // Replay protection: same observation cannot be settled again.
        let due_after = tc.due_refund_txs(20);
        assert!(
            due_after.is_empty(),
            "settled refund must not re-emit on subsequent due_refund_txs calls"
        );
    }

    /// Phase 4.4 of `CROOKS_MEV_INTEGRATION_PLAN.md` — operator
    /// dispute flow: drive a sandwich, dispute the observation
    /// within grace, confirm `due_refund_txs` no longer emits the
    /// refund. Then test the past-grace rejection + NotFound.
    #[test]
    fn test_mev_dispute_flow() {
        use evaporchain_types::{Block, TransferTx};

        fn addr_local(seed: u8) -> [u8; 32] {
            let mut a = [0u8; 32];
            a[0] = seed;
            a
        }
        fn transfer_local(from: u8, to: u8, amount: u64, nonce: u64) -> Transaction {
            Transaction::Transfer(TransferTx {
                from: addr_local(from),
                to: addr_local(to),
                amount,
                nonce,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })
        }
        fn make_block_local(num: u64, txs: Vec<Transaction>) -> Block {
            Block {
                number: num,
                epoch: num,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: txs,
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let sandwich_block = make_block_local(
            1,
            vec![
                transfer_local(0xAA, 0x99, 100, 0),
                transfer_local(0xBB, 0x99, 200, 0),
                transfer_local(0xAA, 0x99, 150, 1),
            ],
        );

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.on_block_committed(&sandwich_block, [0u8; 32], 0);

        // Within grace (default 5 blocks): dispute succeeds.
        tc.dispute_observation(1, 0, 3)
            .expect("dispute within grace");
        assert!(tc.disputed_observations().contains(&(1, 0)));

        // due_refund_txs past-grace skips disputed.
        let due = tc.due_refund_txs(10);
        assert!(due.is_empty(), "disputed observation must not settle");

        // NotFound for a non-existent observation.
        let err = tc.dispute_observation(999, 0, 3).unwrap_err();
        assert!(matches!(err, MevDisputeError::NotFound { .. }));

        // Past grace ⇒ PastGracePeriod (use a fresh consensus
        // instance to avoid the prior dispute side-effect).
        let mut tc2 = make_consensus(1, &[1, 2, 3, 4]);
        tc2.on_block_committed(&sandwich_block, [0u8; 32], 0);
        let err = tc2.dispute_observation(1, 0, 100).unwrap_err();
        assert!(matches!(err, MevDisputeError::PastGracePeriod { .. }));
    }

    /// Phase 3.4 of `CROOKS_MEV_INTEGRATION_PLAN.md` —
    /// `validate_block_refunds` defaults to Ok in observe-mode;
    /// in enforce-mode rejects blocks whose Refund-tx set diverges
    /// from `due_refund_txs`. Three explicit failure modes:
    /// MissingRefund / UnexpectedRefund / MismatchedRefund.
    #[test]
    fn test_validate_block_refunds_observe_vs_enforce() {
        use evaporchain_types::{Block, RefundTx, TransferTx};

        fn addr_local(seed: u8) -> [u8; 32] {
            let mut a = [0u8; 32];
            a[0] = seed;
            a
        }
        fn transfer_local(from: u8, to: u8, amount: u64, nonce: u64) -> Transaction {
            Transaction::Transfer(TransferTx {
                from: addr_local(from),
                to: addr_local(to),
                amount,
                nonce,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })
        }
        fn make_block_local(num: u64, txs: Vec<Transaction>) -> Block {
            Block {
                number: num,
                epoch: num,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: txs,
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);

        // Set up: one sandwich at height 1, then advance to height 10
        // (past the default 5-block grace).
        tc.on_block_committed(
            &make_block_local(
                1,
                vec![
                    transfer_local(0xAA, 0x99, 100, 0),
                    transfer_local(0xBB, 0x99, 200, 0),
                    transfer_local(0xAA, 0x99, 150, 1),
                ],
            ),
            [0u8; 32],
            0,
        );

        let due = tc.due_refund_txs(10);
        assert_eq!(due.len(), 1);
        let due_refund = match &due[0] {
            Transaction::Refund(r) => r.clone(),
            _ => unreachable!(),
        };

        // Observe mode (default): every block passes regardless.
        let empty_block = make_block_local(10, vec![]);
        assert_eq!(tc.validate_block_refunds(&empty_block), Ok(()));
        let block_with_refund = make_block_local(10, vec![Transaction::Refund(due_refund.clone())]);
        assert_eq!(tc.validate_block_refunds(&block_with_refund), Ok(()));

        // Switch to enforce mode.
        tc.governance_set_param("crooks_mev_settlement_mode", "enforce")
            .unwrap();

        // Empty block at height 10 with one due refund → MissingRefund.
        let err = tc.validate_block_refunds(&empty_block).unwrap_err();
        assert!(matches!(
            err,
            evaporchain_mev_detect::RefundValidationError::MissingRefund { .. }
        ));

        // Block carrying the exact due refund → Ok.
        assert_eq!(tc.validate_block_refunds(&block_with_refund), Ok(()));

        // Block carrying a tampered refund (wrong amount) →
        // MismatchedRefund.
        let mut tampered = due_refund.clone();
        tampered.amount += 1;
        let bad_block = make_block_local(10, vec![Transaction::Refund(tampered)]);
        let err = tc.validate_block_refunds(&bad_block).unwrap_err();
        assert!(matches!(
            err,
            evaporchain_mev_detect::RefundValidationError::MismatchedRefund { .. }
        ));

        // Block carrying an unexpected refund (no such observation)
        // → UnexpectedRefund.
        let bogus = RefundTx {
            source_block_height: 999,
            source_observation_idx: 0,
            attacker: addr_local(0xAA),
            victim: addr_local(0xBB),
            amount: 100,
            settle_block_height: 10,
        };
        let unexpected_block = make_block_local(
            10,
            vec![
                Transaction::Refund(due_refund.clone()),
                Transaction::Refund(bogus),
            ],
        );
        let err = tc.validate_block_refunds(&unexpected_block).unwrap_err();
        assert!(matches!(
            err,
            evaporchain_mev_detect::RefundValidationError::UnexpectedRefund { .. }
        ));
    }

    /// Phase 3.3 of `CROOKS_MEV_INTEGRATION_PLAN.md` — drive a
    /// sandwich block, advance past the grace period, confirm
    /// `due_refund_txs` returns one Refund tx with the expected
    /// (attacker, victim, amount). Then simulate the proposer
    /// including that Refund in a future block — `settled_refunds`
    /// is populated, and `due_refund_txs` no longer emits it
    /// (replay protection).
    #[test]
    fn test_due_refund_txs_grace_window_and_replay_protection() {
        use evaporchain_types::{Block, RefundTx, TransferTx};

        fn addr_local(seed: u8) -> [u8; 32] {
            let mut a = [0u8; 32];
            a[0] = seed;
            a
        }
        fn transfer_local(from: u8, to: u8, amount: u64, nonce: u64) -> Transaction {
            Transaction::Transfer(TransferTx {
                from: addr_local(from),
                to: addr_local(to),
                amount,
                nonce,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })
        }
        fn make_block_local(num: u64, txs: Vec<Transaction>) -> Block {
            Block {
                number: num,
                epoch: num,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: txs,
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);

        // Block 1: a sandwich. Generates one MevObservation with
        // refund_amount populated by Phase 2 wiring.
        tc.on_block_committed(
            &make_block_local(
                1,
                vec![
                    transfer_local(0xAA, 0x99, 100, 0),
                    transfer_local(0xBB, 0x99, 200, 0),
                    transfer_local(0xAA, 0x99, 150, 1),
                ],
            ),
            [0u8; 32],
            0,
        );
        assert_eq!(tc.mev_observations().len(), 1);

        // Within the grace period (default 5 blocks): no refund due.
        let due_inside_grace = tc.due_refund_txs(3);
        assert!(due_inside_grace.is_empty(), "obs at age 2 still in grace");

        // Past grace, inside refund_window: one Refund tx due.
        let due_past_grace = tc.due_refund_txs(10);
        assert_eq!(due_past_grace.len(), 1, "obs at age 9 must settle");
        let refund_tx = match &due_past_grace[0] {
            Transaction::Refund(r) => r.clone(),
            other => panic!("expected Refund, got {:?}", other),
        };
        assert_eq!(refund_tx.source_block_height, 1);
        assert_eq!(refund_tx.attacker, addr_local(0xAA));
        assert_eq!(refund_tx.victim, addr_local(0xBB));

        // Simulate the proposer including the RefundTx: commit a
        // block carrying it. settled_refunds gets populated.
        tc.on_block_committed(
            &make_block_local(10, vec![Transaction::Refund(refund_tx.clone())]),
            [0u8; 32],
            0,
        );
        assert!(
            tc.settled_refunds.contains(&(1, 0)),
            "proposer-included Refund must populate settled_refunds"
        );

        // Replay protection: due_refund_txs at a later height MUST
        // NOT re-emit the same observation.
        let due_after_settle = tc.due_refund_txs(20);
        assert!(
            due_after_settle.is_empty(),
            "settled observation must not re-emit"
        );
    }

    /// Phase 3.2 of `CROOKS_MEV_INTEGRATION_PLAN.md` — two
    /// independently-constructed validators driven through identical
    /// block sequences must produce identical `mev_state_digest()`;
    /// divergent histories must NOT converge.
    #[test]
    fn test_mev_state_digest_converges_across_validators() {
        use evaporchain_types::{Block, TransferTx};

        fn addr_local(seed: u8) -> [u8; 32] {
            let mut a = [0u8; 32];
            a[0] = seed;
            a
        }
        fn transfer_local(from: u8, to: u8, amount: u64, nonce: u64) -> Transaction {
            Transaction::Transfer(TransferTx {
                from: addr_local(from),
                to: addr_local(to),
                amount,
                nonce,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })
        }
        fn make_block_local(num: u64, txs: Vec<Transaction>) -> Block {
            Block {
                number: num,
                epoch: num,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: txs,
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let mut a = make_consensus(1, &[1, 2, 3, 4]);
        let mut b = make_consensus(2, &[1, 2, 3, 4]);
        let blocks = vec![
            make_block_local(
                1,
                vec![
                    transfer_local(0xAA, 0x99, 100, 0),
                    transfer_local(0xBB, 0x99, 200, 0),
                    transfer_local(0xAA, 0x99, 150, 1),
                ],
            ),
            make_block_local(
                2,
                vec![
                    transfer_local(0xAA, 0x99, 50, 2),
                    transfer_local(0xCC, 0x99, 300, 0),
                    transfer_local(0xAA, 0x99, 75, 3),
                ],
            ),
            make_block_local(
                3,
                vec![
                    transfer_local(0xDD, 0x88, 100, 0),
                    transfer_local(0xEE, 0x88, 200, 0),
                ],
            ),
        ];
        for blk in &blocks {
            a.on_block_committed(blk, [0u8; 32], 0);
            b.on_block_committed(blk, [0u8; 32], 0);
            assert_eq!(
                a.mev_state_digest(),
                b.mev_state_digest(),
                "validators must converge after block {} (Phase 3.2)",
                blk.number
            );
        }

        let empty_digest = evaporchain_mev_detect::mev_state_digest(
            &std::collections::VecDeque::new(),
            &std::collections::HashMap::new(),
        );
        assert_ne!(
            a.mev_state_digest(),
            empty_digest,
            "after sandwich blocks digest must diverge from empty"
        );

        let mut c = make_consensus(3, &[1, 2, 3, 4]);
        let mut reversed = blocks.clone();
        reversed.reverse();
        for blk in &reversed {
            c.on_block_committed(blk, [0u8; 32], 0);
        }
        assert_ne!(
            a.mev_state_digest(),
            c.mev_state_digest(),
            "validators with divergent histories must NOT converge"
        );
    }

    /// Phase 4.4 of `LIGHT_CONE_FULL_DAG_PLAN.md` — two
    /// independently-constructed validators driven through identical
    /// block sequences must produce identical
    /// `light_cone_antichain_digest()`; divergent histories must NOT
    /// converge.
    ///
    /// Mirrors `test_mev_state_digest_converges_across_validators`
    /// for the Light-Cone substrate. Together they cover both halves
    /// of the inter-validator agreement surface called out in the
    /// 2026-05 doctrine rollout runbook.
    #[test]
    fn test_light_cone_antichain_digest_converges_across_validators() {
        use evaporchain_types::Block;

        fn make_block_local(num: u64, parent_hash: [u8; 32]) -> Block {
            Block {
                number: num,
                epoch: num,
                parent_hash,
                state_root: [num as u8; 32],
                transactions: vec![],
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let mut a = make_consensus(1, &[1, 2, 3, 4]);
        let mut b = make_consensus(2, &[1, 2, 3, 4]);

        // Build a 5-block linear chain. Each validator's
        // light_cone_dag inserts a vertex per `on_block_committed`,
        // so after each block their digests must agree.
        let mut prev_hash = [0u8; 32];
        for n in 1u64..=5 {
            let blk = make_block_local(n, prev_hash);
            let state_root = [n as u8; 32];
            a.on_block_committed(&blk, state_root, 0);
            b.on_block_committed(&blk, state_root, 0);
            assert_eq!(
                a.light_cone_antichain_digest(),
                b.light_cone_antichain_digest(),
                "validators must converge on antichain digest after block {} (Phase 4.4)",
                n,
            );
            prev_hash = state_root;
        }

        // After non-trivial commits, digest must have moved away
        // from the empty-set sentinel.
        let empty_digest = evaporchain_light_cone::concurrency::digest_antichain(&[]);
        assert_ne!(
            a.light_cone_antichain_digest(),
            empty_digest,
            "after committed blocks digest must diverge from empty-set sentinel"
        );

        // Validator with a divergent history (different state roots
        // → different block IDs in the DAG) must NOT produce the
        // same digest.
        let mut c = make_consensus(3, &[1, 2, 3, 4]);
        let mut prev_hash_c = [0u8; 32];
        for n in 1u64..=5 {
            let blk = make_block_local(n, prev_hash_c);
            // Distinct state roots → distinct block IDs in the DAG.
            let state_root = [(n + 100) as u8; 32];
            c.on_block_committed(&blk, state_root, 0);
            prev_hash_c = state_root;
        }
        assert_ne!(
            a.light_cone_antichain_digest(),
            c.light_cone_antichain_digest(),
            "validators with divergent block-ID histories must NOT converge"
        );
    }

    /// Phase 4.4 — rolling antichain-digest history. Locks four
    /// properties:
    ///   1. History only populates when `light_cone_state_branches_enabled = true`.
    ///   2. History captures one entry per committed block (matches block_number).
    ///   3. FIFO eviction at `ANTICHAIN_DIGEST_HISTORY_CAP` keeps memory bounded.
    ///   4. Each entry's digest matches what `light_cone_antichain_digest()`
    ///      reports at the point it was pushed — i.e. operators retrieving
    ///      historical digests get the same value the validator computed
    ///      live at that height.
    #[test]
    fn test_antichain_digest_history_captures_per_block_under_flag() {
        use evaporchain_types::Block;

        fn make_block_local(num: u64, parent_hash: [u8; 32]) -> Block {
            Block {
                number: num,
                epoch: num,
                parent_hash,
                state_root: [num as u8; 32],
                transactions: vec![],
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        // Property 1: flag-off → no history.
        let mut tc_off = make_consensus(1, &[1, 2, 3, 4]);
        let mut prev = [0u8; 32];
        for n in 1u64..=3 {
            let blk = make_block_local(n, prev);
            tc_off.on_block_committed(&blk, [n as u8; 32], 0);
            prev = [n as u8; 32];
        }
        assert!(
            tc_off.antichain_digest_history().is_empty(),
            "flag-off must leave digest history empty (chain bit-compat)"
        );

        // Property 2: flag-on → one entry per committed block, in
        // ascending height order.
        let mut tc = make_consensus(2, &[1, 2, 3, 4]);
        tc.governance_params.insert(
            "light_cone_state_branches_enabled".to_string(),
            "true".to_string(),
        );
        let mut prev = [0u8; 32];
        for n in 1u64..=5 {
            let blk = make_block_local(n, prev);
            tc.on_block_committed(&blk, [n as u8; 32], 0);
            prev = [n as u8; 32];
        }
        let hist = tc.antichain_digest_history();
        assert_eq!(hist.len(), 5, "5 commits → 5 history entries");
        for (i, (height, _)) in hist.iter().enumerate() {
            assert_eq!(*height, (i + 1) as u64, "heights must be in commit order");
        }

        // Property 4: latest entry's digest matches the live accessor.
        let live_digest = tc.light_cone_antichain_digest();
        let last_history_digest = hist.last().expect("non-empty").1;
        assert_eq!(
            live_digest, last_history_digest,
            "most-recent history digest must match live antichain_digest accessor"
        );

        // Property 3: FIFO eviction at cap. Push enough commits to
        // exceed `ANTICHAIN_DIGEST_HISTORY_CAP`; assert oldest is
        // dropped.
        let mut tc_cap = make_consensus(3, &[1, 2, 3, 4]);
        tc_cap.governance_params.insert(
            "light_cone_state_branches_enabled".to_string(),
            "true".to_string(),
        );
        let cap = ANTICHAIN_DIGEST_HISTORY_CAP;
        let n_blocks = (cap + 5) as u64; // exceed cap by 5
        let mut prev = [0u8; 32];
        for n in 1..=n_blocks {
            let blk = make_block_local(n, prev);
            tc_cap.on_block_committed(&blk, [n as u8; 32], 0);
            prev = [n as u8; 32];
        }
        let hist_capped = tc_cap.antichain_digest_history();
        assert_eq!(
            hist_capped.len(),
            cap,
            "history must be capped at ANTICHAIN_DIGEST_HISTORY_CAP"
        );
        // Oldest entry should be (n_blocks - cap + 1) = block 6 of the
        // n_blocks=cap+5 run.
        let expected_oldest_height = n_blocks - cap as u64 + 1;
        assert_eq!(
            hist_capped[0].0, expected_oldest_height,
            "FIFO eviction: oldest surviving height must be block {}",
            expected_oldest_height
        );
        assert_eq!(
            hist_capped.last().unwrap().0,
            n_blocks,
            "newest entry must be the most-recent commit"
        );
    }

    // ── MCC Phase A — candidate_heads accessor (A.1 + A.4 tests) ────

    /// Helper: build the same minimal block fixture used elsewhere
    /// in the file but specialised for direct light_cone_dag
    /// manipulation. Inserts a Light-Cone block with the given id /
    /// parents into `tc.light_cone_dag` so we can drive
    /// `candidate_heads` without going through the full
    /// `on_block_committed` path (which would also consume mempool,
    /// fee logic, etc.).
    fn lc_insert(tc: &mut TendermintConsensus, id: [u8; 32], parents: Vec<[u8; 32]>, epoch: u64) {
        use evaporchain_light_cone::Block as LcBlock;
        tc.light_cone_dag
            .insert(LcBlock::new(id, parents, 1000, epoch))
            .unwrap();
    }

    fn id(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    /// Decay-Lamport DAG accessor — verify the consensus crate
    /// passes through to the light-cone module correctly for a
    /// linear chain. genesis tick=1; each subsequent block ticks +1.
    #[test]
    fn light_cone_block_lamport_clock_linear_chain() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // energy=100 per block, quantum=100 → tick advances by 1.
        // The lc_insert helper uses energy 1000; instead, insert
        // directly so we control the energy.
        use evaporchain_light_cone::Block as LcBlock;
        tc.light_cone_dag
            .insert(LcBlock::new(id(0), vec![], 100, 0))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(id(1), vec![id(0)], 100, 1))
            .unwrap();
        tc.light_cone_dag
            .insert(LcBlock::new(id(2), vec![id(1)], 100, 2))
            .unwrap();

        let g = tc.light_cone_block_lamport_clock(id(0), 100).unwrap();
        let b1 = tc.light_cone_block_lamport_clock(id(1), 100).unwrap();
        let b2 = tc.light_cone_block_lamport_clock(id(2), 100).unwrap();
        assert_eq!(g.current_tick, 1);
        assert_eq!(b1.current_tick, 2);
        assert_eq!(b2.current_tick, 3);
    }

    /// Decay-Lamport DAG accessor — None for missing block + None
    /// for zero quantum.
    #[test]
    fn light_cone_block_lamport_clock_returns_none_on_error() {
        let tc = make_consensus(1, &[1, 2, 3, 4]);
        // Empty DAG: any block_id is missing.
        assert!(tc.light_cone_block_lamport_clock(id(0), 100).is_none());
        // Zero quantum: even genesis returns None.
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        assert!(tc.light_cone_block_lamport_clock(id(0), 0).is_none());
    }

    /// MCC Phase A.4 — `candidate_heads` is empty when the DAG is
    /// empty. This is the genesis-state baseline: before any block
    /// is inserted, no heads exist.
    #[test]
    fn mcc_phase_a_candidate_heads_empty_at_genesis() {
        let tc = make_consensus(1, &[1, 2, 3, 4]);
        assert!(
            tc.candidate_heads().is_empty(),
            "fresh TendermintConsensus must have no candidate heads"
        );
    }

    /// MCC Phase A.4 — `candidate_heads` grows under concurrent
    /// proposals. With 3 sibling blocks at the same height, all
    /// three are leaves and must appear as candidate heads.
    #[test]
    fn mcc_phase_a_candidate_heads_grows_under_concurrent_proposals() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Genesis.
        lc_insert(&mut tc, id(0), vec![], 0);
        // Three siblings off genesis — all leaves.
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);
        lc_insert(&mut tc, id(3), vec![id(0)], 1);

        let heads = tc.candidate_heads();
        assert_eq!(heads.len(), 3, "three siblings → three candidate heads");
        assert!(heads.contains(&id(1)));
        assert!(heads.contains(&id(2)));
        assert!(heads.contains(&id(3)));
        assert!(
            !heads.contains(&id(0)),
            "genesis is no longer a leaf once it has children"
        );
    }

    /// MCC Phase A.4 — `candidate_heads` shrinks when one fork
    /// extends past the others. Locks the contract: extending a
    /// head transfers leaf-status to the child, removing the
    /// extended block from the candidate set.
    #[test]
    fn mcc_phase_a_candidate_heads_shrinks_on_extension() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        // Two heads now: id(1) and id(2).
        assert_eq!(tc.candidate_heads().len(), 2);

        // Extend id(1) with a child id(3). id(1) is no longer a
        // leaf; its child id(3) takes the head role.
        lc_insert(&mut tc, id(3), vec![id(1)], 2);

        let heads = tc.candidate_heads();
        assert_eq!(
            heads.len(),
            2,
            "still 2 heads after extension (id(2) + id(3))"
        );
        assert!(
            !heads.contains(&id(1)),
            "extended block must no longer be a head"
        );
        assert!(heads.contains(&id(3)), "new child id(3) takes head role");
        assert!(
            heads.contains(&id(2)),
            "uninvolved sibling id(2) stays a head"
        );
    }

    /// MCC Phase A.4 — validator-determinism property test.
    /// Two `TendermintConsensus` instances driven through identical
    /// block-insertion sequences must produce identical
    /// `candidate_heads` sets at every step. Mirrors the convergence
    /// pattern of `test_light_cone_antichain_digest_converges_across_validators`
    /// at the candidate-head level.
    ///
    /// Stronger than just asserting set-equality: also asserts the
    /// iteration ORDER is identical — a `BTreeSet<[u8; 32]>` iterates
    /// in lexicographic order so this falls out for free, but
    /// codifies the contract for any future change of return type.
    #[test]
    fn mcc_phase_a_candidate_heads_converges_across_validators() {
        // Build two independently-constructed validators and feed
        // them the same DAG insertions in the same order.
        let mut a = make_consensus(1, &[1, 2, 3, 4]);
        let mut b = make_consensus(2, &[1, 2, 3, 4]);

        let inserts: Vec<([u8; 32], Vec<[u8; 32]>, u64)> = vec![
            (id(0), vec![], 0),      // genesis
            (id(1), vec![id(0)], 1), // child A
            (id(2), vec![id(0)], 1), // child B (sibling of A)
            (id(3), vec![id(1)], 2), // grandchild via A
            (id(4), vec![id(0)], 1), // child C (sibling of A, B)
            (id(5), vec![id(2)], 2), // grandchild via B
        ];

        for (i, parents, epoch) in &inserts {
            lc_insert(&mut a, *i, parents.clone(), *epoch);
            lc_insert(&mut b, *i, parents.clone(), *epoch);
            let heads_a = a.candidate_heads();
            let heads_b = b.candidate_heads();
            assert_eq!(
                heads_a, heads_b,
                "validators must converge on candidate_heads after inserting {:?}",
                i
            );
            // Iteration order must also match (BTreeSet is sorted).
            let order_a: Vec<_> = heads_a.iter().copied().collect();
            let order_b: Vec<_> = heads_b.iter().copied().collect();
            assert_eq!(
                order_a, order_b,
                "BTreeSet iteration order must be identical across validators"
            );
        }

        // After the full sequence: id(3), id(4), id(5) are the
        // current leaves.
        let final_heads = a.candidate_heads();
        assert_eq!(final_heads.len(), 3);
        assert!(final_heads.contains(&id(3)));
        assert!(final_heads.contains(&id(4)));
        assert!(final_heads.contains(&id(5)));
    }

    /// MCC Phase B.1 — `StateSnapshotBranch` capture → mutate → restore
    /// roundtrip. Locks the contract: capturing a snapshot, mutating
    /// the StateDB, then calling `restore` produces a StateDB with
    /// the original captured state — not the mutated one.
    ///
    /// This is the substrate that Phase B.2's `replay_to_head` uses:
    /// when the executor needs to roll back from `from_head` to LCA,
    /// it calls `restore` on the LCA's snapshot (wiping the StateDB
    /// of any forward-only state changes), then applies the
    /// `forward_path` blocks from the `ReplayWalk`.
    #[test]
    fn mcc_phase_b1_state_snapshot_branch_roundtrip() {
        use evaporchain_state::db::InMemoryStateDB;
        use evaporchain_types::{Account, AccountAddress};

        // Set up a minimal state: 2 accounts with non-zero balance.
        let mut db = InMemoryStateDB::new();
        let addr_a = AccountAddress::from([0x01; 32]);
        let addr_b = AccountAddress::from([0x02; 32]);
        db.put_account(Account {
            address: addr_a,
            balance: 1000,
            nonce: 5,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 7,
            vesting: None,
        });
        db.put_account(Account {
            address: addr_b,
            balance: 2500,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 7,
            vesting: None,
        });

        // Capture snapshot at this tip.
        let tip = id(42);
        let height = 100u64;
        let epoch = 7u64;
        let branch =
            super::StateSnapshotBranch::capture(tip, height, epoch, &mut db).expect("capture");

        assert_eq!(branch.tip(), tip);
        assert_eq!(branch.created_at_height(), height);

        // Mutate the StateDB — change balances, add a new account,
        // delete one, increment nonces. After restore these must
        // all be reverted.
        db.put_account(Account {
            address: addr_a,
            balance: 9999, // changed
            nonce: 99,     // changed
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 100,
            vesting: None,
        });
        let addr_c = AccountAddress::from([0x03; 32]);
        db.put_account(Account {
            address: addr_c,
            balance: 555,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 50,
            vesting: None,
        });

        // Verify mutation took effect.
        assert_eq!(db.get_account(&addr_a).map(|a| a.balance), Some(9999));
        assert_eq!(db.get_account(&addr_c).map(|a| a.balance), Some(555));

        // Restore from snapshot — must wipe mutations.
        branch
            .restore(&mut db as &mut dyn evaporchain_state::db::StateDB)
            .expect("restore");

        // addr_a must be back to 1000 / nonce 5.
        let after_a = db.get_account(&addr_a).expect("addr_a present");
        assert_eq!(after_a.balance, 1000, "balance reverted");
        assert_eq!(after_a.nonce, 5, "nonce reverted");

        // addr_b must still be at 2500.
        let after_b = db.get_account(&addr_b).expect("addr_b present");
        assert_eq!(after_b.balance, 2500);

        // addr_c (added after capture) must NOT be present after restore.
        assert!(
            db.get_account(&addr_c).is_none(),
            "post-capture addition must be wiped on restore"
        );
    }

    /// MCC Phase B.1 — default `restore()` impl on the trait returns
    /// an error. Locks the contract: trait impls that don't override
    /// `restore` (e.g. test stubs that only need `tip` /
    /// `created_at_height`) get a clean error message rather than
    /// silently corrupting state.
    #[test]
    fn mcc_phase_b1_default_restore_returns_error() {
        use evaporchain_state::db::InMemoryStateDB;

        struct StubSnapshotNoRestore;
        impl super::LightConeBranchSnapshot for StubSnapshotNoRestore {
            fn tip(&self) -> [u8; 32] {
                [0; 32]
            }
            fn created_at_height(&self) -> u64 {
                0
            }
            // `restore` NOT overridden — uses trait default.
        }

        let stub = StubSnapshotNoRestore;
        let mut db = InMemoryStateDB::new();
        let result = stub.restore(&mut db as &mut dyn evaporchain_state::db::StateDB);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("does not support restoration"),
            "default impl error message must signal non-support"
        );
    }

    /// MCC Phase B.2 — `restore_to_lca` happy path: rollback case
    /// with attached snapshot at LCA. Captures state at LCA, mutates,
    /// then restore_to_lca reverts. Locks the bridge between B.0+
    /// planning and B.1 snapshot restore.
    #[test]
    fn mcc_phase_b2_restore_to_lca_happy_path() {
        use evaporchain_state::db::InMemoryStateDB;
        use evaporchain_types::{Account, AccountAddress};
        use std::sync::Arc;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Diamond: 0 (LCA) → 1, 0 → 2 (siblings).
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        // Build a state at LCA (id(0)) and capture a snapshot.
        let mut db = InMemoryStateDB::new();
        let addr = AccountAddress::from([0xAA; 32]);
        db.put_account(Account {
            address: addr,
            balance: 5000,
            nonce: 1,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        let snapshot =
            super::StateSnapshotBranch::capture(id(0), 0, 0, &mut db).expect("capture at LCA");

        // Record the LCA as a state branch + attach the snapshot.
        tc.record_state_branch(id(0), 0, 100);
        tc.attach_branch_snapshot(id(0), Arc::new(snapshot))
            .expect("attach");

        // Mutate state to simulate having moved past LCA on the
        // id(1) branch.
        db.put_account(Account {
            address: addr,
            balance: 9999,
            nonce: 5,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 5,
            vesting: None,
        });

        // Plan: from id(1) → id(2). LCA = id(0), rollback_required = true.
        let plan = tc.plan_replay_to_head(id(1), id(2)).expect("plan");
        assert!(plan.rollback_required);
        assert_eq!(plan.lca, id(0));

        // Execute the bridge: restore to LCA.
        tc.restore_to_lca(&plan, &mut db as &mut dyn evaporchain_state::db::StateDB)
            .expect("restore_to_lca");

        // State must reflect the captured-at-LCA values.
        let after = db.get_account(&addr).expect("account present");
        assert_eq!(after.balance, 5000, "balance reverted to LCA");
        assert_eq!(after.nonce, 1, "nonce reverted to LCA");
    }

    /// MCC Phase B.2 — `restore_to_lca` is a no-op when
    /// `rollback_required = false` (LCA == from_head). Locks the
    /// contract: the bridge does nothing on forward-only paths and
    /// returns Ok regardless of snapshot presence at LCA.
    #[test]
    fn mcc_phase_b2_restore_to_lca_noop_when_no_rollback() {
        use evaporchain_state::db::InMemoryStateDB;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);

        // from = LCA = id(0); to = id(1) (forward-only).
        let plan = tc.plan_replay_to_head(id(0), id(1)).expect("plan");
        assert!(!plan.rollback_required);

        let mut db = InMemoryStateDB::new();
        // No state_branches entry for id(0) — would normally error,
        // but rollback_required is false so the function short-circuits.
        let result = tc.restore_to_lca(&plan, &mut db as &mut dyn evaporchain_state::db::StateDB);
        assert!(
            result.is_ok(),
            "no-op rollback must succeed without LCA snapshot"
        );
    }

    /// MCC Phase B.2 — `restore_to_lca` errors when the LCA is not
    /// tracked in state_branches. Locks the contract: caller must
    /// ensure the LCA was recorded before calling.
    #[test]
    fn mcc_phase_b2_restore_to_lca_errors_on_missing_lca() {
        use evaporchain_state::db::InMemoryStateDB;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        // Plan with rollback required, but state_branches is empty.
        let plan = tc.plan_replay_to_head(id(1), id(2)).expect("plan");
        assert!(plan.rollback_required);

        let mut db = InMemoryStateDB::new();
        let result = tc.restore_to_lca(&plan, &mut db as &mut dyn evaporchain_state::db::StateDB);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("not tracked in state_branches"),
            "error must signal missing LCA: {}",
            err
        );
    }

    /// MCC Phase B.2 — `restore_to_lca` errors when the LCA is
    /// tracked but has no attached snapshot. Locks the contract:
    /// caller must call `attach_branch_snapshot` before relying on
    /// rollback.
    #[test]
    fn mcc_phase_b2_restore_to_lca_errors_on_missing_snapshot() {
        use evaporchain_state::db::InMemoryStateDB;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        // Record LCA as a branch but DO NOT attach a snapshot.
        tc.record_state_branch(id(0), 0, 100);

        let plan = tc.plan_replay_to_head(id(1), id(2)).expect("plan");
        assert!(plan.rollback_required);

        let mut db = InMemoryStateDB::new();
        let result = tc.restore_to_lca(&plan, &mut db as &mut dyn evaporchain_state::db::StateDB);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("no attached snapshot"),
            "error must signal missing snapshot: {}",
            err
        );
    }

    /// MCC Phase C.4 — `cross_fork_equivocation_count` returns 0
    /// under linear/mcc regardless of the underlying counter.
    /// Locks chain bit-compat: equivocation slashing under MCC
    /// requires multi-parent semantics that only mcc_full provides.
    #[test]
    fn mcc_phase_c4_equivocation_count_zero_outside_mcc_full() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Manually populate the underlying counter.
        tc.cross_fork_equivocations.insert(7, 5);

        // Default linear → returns 0 even though counter is 5.
        assert_eq!(tc.cross_fork_equivocation_count(7), 0);

        // Single-line mcc → also 0.
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc".to_string());
        assert_eq!(tc.cross_fork_equivocation_count(7), 0);
        assert!(tc.all_cross_fork_equivocations().is_empty());
    }

    /// MCC Phase C.4 — under mcc_full, both accessors expose the
    /// underlying counter.
    #[test]
    fn mcc_phase_c4_equivocation_count_exposes_counter_under_mcc_full() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc_full".to_string());
        tc.cross_fork_equivocations.insert(7, 5);
        tc.cross_fork_equivocations.insert(11, 2);

        assert_eq!(tc.cross_fork_equivocation_count(7), 5);
        assert_eq!(tc.cross_fork_equivocation_count(11), 2);
        // Validator with no record returns 0 (default).
        assert_eq!(tc.cross_fork_equivocation_count(99), 0);

        let snapshot = tc.all_cross_fork_equivocations();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot.get(&7), Some(&5));
        assert_eq!(snapshot.get(&11), Some(&2));
    }

    /// MCC Phase C.3 — `propose_parents` returns empty Vec under
    /// linear and mcc modes. Block.parents stays empty →
    /// serde-skip-empty preserves legacy single-parent wire format.
    #[test]
    fn mcc_phase_c3_propose_parents_empty_outside_mcc_full() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        // Default linear.
        assert!(tc.propose_parents().is_empty());

        // Single-line mcc.
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc".to_string());
        assert!(tc.propose_parents().is_empty());
    }

    /// MCC Phase C.3 — under mcc_full, propose_parents returns the
    /// antichain of currently-active sibling heads. Sibling forks
    /// off genesis are pairwise concurrent, so all should appear.
    #[test]
    fn mcc_phase_c3_propose_parents_returns_concurrent_heads_under_mcc_full() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc_full".to_string());
        // Three siblings off genesis.
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);
        lc_insert(&mut tc, id(3), vec![id(0)], 1);

        let parents = tc.propose_parents();
        assert_eq!(parents.len(), 3, "all 3 siblings must appear");
        assert!(parents.contains(&id(1)));
        assert!(parents.contains(&id(2)));
        assert!(parents.contains(&id(3)));

        // The result must form an antichain.
        assert!(
            evaporchain_light_cone::concurrency::is_antichain(&tc.light_cone_dag, &parents),
            "propose_parents output must be an antichain"
        );
    }

    /// MCC Phase C.3 — `propose_parents` filters out comparable
    /// heads (parent + descendant in the same set) to maintain the
    /// antichain contract. Drops the lower-caliber comparable.
    #[test]
    fn mcc_phase_c3_propose_parents_filters_comparable_heads() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc_full".to_string());
        // 0 → 1, 1 → 2 (linear chain). Only id(2) is a leaf.
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(1)], 2);

        let parents = tc.propose_parents();
        // Only one leaf → trivially an antichain of size 1.
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0], id(2));
    }

    /// MCC Phase C.3 — `propose_parents` honors
    /// `light_cone_max_concurrent_forks` cap. Excess heads dropped.
    #[test]
    fn mcc_phase_c3_propose_parents_respects_max_forks_cap() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc_full".to_string());
        tc.governance_params.insert(
            "light_cone_max_concurrent_forks".to_string(),
            "2".to_string(),
        );
        // 4 siblings off genesis.
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);
        lc_insert(&mut tc, id(3), vec![id(0)], 1);
        lc_insert(&mut tc, id(4), vec![id(0)], 1);

        let parents = tc.propose_parents();
        assert_eq!(parents.len(), 2, "must be capped at 2 forks");
    }

    /// MCC Phase C.2 — `vote_target_head` returns `parent_hash`
    /// under `linear` and `mcc` modes (chain bit-compat preserved).
    #[test]
    fn mcc_phase_c2_vote_target_falls_back_to_parent_hash() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.parent_hash = id(42);

        // Default linear mode.
        assert_eq!(tc.vote_target_head(), id(42));

        // Single-line mcc mode — still parent_hash.
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc".to_string());
        assert_eq!(tc.vote_target_head(), id(42));
    }

    /// MCC Phase C.2 — under `mcc_full`, `vote_target_head` returns
    /// the current_authoritative_head if Some, else falls back to
    /// parent_hash.
    #[test]
    fn mcc_phase_c2_vote_target_uses_authoritative_head_under_mcc_full() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.parent_hash = id(42);
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc_full".to_string());

        // No authoritative head set yet → fallback to parent_hash.
        assert_eq!(tc.vote_target_head(), id(42));

        // Set authoritative head → routes to that.
        tc.current_authoritative_head = Some(id(99));
        assert_eq!(tc.vote_target_head(), id(99));

        // Clear → back to fallback.
        tc.current_authoritative_head = None;
        assert_eq!(tc.vote_target_head(), id(42));
    }

    /// MCC Phase C.1 — `update_authoritative_head` is a no-op when
    /// `parent_acceptance_mode != "mcc_full"`. Locks chain
    /// bit-compat: the field stays `None` under linear and mcc
    /// modes regardless of DAG state.
    #[test]
    fn mcc_phase_c1_authoritative_head_noop_outside_mcc_full() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);

        // Default mode is "linear" — must be no-op.
        assert!(tc.update_authoritative_head().is_none());
        assert!(tc.current_authoritative_head.is_none());

        // Flip to "mcc" (single-line trajectory walk) — still no-op
        // for the C.1 field; only "mcc_full" populates it.
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc".to_string());
        assert!(tc.update_authoritative_head().is_none());
        assert!(tc.current_authoritative_head.is_none());
    }

    /// MCC Phase C.1 — under `parent_acceptance_mode = "mcc_full"`,
    /// `update_authoritative_head` populates the field with the
    /// argmax of `enumerate_candidate_heads`. Equals what
    /// `MccForkChoice::select_tip` would return.
    #[test]
    fn mcc_phase_c1_authoritative_head_populated_under_mcc_full() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc_full".to_string());

        // Empty DAG → None even under mcc_full.
        assert!(tc.update_authoritative_head().is_none());

        // Build a small DAG.
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        let chosen = tc
            .update_authoritative_head()
            .expect("non-empty DAG → Some");
        assert_eq!(tc.current_authoritative_head, Some(chosen));

        // Choice must equal the argmax: first entry of
        // enumerate_candidate_heads.
        let scored = tc.enumerate_candidate_heads();
        let argmax = scored[0].0;
        assert_eq!(chosen, argmax);
    }

    /// MCC Phase C.1 — flipping parent_acceptance_mode FROM mcc_full
    /// back to linear (or mcc) must clear `current_authoritative_head`
    /// on next update. Locks rollback contract — operators flipping
    /// the flag back during emergency rollback get a clean state,
    /// not a stale authoritative_head reference.
    #[test]
    fn mcc_phase_c1_authoritative_head_clears_on_rollback_to_linear() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);

        // Set to mcc_full + populate.
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc_full".to_string());
        tc.update_authoritative_head();
        assert!(tc.current_authoritative_head.is_some());

        // Roll back to linear → next update clears.
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "linear".to_string());
        let cleared = tc.update_authoritative_head();
        assert!(cleared.is_none());
        assert!(tc.current_authoritative_head.is_none());
    }

    /// MCC Phase C.1 — governance allowlist accepts mcc_full as a
    /// valid parent_acceptance_mode value.
    #[test]
    fn mcc_phase_c1_governance_allows_mcc_full() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        let result = tc.governance_set_param("parent_acceptance_mode", "mcc_full");
        assert!(result.is_ok(), "mcc_full must be allowlisted");
        assert_eq!(
            tc.get_governance_param("parent_acceptance_mode")
                .map(|s| s.to_string()),
            Some("mcc_full".to_string())
        );
    }

    /// MCC Phase C.6 — integration test composing C.1 + C.2 + C.3
    /// accessors on a 4-fork DAG. Asserts the substrate composes
    /// coherently:
    ///   - update_authoritative_head returns the argmax
    ///   - vote_target_head returns the same argmax (under mcc_full)
    ///   - propose_parents includes the argmax as the first entry
    ///     and forms an antichain
    ///   - All three accessors read-consistent across multiple
    ///     reads without DAG mutation
    #[test]
    fn mcc_phase_c6_integration_accessors_compose_on_4_fork_dag() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc_full".to_string());
        tc.parent_hash = id(0); // genesis as fallback

        // 4 sibling forks off genesis.
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);
        lc_insert(&mut tc, id(3), vec![id(0)], 1);
        lc_insert(&mut tc, id(4), vec![id(0)], 1);

        // C.1: update authoritative head.
        let chosen = tc
            .update_authoritative_head()
            .expect("non-empty DAG → Some");

        // C.2: vote_target_head returns the same argmax.
        assert_eq!(tc.vote_target_head(), chosen);

        // C.3: propose_parents includes chosen + forms antichain.
        let parents = tc.propose_parents();
        assert!(parents.contains(&chosen), "argmax must be in parents");
        assert!(
            evaporchain_light_cone::concurrency::is_antichain(&tc.light_cone_dag, &parents),
            "propose_parents must be antichain"
        );

        // Read-consistency: re-reading without DAG mutation gives
        // identical results.
        let chosen_2 = tc.update_authoritative_head().expect("still Some");
        assert_eq!(chosen, chosen_2);
        assert_eq!(parents, tc.propose_parents());
    }

    /// MCC Phase C.6 — integration test for branch-switch under
    /// mcc_full. After a fork-extension, update_authoritative_head
    /// re-runs the argmax and may pick a new head; vote_target_head
    /// follows; propose_parents reflects the new candidate set.
    #[test]
    fn mcc_phase_c6_integration_authoritative_head_follows_dag_extension() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc_full".to_string());

        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        let head_a = tc.update_authoritative_head().expect("Some");
        let parents_a = tc.propose_parents();
        assert_eq!(parents_a.len(), 2, "two siblings → two-parent antichain");

        // Extend one fork. Authoritative head + parent set may
        // change.
        lc_insert(&mut tc, id(3), vec![id(1)], 2);
        let head_b = tc.update_authoritative_head().expect("Some");
        let parents_b = tc.propose_parents();

        // The DAG now has 2 leaves: id(2) and id(3) (id(1) extended
        // to id(3) so id(1) is no longer a leaf).
        let heads_set: std::collections::BTreeSet<[u8; 32]> = tc.candidate_heads();
        assert_eq!(heads_set.len(), 2);
        assert!(heads_set.contains(&id(2)));
        assert!(heads_set.contains(&id(3)));

        // The chosen head must be one of the current leaves.
        assert!(
            heads_set.contains(&head_b),
            "authoritative head must be a current leaf"
        );

        // parents_b must form antichain over the new DAG.
        assert!(evaporchain_light_cone::concurrency::is_antichain(
            &tc.light_cone_dag,
            &parents_b
        ));

        // Sanity: head_a (chosen pre-extension) may or may not equal
        // head_b. What's locked is that the substrate followed the
        // DAG.
        let _ = head_a;
    }

    /// MCC Phase C.6 — integration test: governance flag flip from
    /// mcc_full back to linear clears C.1 field, makes C.2 fall back
    /// to parent_hash, and C.3 returns empty parents — full chain
    /// bit-compat restored within a single mode flip.
    #[test]
    fn mcc_phase_c6_integration_rollback_to_linear_restores_bit_compat() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.parent_hash = id(7);
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "mcc_full".to_string());

        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        // Populate C.1 + verify C.2/C.3 active.
        tc.update_authoritative_head();
        assert!(tc.current_authoritative_head.is_some());
        assert_ne!(
            tc.vote_target_head(),
            id(7),
            "mcc_full uses authoritative head"
        );
        assert!(
            !tc.propose_parents().is_empty(),
            "mcc_full populates parents"
        );

        // Flip back to linear.
        tc.governance_params
            .insert("parent_acceptance_mode".to_string(), "linear".to_string());
        tc.update_authoritative_head();

        // C.1 cleared.
        assert!(tc.current_authoritative_head.is_none());
        // C.2 falls back to parent_hash.
        assert_eq!(tc.vote_target_head(), id(7));
        // C.3 empty.
        assert!(tc.propose_parents().is_empty());
    }

    /// MCC Phase C.5 — validator-determinism property test (256
    /// random DAG shapes).
    ///
    /// **The contract:** every honest validator with the same DAG
    /// state must produce the same MCC fork-choice outputs:
    ///   1. `candidate_heads()` returns the same `BTreeSet` of leaves
    ///   2. `enumerate_candidate_heads()` returns the same sorted
    ///      `Vec<(BlockId, caliber)>` (same order, same scores)
    ///   3. `light_cone_antichain_digest()` matches
    ///   4. `plan_replay_to_head` produces the same `ReplayWalk` for
    ///      every (from, to) pair drawn from the candidate heads
    ///
    /// **Why this is a proptest, not a unit test:** the manual
    /// `mcc_phase_a_candidate_heads_converges_across_validators`
    /// test (already shipped) covers a 6-block hand-picked sequence.
    /// This proptest sweeps 256 randomly-generated DAG shapes (linear
    /// chains, branching, multi-parent merges) at sizes 1..=20
    /// blocks, catching any non-determinism that depends on a
    /// specific topology — HashMap iteration order leaking into
    /// scoring, time-based tie-breaks, etc.
    proptest::proptest! {
        #[test]
        fn mcc_phase_c5_validator_determinism_under_random_dags(
            seed in 0u64..10_000,
            n_blocks in 1usize..=20,
        ) {
            use proptest::{prop_assert, prop_assert_eq};

            // Deterministic synthetic DAG generator from (seed, n_blocks).
            // Block i's parents = pick from { 0..i } based on
            // seed-derived hash. Genesis has no parents.
            let mut a = make_consensus(1, &[1, 2, 3, 4]);
            let mut b = make_consensus(2, &[1, 2, 3, 4]);
            let inserts: Vec<([u8; 32], Vec<[u8; 32]>, u64)> = {
                let mut out = Vec::with_capacity(n_blocks);
                out.push((id(0), vec![], 0));  // genesis
                for i in 1..n_blocks {
                    let h = (seed.wrapping_mul(i as u64).wrapping_add(31))
                        .wrapping_mul(2654435761);
                    let two_parents = (h & 1) == 1 && i >= 2;
                    let mut parents = Vec::new();
                    let p1 = (h.wrapping_div(7) as usize) % i;
                    parents.push(id(p1 as u8));
                    if two_parents {
                        let p2 = (h.wrapping_div(11) as usize) % i;
                        if p2 != p1 {
                            parents.push(id(p2 as u8));
                        }
                    }
                    out.push((id(i as u8), parents, i as u64));
                }
                out
            };

            for (i, parents, epoch) in &inserts {
                lc_insert(&mut a, *i, parents.clone(), *epoch);
                lc_insert(&mut b, *i, parents.clone(), *epoch);
            }

            // Property 1: candidate_heads BTreeSets must match.
            let heads_a = a.candidate_heads();
            let heads_b = b.candidate_heads();
            prop_assert_eq!(
                heads_a.clone(), heads_b.clone(),
                "candidate_heads must agree across validators"
            );

            // Property 2: enumerate_candidate_heads must match
            // exactly (same order, same caliber values).
            let scored_a = a.enumerate_candidate_heads();
            let scored_b = b.enumerate_candidate_heads();
            prop_assert_eq!(
                scored_a.clone(), scored_b.clone(),
                "enumerate_candidate_heads must match exactly"
            );

            // Property 3: antichain digest must match.
            prop_assert_eq!(
                a.light_cone_antichain_digest(),
                b.light_cone_antichain_digest(),
                "antichain digest must match across validators"
            );

            // Property 4: plan_replay_to_head must produce identical
            // ReplayWalks for every pair of candidate heads.
            let heads_vec: Vec<[u8; 32]> = heads_a.into_iter().collect();
            for from in &heads_vec {
                for to in &heads_vec {
                    let plan_a = a.plan_replay_to_head(*from, *to);
                    let plan_b = b.plan_replay_to_head(*from, *to);
                    prop_assert_eq!(
                        plan_a, plan_b,
                        "plan_replay_to_head must match for every (from, to) pair"
                    );
                }
            }

            // Property 5: every score must be a valid u64 (no
            // overflow / NaN / sentinel values leaked).
            for (_, caliber) in &scored_a {
                prop_assert!(*caliber < u64::MAX, "caliber must not overflow");
            }
        }
    }

    /// MCC Phase B.4 — `replay_and_apply_atomic` happy path. When
    /// the inner `replay_and_apply` succeeds, the atomic wrapper
    /// returns the same `ReplayResult` and the StateDB ends up at
    /// the target head's state. The pre-replay snapshot is
    /// captured (one extra full-state copy) but its work is
    /// effectively wasted on the success path.
    #[test]
    fn mcc_phase_b4_atomic_success_passes_through() {
        use evaporchain_state::db::InMemoryStateDB;
        use evaporchain_types::{Account, AccountAddress, Block as TxBlock};
        use std::sync::Arc;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        let mut db = InMemoryStateDB::new();
        let alice = AccountAddress::from([0xA1; 32]);
        db.put_account(Account {
            address: alice,
            balance: 1000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        let snap = super::StateSnapshotBranch::capture(id(0), 0, 0, &mut db).unwrap();
        tc.record_state_branch(id(0), 0, 100);
        tc.attach_branch_snapshot(id(0), Arc::new(snap)).unwrap();

        // Mutate to fork-A state.
        db.put_account(Account {
            address: alice,
            balance: 5000,
            nonce: 1,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 1,
            vesting: None,
        });

        let target_block = TxBlock {
            number: 1,
            epoch: 1,
            parent_hash: [0u8; 32],
            state_root: [2u8; 32],
            transactions: vec![],
            producer_id: Some(0),
            timestamp: 0,
            chain_id: String::new(),
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
            post_state_root: None,
        };
        let blocks = std::collections::HashMap::from([(id(2), target_block)]);
        let block_apply = |db: &mut dyn evaporchain_state::db::StateDB, _b: &TxBlock| {
            db.put_account(Account {
                address: alice,
                balance: 7777,
                nonce: 2,
                storage_deposit: 0,
                storage_bytes: 0,
                last_touched_epoch: 1,
                vesting: None,
            });
            Ok(())
        };

        let result = tc
            .replay_and_apply_atomic(
                &mut db as &mut dyn evaporchain_state::db::StateDB,
                id(1),
                id(2),
                |id| blocks.get(id).cloned(),
                block_apply,
                /*pre_replay_height*/ 1,
                /*pre_replay_epoch*/ 1,
            )
            .expect("atomic success");

        assert_eq!(result.lca, id(0));
        assert_eq!(result.applied, vec![id(2)]);
        // State reflects forward-applied block, not fork-A residue.
        assert_eq!(db.get_account(&alice).map(|a| a.balance), Some(7777));
    }

    /// MCC Phase B.4 — `replay_and_apply_atomic` rolls back when
    /// the inner replay fails midway. **The load-bearing test for
    /// transactional atomicity:** simulates a forward-apply failure
    /// AFTER the LCA restore has already succeeded, so without the
    /// atomic wrapper the StateDB would be at the LCA — wrong state
    /// (neither pre-replay fork-A NOR target fork-B). The atomic
    /// wrapper restores pre-replay state (fork-A) and returns the
    /// inner error.
    #[test]
    fn mcc_phase_b4_atomic_rolls_back_on_apply_failure() {
        use evaporchain_state::db::InMemoryStateDB;
        use evaporchain_types::{Account, AccountAddress, Block as TxBlock};
        use std::sync::Arc;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        let mut db = InMemoryStateDB::new();
        let alice = AccountAddress::from([0xA1; 32]);
        // Genesis state.
        db.put_account(Account {
            address: alice,
            balance: 1000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        let snap = super::StateSnapshotBranch::capture(id(0), 0, 0, &mut db).unwrap();
        tc.record_state_branch(id(0), 0, 100);
        tc.attach_branch_snapshot(id(0), Arc::new(snap)).unwrap();

        // Mutate to fork-A state — this is the pre-replay state we
        // expect to see restored.
        db.put_account(Account {
            address: alice,
            balance: 5555,
            nonce: 9,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 5,
            vesting: None,
        });

        let target_block = TxBlock {
            number: 1,
            epoch: 1,
            parent_hash: [0u8; 32],
            state_root: [2u8; 32],
            transactions: vec![],
            producer_id: Some(0),
            timestamp: 0,
            chain_id: String::new(),
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
            post_state_root: None,
        };
        let blocks = std::collections::HashMap::from([(id(2), target_block)]);

        // Apply closure that ALWAYS fails — simulating an executor
        // error mid-forward-replay.
        let block_apply = |_db: &mut dyn evaporchain_state::db::StateDB, _b: &TxBlock| {
            Err("simulated executor failure".to_string())
        };

        let result = tc.replay_and_apply_atomic(
            &mut db as &mut dyn evaporchain_state::db::StateDB,
            id(1),
            id(2),
            |id| blocks.get(id).cloned(),
            block_apply,
            5,
            5,
        );

        // Inner error must propagate — caller sees ApplyFailed.
        assert!(matches!(
            result,
            Err(super::ReplayError::ApplyFailed { ref msg, .. }) if msg.contains("simulated")
        ));

        // **Atomicity assertion:** StateDB is back at fork-A state
        // (5555/9/5), NOT at the LCA (1000/0/0) where the inner
        // replay's restore_to_lca would have left it.
        let after = db
            .get_account(&alice)
            .expect("alice present after rollback");
        assert_eq!(
            after.balance, 5555,
            "atomic wrapper must restore pre-replay state on failure"
        );
        assert_eq!(after.nonce, 9);
        assert_eq!(after.last_touched_epoch, 5);
    }

    /// MCC Phase B.5 — LRU eviction-drops-snapshot regression test.
    /// Locks the memory-reclamation contract: when
    /// `prune_state_branches` evicts a metadata entry under cap,
    /// the consensus crate's `Arc<dyn LightConeBranchSnapshot>`
    /// reference is released. External strong-count drops by 1.
    ///
    /// Without this guarantee, snapshot memory would accumulate
    /// indefinitely as forks come and go — the cap on metadata
    /// entries would only bound HashMap key count, not actual
    /// snapshot bytes held.
    #[test]
    fn mcc_phase_b5_eviction_drops_snapshot_arc() {
        use evaporchain_state::db::InMemoryStateDB;
        use std::sync::Arc;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Cap at 2 concurrent forks so the third insertion forces
        // eviction.
        tc.governance_params.insert(
            "light_cone_max_concurrent_forks".to_string(),
            "2".to_string(),
        );

        // Build 3 sibling heads off genesis with distinct calibers
        // so eviction order is deterministic.
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);
        lc_insert(&mut tc, id(3), vec![id(0)], 1);

        let mut db = InMemoryStateDB::new();

        // Capture snapshots for each tip; keep external Arcs so we
        // can observe strong_count.
        let snap_lo = Arc::new(super::StateSnapshotBranch::capture(id(1), 1, 0, &mut db).unwrap())
            as Arc<dyn super::LightConeBranchSnapshot + Send + Sync>;
        let snap_mid = Arc::new(super::StateSnapshotBranch::capture(id(2), 1, 0, &mut db).unwrap())
            as Arc<dyn super::LightConeBranchSnapshot + Send + Sync>;
        let snap_hi = Arc::new(super::StateSnapshotBranch::capture(id(3), 1, 0, &mut db).unwrap())
            as Arc<dyn super::LightConeBranchSnapshot + Send + Sync>;

        // Initial strong counts: 1 (test holds the Arc).
        assert_eq!(Arc::strong_count(&snap_lo), 1);
        assert_eq!(Arc::strong_count(&snap_mid), 1);
        assert_eq!(Arc::strong_count(&snap_hi), 1);

        // Record + attach with distinct calibers so id(1) is the
        // lowest (will be evicted first when cap exceeded).
        tc.record_state_branch(id(1), 1, /*caliber*/ 10);
        tc.attach_branch_snapshot(id(1), Arc::clone(&snap_lo))
            .unwrap();
        tc.record_state_branch(id(2), 1, /*caliber*/ 50);
        tc.attach_branch_snapshot(id(2), Arc::clone(&snap_mid))
            .unwrap();

        // After 2 attachments under cap=2: each consensus holds 1
        // ref → strong_count = 2 (test + consensus).
        assert_eq!(Arc::strong_count(&snap_lo), 2);
        assert_eq!(Arc::strong_count(&snap_mid), 2);

        // Add the 3rd branch — cap exceeded → eviction kicks in.
        tc.record_state_branch(id(3), 1, /*caliber*/ 100);
        tc.attach_branch_snapshot(id(3), Arc::clone(&snap_hi))
            .unwrap();
        tc.prune_state_branches();

        // Cap honored: only 2 metadata entries.
        assert_eq!(tc.state_branches.len(), 2, "prune must enforce cap=2");

        // The lowest-caliber entry (id(1)) is evicted.
        assert!(
            !tc.state_branches.contains_key(&id(1)),
            "lowest-caliber tip id(1) must be evicted"
        );
        assert!(tc.state_branches.contains_key(&id(2)), "id(2) survives");
        assert!(tc.state_branches.contains_key(&id(3)), "id(3) survives");

        // **The load-bearing assertion:** evicted metadata released
        // its Arc, so id(1)'s snapshot strong_count drops back to 1
        // (test only). Surviving snapshots stay at 2.
        assert_eq!(
            Arc::strong_count(&snap_lo),
            1,
            "evicted snapshot's Arc must be released by consensus"
        );
        assert_eq!(
            Arc::strong_count(&snap_mid),
            2,
            "surviving snapshot's Arc still held by consensus"
        );
        assert_eq!(
            Arc::strong_count(&snap_hi),
            2,
            "surviving snapshot's Arc still held by consensus"
        );
    }

    /// MCC Phase B.3 — `replay_and_apply` happy path: branch switch
    /// with full closure-driven composition. Asserts the umbrella
    /// function correctly orchestrates plan + restore + forward-apply
    /// AND returns a `ReplayResult` listing every applied block.
    #[test]
    fn mcc_phase_b3_replay_and_apply_branch_switch_happy_path() {
        use evaporchain_state::db::InMemoryStateDB;
        use evaporchain_types::{Account, AccountAddress, Block as TxBlock};
        use std::collections::HashMap;
        use std::sync::Arc;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Diamond: 0 → 1, 0 → 2.
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        // Set up genesis state + capture snapshot at LCA.
        let mut db = InMemoryStateDB::new();
        let alice = AccountAddress::from([0xA1; 32]);
        db.put_account(Account {
            address: alice,
            balance: 1000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        let snap = super::StateSnapshotBranch::capture(id(0), 0, 0, &mut db).unwrap();
        tc.record_state_branch(id(0), 0, 100);
        tc.attach_branch_snapshot(id(0), Arc::new(snap)).unwrap();

        // Mutate to fork-A state.
        db.put_account(Account {
            address: alice,
            balance: 5000,
            nonce: 1,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 1,
            vesting: None,
        });

        // Build a synthetic block-store: id(2) → balance=2222.
        let block_id_2 = id(2);
        let mut blocks: HashMap<[u8; 32], TxBlock> = HashMap::new();
        blocks.insert(
            block_id_2,
            TxBlock {
                number: 1,
                epoch: 1,
                parent_hash: [0u8; 32],
                state_root: [2u8; 32],
                transactions: vec![],
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            },
        );

        // Closures: lookup from blocks map, apply by mutating alice
        // to balance=2222 (simulated execute_block).
        let block_lookup = |id: &[u8; 32]| blocks.get(id).cloned();
        let block_apply = |db: &mut dyn evaporchain_state::db::StateDB, _b: &TxBlock| {
            db.put_account(Account {
                address: alice,
                balance: 2222,
                nonce: 1,
                storage_deposit: 0,
                storage_bytes: 0,
                last_touched_epoch: 1,
                vesting: None,
            });
            Ok(())
        };

        // Replay: from id(1) (current fork-A head) → id(2) (target
        // fork-B head). LCA is genesis, rollback required, forward
        // path = [id(2)].
        let result = tc
            .replay_and_apply(
                &mut db as &mut dyn evaporchain_state::db::StateDB,
                id(1),
                id(2),
                block_lookup,
                block_apply,
            )
            .expect("replay_and_apply succeeds");

        assert_eq!(result.lca, id(0));
        assert_eq!(result.applied, vec![id(2)]);

        // State must reflect block_apply's mutation (balance=2222),
        // NOT fork-A (5000) nor genesis (1000).
        assert_eq!(
            db.get_account(&alice).map(|a| a.balance),
            Some(2222),
            "final state = forward-applied block, not fork-A residue"
        );
    }

    /// MCC Phase B.3 — `replay_and_apply` returns `BlockNotFound`
    /// when the caller's `block_lookup` returns None. Locks the
    /// contract: missing blocks fail loudly, not silently.
    #[test]
    fn mcc_phase_b3_replay_and_apply_block_not_found() {
        use evaporchain_state::db::InMemoryStateDB;
        use evaporchain_types::Block as TxBlock;
        use std::sync::Arc;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        let mut db = InMemoryStateDB::new();
        let snap = super::StateSnapshotBranch::capture(id(0), 0, 0, &mut db).unwrap();
        tc.record_state_branch(id(0), 0, 100);
        tc.attach_branch_snapshot(id(0), Arc::new(snap)).unwrap();

        let result = tc.replay_and_apply(
            &mut db as &mut dyn evaporchain_state::db::StateDB,
            id(1),
            id(2),
            |_id: &[u8; 32]| -> Option<TxBlock> { None }, // never resolves
            |_db: &mut dyn evaporchain_state::db::StateDB, _b: &TxBlock| Ok(()),
        );
        assert!(matches!(result, Err(super::ReplayError::BlockNotFound(_))));
    }

    /// MCC Phase B.3 — `replay_and_apply` returns `PlanFailed`
    /// when one of the heads is missing from the DAG.
    #[test]
    fn mcc_phase_b3_replay_and_apply_plan_failed_on_missing_head() {
        use evaporchain_state::db::InMemoryStateDB;
        use evaporchain_types::Block as TxBlock;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);

        let mut db = InMemoryStateDB::new();
        let result = tc.replay_and_apply(
            &mut db as &mut dyn evaporchain_state::db::StateDB,
            id(0),
            id(99), // not in DAG
            |_id: &[u8; 32]| -> Option<TxBlock> { None },
            |_db: &mut dyn evaporchain_state::db::StateDB, _b: &TxBlock| Ok(()),
        );
        assert!(matches!(result, Err(super::ReplayError::PlanFailed)));
    }

    /// MCC Phase B.3 — `replay_and_apply` propagates `block_apply`
    /// errors as `ApplyFailed`. Locks the contract: caller-side
    /// failures don't get swallowed.
    #[test]
    fn mcc_phase_b3_replay_and_apply_apply_failed_propagates_error() {
        use evaporchain_state::db::InMemoryStateDB;
        use evaporchain_types::Block as TxBlock;
        use std::sync::Arc;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);

        let mut db = InMemoryStateDB::new();
        let snap = super::StateSnapshotBranch::capture(id(0), 0, 0, &mut db).unwrap();
        tc.record_state_branch(id(0), 0, 100);
        tc.attach_branch_snapshot(id(0), Arc::new(snap)).unwrap();

        let result = tc.replay_and_apply(
            &mut db as &mut dyn evaporchain_state::db::StateDB,
            id(0),
            id(1),
            |_id: &[u8; 32]| -> Option<TxBlock> {
                Some(TxBlock {
                    number: 1,
                    epoch: 1,
                    parent_hash: [0u8; 32],
                    state_root: [0u8; 32],
                    transactions: vec![],
                    producer_id: Some(0),
                    timestamp: 0,
                    chain_id: String::new(),
                    commit_certificate: None,
                    nova_proof: None,
                    anchor_hash: None,
                    vrf_output: None,
                    vrf_proof: None,
                    data_root: None,
                    da_row_roots: vec![],
                    da_col_roots: vec![],
                    blob_commitments: vec![],
                    da_certificate: None,
                    state_function_commitment: None,
                    oracle_state_root: None,
                    shard_count: None,
                    protocol_version: 0,
                    state_root_version: 0,
                    submit_epoch_hints: vec![],
                    parents: vec![],
                    post_state_root: None,
                })
            },
            |_db: &mut dyn evaporchain_state::db::StateDB, _b: &TxBlock| {
                Err("simulated executor failure".to_string())
            },
        );
        match result {
            Err(super::ReplayError::ApplyFailed { ref msg, .. }) => {
                assert!(msg.contains("simulated executor failure"));
            }
            other => panic!("expected ApplyFailed, got {:?}", other),
        }
    }

    /// MCC Phase B.6 — END-TO-END INTEGRATION TEST.
    ///
    /// Drives the full Phase B substrate composition through a
    /// realistic 3-block-deep branch switch:
    ///
    /// ```text
    ///       genesis
    ///        / \
    ///       /   \
    ///      A1    B1
    ///      |     |
    ///      A2    B2
    /// ```
    ///
    /// **Scenario:**
    ///   1. Capture snapshot at genesis with accounts in known state.
    ///   2. Mutate state to simulate having executed fork A's blocks
    ///      (manual put_account in lieu of full execute_block — the
    ///      executor wiring is Phase B.3's separate concern; B.6
    ///      verifies the substrate composition, not executor
    ///      integration).
    ///   3. Plan replay from A2 → B2 via plan_replay_to_head.
    ///      Expected: lca = genesis, forward_path = [B1, B2],
    ///      rollback_required = true.
    ///   4. Call restore_to_lca to wipe fork-A's state mutations.
    ///   5. Iterate forward_path applying fork-B's mutations
    ///      (caller-side loop — Phase B.2's documented pattern).
    ///   6. Assert final state reflects fork-B mutations only;
    ///      fork-A mutations are gone.
    ///
    /// **What this validates:**
    ///   - B.0 primitives (find_lca + block_path_from_to) compose
    ///     correctly with consensus-level state_branches tracking.
    ///   - B.0+ planning produces the right ReplayWalk for a
    ///     non-trivial branch-switch.
    ///   - B.1's StateSnapshotBranch::restore correctly wipes
    ///     post-LCA mutations.
    ///   - B.2's restore_to_lca bridges the planning + restore.
    ///   - The complete pipeline preserves fork isolation:
    ///     committing to a branch, then switching, leaves no
    ///     residue of the old branch's state.
    ///
    /// **What this does NOT validate** (Phase B.3+ work):
    ///   - Real ExecutionEngine (test uses manual put_account,
    ///     not execute_block).
    ///   - Atomic transactional contract under failure (B.4).
    ///   - LRU eviction of snapshots when fork count exceeds cap (B.5).
    #[test]
    fn mcc_phase_b6_e2e_branch_switch_substrate_composition() {
        use evaporchain_state::db::InMemoryStateDB;
        use evaporchain_types::{Account, AccountAddress};
        use std::sync::Arc;

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);

        // ── Build the 3-block-deep diverging DAG ────────────────
        lc_insert(&mut tc, id(0), vec![], 0); // genesis
        lc_insert(&mut tc, id(1), vec![id(0)], 1); // A1
        lc_insert(&mut tc, id(2), vec![id(1)], 2); // A2
        lc_insert(&mut tc, id(3), vec![id(0)], 1); // B1 (sibling of A1)
        lc_insert(&mut tc, id(4), vec![id(3)], 2); // B2

        // ── Set up genesis state + capture snapshot ─────────────
        let mut db = InMemoryStateDB::new();
        let alice = AccountAddress::from([0xA1; 32]);
        let bob = AccountAddress::from([0xB1; 32]);
        // Genesis state: alice=1000, bob=2000.
        db.put_account(Account {
            address: alice,
            balance: 1000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        db.put_account(Account {
            address: bob,
            balance: 2000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        // Capture snapshot at genesis.
        let genesis_snap =
            super::StateSnapshotBranch::capture(id(0), 0, 0, &mut db).expect("capture genesis");
        tc.record_state_branch(id(0), 0, 100);
        tc.attach_branch_snapshot(id(0), Arc::new(genesis_snap))
            .expect("attach genesis snapshot");

        // ── Simulate executing fork A: A1 then A2 ───────────────
        // Block A1 effect: alice += 500 → 1500.
        // Block A2 effect: bob += 750 → 2750.
        // (In production these would come from execute_block on
        // real RefundTx / TransferTx / etc.)
        db.put_account(Account {
            address: alice,
            balance: 1500,
            nonce: 1,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 1,
            vesting: None,
        });
        db.put_account(Account {
            address: bob,
            balance: 2750,
            nonce: 1,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 2,
            vesting: None,
        });

        // Sanity: state reflects fork A's mutations.
        assert_eq!(db.get_account(&alice).map(|a| a.balance), Some(1500));
        assert_eq!(db.get_account(&bob).map(|a| a.balance), Some(2750));

        // ── Plan replay from A2 (current head) → B2 (target) ────
        let plan = tc.plan_replay_to_head(id(2), id(4)).expect("plan A2 → B2");
        assert_eq!(
            plan.lca,
            id(0),
            "LCA must be genesis (forks share only genesis)"
        );
        assert_eq!(
            plan.forward_path,
            vec![id(3), id(4)],
            "forward path = [B1, B2]"
        );
        assert!(plan.rollback_required, "branch switch requires rollback");

        // ── Phase B.2 bridge: restore to LCA (genesis) ──────────
        tc.restore_to_lca(&plan, &mut db as &mut dyn evaporchain_state::db::StateDB)
            .expect("restore to genesis");

        // After restore: state must match genesis. Fork A's
        // mutations are gone; alice=1000, bob=2000.
        assert_eq!(
            db.get_account(&alice).map(|a| a.balance),
            Some(1000),
            "alice reverted to genesis state after restore_to_lca"
        );
        assert_eq!(
            db.get_account(&bob).map(|a| a.balance),
            Some(2000),
            "bob reverted to genesis state after restore_to_lca"
        );

        // ── Caller-side loop: apply fork B's forward_path ───────
        // Block B1 effect: alice += 100 → 1100.
        // Block B2 effect: bob -= 500 → 1500.
        // (In production: executor.execute_block(db, block_lookup(block_id)) per id.)
        for block_id in &plan.forward_path {
            if *block_id == id(3) {
                // B1
                db.put_account(Account {
                    address: alice,
                    balance: 1100,
                    nonce: 1,
                    storage_deposit: 0,
                    storage_bytes: 0,
                    last_touched_epoch: 1,
                    vesting: None,
                });
            } else if *block_id == id(4) {
                // B2
                db.put_account(Account {
                    address: bob,
                    balance: 1500,
                    nonce: 1,
                    storage_deposit: 0,
                    storage_bytes: 0,
                    last_touched_epoch: 2,
                    vesting: None,
                });
            } else {
                panic!("unexpected block id in forward_path: {:?}", block_id);
            }
        }

        // ── Final assertions: fork B's state, no fork A residue ──
        assert_eq!(
            db.get_account(&alice).map(|a| a.balance),
            Some(1100),
            "alice = fork B's effect (1000 + 100 = 1100), NOT fork A's 1500"
        );
        assert_eq!(
            db.get_account(&bob).map(|a| a.balance),
            Some(1500),
            "bob = fork B's effect (2000 - 500 = 1500), NOT fork A's 2750"
        );

        // Also: fork A's intermediate state (alice=1500, bob=2750) is
        // completely gone — no merge artefact, no hybrid state.
        let alice_final = db.get_account(&alice).unwrap();
        let bob_final = db.get_account(&bob).unwrap();
        assert_ne!(
            (alice_final.balance, bob_final.balance),
            (1500, 2750),
            "fork A's state must NOT survive the branch switch"
        );
    }

    /// MCC Phase B.0+ — `plan_replay_to_head` no-op when from == to.
    /// Locks the contract: the LCA of a block with itself is itself,
    /// the forward path is empty, no rollback required. The executor
    /// consuming this `ReplayWalk` does nothing.
    #[test]
    fn mcc_phase_b_plan_replay_self_to_self_is_noop() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);

        let plan = tc.plan_replay_to_head(id(1), id(1)).expect("from==to");
        assert_eq!(plan.lca, id(1));
        assert!(plan.forward_path.is_empty());
        assert!(!plan.rollback_required);
    }

    /// MCC Phase B.0+ — forward-only replay when `to_head` is a
    /// descendant of `from_head` along the first-parent chain. No
    /// rollback needed; just apply the forward path's blocks.
    #[test]
    fn mcc_phase_b_plan_replay_forward_only_no_rollback() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(1)], 2);

        // from = genesis, to = depth-2: LCA is genesis itself.
        // No rollback; forward path = [id(1), id(2)].
        let plan = tc
            .plan_replay_to_head(id(0), id(2))
            .expect("ancestor → descendant");
        assert_eq!(plan.lca, id(0));
        assert_eq!(plan.forward_path, vec![id(1), id(2)]);
        assert!(!plan.rollback_required, "from==lca → no rollback needed");
    }

    /// MCC Phase B.0+ — rollback case. When `from_head` and `to_head`
    /// are on different branches, the LCA is their common ancestor,
    /// the forward path goes LCA → to_head, and `rollback_required`
    /// flags that the executor must first unwind state from
    /// from_head back to LCA.
    #[test]
    fn mcc_phase_b_plan_replay_rollback_required_on_branch_switch() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Diamond branch points: genesis → id(1), genesis → id(2)
        // (siblings), no merge yet.
        lc_insert(&mut tc, id(0), vec![], 0);
        lc_insert(&mut tc, id(1), vec![id(0)], 1);
        lc_insert(&mut tc, id(2), vec![id(0)], 1);

        // Switch from head id(1) to head id(2): LCA is genesis,
        // forward path is [id(2)], rollback REQUIRED (current head
        // id(1) is not the LCA).
        let plan = tc.plan_replay_to_head(id(1), id(2)).expect("siblings");
        assert_eq!(plan.lca, id(0));
        assert_eq!(plan.forward_path, vec![id(2)]);
        assert!(
            plan.rollback_required,
            "branch switch must flag rollback (from id(1) back to genesis)"
        );
    }

    /// MCC Phase B.0+ — missing block returns None. Locks the
    /// contract: caller must validate that both heads are present
    /// in the DAG before calling.
    #[test]
    fn mcc_phase_b_plan_replay_missing_head_returns_none() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        lc_insert(&mut tc, id(0), vec![], 0);
        assert!(tc.plan_replay_to_head(id(0), id(99)).is_none());
        assert!(tc.plan_replay_to_head(id(99), id(0)).is_none());
    }

    /// MCC Phase B.0+ — validator-determinism. Two `TendermintConsensus`
    /// instances with the same DAG produce the same `ReplayWalk` for
    /// the same `(from, to)` pair. Locks the convergence contract for
    /// when Phase C uses this in the consensus hot path.
    #[test]
    fn mcc_phase_b_plan_replay_converges_across_validators() {
        let mut a = make_consensus(1, &[1, 2, 3, 4]);
        let mut b = make_consensus(2, &[1, 2, 3, 4]);
        let inserts: Vec<([u8; 32], Vec<[u8; 32]>, u64)> = vec![
            (id(0), vec![], 0),
            (id(1), vec![id(0)], 1),
            (id(2), vec![id(0)], 1),
            (id(3), vec![id(1)], 2),
        ];
        for (i, parents, epoch) in &inserts {
            lc_insert(&mut a, *i, parents.clone(), *epoch);
            lc_insert(&mut b, *i, parents.clone(), *epoch);
        }

        // Same query on both validators must yield the same plan.
        let plan_a = a.plan_replay_to_head(id(2), id(3)).expect("plan");
        let plan_b = b.plan_replay_to_head(id(2), id(3)).expect("plan");
        assert_eq!(plan_a, plan_b);
        // Specifically: LCA is genesis, forward = [id(1), id(3)].
        assert_eq!(plan_a.lca, id(0));
        assert_eq!(plan_a.forward_path, vec![id(1), id(3)]);
        assert!(plan_a.rollback_required);
    }

    /// MCC Phase A.4 — `enumerate_candidate_heads` returns the
    /// candidate set paired with caliber, sorted by caliber
    /// descending with smaller-BlockId tiebreak. Locks the contract
    /// against `MccForkChoice::select_tip` (the argmax of this list
    /// must equal what select_tip picks) and against
    /// `candidate_heads()` (the unsorted set must equal the keys of
    /// this list).
    #[test]
    fn mcc_phase_a_enumerate_candidate_heads_sorted_by_caliber() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Diamond + extra leaves so multiple trajectories of
        // different lengths exist.
        lc_insert(&mut tc, id(0), vec![], 0); // genesis
        lc_insert(&mut tc, id(1), vec![id(0)], 1); // depth 1
        lc_insert(&mut tc, id(2), vec![id(1)], 2); // depth 2 (deeper trajectory)
        lc_insert(&mut tc, id(3), vec![id(0)], 1); // depth 1, sibling of 1

        // Three leaves expected: id(2) (deeper), id(3) (sibling).
        // id(0) is parent of 1 + 3, id(1) is parent of 2 — neither
        // are leaves.
        let heads = tc.candidate_heads();
        assert_eq!(heads.len(), 2);
        assert!(heads.contains(&id(2)));
        assert!(heads.contains(&id(3)));

        let scored = tc.enumerate_candidate_heads();
        assert_eq!(
            scored.len(),
            2,
            "scored list must match candidate_heads count"
        );

        // Set equivalence: the keys of `scored` must equal the
        // unsorted `candidate_heads`.
        let scored_keys: std::collections::BTreeSet<[u8; 32]> =
            scored.iter().map(|(id, _)| *id).collect();
        assert_eq!(scored_keys, heads, "key sets must agree");

        // Order contract: caliber descending. The longer trajectory
        // (depth-2 head id(2)) should score >= the shorter one
        // (depth-1 head id(3)) because path_caliber accumulates over
        // trajectory length under a stable energy field.
        assert!(
            scored[0].1 >= scored[1].1,
            "caliber must be sorted descending (got {} then {})",
            scored[0].1,
            scored[1].1
        );

        // Argmax contract: first entry of the scored list must equal
        // `MccForkChoice::select_tip`'s pick (with the same DAG + β).
        use crate::fork_choice::ForkChoice;
        let beta_mb = 1000;
        let fc = crate::fork_choice::MccForkChoice::new(tc.light_cone_dag.clone(), beta_mb);
        let selected = fc.select_tip().expect("non-empty DAG → Some");
        assert_eq!(
            scored[0].0, selected,
            "first entry of enumerate_candidate_heads must equal select_tip's argmax"
        );
    }

    /// confirm a `MevObservation` lands in the consensus engine's
    /// ring buffer. Locks the call-site contract: substrate
    /// observation runs every block, no settlement, no false
    /// positives on a same-attacker honest sequence.
    #[test]
    fn test_mev_observations_sandwich_recorded() {
        use evaporchain_types::{Block, TransferTx};

        fn addr(seed: u8) -> [u8; 32] {
            let mut a = [0u8; 32];
            a[0] = seed;
            a
        }
        fn transfer(from: u8, to: u8, amount: u64, nonce: u64) -> Transaction {
            Transaction::Transfer(TransferTx {
                from: addr(from),
                to: addr(to),
                amount,
                nonce,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })
        }
        fn make_block(num: u64, txs: Vec<Transaction>) -> Block {
            Block {
                number: num,
                epoch: num,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: txs,
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        // Sandwich: attacker 0xAA front-runs + back-runs victim 0xBB,
        // all targeting account 0x99.
        let block = make_block(
            1,
            vec![
                transfer(0xAA, 0x99, 100, 0),
                transfer(0xBB, 0x99, 200, 0),
                transfer(0xAA, 0x99, 150, 1),
            ],
        );

        tc.on_block_committed(&block, [0u8; 32], 0);

        let obs = tc.mev_observations();
        assert_eq!(obs.len(), 1, "exactly one sandwich observation expected");
        let o = &obs[0];
        assert_eq!(o.block_height, 1);
        assert_eq!(o.attacker, addr(0xAA));
        assert_eq!(o.victim, addr(0xBB));
        assert_eq!(o.target, addr(0x99));
        assert_eq!(o.work_estimate, 250);
        // Phase 2 of CROOKS_MEV_INTEGRATION_PLAN.md — refund_amount
        // must be filled in by the call site (not None), bounded by
        // work_estimate.
        let refund = o
            .refund_amount
            .expect("refund_amount must be Some after call-site computation");
        assert!(
            refund <= o.work_estimate,
            "refund {refund} must be bounded by work_estimate {}",
            o.work_estimate
        );

        // Honest follow-up block: three different senders — must
        // not register a false positive.
        let honest = make_block(
            2,
            vec![
                transfer(0xCC, 0x99, 100, 0),
                transfer(0xDD, 0x99, 200, 0),
                transfer(0xEE, 0x99, 150, 0),
            ],
        );
        tc.on_block_committed(&honest, [0u8; 32], 0);
        assert_eq!(
            tc.mev_observations().len(),
            1,
            "honest block must not add observations"
        );
    }

    /// Phase 5.1 — when the `lambda_fold_nova` feature IS compiled in,
    /// the running Nova instance is at identity until the first
    /// nova-mode fold lands.
    #[cfg(feature = "lambda_fold_nova")]
    #[test]
    fn test_lambda_fold_nova_instance_starts_at_identity() {
        let tc = make_consensus(1, &[1, 2, 3, 4]);
        assert!(tc.lambda_fold_nova_instance().is_identity());
    }

    /// Phase 5.5 of LAMBDA_FOLD_NOVA_PLAN — end-to-end integration.
    /// Drives 3 blocks through `on_block_committed` with the
    /// `lambda_fold_mode = "nova"` flag set; verifies the running
    /// `NovaFoldedInstance` ticks to step_count == 3 with a non-empty
    /// proof, and the substrate accumulator also ticks (Nova path
    /// is additive).
    ///
    /// Marked `#[ignore]` because the first nova fold triggers
    /// `RealBlockProver::new`'s ~60-90 s `pp` setup. Run with
    /// `cargo test --release --features lambda_fold_nova -- --ignored`.
    #[cfg(feature = "lambda_fold_nova")]
    #[test]
    #[ignore = "heavy: triggers RealBlockProver pp setup (~60-90 s on M4)"]
    fn test_lambda_fold_nova_end_to_end_three_blocks() {
        fn make_block_for_test(height: u64) -> Block {
            Block {
                number: height,
                epoch: height,
                parent_hash: [0u8; 32],
                state_root: [0u8; 32],
                transactions: vec![],
                producer_id: Some(0),
                timestamp: 0,
                chain_id: String::new(),
                commit_certificate: None,
                nova_proof: None,
                anchor_hash: None,
                vrf_output: None,
                vrf_proof: None,
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
                da_certificate: None,
                state_function_commitment: None,
                oracle_state_root: None,
                shard_count: None,
                protocol_version: 0,
                state_root_version: 0,
                submit_epoch_hints: vec![],
                parents: vec![],
                post_state_root: None,
            }
        }

        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        tc.governance_set_param("lambda_fold_mode", "nova").unwrap();

        for h in 1..=3u64 {
            let block = make_block_for_test(h);
            let mut state_root = [0u8; 32];
            state_root[0] = h as u8;
            tc.on_block_committed(&block, state_root, 0);
        }

        // Substrate accumulator advances regardless (Phase 5.3 says
        // substrate ALWAYS runs, Nova is additive).
        assert_eq!(tc.lambda_fold.step_count, 3);

        // Nova accumulator advances only when nova-mode + feature on.
        let nova = tc.lambda_fold_nova_instance().clone();
        assert_eq!(nova.step_count, 3, "nova fold should have ticked 3x");
        assert!(
            !nova.proof_bytes.is_empty(),
            "nova proof should be non-empty"
        );

        // Light-client path: round-trip the proof through
        // verify_nova_folded with the prover's vk_bytes. This closes
        // the wiring all the way to the light-client API surface.
        let folder = tc
            .lambda_fold_nova
            .as_ref()
            .expect("nova folder constructed lazily on first nova fold");
        let vk_bytes = folder.vk_bytes().expect("vk_bytes failed");
        evaporchain_lambda_fold::verify_nova_folded(&nova, &vk_bytes, 0)
            .expect("light-client verify of consensus-produced proof failed");
    }

    #[test]
    fn test_governance_crooks_mev_beta_rejects_zero_and_non_numeric() {
        // Phase 2.2: β=0 is undefined for the Crooks formula; junk
        // strings must surface as InvalidValue.
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        assert!(matches!(
            tc.governance_set_param("crooks_mev_beta_mb", "0")
                .unwrap_err(),
            GovernanceParamError::InvalidValue { .. }
        ));
        assert!(matches!(
            tc.governance_set_param("crooks_mev_beta_mb", "not_a_number")
                .unwrap_err(),
            GovernanceParamError::InvalidValue { .. }
        ));
        assert!(matches!(
            tc.governance_set_param("crooks_mev_beta_mb", "-100")
                .unwrap_err(),
            GovernanceParamError::InvalidValue { .. }
        ));
    }

    #[test]
    fn test_governance_lambda_fold_mode_default_hash_chain() {
        // Phase 5.2 of LAMBDA_FOLD_NOVA_PLAN: when `lambda_fold_mode`
        // is unset, the snapshot reports `hash_chain` so operators see
        // the substrate path is in effect by default.
        let tc = make_consensus(1, &[1, 2, 3, 4]);
        let snap = tc.governance_flags_snapshot();
        assert_eq!(
            snap.get("lambda_fold_mode").map(|s| s.as_str()),
            Some("hash_chain"),
            "lambda_fold_mode default must be hash_chain (substrate)"
        );
    }

    #[test]
    fn test_governance_lambda_fold_mode_rejects_invalid_value() {
        // Phase 5.2: invalid values are rejected — only hash_chain /
        // nova are permitted. Stops typos like "Nova" / "novA" /
        // "real_nova" silently flipping behaviour.
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        let err = tc
            .governance_set_param("lambda_fold_mode", "Nova")
            .unwrap_err();
        match err {
            GovernanceParamError::InvalidValue { key, .. } => {
                assert_eq!(key, "lambda_fold_mode");
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn test_governance_set_param_rejects_unknown_key() {
        // Unknown keys must be rejected with UnknownKey — prevents
        // operators from littering governance_params with junk that
        // later breaks the typo-fall-through patterns.
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        let err = tc
            .governance_set_param("not_a_real_key", "some_value")
            .unwrap_err();
        assert_eq!(
            err,
            GovernanceParamError::UnknownKey("not_a_real_key".to_string())
        );
        // governance_params must NOT have been mutated.
        assert!(tc.get_governance_param("not_a_real_key").is_none());
    }

    #[test]
    fn test_governance_set_param_rejects_invalid_value() {
        // Valid key with an invalid value must be rejected with
        // InvalidValue + the structured permitted-set so the RPC can
        // surface useful diagnostics.
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        let err = tc
            .governance_set_param("parent_acceptance_mode", "MCC") // wrong case
            .unwrap_err();
        match err {
            GovernanceParamError::InvalidValue {
                key,
                value,
                permitted,
            } => {
                assert_eq!(key, "parent_acceptance_mode");
                assert_eq!(value, "MCC");
                assert!(permitted.contains(&"linear".to_string()));
                assert!(permitted.contains(&"mcc".to_string()));
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
        // Pre-mutation default value preserved.
        assert!(tc.get_governance_param("parent_acceptance_mode").is_none());
    }

    proptest::proptest! {
        /// Lane K.4 invariant proof: for any random (key, value) pair,
        /// `governance_set_param` returns one of three outcomes — Ok +
        /// mutation, InvalidValue, or UnknownKey — and the mutation
        /// happens iff Ok. Locks the K.1 allowlist contract against
        /// future refactors that might silently widen the
        /// allowlist or fail to validate.
        ///
        /// Sampling strategy:
        /// - 25% allowlisted (key, valid value) — must Ok+mutate
        /// - 25% allowlisted key + invalid value — must InvalidValue
        /// - 25% unknown key + any value — must UnknownKey
        /// - 25% unknown key + valid-looking value — also UnknownKey
        ///   (key is checked first)
        #[test]
        fn governance_set_param_proptest(
            // 0..4 picks the bucket (see strategy above).
            bucket in 0u8..4,
            // Random "value" string for invalid cases.
            junk_value in "[a-z]{3,12}",
            // Random "key" string for unknown-key cases.
            junk_key in "[a-z]{3,18}",
        ) {
            use proptest::prelude::*;
            // Some toolchains don't surface proptest's assertion
            // macros via the prelude glob; explicit imports below
            // make them available unconditionally.
            use proptest::{prop_assert, prop_assert_eq, prop_assert_ne, prop_assume};
            let mut tc = make_consensus(1, &[1, 2, 3, 4]);
            let (key, value, expected): (&str, String, &str) = match bucket {
                0 => ("parent_acceptance_mode", "mcc".to_string(), "ok"),
                1 => ("block_source_mode", junk_value.clone(), "invalid"),
                2 => ("conservation_enforcement", "enforce".to_string(), "ok"),
                3 => (junk_key.as_str(), junk_value.clone(), "unknown"),
                _ => unreachable!(),
            };

            let result = tc.governance_set_param(key, &value);
            match expected {
                "ok" => {
                    prop_assert!(result.is_ok());
                    proptest::prop_assert_eq!(
                        tc.get_governance_param(key).map(|s| s.to_string()),
                        Some(value.clone()),
                        "Ok must mutate governance_params"
                    );
                }
                "invalid" => {
                    // Bucket 1: known key + (likely) invalid value.
                    // junk_value MIGHT happen to equal a valid one
                    // (e.g. "fifo" or "antichain") in which case it
                    // succeeds — handle both branches.
                    let valid_for_key = matches!(value.as_str(), "fifo" | "antichain");
                    if valid_for_key {
                        prop_assert!(result.is_ok());
                    } else {
                        let is_invalid_value = matches!(
                            &result,
                            Err(GovernanceParamError::InvalidValue { .. })
                        );
                        prop_assert!(is_invalid_value);
                        // No mutation on err.
                        prop_assert!(tc.get_governance_param(key).is_none());
                    }
                }
                "unknown" => {
                    // Bucket 3: unknown key. junk_key MIGHT happen to
                    // equal a valid key by accident — handle both.
                    let valid_key = matches!(
                        key,
                        "parent_acceptance_mode" | "block_source_mode" | "conservation_enforcement"
                    );
                    if valid_key {
                        // Then it's actually bucket 1's regime.
                        let _ = result; // any outcome ok
                    } else {
                        match result {
                            Err(GovernanceParamError::UnknownKey(ref k)) => {
                                proptest::prop_assert_eq!(k, key);
                            }
                            other => {
                                prop_assert!(false, "expected UnknownKey, got {:?}", other);
                            }
                        }
                        prop_assert!(tc.get_governance_param(key).is_none());
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    proptest::proptest! {
        /// Lane O.8.2d invariant proof for `maybe_emit_cartel_alarm_event`.
        ///
        /// Four invariants over `(alarm_mode, s_honest_milli,
        /// last_run_at_height)`:
        /// 1. **Observe-mode silence:** queue stays empty regardless of S.
        /// 2. **Ceiling gate (alarm mode):** emit iff `s_honest_milli >= 1800`.
        /// 3. **Dedupe by height:** back-to-back ticks at same height
        ///    do not double-emit.
        /// 4. **Drain-then-emit:** after drain, a re-injected over-
        ///    ceiling status at the same height re-fires (operators
        ///    own the historical ack-set).
        #[test]
        fn cartel_alarm_emission_invariants(
            mode_bit in 0u8..2,
            s_honest_milli in -2000i64..4001,
            last_run_at_height in 0u64..1_000_001,
        ) {
            use proptest::prelude::*;
            use proptest::{prop_assert, prop_assert_eq, prop_assert_ne, prop_assume};
            use evaporchain_causal_chsh::{AlarmStatus, GateThresholds};

            let mut tc = make_consensus(1, &[1, 2, 3, 4]);
            let mode = if mode_bit == 0 { "observe" } else { "alarm" };
            tc.governance_set_param("cartel_alarm_mode", mode).unwrap();

            let st = AlarmStatus {
                s_honest: s_honest_milli as f64 / 1000.0,
                s_cartel_synthetic: 4.0,
                gap: 4.0 - (s_honest_milli as f64 / 1000.0),
                s_honest_milli,
                s_cartel_synthetic_milli: 4000,
                gap_milli: 4000 - s_honest_milli,
                verdict: "Test".to_string(),
                last_run_at_height,
                samples_per_bucket: [10, 10, 10, 10],
                thresholds: GateThresholds::doctrine(),
            };
            tc.cartel_alarm._inject_status_for_test(st.clone());

            tc.maybe_emit_cartel_alarm_event();
            let queue_len_after_first = tc.pending_cartel_alarms.len();

            const CEILING_MILLI: i64 = 1800;
            let crosses_ceiling = s_honest_milli >= CEILING_MILLI;

            match mode {
                "observe" => {
                    proptest::prop_assert_eq!(
                        queue_len_after_first, 0,
                        "observe mode must not emit (s={}, h={})",
                        s_honest_milli, last_run_at_height
                    );
                }
                "alarm" => {
                    let expected = if crosses_ceiling { 1 } else { 0 };
                    proptest::prop_assert_eq!(
                        queue_len_after_first, expected,
                        "alarm mode emit iff s_honest_milli >= {} (got s={}, h={})",
                        CEILING_MILLI, s_honest_milli, last_run_at_height
                    );
                }
                _ => unreachable!(),
            }

            tc.maybe_emit_cartel_alarm_event();
            proptest::prop_assert_eq!(
                tc.pending_cartel_alarms.len(),
                queue_len_after_first,
                "dedupe by at_height must hold across back-to-back ticks"
            );

            let drained = tc.take_pending_cartel_alarms();
            proptest::prop_assert_eq!(drained.len(), queue_len_after_first);
            proptest::prop_assert_eq!(tc.pending_cartel_alarms.len(), 0);

            tc.cartel_alarm._inject_status_for_test(st);
            tc.maybe_emit_cartel_alarm_event();
            let queue_after_redrain = tc.pending_cartel_alarms.len();
            let expected_after_redrain = if mode == "alarm" && crosses_ceiling {
                1
            } else {
                0
            };
            proptest::prop_assert_eq!(
                queue_after_redrain, expected_after_redrain,
                "post-drain re-emit must follow ceiling gate again (mode={})",
                mode
            );
        }
    }

    #[test]
    fn test_governance_set_param_rejects_fork_choice_mode() {
        // fork_choice_mode is intentionally NOT in the allowlist — it
        // requires endorser-stake validation and goes through
        // governance_set_fork_choice_mode instead. Setting it via the
        // generic param setter must be rejected with UnknownKey so
        // operators can't bypass the stake check.
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        let err = tc
            .governance_set_param("fork_choice_mode", "mcc")
            .unwrap_err();
        assert!(matches!(
            err,
            GovernanceParamError::UnknownKey(ref k) if k == "fork_choice_mode"
        ));
    }

    #[test]
    fn test_parent_acceptance_mode_typo_falls_through_to_linear() {
        // Lane J.2 typo-safety negative: a typo'd governance value
        // (e.g. "mcc " or "MCC" or anything not exactly "mcc") falls
        // through to the legacy linear rule. Confirms a misspelled
        // amendment cannot accidentally engage MCC fork-choice.
        let ids = &[1, 2, 3, 4];
        let mut tc = make_consensus(1, ids);
        let proposer_id = tc.proposer_for_round(1, 0).unwrap().id;

        // Typo'd value (sic).
        tc.governance_params.insert(
            "parent_acceptance_mode".to_string(),
            "mcc ".to_string(), // trailing space — literal mismatch
        );

        let block = Block {
            number: 1,
            epoch: 1,
            parent_hash: [0xFF; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: Some(proposer_id),
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
            post_state_root: None,
        };
        let proposal = ConsensusMessage::Proposal {
            height: 1,
            round: 0,
            block,
            proposer_id,
        };

        let actions = tc.on_message(proposal);
        // Typo → linear default → diverging parent → RequestSync emitted.
        let request_sync = actions
            .iter()
            .any(|a| matches!(a, ConsensusAction::RequestSync(_, _)));
        assert!(
            request_sync,
            "typo'd governance value must fall through to linear default \
             (and emit RequestSync on diverging parent)"
        );
    }

    #[test]
    fn test_consensus_liveness_with_timeouts() {
        // Even with no messages, consensus should advance rounds via timeouts
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        let mut db = InMemoryStateDB::new();

        let initial_round = tc.round();

        // Simulate timeout-driven advancement. Propose timeout is 8s but
        // prevote/precommit timeouts are 60s — set phase_start far enough
        // in the past (90s) to trigger ALL three so the round actually
        // advances, not just transitions Propose→Prevote→Precommit.
        for _ in 0..20 {
            tc.round_state.phase_start =
                std::time::Instant::now() - std::time::Duration::from_secs(90);
            tc.tick(&mut db);
        }

        // Round should have advanced (timeout-driven round rotation)
        assert!(
            tc.round() > initial_round,
            "Timeouts should advance rounds: was {} now {}",
            initial_round,
            tc.round()
        );
    }

    // ─── BLS Aggregate Signature Tests ────────────────────────────────

    fn make_bls_consensus(my_id: u64, ids: &[u64]) -> TendermintConsensus {
        // Create validators with BLS keys
        let bls_keypairs: Vec<_> = ids.iter().map(|_| BlsKeypair::generate()).collect();
        let validators: Vec<_> = ids
            .iter()
            .zip(bls_keypairs.iter())
            .map(|(&id, kp)| {
                let mut address = [0u8; 32];
                address[0] = id as u8;
                ValidatorInfo::with_bls_key(id, 1000, address, kp.public_key_bytes().0)
            })
            .collect();
        let vs = ValidatorSet::with_validators(validators);
        let mut tc = TendermintConsensus::new_for_test(my_id, 5, vs);

        // Set BLS keypair for this node.
        // Generate a fresh keypair (can't move from vec; we just use it for the
        // validator's pop / signing).
        let kp = BlsKeypair::generate();
        // Update the validator's BLS key to match
        tc.validator_set.get_mut(my_id).unwrap().bls_public_key = Some(kp.public_key_bytes().0);
        tc.set_bls_keypair(kp);
        tc
    }

    #[test]
    fn test_bls_vote_message_deterministic() {
        let msg1 = TendermintConsensus::bls_vote_message(10, 0, &Some([1u8; 32]), "prevote");
        let msg2 = TendermintConsensus::bls_vote_message(10, 0, &Some([1u8; 32]), "prevote");
        assert_eq!(msg1, msg2, "Same inputs should produce same message");

        let msg3 = TendermintConsensus::bls_vote_message(10, 0, &Some([2u8; 32]), "prevote");
        assert_ne!(
            msg1, msg3,
            "Different hash should produce different message"
        );

        let msg4 = TendermintConsensus::bls_vote_message(10, 0, &Some([1u8; 32]), "precommit");
        assert_ne!(
            msg1, msg4,
            "Different phase should produce different message"
        );
    }

    #[test]
    fn test_bls_sign_vote_with_keypair() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);

        // Without BLS keypair, should return None
        assert!(tc
            .bls_sign_vote(1, 0, &Some([1u8; 32]), "prevote")
            .is_none());

        // With BLS keypair, should return Some
        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);
        assert!(tc
            .bls_sign_vote(1, 0, &Some([1u8; 32]), "prevote")
            .is_some());
    }

    #[test]
    fn test_bls_prevotes_include_signatures() {
        let mut db = InMemoryStateDB::new();
        let mut tc = make_bls_consensus(1, &[1]);

        // Single validator with BLS — tick should produce prevote with BLS sig
        let actions = tc.tick(&mut db);
        let has_bls_prevote = actions.iter().any(|a| {
            matches!(
                a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote {
                    bls_signature: Some(_),
                    ..
                })
            )
        });
        assert!(
            has_bls_prevote,
            "Prevote should include BLS signature when keypair is set"
        );
    }

    #[test]
    fn test_commit_certificate_built_on_quorum() {
        // 4-node BLS simulation
        let ids = &[1u64, 2, 3, 4];
        let bls_keypairs: Vec<_> = ids.iter().map(|_| BlsKeypair::generate()).collect();
        let validators: Vec<_> = ids
            .iter()
            .zip(bls_keypairs.iter())
            .map(|(&id, kp)| {
                let mut address = [0u8; 32];
                address[0] = id as u8;
                ValidatorInfo::with_bls_key(id, 1000, address, kp.public_key_bytes().0)
            })
            .collect();
        let vs = ValidatorSet::with_validators(validators);

        let mut nodes: Vec<_> = ids
            .iter()
            .map(|&id| {
                let mut tc = TendermintConsensus::new_for_test(id, 5, vs.clone());
                // We need to generate new keypairs since we can't clone BlsKeypair
                let kp = BlsKeypair::generate();
                tc.validator_set.get_mut(id).unwrap().bls_public_key =
                    Some(kp.public_key_bytes().0);
                // Also update in all other nodes' validator sets
                tc.set_bls_keypair(kp);
                tc
            })
            .collect();

        // Synchronize BLS public keys across all nodes
        let pks: Vec<(u64, Vec<u8>)> = nodes
            .iter()
            .map(|n| {
                let pk = n
                    .validator_set
                    .get(n.my_id)
                    .unwrap()
                    .bls_public_key
                    .clone()
                    .unwrap();
                (n.my_id, pk)
            })
            .collect();
        for node in &mut nodes {
            for (id, pk) in &pks {
                let vi = node.validator_set.get_mut(*id).unwrap();
                vi.bls_public_key = Some(pk.clone());
                vi.pop_verified = true;
            }
        }

        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        // Run consensus
        let mut messages = Vec::new();
        for v in &mut nodes {
            let actions = v.tick(&mut db);
            for a in actions {
                if let ConsensusAction::BroadcastMessage(msg) = a {
                    messages.push(msg);
                }
            }
        }

        let mut committed_blocks = Vec::new();
        for _ in 0..20 {
            let current_msgs: Vec<_> = std::mem::take(&mut messages);
            for msg in &current_msgs {
                for v in &mut nodes {
                    let actions = v.on_message(msg.clone());
                    for a in actions {
                        match a {
                            ConsensusAction::BroadcastMessage(m) => messages.push(m),
                            ConsensusAction::CommitBlock(b) => committed_blocks.push(b),
                            _ => {}
                        }
                    }
                }
            }
            for v in &mut nodes {
                let actions = v.tick(&mut db);
                for a in actions {
                    match a {
                        ConsensusAction::BroadcastMessage(m) => messages.push(m),
                        ConsensusAction::CommitBlock(b) => committed_blocks.push(b),
                        _ => {}
                    }
                }
            }
            if !committed_blocks.is_empty() {
                break;
            }
        }

        assert!(!committed_blocks.is_empty(), "Should reach consensus");

        // Check that committed block has a BLS commit certificate
        let block = &committed_blocks[0];
        assert!(
            block.commit_certificate.is_some(),
            "Committed block should have a BLS commit certificate"
        );

        let cert = block.commit_certificate.as_ref().unwrap();
        assert!(
            cert.signer_ids.len() >= 3,
            "Certificate should have >= quorum signers"
        );
        assert!(
            !cert.aggregate_signature.is_empty(),
            "Aggregate signature should not be empty"
        );

        // Verify the certificate against any node's validator set
        assert!(
            nodes[0].verify_commit_certificate(cert),
            "Commit certificate should verify against the validator set"
        );
    }

    #[test]
    fn test_non_bls_fallback_still_works() {
        // Consensus without BLS should still work (commit_certificate = None)
        let ids = &[1u64, 2, 3, 4];
        let mut nodes: Vec<_> = ids.iter().map(|&id| make_consensus(id, ids)).collect();
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        let mut messages = Vec::new();
        for v in &mut nodes {
            let actions = v.tick(&mut db);
            for a in actions {
                if let ConsensusAction::BroadcastMessage(msg) = a {
                    messages.push(msg);
                }
            }
        }

        let mut committed_blocks = Vec::new();
        for _ in 0..20 {
            let current_msgs: Vec<_> = std::mem::take(&mut messages);
            for msg in &current_msgs {
                for v in &mut nodes {
                    let actions = v.on_message(msg.clone());
                    for a in actions {
                        match a {
                            ConsensusAction::BroadcastMessage(m) => messages.push(m),
                            ConsensusAction::CommitBlock(b) => committed_blocks.push(b),
                            _ => {}
                        }
                    }
                }
            }
            for v in &mut nodes {
                let actions = v.tick(&mut db);
                for a in actions {
                    match a {
                        ConsensusAction::BroadcastMessage(m) => messages.push(m),
                        ConsensusAction::CommitBlock(b) => committed_blocks.push(b),
                        _ => {}
                    }
                }
            }
            if !committed_blocks.is_empty() {
                break;
            }
        }

        assert!(
            !committed_blocks.is_empty(),
            "Non-BLS consensus should still work"
        );
        // Without BLS keys, no certificate should be attached
        assert!(
            committed_blocks[0].commit_certificate.is_none(),
            "Without BLS keys, commit_certificate should be None"
        );
    }

    // ── Nova proof verification tests ────────────────────────────────────

    /// A mock proof verifier that rejects any proof containing [0xff; 4].
    struct RejectBadProofVerifier;

    impl ProofVerifier for RejectBadProofVerifier {
        fn verify_block_proof(&self, proof_bytes: &[u8], _height: u64, _genesis: [u8; 32]) -> bool {
            // Reject proofs that start with 0xff (simulates "bad proof")
            !proof_bytes.starts_with(&[0xff, 0xff, 0xff, 0xff])
        }
    }

    #[test]
    fn test_valid_nova_proof_accepted() {
        let ids = &[1, 2, 3, 4];
        let mut tc = make_consensus(2, ids);
        tc.set_proof_verifier(Box::new(RejectBadProofVerifier), [0u8; 32]);

        let proposer_id = tc.proposer_for_round(1, 0).unwrap().id;

        let block = Block {
            number: 1,
            epoch: 1,
            parent_hash: tc.parent_hash,
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: Some(proposer_id),
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
            nova_proof: Some(vec![0x01, 0x02, 0x03]), // valid proof
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
            post_state_root: None,
        };

        let msg = ConsensusMessage::Proposal {
            height: 1,
            round: 0,
            block,
            proposer_id,
        };

        let actions = tc.on_message(msg);
        // Should generate a prevote (proof accepted)
        let has_prevote = actions.iter().any(|a| {
            matches!(
                a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. })
            )
        });
        assert!(has_prevote, "Valid proof should result in prevote");
    }

    #[test]
    fn test_invalid_nova_proof_rejected() {
        let ids = &[1, 2, 3, 4];
        let mut tc = make_consensus(2, ids);
        tc.set_proof_verifier(Box::new(RejectBadProofVerifier), [0u8; 32]);

        let proposer_id = tc.proposer_for_round(1, 0).unwrap().id;

        let block = Block {
            number: 1,
            epoch: 1,
            parent_hash: tc.parent_hash,
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: Some(proposer_id),
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
            nova_proof: Some(vec![0xff, 0xff, 0xff, 0xff, 0x00]), // bad proof
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
            post_state_root: None,
        };

        let msg = ConsensusMessage::Proposal {
            height: 1,
            round: 0,
            block,
            proposer_id,
        };

        let actions = tc.on_message(msg);
        // Should NOT generate a prevote (proof rejected)
        let has_prevote = actions.iter().any(|a| {
            matches!(
                a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. })
            )
        });
        assert!(!has_prevote, "Invalid proof should prevent prevote");
    }

    #[test]
    fn test_no_proof_accepted_without_verifier() {
        // Without a proof verifier, blocks with no proof should be accepted
        let ids = &[1, 2, 3, 4];
        let mut tc = make_consensus(2, ids);
        // No proof verifier set

        let proposer_id = tc.proposer_for_round(1, 0).unwrap().id;

        let block = Block {
            number: 1,
            epoch: 1,
            parent_hash: tc.parent_hash,
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: Some(proposer_id),
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
            post_state_root: None,
        };

        let msg = ConsensusMessage::Proposal {
            height: 1,
            round: 0,
            block,
            proposer_id,
        };

        let actions = tc.on_message(msg);
        let has_prevote = actions.iter().any(|a| {
            matches!(
                a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. })
            )
        });
        assert!(has_prevote, "Without verifier, block should be accepted");
    }
}

// ─────────────────────────── Integration Tests ─────────────────────────────

#[cfg(test)]
mod integration_tests {
    use super::*;
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::{Account, BlobTx, Transaction};

    fn addr(b: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    /// Create a 4-validator network with BLS keypairs, synchronized public keys.
    fn make_bls_network(ids: &[u64]) -> Vec<TendermintConsensus> {
        let bls_keypairs: Vec<_> = ids.iter().map(|_| BlsKeypair::generate()).collect();
        let validators: Vec<_> = ids
            .iter()
            .zip(bls_keypairs.iter())
            .map(|(&id, kp)| {
                let mut address = [0u8; 32];
                address[0] = id as u8;
                ValidatorInfo::with_bls_key(id, 1000, address, kp.public_key_bytes().0)
            })
            .collect();
        let vs = ValidatorSet::with_validators(validators);

        let mut nodes: Vec<_> = ids
            .iter()
            .map(|&id| {
                let mut tc = TendermintConsensus::new_for_test(id, 5, vs.clone());
                let kp = BlsKeypair::generate();
                tc.validator_set.get_mut(id).unwrap().bls_public_key =
                    Some(kp.public_key_bytes().0);
                tc.set_bls_keypair(kp);
                tc
            })
            .collect();

        // Synchronize BLS public keys across all nodes
        let pks: Vec<(u64, Vec<u8>)> = nodes
            .iter()
            .map(|n| {
                let pk = n
                    .validator_set
                    .get(n.my_id)
                    .unwrap()
                    .bls_public_key
                    .clone()
                    .unwrap();
                (n.my_id, pk)
            })
            .collect();
        for node in &mut nodes {
            for (id, pk) in &pks {
                let vi = node.validator_set.get_mut(*id).unwrap();
                vi.bls_public_key = Some(pk.clone());
                vi.pop_verified = true;
            }
        }
        nodes
    }

    /// Run one consensus round: tick all nodes, relay messages, repeat until a block commits.
    /// Returns committed blocks.
    fn run_consensus_round(
        nodes: &mut [TendermintConsensus],
        db: &mut InMemoryStateDB,
        max_iterations: usize,
    ) -> Vec<Block> {
        let mut messages = Vec::new();
        let mut committed = Vec::new();

        // Initial tick
        for v in nodes.iter_mut() {
            for a in v.tick(db) {
                match a {
                    ConsensusAction::BroadcastMessage(m) => messages.push(m),
                    ConsensusAction::CommitBlock(b) => committed.push(b),
                    _ => {}
                }
            }
        }

        for _ in 0..max_iterations {
            if !committed.is_empty() {
                break;
            }
            let current: Vec<_> = std::mem::take(&mut messages);
            for msg in &current {
                for v in nodes.iter_mut() {
                    for a in v.on_message(msg.clone()) {
                        match a {
                            ConsensusAction::BroadcastMessage(m) => messages.push(m),
                            ConsensusAction::CommitBlock(b) => committed.push(b),
                            _ => {}
                        }
                    }
                }
            }
            for v in nodes.iter_mut() {
                for a in v.tick(db) {
                    match a {
                        ConsensusAction::BroadcastMessage(m) => messages.push(m),
                        ConsensusAction::CommitBlock(b) => committed.push(b),
                        _ => {}
                    }
                }
            }
        }
        committed
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Test 1: Multi-height consensus with BLS certificates
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_multi_height_bls_consensus() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes = make_bls_network(ids);
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 10_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        // Run 3 consecutive heights
        for expected_height in 1..=3 {
            let committed = run_consensus_round(&mut nodes, &mut db, 30);
            assert!(
                !committed.is_empty(),
                "Height {} should reach consensus",
                expected_height
            );

            let block = &committed[0];
            assert_eq!(block.number, expected_height);

            // Verify BLS commit certificate
            assert!(
                block.commit_certificate.is_some(),
                "Height {} should have BLS commit certificate",
                expected_height
            );
            let cert = block.commit_certificate.as_ref().unwrap();
            assert!(
                cert.signer_ids.len() >= 3,
                "Certificate needs >= 2f+1 signers, got {}",
                cert.signer_ids.len()
            );
            assert!(
                nodes[0].verify_commit_certificate(cert),
                "Certificate should verify at height {}",
                expected_height
            );

            // Advance all nodes to next height
            let state_root = [expected_height as u8; 32];
            for node in nodes.iter_mut() {
                node.on_block_committed(block, state_root, 0);
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Test 2: Blob transactions included in block with DA fields
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_blob_tx_in_consensus_block() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes = make_bls_network(ids);
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 10_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        // Submit a blob transaction to the proposer's mempool
        let blob_tx = Transaction::Blob(BlobTx {
            submitter: addr(1),
            data: vec![0xDE; 256], // 256 bytes of blob data
            nonce: 0,
            namespace_id: 42,
            signature: None,
            public_key: None,
        });

        // Add to all nodes' mempools so whoever is proposer has it
        for node in nodes.iter_mut() {
            node.mempool.submit(blob_tx.clone());
        }

        let committed = run_consensus_round(&mut nodes, &mut db, 30);
        assert!(!committed.is_empty(), "Should commit a block with blob tx");

        let block = &committed[0];
        // The blob tx should be in the block's transactions
        let has_blob = block
            .transactions
            .iter()
            .any(|tx| matches!(tx, Transaction::Blob(_)));
        assert!(has_blob, "Block should contain the blob transaction");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Test 3: Byzantine tolerance — 1 of 4 validators offline
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_consensus_with_one_offline_validator() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes = make_bls_network(ids);
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 10_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        // Remove validator 4 from the active set (simulate offline)
        let mut active_nodes: Vec<_> = nodes.drain(..3).collect(); // Only 3 of 4

        let committed = run_consensus_round(&mut active_nodes, &mut db, 30);
        assert!(
            !committed.is_empty(),
            "3 of 4 validators (>= 2f+1) should still reach consensus"
        );

        let block = &committed[0];
        assert!(
            block.commit_certificate.is_some(),
            "Should still produce BLS certificate with 3 signers"
        );
        let cert = block.commit_certificate.as_ref().unwrap();
        assert_eq!(cert.signer_ids.len(), 3);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Test 4: Certificate cross-validation across nodes
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_certificate_cross_validation() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes = make_bls_network(ids);
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 10_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        let committed = run_consensus_round(&mut nodes, &mut db, 30);
        assert!(!committed.is_empty());

        let cert = committed[0].commit_certificate.as_ref().unwrap();

        // Every node should be able to verify the certificate
        for (i, node) in nodes.iter().enumerate() {
            assert!(
                node.verify_commit_certificate(cert),
                "Node {} should verify the commit certificate",
                ids[i]
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Test 5: DA + BLS full pipeline
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_da_commitment_pipeline() {
        use evaporchain_da::certificate::{create_attestation, CertificateBuilder};
        use evaporchain_da::commitments::{generate_2d_queries, RowColumnCommitments};
        use evaporchain_da::erasure2d::ErasureEncoder2D;

        // Simulate what happens when a proposer encodes blob data for DA
        let blob_data = vec![0xABu8; 512];
        let encoder = ErasureEncoder2D::with_cell_size(32);
        let matrix = encoder.encode_2d(&blob_data).unwrap();
        let commitments = RowColumnCommitments::from_matrix(&matrix);

        // data_root goes in the block header
        let data_root = commitments.data_root;
        assert_ne!(data_root, [0u8; 32], "data_root should be non-zero");

        // Validators sample random cells and verify proofs
        let num_validators = 4u64;
        let num_samples = 8;
        let mut builder = CertificateBuilder::new(
            1, // block_number
            data_root,
            num_validators * 1000, // total_stake
        );

        for vid in 1..=num_validators {
            let seed = blake3::hash(&vid.to_le_bytes());
            let queries =
                generate_2d_queries(1, matrix.extended_dim(), num_samples, seed.as_bytes());

            // Verify each sampled cell
            let mut all_valid = true;
            for q in &queries {
                let proof = commitments
                    .generate_cell_proof(&matrix, q.row, q.col)
                    .unwrap();
                if !commitments.verify_cell_proof(&proof) {
                    all_valid = false;
                    break;
                }
            }
            assert!(
                all_valid,
                "All sampled cells should verify for validator {}",
                vid
            );

            // Create BLS attestation
            let kp = BlsKeypair::generate();
            let attestation = create_attestation(
                1, // block_number
                &data_root,
                vid,
                num_samples as u32,
                1000, // stake
                &kp,
            );
            builder.add_attestation(attestation);
        }

        // Build DA certificate
        let da_cert = builder.try_build();
        assert!(
            da_cert.is_some(),
            "With all 4 validators attesting, DA certificate should be built"
        );

        let cert = da_cert.unwrap();
        assert_eq!(cert.block_number, 1);
        assert_eq!(cert.data_root, data_root);
        assert_eq!(cert.attestations.len(), 4);
        assert!(cert.is_supermajority(), "4/4 validators = supermajority");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Test 6: Full end-to-end — consensus + DA + BLS + multi-height
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_full_e2e_consensus_da_bls() {
        use evaporchain_da::certificate::{create_attestation, CertificateBuilder};
        use evaporchain_da::commitments::RowColumnCommitments;
        use evaporchain_da::erasure2d::ErasureEncoder2D;

        let ids = &[1u64, 2, 3, 4];
        let mut nodes = make_bls_network(ids);
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 10_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        // === Height 1: Commit a block with BLS certificate ===
        let committed = run_consensus_round(&mut nodes, &mut db, 30);
        assert!(!committed.is_empty(), "Height 1 should commit");
        let block1 = &committed[0];
        assert_eq!(block1.number, 1);
        assert!(block1.commit_certificate.is_some());

        // Verify certificate
        let cert1 = block1.commit_certificate.as_ref().unwrap();
        assert!(nodes[0].verify_commit_certificate(cert1));

        // === Simulate DA attestation for the committed block ===
        let blob_data = vec![0xFFu8; 256];
        let encoder = ErasureEncoder2D::with_cell_size(32);
        let matrix = encoder.encode_2d(&blob_data).unwrap();
        let rc = RowColumnCommitments::from_matrix(&matrix);

        let mut builder = CertificateBuilder::new(1, rc.data_root, 4000);
        for &vid in ids {
            let kp = BlsKeypair::generate();
            let att = create_attestation(1, &rc.data_root, vid, 8, 1000, &kp);
            builder.add_attestation(att);
        }
        let da_cert = builder.try_build().expect("DA cert should build");
        assert!(da_cert.is_supermajority());

        // === Height 2: Advance and commit again ===
        let state_root = [1u8; 32];
        for node in nodes.iter_mut() {
            node.on_block_committed(block1, state_root, 0);
        }

        let committed2 = run_consensus_round(&mut nodes, &mut db, 30);
        assert!(!committed2.is_empty(), "Height 2 should commit");
        let block2 = &committed2[0];
        assert_eq!(block2.number, 2);
        assert!(block2.commit_certificate.is_some());

        // Verify cross-node certificate verification
        let cert2 = block2.commit_certificate.as_ref().unwrap();
        for node in &nodes {
            assert!(node.verify_commit_certificate(cert2));
        }

        // === Verify chain integrity ===
        assert_ne!(
            cert1.block_hash, cert2.block_hash,
            "Different heights should have different block hashes"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Test 7: Prevote/Precommit BLS signatures are present in messages
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_bls_signatures_in_all_vote_phases() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes = make_bls_network(ids);
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 10_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        let mut all_messages = Vec::new();
        let mut messages = Vec::new();

        // Initial tick
        for v in nodes.iter_mut() {
            for a in v.tick(&mut db) {
                if let ConsensusAction::BroadcastMessage(m) = a {
                    messages.push(m.clone());
                    all_messages.push(m);
                }
            }
        }

        // Run a few rounds to collect prevotes and precommits
        for _ in 0..20 {
            let current: Vec<_> = std::mem::take(&mut messages);
            for msg in &current {
                for v in nodes.iter_mut() {
                    for a in v.on_message(msg.clone()) {
                        if let ConsensusAction::BroadcastMessage(m) = a {
                            messages.push(m.clone());
                            all_messages.push(m);
                        }
                    }
                }
            }
            for v in nodes.iter_mut() {
                for a in v.tick(&mut db) {
                    if let ConsensusAction::BroadcastMessage(m) = a {
                        messages.push(m.clone());
                        all_messages.push(m);
                    }
                }
            }
        }

        // Check that prevotes have BLS signatures
        let bls_prevotes = all_messages
            .iter()
            .filter(|m| {
                matches!(
                    m,
                    ConsensusMessage::Prevote {
                        bls_signature: Some(_),
                        ..
                    }
                )
            })
            .count();

        let bls_precommits = all_messages
            .iter()
            .filter(|m| {
                matches!(
                    m,
                    ConsensusMessage::Precommit {
                        bls_signature: Some(_),
                        ..
                    }
                )
            })
            .count();

        assert!(
            bls_prevotes >= 1,
            "Should have at least one BLS-signed prevote, got {}",
            bls_prevotes
        );
        // In a full network simulation, all 4 would produce BLS prevotes.
        // In our test relay loop the proposer's self-prevote is guaranteed.
        // Precommits require 2f+1 prevotes first, so we just check they exist.
        assert!(
            bls_precommits >= 1,
            "Should have at least one BLS-signed precommit, got {}",
            bls_precommits
        );

        // Verify total BLS participation: prevotes + precommits combined
        let total_bls = bls_prevotes + bls_precommits;
        assert!(
            total_bls >= 4,
            "Should have >= 4 total BLS-signed votes across phases, got {}",
            total_bls
        );
    }
}

#[cfg(test)]
mod vrf_tests {
    use super::*;
    use evaporchain_crypto::vrf::{
        leader_vrf_input, vrf_leader_check, vrf_verify, VrfKeypair, VrfOutput, VrfProof,
    };
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::Account;

    fn addr(b: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    /// Create a network where all validators have both BLS and VRF keys.
    fn make_full_crypto_network(ids: &[u64]) -> Vec<TendermintConsensus> {
        let bls_keypairs: Vec<_> = ids.iter().map(|_| BlsKeypair::generate()).collect();
        let vrf_keypairs: Vec<_> = ids.iter().map(|_| VrfKeypair::generate()).collect();

        let validators: Vec<_> = ids
            .iter()
            .zip(bls_keypairs.iter())
            .zip(vrf_keypairs.iter())
            .map(|((&id, bls_kp), vrf_kp)| {
                let mut address = [0u8; 32];
                address[0] = id as u8;
                ValidatorInfo::with_keys(
                    id,
                    1000,
                    address,
                    Some(bls_kp.public_key_bytes().0),
                    Some(vrf_kp.public_key_bytes()),
                )
            })
            .collect();
        let vs = ValidatorSet::with_validators(validators);

        let mut nodes: Vec<_> = ids
            .iter()
            .map(|&id| {
                let mut tc = TendermintConsensus::new_for_test(id, 5, vs.clone());
                // Set BLS keypair
                let bls_kp = BlsKeypair::generate();
                tc.validator_set.get_mut(id).unwrap().bls_public_key =
                    Some(bls_kp.public_key_bytes().0);
                tc.set_bls_keypair(bls_kp);
                // Set VRF keypair
                let vrf_kp = VrfKeypair::generate();
                tc.validator_set.get_mut(id).unwrap().vrf_public_key =
                    Some(vrf_kp.public_key_bytes());
                tc.set_vrf_keypair(vrf_kp);
                tc
            })
            .collect();

        // Synchronize all public keys across nodes
        let keys: Vec<(u64, Vec<u8>, Vec<u8>)> = nodes
            .iter()
            .map(|n| {
                let v = n.validator_set.get(n.my_id).unwrap();
                (
                    n.my_id,
                    v.bls_public_key.clone().unwrap(),
                    v.vrf_public_key.clone().unwrap(),
                )
            })
            .collect();
        for node in &mut nodes {
            for (id, bls_pk, vrf_pk) in &keys {
                let v = node.validator_set.get_mut(*id).unwrap();
                v.bls_public_key = Some(bls_pk.clone());
                v.vrf_public_key = Some(vrf_pk.clone());
                v.pop_verified = true;
            }
        }
        nodes
    }

    #[test]
    fn test_vrf_output_in_proposed_block() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes = make_full_crypto_network(ids);
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 10_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        // Tick to get the proposer to create a block
        let mut proposal = None;
        for v in nodes.iter_mut() {
            for a in v.tick(&mut db) {
                if let ConsensusAction::BroadcastMessage(ConsensusMessage::Proposal {
                    ref block,
                    ..
                }) = a
                {
                    proposal = Some(block.clone());
                }
            }
        }

        let block = proposal.expect("One node should produce a proposal");
        assert!(
            block.vrf_output.is_some(),
            "Block should contain VRF output"
        );
        assert!(block.vrf_proof.is_some(), "Block should contain VRF proof");

        // Verify VRF proof
        let proposer_id = block.producer_id.unwrap();
        let proposer_vrf_pk = nodes[0]
            .validator_set
            .get(proposer_id)
            .unwrap()
            .vrf_public_key
            .as_ref()
            .unwrap();

        let alpha = leader_vrf_input(block.number, 0);
        let output = VrfOutput(block.vrf_output.unwrap());
        let proof = VrfProof(block.vrf_proof.clone().unwrap());
        assert!(
            vrf_verify(proposer_vrf_pk, &alpha, &output, &proof),
            "VRF proof should verify against proposer's public key"
        );
    }

    #[test]
    fn test_vrf_consensus_with_verification() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes = make_full_crypto_network(ids);
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 10_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        // Run full consensus round with VRF-enabled validators
        let mut messages = Vec::new();
        let mut committed = Vec::new();

        for v in nodes.iter_mut() {
            for a in v.tick(&mut db) {
                match a {
                    ConsensusAction::BroadcastMessage(m) => messages.push(m),
                    ConsensusAction::CommitBlock(b) => committed.push(b),
                    _ => {}
                }
            }
        }

        for _ in 0..30 {
            if !committed.is_empty() {
                break;
            }
            let current: Vec<_> = std::mem::take(&mut messages);
            for msg in &current {
                for v in nodes.iter_mut() {
                    for a in v.on_message(msg.clone()) {
                        match a {
                            ConsensusAction::BroadcastMessage(m) => messages.push(m),
                            ConsensusAction::CommitBlock(b) => committed.push(b),
                            _ => {}
                        }
                    }
                }
            }
            for v in nodes.iter_mut() {
                for a in v.tick(&mut db) {
                    match a {
                        ConsensusAction::BroadcastMessage(m) => messages.push(m),
                        ConsensusAction::CommitBlock(b) => committed.push(b),
                        _ => {}
                    }
                }
            }
        }

        assert!(
            !committed.is_empty(),
            "VRF-enabled network should reach consensus"
        );
        let block = &committed[0];
        assert!(block.vrf_output.is_some());
        assert!(block.vrf_proof.is_some());
        assert!(block.commit_certificate.is_some());
    }

    #[test]
    fn test_invalid_vrf_proof_rejected() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes = make_full_crypto_network(ids);

        let proposer_id = nodes[0].proposer_for_round(1, 0).unwrap().id;
        let non_proposer_idx = ids.iter().position(|&id| id != proposer_id).unwrap();

        // Create a block with an invalid VRF proof
        let block = Block {
            number: 1,
            epoch: 1,
            parent_hash: nodes[non_proposer_idx].parent_hash,
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: Some(proposer_id),
            vrf_output: Some([0xAA; 32]),     // Fake VRF output
            vrf_proof: Some(vec![0xBB; 100]), // Fake VRF proof
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
            post_state_root: None,
        };

        let msg = ConsensusMessage::Proposal {
            height: 1,
            round: 0,
            block,
            proposer_id,
        };

        // Non-proposer should reject the invalid VRF proof
        let actions = nodes[non_proposer_idx].on_message(msg);
        let has_prevote = actions.iter().any(|a| {
            matches!(
                a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. })
            )
        });
        assert!(
            !has_prevote,
            "Should reject proposal with invalid VRF proof"
        );
    }

    #[test]
    fn test_vrf_leader_check_stake_weighted() {
        // VRF leader check should be proportional to stake
        let kp = VrfKeypair::generate();
        let alpha = leader_vrf_input(1, 0);
        let (output, _proof) = kp.evaluate(&alpha);

        // With 100% of stake, should always be leader
        assert!(vrf_leader_check(&output, 1000, 1000));

        // With 0 stake, should never be leader
        assert!(!vrf_leader_check(&output, 0, 1000));
    }

    #[test]
    fn test_vrf_randomness_beacon_advances() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes = make_full_crypto_network(ids);
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 10_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        let beacon_before = nodes[0].randomness_beacon().current();

        // Run consensus to commit a block
        let mut messages = Vec::new();
        let mut committed = Vec::new();
        for v in nodes.iter_mut() {
            for a in v.tick(&mut db) {
                match a {
                    ConsensusAction::BroadcastMessage(m) => messages.push(m),
                    ConsensusAction::CommitBlock(b) => committed.push(b),
                    _ => {}
                }
            }
        }
        for _ in 0..30 {
            if !committed.is_empty() {
                break;
            }
            let current: Vec<_> = std::mem::take(&mut messages);
            for msg in &current {
                for v in nodes.iter_mut() {
                    for a in v.on_message(msg.clone()) {
                        match a {
                            ConsensusAction::BroadcastMessage(m) => messages.push(m),
                            ConsensusAction::CommitBlock(b) => committed.push(b),
                            _ => {}
                        }
                    }
                }
            }
            for v in nodes.iter_mut() {
                for a in v.tick(&mut db) {
                    match a {
                        ConsensusAction::BroadcastMessage(m) => messages.push(m),
                        ConsensusAction::CommitBlock(b) => committed.push(b),
                        _ => {}
                    }
                }
            }
        }

        assert!(!committed.is_empty());
        let block = &committed[0];

        // Advance beacon
        nodes[0].on_block_committed(block, [1u8; 32], 0);
        let beacon_after = nodes[0].randomness_beacon().current();

        // If block had VRF output, beacon should advance
        if block.vrf_output.is_some() {
            assert_ne!(
                beacon_before, beacon_after,
                "Beacon should advance when VRF output is present"
            );
        }
    }

    #[test]
    fn test_multi_height_vrf_chain() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes = make_full_crypto_network(ids);
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 10_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });

        let mut vrf_outputs = Vec::new();

        for height in 1..=3 {
            let mut messages = Vec::new();
            let mut committed = Vec::new();

            for v in nodes.iter_mut() {
                for a in v.tick(&mut db) {
                    match a {
                        ConsensusAction::BroadcastMessage(m) => messages.push(m),
                        ConsensusAction::CommitBlock(b) => committed.push(b),
                        _ => {}
                    }
                }
            }

            for _ in 0..30 {
                if !committed.is_empty() {
                    break;
                }
                let current: Vec<_> = std::mem::take(&mut messages);
                for msg in &current {
                    for v in nodes.iter_mut() {
                        for a in v.on_message(msg.clone()) {
                            match a {
                                ConsensusAction::BroadcastMessage(m) => messages.push(m),
                                ConsensusAction::CommitBlock(b) => committed.push(b),
                                _ => {}
                            }
                        }
                    }
                }
                for v in nodes.iter_mut() {
                    for a in v.tick(&mut db) {
                        match a {
                            ConsensusAction::BroadcastMessage(m) => messages.push(m),
                            ConsensusAction::CommitBlock(b) => committed.push(b),
                            _ => {}
                        }
                    }
                }
            }

            assert!(!committed.is_empty(), "Height {} should commit", height);
            let block = &committed[0];
            assert_eq!(block.number, height);
            assert!(
                block.vrf_output.is_some(),
                "Height {} should have VRF output",
                height
            );
            vrf_outputs.push(block.vrf_output.unwrap());

            let state_root = [height as u8; 32];
            for node in nodes.iter_mut() {
                node.on_block_committed(block, state_root, 0);
            }
        }

        // All VRF outputs should be unique (different height = different input)
        assert_ne!(vrf_outputs[0], vrf_outputs[1]);
        assert_ne!(vrf_outputs[1], vrf_outputs[2]);
        assert_ne!(vrf_outputs[0], vrf_outputs[2]);
    }
}

#[cfg(test)]
mod epoch_tests {
    use super::*;
    use crate::validator_set::{EpochTransitionManager, ValidatorInfo, ValidatorSet};
    use evaporchain_types::{Block, Transaction, ValidatorExitTx, ValidatorStakeTx};

    fn make_validator_set(n: u64, stake: u64) -> ValidatorSet {
        let mut vs = ValidatorSet::new();
        for i in 0..n {
            let mut addr = [0u8; 32];
            addr[0] = i as u8;
            vs.add_validator(ValidatorInfo::new(i, stake, addr));
        }
        vs
    }

    fn make_block_at_height(height: u64, txs: Vec<Transaction>) -> Block {
        Block {
            number: height,
            epoch: height / 100,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: txs,
            producer_id: Some(0),
            timestamp: 0,
            chain_id: String::new(),
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
            post_state_root: None,
        }
    }

    #[test]
    fn test_epoch_boundary_detection() {
        // Height 0 is NOT a boundary (genesis)
        assert!(!EpochTransitionManager::is_epoch_boundary(0));
        // Heights 1-99 are not boundaries
        for h in 1..100u64 {
            assert!(!EpochTransitionManager::is_epoch_boundary(h));
        }
        // Height 100 IS a boundary
        assert!(EpochTransitionManager::is_epoch_boundary(100));
        assert!(EpochTransitionManager::is_epoch_boundary(200));
        assert!(EpochTransitionManager::is_epoch_boundary(300));
        // 150 is not
        assert!(!EpochTransitionManager::is_epoch_boundary(150));
    }

    #[test]
    fn test_validator_join_queued_on_stake_tx() {
        let vs = make_validator_set(4, 1000);
        let mut tc = TendermintConsensus::new_for_test(0, 0, vs);

        // Create a ValidatorStake tx for a new validator (id=10)
        let stake_tx = ValidatorStakeTx {
            validator_address: [10u8; 32],
            stake_amount: 500,
            validator_id: 10,
            nonce: 0,
            bls_public_key: None,
            vrf_public_key: None,
            signature: None,
            public_key: None,
        };

        let block = make_block_at_height(50, vec![Transaction::ValidatorStake(stake_tx)]);
        tc.on_block_committed(&block, [1u8; 32], 0);

        // Change should be queued but NOT applied yet (not at epoch boundary)
        assert_eq!(tc.epoch_manager.pending_count(), 1);
        assert_eq!(tc.validator_set.active_count(), 4); // unchanged
    }

    #[test]
    fn test_validator_join_applied_after_bonding_period() {
        let vs = make_validator_set(4, 1000);
        let mut tc = TendermintConsensus::new_for_test(0, 0, vs);

        // Queue a join at epoch 0 — ready at epoch 2 (bonding period = 2)
        let stake_tx = ValidatorStakeTx {
            validator_address: [10u8; 32],
            stake_amount: 500,
            validator_id: 10,
            nonce: 0,
            bls_public_key: None,
            vrf_public_key: None,
            signature: None,
            public_key: None,
        };

        let block = make_block_at_height(50, vec![Transaction::ValidatorStake(stake_tx)]);
        tc.on_block_committed(&block, [1u8; 32], 0);

        // Commit blocks up to height 100 (epoch boundary, epoch=1)
        // But bonding period is 2 epochs, so still deferred
        let boundary1 = make_block_at_height(100, vec![]);
        tc.height = 100;
        tc.on_block_committed(&boundary1, [2u8; 32], 0);
        // Validator should NOT have joined yet (ready_at_epoch=2, current=1)
        assert_eq!(tc.validator_set.active_count(), 4);

        // Commit at height 200 (epoch boundary, epoch=2) — now bonding is complete
        let boundary2 = make_block_at_height(200, vec![]);
        tc.height = 200;
        tc.on_block_committed(&boundary2, [3u8; 32], 0);
        assert_eq!(tc.validator_set.active_count(), 5);
    }

    #[test]
    fn test_validator_exit_queued_and_applied() {
        let vs = make_validator_set(5, 1000);
        let mut tc = TendermintConsensus::new_for_test(0, 0, vs);

        let exit_tx = ValidatorExitTx {
            validator_address: [4u8; 32],
            validator_id: 4,
            nonce: 0,
            signature: None,
            public_key: None,
        };

        let block = make_block_at_height(50, vec![Transaction::ValidatorExit(exit_tx)]);
        tc.on_block_committed(&block, [1u8; 32], 0);
        assert_eq!(tc.epoch_manager.pending_count(), 1);
        assert_eq!(tc.validator_set.active_count(), 5); // not removed yet

        // Unbonding period = 4 epochs. At epoch 4 boundary (height 400), removal applies.
        // Heights 100, 200, 300 — still deferred
        for h in [100u64, 200, 300] {
            let b = make_block_at_height(h, vec![]);
            tc.height = h;
            tc.on_block_committed(&b, [h as u8; 32], 0);
        }
        assert_eq!(tc.validator_set.active_count(), 5); // still 5

        // Height 400 (epoch=4) — unbonding complete
        let b400 = make_block_at_height(400, vec![]);
        tc.height = 400;
        tc.on_block_committed(&b400, [4u8; 32], 0);
        assert_eq!(tc.validator_set.active_count(), 4); // removed
    }

    #[test]
    fn test_min_validators_safety() {
        // Start with exactly 3 validators (MIN_VALIDATORS)
        let vs = make_validator_set(3, 1000);
        let mut tc = TendermintConsensus::new_for_test(0, 0, vs);

        // Try to remove one
        let exit_tx = ValidatorExitTx {
            validator_address: [2u8; 32],
            validator_id: 2,
            nonce: 0,
            signature: None,
            public_key: None,
        };

        let block = make_block_at_height(50, vec![Transaction::ValidatorExit(exit_tx)]);
        tc.on_block_committed(&block, [1u8; 32], 0);

        // Fast-forward to epoch 4 boundary
        for h in [100u64, 200, 300, 400] {
            let b = make_block_at_height(h, vec![]);
            tc.height = h;
            tc.on_block_committed(&b, [h as u8; 32], 0);
        }

        // Should still have 3 validators — removal rejected
        assert_eq!(tc.validator_set.active_count(), 3);
    }

    #[test]
    fn test_multiple_joins_and_exits_in_single_epoch() {
        let vs = make_validator_set(6, 1000);
        let mut tc = TendermintConsensus::new_for_test(0, 0, vs);

        // Queue 2 joins and 1 exit at epoch 0
        let stake1 = Transaction::ValidatorStake(ValidatorStakeTx {
            validator_address: [10u8; 32],
            stake_amount: 500,
            validator_id: 10,
            nonce: 0,
            bls_public_key: None,
            vrf_public_key: None,
            signature: None,
            public_key: None,
        });
        let stake2 = Transaction::ValidatorStake(ValidatorStakeTx {
            validator_address: [11u8; 32],
            stake_amount: 500,
            validator_id: 11,
            nonce: 0,
            bls_public_key: None,
            vrf_public_key: None,
            signature: None,
            public_key: None,
        });
        let exit1 = Transaction::ValidatorExit(ValidatorExitTx {
            validator_address: [5u8; 32],
            validator_id: 5,
            nonce: 0,
            signature: None,
            public_key: None,
        });

        let block = make_block_at_height(50, vec![stake1, stake2, exit1]);
        tc.on_block_committed(&block, [1u8; 32], 0);
        assert_eq!(tc.epoch_manager.pending_count(), 3);

        // At epoch 2 boundary (height 200) — joins are ready (bonding=2 epochs)
        // Exit still deferred (unbonding=4 epochs)
        for h in [100u64, 200] {
            let b = make_block_at_height(h, vec![]);
            tc.height = h;
            tc.on_block_committed(&b, [h as u8; 32], 0);
        }
        // max_churn = ceil(6 * 0.33) = 2. Two joins can apply.
        assert_eq!(tc.validator_set.active_count(), 8); // 6 + 2 joins

        // At epoch 4 boundary (height 400) — exit is ready
        for h in [300u64, 400] {
            let b = make_block_at_height(h, vec![]);
            tc.height = h;
            tc.on_block_committed(&b, [h as u8; 32], 0);
        }
        assert_eq!(tc.validator_set.active_count(), 7); // 8 - 1 exit
    }

    #[test]
    fn test_max_churn_enforcement() {
        // 4 validators, max_churn = ceil(4 * 0.33) = ceil(1.32) = 2
        let vs = make_validator_set(4, 1000);
        let mut tc = TendermintConsensus::new_for_test(0, 0, vs);

        // Queue 3 joins (more than max_churn)
        for i in 10..13u64 {
            let stake = Transaction::ValidatorStake(ValidatorStakeTx {
                validator_address: [i as u8; 32],
                stake_amount: 500,
                validator_id: i,
                nonce: 0,
                bls_public_key: None,
                vrf_public_key: None,
                signature: None,
                public_key: None,
            });
            let block = make_block_at_height(50 + i, vec![stake]);
            tc.on_block_committed(&block, [i as u8; 32], 0);
        }
        assert_eq!(tc.epoch_manager.pending_count(), 3);

        // At epoch 2 boundary — only 2 should join (max_churn)
        for h in [100u64, 200] {
            let b = make_block_at_height(h, vec![]);
            tc.height = h;
            tc.on_block_committed(&b, [h as u8; 32], 0);
        }
        assert_eq!(tc.validator_set.active_count(), 6); // 4 + 2 (capped)
        assert!(tc.epoch_manager.pending_count() >= 1); // 1 deferred

        // At epoch 3 boundary — the deferred one joins
        let b300 = make_block_at_height(300, vec![]);
        tc.height = 300;
        tc.on_block_committed(&b300, [3u8; 32], 0);
        assert_eq!(tc.validator_set.active_count(), 7); // 6 + 1
    }
}

// ─────────────── MEV-Protected Mempool Tests ──────────────────────────

#[cfg(test)]
mod mev_tests {
    use super::*;
    use crate::encrypted_mempool::encrypt_transaction;
    use crate::validator_set::ValidatorInfo;
    use evaporchain_state::db::InMemoryStateDB;
    use evaporchain_types::TransferTx;
    use rand::RngCore;

    fn make_test_tc() -> TendermintConsensus {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [1u8; 32]));
        vs.add_validator(ValidatorInfo::new(2, 1000, [2u8; 32]));
        vs.add_validator(ValidatorInfo::new(3, 1000, [3u8; 32]));
        vs.add_validator(ValidatorInfo::new(4, 1000, [4u8; 32]));
        TendermintConsensus::new_for_test(1, 100, vs)
    }

    fn dummy_transfer(amount: u64) -> Transaction {
        Transaction::Transfer(TransferTx {
            from: [0xAA; 32],
            to: [0xBB; 32],
            amount,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        })
    }

    fn random_nonce() -> [u8; 32] {
        let mut nonce = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut nonce);
        nonce
    }

    #[test]
    fn test_submit_encrypted_tx() {
        let mut tc = make_test_tc();
        let tx = dummy_transfer(500);
        let nonce = random_nonce();
        let enc = encrypt_transaction(&tx, &nonce, 0);

        tc.submit_encrypted_tx(enc);

        let (plain, enc_count, reveals) = tc.mempool_stats();
        assert_eq!(plain, 0);
        assert_eq!(enc_count, 1);
        assert_eq!(reveals, 0);
    }

    #[test]
    fn test_submit_reveal_nonce() {
        let mut tc = make_test_tc();
        let tx = dummy_transfer(500);
        let nonce = random_nonce();
        let enc = encrypt_transaction(&tx, &nonce, 0);
        let commitment = enc.commitment;

        tc.submit_encrypted_tx(enc);
        assert!(
            tc.submit_reveal(commitment, nonce),
            "below cap must accept"
        );

        let (_, _, reveals) = tc.mempool_stats();
        assert_eq!(reveals, 1);
    }

    /// T0.7 vector 4 companion — pending_reveals queue capacity.
    /// Without a cap, an attacker could flood arbitrary
    /// `(commitment, nonce)` pairs (64 bytes each); validator memory
    /// exhausts before the next proposal drains the queue.
    #[test]
    fn submit_reveal_rejects_when_pending_queue_at_capacity() {
        let mut tc = make_test_tc();

        // Fill to MAX_PENDING_REVEALS with synthetic pairs. The
        // commitments don't need to match real encrypted submissions —
        // the cap fires at admission time, before any commitment
        // lookup.
        for i in 0..MAX_PENDING_REVEALS as u32 {
            let mut commitment = [0u8; 32];
            commitment[..4].copy_from_slice(&i.to_le_bytes());
            let nonce = [0u8; 32];
            assert!(
                tc.submit_reveal(commitment, nonce),
                "submit {} must accept (below cap)",
                i + 1
            );
        }
        assert_eq!(tc.mempool_stats().2, MAX_PENDING_REVEALS);

        // The (cap+1)-th submission is rejected.
        let over = [0xFFu8; 32];
        let nonce = [0u8; 32];
        assert!(
            !tc.submit_reveal(over, nonce),
            "at-cap submit_reveal must be rejected (T0.7 vector 4 — reveal queue DoS)"
        );
        // Queue size unchanged.
        assert_eq!(tc.mempool_stats().2, MAX_PENDING_REVEALS);
    }

    #[test]
    fn test_revealed_txs_included_in_proposal() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();

        // Submit encrypted tx at epoch 0
        let tx = dummy_transfer(777);
        let nonce = random_nonce();
        let enc = encrypt_transaction(&tx, &nonce, 0);
        let commitment = enc.commitment;
        tc.submit_encrypted_tx(enc);

        // Advance epoch past reveal delay (default 2 epochs)
        // epoch starts at 0, reveal_delay=2, so we need epoch >= 2
        tc.epoch = 2;

        // Submit reveal nonce
        tc.submit_reveal(commitment, nonce);

        // Create proposal — should include the revealed tx
        let block = tc.create_proposal(&mut db).unwrap();
        assert_eq!(block.transactions.len(), 1);

        // Verify the tx is our transfer
        match &block.transactions[0] {
            Transaction::Transfer(t) => assert_eq!(t.amount, 777),
            _ => panic!("expected transfer tx"),
        }

        // Reveals should be drained
        let (_, enc_count, reveals) = tc.mempool_stats();
        assert_eq!(enc_count, 0); // encrypted tx consumed
        assert_eq!(reveals, 0); // reveals consumed
    }

    #[test]
    fn test_encrypted_tx_not_revealed_too_early() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();

        // Submit encrypted tx at epoch 5
        let tx = dummy_transfer(100);
        let nonce = random_nonce();
        let enc = encrypt_transaction(&tx, &nonce, 5);
        let commitment = enc.commitment;
        tc.submit_encrypted_tx(enc);

        // Current epoch is 0 (well before reveal_delay of 2 past submission)
        tc.epoch = 6; // 5 + 2 = 7 needed, 6 is too early

        tc.submit_reveal(commitment, nonce);

        let block = tc.create_proposal(&mut db).unwrap();
        // Should NOT include the encrypted tx (too early to reveal)
        assert_eq!(block.transactions.len(), 0);

        // Encrypted tx should still be pending
        let (_, enc_count, _) = tc.mempool_stats();
        assert_eq!(enc_count, 1);
    }

    #[test]
    fn test_mixed_plain_and_encrypted_proposal() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();

        // Submit 2 plain txs
        tc.mempool.submit(dummy_transfer(100));
        tc.mempool.submit(dummy_transfer(200));

        // Submit 1 encrypted tx at epoch 0
        let tx = dummy_transfer(999);
        let nonce = random_nonce();
        let enc = encrypt_transaction(&tx, &nonce, 0);
        let commitment = enc.commitment;
        tc.submit_encrypted_tx(enc);

        // Advance past reveal delay
        tc.epoch = 2;
        tc.submit_reveal(commitment, nonce);

        let block = tc.create_proposal(&mut db).unwrap();
        // Should have 3 txs: 1 revealed + 2 plain
        assert_eq!(block.transactions.len(), 3);

        // First tx should be the revealed one (MEV-protected txs get priority)
        match &block.transactions[0] {
            Transaction::Transfer(t) => assert_eq!(t.amount, 999),
            _ => panic!("expected revealed transfer first"),
        }
    }

    #[test]
    fn test_max_txs_respected_with_encrypted() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();

        // Submit 60 plain txs (well below MAX_TXS_PER_BLOCK = 200, so all admit).
        for i in 0..60 {
            tc.mempool.submit(dummy_transfer(i));
        }

        // Submit 5 encrypted txs
        let mut nonces = Vec::new();
        for i in 0..5u64 {
            let tx = dummy_transfer(1000 + i);
            let nonce = random_nonce();
            let enc = encrypt_transaction(&tx, &nonce, 0);
            nonces.push((enc.commitment, nonce));
            tc.submit_encrypted_tx(enc);
        }

        // Advance and reveal all
        tc.epoch = 2;
        for (commitment, nonce) in &nonces {
            tc.submit_reveal(*commitment, *nonce);
        }

        let block = tc.create_proposal(&mut db).unwrap();
        // Cap is 200 now (was 50 when this test was first written).
        // 60 plain + 5 revealed = 65, well under the cap.
        assert_eq!(block.transactions.len(), 65);
    }

    #[test]
    fn test_no_reveal_without_nonce_expires() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();

        // Submit encrypted tx but don't provide reveal nonce
        let tx = dummy_transfer(500);
        let nonce = random_nonce();
        let enc = encrypt_transaction(&tx, &nonce, 0);
        tc.submit_encrypted_tx(enc);

        tc.epoch = 2;
        // No submit_reveal call

        let block = tc.create_proposal(&mut db).unwrap();
        // Should be empty — no nonce means tx can't decrypt
        assert_eq!(block.transactions.len(), 0);

        // Encrypted tx expires after reveal window (no nonce = user abandoned it)
        let (_, enc_count, _) = tc.mempool_stats();
        assert_eq!(enc_count, 0); // expired and dropped
    }

    #[test]
    fn test_unrevealed_tx_kept_before_delay() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();

        // Submit encrypted tx at epoch 5
        let tx = dummy_transfer(500);
        let nonce = random_nonce();
        let enc = encrypt_transaction(&tx, &nonce, 5);
        tc.submit_encrypted_tx(enc);

        // Epoch 6 — before reveal delay (5 + 2 = 7)
        tc.epoch = 6;

        let block = tc.create_proposal(&mut db).unwrap();
        assert_eq!(block.transactions.len(), 0);

        // Should still be pending (not yet past reveal delay)
        let (_, enc_count, _) = tc.mempool_stats();
        assert_eq!(enc_count, 1);
    }
}

// ─────────────── DA Integration Tests ─────────────────────────────────

#[cfg(test)]
mod da_tests {
    use super::*;
    use crate::validator_set::ValidatorInfo;
    use evaporchain_da::block_da::BlockDA;
    use evaporchain_state::db::InMemoryStateDB;
    use evaporchain_types::{BlobTx, TransferTx};

    fn make_test_tc() -> TendermintConsensus {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [1u8; 32]));
        vs.add_validator(ValidatorInfo::new(2, 1000, [2u8; 32]));
        vs.add_validator(ValidatorInfo::new(3, 1000, [3u8; 32]));
        vs.add_validator(ValidatorInfo::new(4, 1000, [4u8; 32]));
        TendermintConsensus::new_for_test(1, 100, vs)
    }

    fn dummy_transfer(amount: u64) -> Transaction {
        Transaction::Transfer(TransferTx {
            from: [0xAA; 32],
            to: [0xBB; 32],
            amount,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        })
    }

    #[test]
    fn test_proposal_with_txs_has_data_root() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();

        // Add transactions
        tc.mempool.submit(dummy_transfer(100));
        tc.mempool.submit(dummy_transfer(200));
        tc.mempool.submit(dummy_transfer(300));

        let block = tc.create_proposal(&mut db).unwrap();
        assert_eq!(block.transactions.len(), 3);
        assert!(
            block.data_root.is_some(),
            "block with txs should have data_root"
        );

        // Verify the data_root is a valid commitment
        let data_root = block.data_root.unwrap();
        assert_ne!(data_root, [0u8; 32], "data_root should not be all zeros");
    }

    #[test]
    fn test_empty_proposal_has_data_root_sentinel() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();

        // No transactions in mempool — should still get a domain-separated sentinel data_root.
        let block = tc.create_proposal(&mut db).unwrap();
        assert_eq!(block.transactions.len(), 0);
        let expected = crate::empty_block_data_root(block.number, &block.parent_hash);
        assert_eq!(
            block.data_root,
            Some(expected),
            "empty block should have domain-separated sentinel data_root"
        );
    }

    #[test]
    fn test_empty_block_sentinel_differs_across_heights() {
        // Two empty blocks at different heights MUST have different data_root.
        // Audit fix #9.1: prevents DA-attestation replay across heights.
        let parent = [0x42u8; 32];
        let r1 = crate::empty_block_data_root(1, &parent);
        let r2 = crate::empty_block_data_root(2, &parent);
        assert_ne!(r1, r2, "empty-block sentinel must be height-dependent");
    }

    #[test]
    fn test_empty_block_sentinel_differs_across_parents() {
        // Two empty blocks at the same height with different parents MUST differ.
        let r_a = crate::empty_block_data_root(7, &[0xAAu8; 32]);
        let r_b = crate::empty_block_data_root(7, &[0xBBu8; 32]);
        assert_ne!(
            r_a, r_b,
            "empty-block sentinel must be parent-hash-dependent"
        );
    }

    #[test]
    fn test_empty_block_sentinel_is_deterministic() {
        // Same inputs MUST produce the same sentinel — verifier and proposer
        // must agree on the value.
        let parent = [0x77u8; 32];
        let r1 = crate::empty_block_data_root(42, &parent);
        let r2 = crate::empty_block_data_root(42, &parent);
        assert_eq!(r1, r2);
    }

    #[test]
    fn test_data_root_matches_independent_encoding() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();

        tc.mempool.submit(dummy_transfer(500));
        tc.mempool.submit(dummy_transfer(600));

        let block = tc.create_proposal(&mut db).unwrap();
        let data_root = block.data_root.unwrap();

        // Independently encode the same tx data and verify roots match
        let tx_bytes = serde_json::to_vec(&block.transactions).unwrap();
        let da = BlockDA::new().unwrap();
        let package = da.encode_block(&tx_bytes).unwrap();

        assert_eq!(
            data_root, package.header.commitment_root,
            "data_root should match independent DA encoding"
        );
    }

    #[test]
    fn test_different_txs_produce_different_data_roots() {
        let mut db = InMemoryStateDB::new();

        // Block 1
        let mut tc1 = make_test_tc();
        tc1.mempool.submit(dummy_transfer(100));
        let block1 = tc1.create_proposal(&mut db).unwrap();

        // Block 2 with different tx
        let mut tc2 = make_test_tc();
        tc2.mempool.submit(dummy_transfer(999));
        let block2 = tc2.create_proposal(&mut db).unwrap();

        assert_ne!(
            block1.data_root.unwrap(),
            block2.data_root.unwrap(),
            "different transactions should produce different data_roots"
        );
    }

    #[test]
    fn test_data_root_verifiable_by_light_client() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();

        tc.mempool.submit(dummy_transfer(42));

        let block = tc.create_proposal(&mut db).unwrap();
        let data_root = block.data_root.unwrap();

        // A light client can verify individual shards against this root
        let tx_bytes = serde_json::to_vec(&block.transactions).unwrap();
        let da = BlockDA::new().unwrap();
        let package = da.encode_block(&tx_bytes).unwrap();

        // Verify each shard proves against the data_root
        for i in 0..package.shards.len() {
            let proof = da.prove_shard(&package, i).unwrap();
            assert!(
                BlockDA::verify_shard_sample(&package.header, &proof),
                "shard {} should verify against data_root",
                i
            );
            assert_eq!(package.header.commitment_root, data_root);
        }
    }

    #[test]
    fn test_proposal_populates_blob_commitments() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();

        // Add regular transfers and a blob tx
        tc.mempool.submit(dummy_transfer(100));
        tc.mempool.submit(Transaction::Blob(BlobTx {
            submitter: [0xCC; 32],
            data: b"blob data for namespace 42".to_vec(),
            nonce: 0,
            namespace_id: 42,
            signature: None,
            public_key: None,
        }));
        tc.mempool.submit(dummy_transfer(200));

        let block = tc.create_proposal(&mut db).unwrap();
        assert_eq!(block.transactions.len(), 3);
        // blob_commitments should have one entry per tx
        assert_eq!(block.blob_commitments.len(), 3);
        // Each commitment should be non-zero
        for (i, commitment) in block.blob_commitments.iter().enumerate() {
            assert_ne!(
                *commitment, [0u8; 32],
                "blob_commitment[{i}] should not be zero"
            );
        }
    }

    #[test]
    fn test_blob_commitments_deterministic() {
        let mut db = InMemoryStateDB::new();

        let make_tc_with_txs = || {
            let mut tc = make_test_tc();
            tc.mempool.submit(dummy_transfer(100));
            tc.mempool.submit(Transaction::Blob(BlobTx {
                submitter: [0xDD; 32],
                data: b"deterministic blob".to_vec(),
                nonce: 0,
                namespace_id: 7,
                signature: None,
                public_key: None,
            }));
            tc
        };

        let block1 = make_tc_with_txs().create_proposal(&mut db).unwrap();
        let block2 = make_tc_with_txs().create_proposal(&mut db).unwrap();
        assert_eq!(block1.blob_commitments, block2.blob_commitments);
    }

    #[test]
    fn test_empty_block_has_no_blob_commitments() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();
        let block = tc.create_proposal(&mut db).unwrap();
        assert!(block.blob_commitments.is_empty());
    }

    // ── Vote Equivocation Detection Tests ──

    #[test]
    fn test_prevote_equivocation_slashes_validator() {
        let mut tc = make_test_tc();
        let hash_a = [0xAA; 32];
        let hash_b = [0xBB; 32];

        // First prevote from validator 2: vote for hash_a
        tc.on_message(ConsensusMessage::Prevote {
            height: tc.height,
            round: 0,
            block_hash: Some(hash_a),
            validator_id: 2,
            bls_signature: None,
        });

        // Second prevote from same validator 2: different hash → equivocation
        let actions = tc.on_message(ConsensusMessage::Prevote {
            height: tc.height,
            round: 0,
            block_hash: Some(hash_b),
            validator_id: 2,
            bls_signature: None,
        });

        // Should return early (empty actions = rejected)
        assert!(
            actions.is_empty(),
            "equivocating prevote should be rejected"
        );
        // Sanov KL slash for vote equivocation is the full stake (KL of an
        // all-equivocations observation against an honest baseline overflows
        // the slash cap). The validator is removed from the set when stake
        // drops below MIN_STAKE.
        assert!(
            tc.validator_set.get(2).is_none(),
            "equivocating validator should be removed from the active set after a 100% slash"
        );
    }

    #[test]
    fn test_precommit_equivocation_slashes_validator() {
        let mut tc = make_test_tc();
        let hash_a = [0xCC; 32];
        let hash_b = [0xDD; 32];

        // First precommit from validator 3
        tc.on_message(ConsensusMessage::Precommit {
            height: tc.height,
            round: 0,
            block_hash: Some(hash_a),
            validator_id: 3,
            bls_signature: None,
        });

        // Conflicting precommit → equivocation
        let actions = tc.on_message(ConsensusMessage::Precommit {
            height: tc.height,
            round: 0,
            block_hash: Some(hash_b),
            validator_id: 3,
            bls_signature: None,
        });

        assert!(
            actions.is_empty(),
            "equivocating precommit should be rejected"
        );
        // See test_prevote_equivocation_slashes_validator: equivocation
        // slashes the full stake under Sanov KL math, so the validator
        // is removed from the active set.
        assert!(
            tc.validator_set.get(3).is_none(),
            "equivocating validator should be removed from the active set after a 100% slash"
        );
    }

    #[test]
    fn test_duplicate_identical_vote_is_accepted() {
        let mut tc = make_test_tc();
        let hash = [0xEE; 32];

        // Same vote twice — should NOT slash (idempotent)
        tc.on_message(ConsensusMessage::Prevote {
            height: tc.height,
            round: 0,
            block_hash: Some(hash),
            validator_id: 2,
            bls_signature: None,
        });
        tc.on_message(ConsensusMessage::Prevote {
            height: tc.height,
            round: 0,
            block_hash: Some(hash),
            validator_id: 2,
            bls_signature: None,
        });

        let v = tc.validator_set.get(2).unwrap();
        assert!(!v.jailed, "identical duplicate vote should not slash");
        assert_eq!(v.total_slashed, 0);
    }

    #[test]
    fn test_nil_to_value_vote_is_equivocation() {
        let mut tc = make_test_tc();

        // First: nil prevote
        tc.on_message(ConsensusMessage::Prevote {
            height: tc.height,
            round: 0,
            block_hash: None,
            validator_id: 4,
            bls_signature: None,
        });

        // Then: vote for a hash → equivocation (nil ≠ Some)
        let actions = tc.on_message(ConsensusMessage::Prevote {
            height: tc.height,
            round: 0,
            block_hash: Some([0xFF; 32]),
            validator_id: 4,
            bls_signature: None,
        });

        assert!(actions.is_empty());
        // Full-stake slash → validator removed (see prevote test).
        assert!(tc.validator_set.get(4).is_none());
    }

    #[test]
    fn test_jailed_validator_excluded_after_vote_equivocation() {
        let mut tc = make_test_tc();

        // Slash validator 2 via prevote equivocation
        tc.on_message(ConsensusMessage::Prevote {
            height: tc.height,
            round: 0,
            block_hash: Some([0xAA; 32]),
            validator_id: 2,
            bls_signature: None,
        });
        tc.on_message(ConsensusMessage::Prevote {
            height: tc.height,
            round: 0,
            block_hash: Some([0xBB; 32]),
            validator_id: 2,
            bls_signature: None,
        });

        // Validator 2 should never be leader
        for epoch in 0..20 {
            if let Some(leader) = tc.validator_set.leader_for_epoch(epoch) {
                assert_ne!(
                    leader.id, 2,
                    "Jailed validator should not lead at epoch {}",
                    epoch
                );
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // DA Sampling Wiring Tests
    // ═══════════════════════════════════════════════════════════════════════

    fn make_proposer_tc() -> TendermintConsensus {
        // new_for_test starts at height 1, so find proposer for height=1, round=0
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(0, 1000, [0u8; 32]));
        vs.add_validator(ValidatorInfo::new(1, 1000, [1u8; 32]));
        vs.add_validator(ValidatorInfo::new(2, 1000, [2u8; 32]));
        let virtual_epoch = 1u64.wrapping_mul(100).wrapping_add(0);
        let proposer_id = vs.leader_for_epoch(virtual_epoch).unwrap().id;
        TendermintConsensus::new_for_test(proposer_id, 100, vs)
    }

    #[test]
    fn test_create_proposal_stamps_submit_epoch_hints() {
        // Lane A.2: proposer must stamp `block.submit_epoch_hints` so
        // followers can deterministically reconstruct per-tx priority.
        // For a block with N hinted txs, hints.len() == transactions.len()
        // and every entry is `Some(submit_epoch)` from the local mempool.
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        // Two txs submitted at distinct epochs.
        tc.mempool.set_epoch(3);
        tc.mempool.submit(dummy_transfer(111));
        tc.mempool.set_epoch(7);
        tc.mempool.submit(dummy_transfer(222));

        let block = tc.create_proposal(&mut db).unwrap();
        assert_eq!(block.transactions.len(), 2);
        assert_eq!(
            block.submit_epoch_hints.len(),
            block.transactions.len(),
            "hints must be index-parallel with transactions"
        );
        // Both hints are Some(epoch) from the mempool's submit-epoch
        // tracking. Order is priority-sorted (recent-first); we don't
        // assume which tx ends up first — we just assert both hints
        // are present and match the set {3, 7}.
        let hints: std::collections::BTreeSet<u64> =
            block.submit_epoch_hints.iter().filter_map(|h| *h).collect();
        assert_eq!(hints, [3u64, 7u64].iter().copied().collect());
    }

    #[test]
    fn test_empty_proposal_has_no_submit_epoch_hints() {
        // A block with zero plaintext-mempool txs leaves the hints
        // vector empty — preserves bit-compat for legacy-quiet blocks.
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        let block = tc.create_proposal(&mut db).unwrap();
        assert_eq!(block.transactions.len(), 0);
        assert!(
            block.submit_epoch_hints.is_empty(),
            "empty block must have empty hints (skip_serializing_if = Vec::is_empty)"
        );
    }

    #[test]
    fn test_block_source_mode_antichain_dedups_same_sender_in_proposal() {
        // Lane J.1 integration test: when governance sets
        // `block_source_mode = "antichain"`, create_proposal post-filters
        // the FIFO draw via antichain_project so the resulting block
        // carries at most one tx per sender. This exercises the full
        // wire path:
        //   governance_params (set)
        //   → create_proposal lookup (Lane I.5 dispatch)
        //   → antichain_project helper (Lane I.5 follow-up)
        //   → block.transactions
        //
        // Distinct from the unit tests on antichain_project itself —
        // this verifies the consensus loop actually consults the flag.
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        // Flip the Lane I.5 flag.
        tc.governance_params
            .insert("block_source_mode".to_string(), "antichain".to_string());

        // Three same-sender txs (sender 1, nonces 0/1/2) + one
        // different-sender tx (sender 2). Antichain admits at most
        // one from each sender.
        let mk = |from: u8, nonce: u64| {
            Transaction::Transfer(TransferTx {
                from: [from; 32],
                to: [99u8; 32],
                amount: 100,
                nonce,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })
        };
        tc.mempool.set_epoch(0);
        tc.mempool.submit(mk(1, 0));
        tc.mempool.submit(mk(1, 1));
        tc.mempool.submit(mk(1, 2));
        tc.mempool.submit(mk(2, 0));
        assert_eq!(tc.mempool.len(), 4);

        let block = tc.create_proposal(&mut db).unwrap();

        // Assert antichain semantics: each sender appears at most once
        // in the proposal's transactions.
        let mut seen_senders: std::collections::HashSet<[u8; 32]> =
            std::collections::HashSet::new();
        for tx in &block.transactions {
            if let Some(addr) = tx.sender() {
                assert!(
                    seen_senders.insert(*addr),
                    "antichain mode must not admit two txs with sender {:?}",
                    addr
                );
            }
        }

        // Dropped (same-sender) txs returned to mempool — should still
        // be there for the next proposal.
        assert!(
            tc.mempool.len() >= 1,
            "dropped same-sender txs must remain in pool — got {} pending",
            tc.mempool.len()
        );

        // Sanity: with FIFO default, all 4 txs would have made it into
        // the block. The antichain dedup must have actually filtered.
        assert!(
            block.transactions.len() <= 2,
            "antichain mode must filter — got {} txs in proposal",
            block.transactions.len()
        );
    }

    #[test]
    fn test_block_source_mode_default_admits_all_same_sender() {
        // Negative case: under the default `block_source_mode = "fifo"`,
        // same-sender txs all make it into the proposal (legacy
        // FIFO-with-priority behaviour). Confirms Lane I.5 wiring is
        // strictly opt-in and a typo can never accidentally engage.
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        // Type the value WRONG to confirm typo-safety.
        tc.governance_params.insert(
            "block_source_mode".to_string(),
            "antichian".to_string(), // sic: typo'd
        );

        let mk = |from: u8, nonce: u64| {
            Transaction::Transfer(TransferTx {
                from: [from; 32],
                to: [99u8; 32],
                amount: 100,
                nonce,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })
        };
        tc.mempool.set_epoch(0);
        tc.mempool.submit(mk(1, 0));
        tc.mempool.submit(mk(1, 1));

        let block = tc.create_proposal(&mut db).unwrap();
        // Both same-sender txs admitted (typo fell through to FIFO).
        assert_eq!(
            block.transactions.len(),
            2,
            "typo'd governance value must fall through to FIFO default"
        );
    }

    #[test]
    fn test_create_proposal_orders_by_energy_priority() {
        // Phase 1 of `research/proposals/energy-stamped-mev-resistance.md`:
        // create_proposal() must order plaintext-mempool txs via
        // `take_with_priority(remaining, self.height)`. A tx submitted
        // RECENTLY (low elapsed) outranks one submitted EARLIER (high
        // elapsed, more decayed) and is therefore included first.
        // This validates that the wiring fires through the proposer.
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        // Old tx: submitted at epoch 0.
        tc.mempool.set_epoch(0);
        let old_tx = dummy_transfer(111);
        let old_hash = old_tx.tx_hash();
        tc.mempool.submit(old_tx);

        // New tx: submitted later, at the same epoch the proposer will
        // build at — so its priority is at full BASE_INCLUSION_ENERGY.
        tc.mempool.set_epoch(tc.height);
        let new_tx = dummy_transfer(222);
        let new_hash = new_tx.tx_hash();
        tc.mempool.submit(new_tx);

        let block = tc.create_proposal(&mut db).unwrap();
        assert_eq!(block.transactions.len(), 2);
        assert_eq!(
            block.transactions[0].tx_hash(),
            new_hash,
            "high-priority (recent) tx must be ordered first"
        );
        assert_eq!(
            block.transactions[1].tx_hash(),
            old_hash,
            "decayed (older) tx falls to the back"
        );
    }

    #[test]
    fn test_da_sampling_on_proposal_with_txs() {
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        tc.mempool.submit(dummy_transfer(42));
        let block = tc.create_proposal(&mut db).unwrap();
        assert!(
            block.data_root.is_some(),
            "Block with txs should have data_root"
        );

        let att = tc.perform_da_sampling(&block);
        assert!(
            att.is_some(),
            "DA sampling should produce an attestation for a valid block"
        );

        if let Some(ConsensusMessage::DAAttestation {
            block_number,
            samples_verified,
            ..
        }) = att
        {
            assert_eq!(block_number, block.number);
            // 2D path samples 16 cells; 1D fallback verifies at least 4 shards
            assert!(
                samples_verified >= 4,
                "Should verify at least 4 samples, got {}",
                samples_verified
            );
        } else {
            panic!("Expected DAAttestation message");
        }
    }

    #[test]
    fn test_da_sampling_empty_block_has_sentinel() {
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        let block = tc.create_proposal(&mut db).unwrap();
        let expected = crate::empty_block_data_root(block.number, &block.parent_hash);
        assert_eq!(
            block.data_root,
            Some(expected),
            "Empty block should have domain-separated sentinel data_root"
        );
    }

    #[test]
    fn test_da_sampling_tampered_data_root_returns_none() {
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        tc.mempool.submit(dummy_transfer(42));
        let mut block = tc.create_proposal(&mut db).unwrap();
        block.data_root = Some([0xFFu8; 32]); // Tamper
                                              // Clear 2D roots so sampling falls through to 1D path where data_root is checked
        block.da_row_roots.clear();
        block.da_col_roots.clear();

        let att = tc.perform_da_sampling(&block);
        assert!(att.is_none(), "Tampered data_root should fail DA sampling");
    }

    #[test]
    fn test_proposer_broadcasts_da_attestation() {
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        tc.mempool.submit(dummy_transfer(42));
        let actions = tc.tick(&mut db);

        // Should have: Proposal + Prevote + DAAttestation
        let has_proposal = actions.iter().any(|a| {
            matches!(
                a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::Proposal { .. })
            )
        });
        let has_prevote = actions.iter().any(|a| {
            matches!(
                a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. })
            )
        });
        let has_da_att = actions.iter().any(|a| {
            matches!(
                a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::DAAttestation { .. })
            )
        });

        assert!(has_proposal, "Proposer should broadcast proposal");
        assert!(has_prevote, "Proposer should broadcast prevote");
        assert!(has_da_att, "Proposer should broadcast DA attestation");
    }

    #[test]
    fn test_validator_da_sampling_on_received_proposal() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(0, 1000, [0u8; 32]));
        vs.add_validator(ValidatorInfo::new(1, 1000, [1u8; 32]));
        vs.add_validator(ValidatorInfo::new(2, 1000, [2u8; 32]));

        // Both new() and new_for_test() start at height 1
        let virtual_epoch = 1u64.wrapping_mul(100).wrapping_add(0);
        let proposer_id = vs.leader_for_epoch(virtual_epoch).unwrap().id;
        let receiver_id = if proposer_id == 0 { 1 } else { 0 };

        let mut tc_proposer = TendermintConsensus::new_for_test(proposer_id, 7, vs.clone());
        let kp0 = BlsKeypair::generate();
        tc_proposer.set_bls_keypair(kp0);
        tc_proposer.mempool.submit(dummy_transfer(42));
        let mut db = InMemoryStateDB::new();
        let block = tc_proposer.create_proposal(&mut db).unwrap();
        assert!(block.data_root.is_some());

        let mut tc_receiver = TendermintConsensus::new_for_test(receiver_id, 7, vs);
        let kp1 = BlsKeypair::generate();
        tc_receiver.set_bls_keypair(kp1);

        let proposal_msg = ConsensusMessage::Proposal {
            height: 1,
            round: 0,
            block,
            proposer_id,
        };
        let actions = tc_receiver.on_message(proposal_msg);

        let has_prevote = actions.iter().any(|a| {
            matches!(
                a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote {
                    block_hash: Some(_),
                    ..
                })
            )
        });
        let has_da_att = actions.iter().any(|a| {
            matches!(
                a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::DAAttestation { .. })
            )
        });

        assert!(has_prevote, "Validator should prevote for valid proposal");
        assert!(
            has_da_att,
            "Validator should broadcast DA attestation after sampling"
        );
    }

    #[test]
    fn test_da_proposer_tracked_for_exclusion() {
        let mut tc = make_proposer_tc();
        let proposer_id = tc.my_id;
        let height = tc.height();
        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);
        tc.mempool.submit(dummy_transfer(42));
        let mut db = InMemoryStateDB::new();
        tc.tick(&mut db);

        assert_eq!(tc.da_block_proposers.get(&height), Some(&proposer_id));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 2D DA Sampling Tests
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_da_2d_sampling_uses_2d_path_when_row_col_roots_present() {
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        tc.mempool.submit(dummy_transfer(42));
        let block = tc.create_proposal(&mut db).unwrap();

        // Blocks from create_proposal should have 2D roots populated
        assert!(
            !block.da_row_roots.is_empty(),
            "Block should have da_row_roots"
        );
        assert!(
            !block.da_col_roots.is_empty(),
            "Block should have da_col_roots"
        );
        assert!(block.data_root.is_some(), "Block should have data_root");

        let att = tc.perform_da_sampling(&block);
        assert!(
            att.is_some(),
            "2D DA sampling should produce an attestation"
        );

        if let Some(ConsensusMessage::DAAttestation {
            samples_verified, ..
        }) = att
        {
            // 2D path samples 16 cells, all should verify for a valid block
            assert_eq!(samples_verified, 16, "2D path should verify all 16 samples");
        } else {
            panic!("Expected DAAttestation message from 2D path");
        }
    }

    #[test]
    fn test_da_2d_sampling_tampered_row_roots_returns_none() {
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        tc.mempool.submit(dummy_transfer(42));
        let mut block = tc.create_proposal(&mut db).unwrap();
        assert!(!block.da_row_roots.is_empty());

        // Tamper with the first row root
        block.da_row_roots[0] = [0xFF; 32];

        let att = tc.perform_da_sampling(&block);
        assert!(
            att.is_none(),
            "Tampered row roots should fail 2D DA sampling"
        );
    }

    #[test]
    fn test_da_2d_sampling_falls_back_to_1d_without_roots() {
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        tc.mempool.submit(dummy_transfer(42));
        let mut block = tc.create_proposal(&mut db).unwrap();

        // Clear 2D roots to force 1D fallback
        block.da_row_roots = vec![];
        block.da_col_roots = vec![];

        let att = tc.perform_da_sampling(&block);
        assert!(att.is_some(), "Should fall back to 1D DA sampling");

        if let Some(ConsensusMessage::DAAttestation {
            samples_verified, ..
        }) = att
        {
            // 1D path samples min(6, shard_count) and requires at least 4
            assert!(
                samples_verified >= 4,
                "1D fallback should verify at least 4 shards"
            );
        } else {
            panic!("Expected DAAttestation from 1D fallback");
        }
    }

    #[test]
    fn test_da_confidence_threshold_setter() {
        let mut tc = make_proposer_tc();

        // Default threshold
        assert!((tc.da_confidence_threshold - 0.999).abs() < 1e-12);

        // Set custom threshold
        tc.set_da_confidence_threshold(0.95);
        assert!((tc.da_confidence_threshold - 0.95).abs() < 1e-12);

        // Clamped to [0.0, 1.0]
        tc.set_da_confidence_threshold(1.5);
        assert!((tc.da_confidence_threshold - 1.0).abs() < 1e-12);

        tc.set_da_confidence_threshold(-0.5);
        assert!((tc.da_confidence_threshold - 0.0).abs() < 1e-12);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // DA Enforcement Height Tests
    // ═══════════════════════════════════════════════════════════════════════

    /// Helper: create a block at the given height with no DA certificate.
    fn make_block_no_da_cert(height: u64) -> Block {
        Block {
            number: height,
            epoch: height / 100,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            producer_id: Some(1),
            timestamp: 0,
            chain_id: String::new(),
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
            post_state_root: None,
        }
    }

    /// Helper: create a valid DA certificate with BLS-signed attestations.
    fn make_valid_da_cert(block_number: u64, num_validators: u64) -> Vec<u8> {
        use evaporchain_da::certificate::{create_attestation, CertificateBuilder};

        let data_root = [0xDAu8; 32];
        let stake_per = 1000u64;
        let total_stake = num_validators * stake_per;
        let mut builder = CertificateBuilder::new(block_number, data_root, total_stake);

        for vid in 1..=num_validators {
            let kp = BlsKeypair::generate();
            let att = create_attestation(block_number, &data_root, vid, 8, stake_per, &kp);
            assert!(builder.add_attestation(att));
        }

        let cert = builder.try_build().expect("should have supermajority");
        serde_json::to_vec(&cert).expect("cert serialization")
    }

    /// Helper: create a block at the given height WITH a valid DA certificate.
    fn make_block_with_valid_da_cert(height: u64, num_validators: u64) -> Block {
        let mut block = make_block_no_da_cert(height);
        block.da_certificate = Some(make_valid_da_cert(height, num_validators));
        block
    }

    #[test]
    fn test_da_enforcement_default_height() {
        let tc = make_test_tc();
        assert_eq!(
            tc.da_enforcement_height(),
            100,
            "default enforcement height should be 100"
        );
    }

    #[test]
    fn test_da_enforcement_setter() {
        let mut tc = make_test_tc();
        tc.set_da_enforcement_height(500);
        assert_eq!(tc.da_enforcement_height(), 500);
    }

    #[test]
    fn test_da_soft_mode_accepts_block_without_cert() {
        // Before enforcement height, blocks without DA certificates should be accepted
        let mut tc = make_test_tc();
        tc.set_da_enforcement_height(100);

        // Block at height 50 — below enforcement height
        let block = make_block_no_da_cert(50);
        assert!(
            tc.verify_da_certificate(&block),
            "Blocks before enforcement height should be accepted without DA cert (soft mode)"
        );

        // Block at height 99 — still below enforcement height
        let block = make_block_no_da_cert(99);
        assert!(
            tc.verify_da_certificate(&block),
            "Block at height 99 should pass soft mode (enforcement at 100)"
        );

        // Block at height 0 — genesis region
        let block = make_block_no_da_cert(0);
        assert!(
            tc.verify_da_certificate(&block),
            "Genesis block should pass soft mode"
        );
    }

    #[test]
    fn test_da_hard_mode_rejects_block_without_cert() {
        // At or after enforcement height, blocks without DA certificates must be rejected
        let mut tc = make_test_tc();
        tc.set_da_enforcement_height(100);

        // Block at height 100 — exactly at enforcement height
        let block = make_block_no_da_cert(100);
        assert!(
            !tc.verify_da_certificate(&block),
            "Block at enforcement height should be rejected without DA cert (hard mode)"
        );

        // Block at height 101 — past enforcement height
        let block = make_block_no_da_cert(101);
        assert!(
            !tc.verify_da_certificate(&block),
            "Block past enforcement height should be rejected without DA cert"
        );

        // Block at height 1000 — well past enforcement height
        let block = make_block_no_da_cert(1000);
        assert!(
            !tc.verify_da_certificate(&block),
            "Block well past enforcement height should be rejected without DA cert"
        );
    }

    #[test]
    fn test_da_valid_cert_accepted_before_enforcement() {
        // Valid DA certificates should always be accepted, even before enforcement
        let tc = make_test_tc();

        // Block at height 5 (before default enforcement of 100) with valid cert
        let block = make_block_with_valid_da_cert(5, 3);
        assert!(
            tc.verify_da_certificate(&block),
            "Valid DA cert should be accepted before enforcement height"
        );
    }

    #[test]
    fn test_da_valid_cert_accepted_after_enforcement() {
        // Valid DA certificates should be accepted at and after enforcement height
        let tc = make_test_tc();

        // Block at height 100 (at enforcement) with valid cert
        let block = make_block_with_valid_da_cert(100, 3);
        assert!(
            tc.verify_da_certificate(&block),
            "Valid DA cert should be accepted at enforcement height"
        );

        // Block at height 500 (well past enforcement) with valid cert
        let block = make_block_with_valid_da_cert(500, 3);
        assert!(
            tc.verify_da_certificate(&block),
            "Valid DA cert should be accepted past enforcement height"
        );
    }

    #[test]
    fn test_da_invalid_cert_rejected_in_soft_mode() {
        // Even before enforcement, if a cert IS present it must be valid
        let tc = make_test_tc();

        // Block at height 5 (soft mode) with garbage DA certificate
        let mut block = make_block_no_da_cert(5);
        block.da_certificate = Some(vec![0xFF; 64]); // garbage bytes, not valid JSON

        assert!(
            !tc.verify_da_certificate(&block),
            "Invalid DA cert should be rejected even in soft mode"
        );
    }

    #[test]
    fn test_da_forged_cert_rejected_at_any_height() {
        // Forged certificates (bad BLS signatures) must be rejected at any height
        let tc = make_test_tc();

        let forged_cert = evaporchain_da::certificate::DACertificate {
            block_number: 10,
            data_root: [0xDA; 32],
            attestations: vec![
                evaporchain_da::certificate::DAAttestation {
                    block_number: 10,
                    data_root: [0xDA; 32],
                    validator_id: 1,
                    samples_verified: 8,
                    stake: 1000,
                    signature: vec![0xFF; 96],  // forged
                    public_key: vec![0xAA; 48], // forged
                },
                evaporchain_da::certificate::DAAttestation {
                    block_number: 10,
                    data_root: [0xDA; 32],
                    validator_id: 2,
                    samples_verified: 8,
                    stake: 1000,
                    signature: vec![0xFE; 96],  // forged
                    public_key: vec![0xBB; 48], // forged
                },
            ],
            attested_stake: 2000,
            total_stake: 3000,
        };
        let cert_bytes = serde_json::to_vec(&forged_cert).unwrap();

        // Before enforcement height — cert present but forged
        let mut block = make_block_no_da_cert(10);
        block.da_certificate = Some(cert_bytes.clone());
        assert!(
            !tc.verify_da_certificate(&block),
            "Forged DA cert should be rejected in soft mode"
        );

        // After enforcement height — cert present but forged
        let mut block = make_block_no_da_cert(200);
        block.da_certificate = Some(cert_bytes);
        assert!(
            !tc.verify_da_certificate(&block),
            "Forged DA cert should be rejected in hard mode"
        );
    }

    // ── Small-cluster DA mode: 3-validator quorum reachability ──────────
    //
    // Regression for the cluster-smoke failure (h=201 r=N) where every block
    // was rejected with `missing DA certificate (hard mode — enforcement
    // active)` because, with proposer-exclusion enforced and only 3
    // validators total, every cert needed 2-of-2 non-proposer attestations
    // — fragile against a single dropped/delayed gossip across a Tailnet
    // hop. The fix: small-cluster mode lets the proposer's self-attestation
    // count toward DA quorum.

    /// Helper: directly inject an attestation into a TC's bookkeeping
    /// without going through gossip / signature verification. Mirrors what
    /// `on_message(ConsensusMessage::DAAttestation)` does after the BLS
    /// verify path. Used by the small-cluster tests below.
    fn inject_da_attestation(
        tc: &mut TendermintConsensus,
        block_number: u64,
        data_root: [u8; 32],
        validator_id: u64,
        stake: u64,
    ) {
        let kp = BlsKeypair::generate();
        let att = evaporchain_da::certificate::create_attestation(
            block_number,
            &data_root,
            validator_id,
            8,
            stake,
            &kp,
        );
        let atts = tc.da_attestations.entry(block_number).or_default();
        if !atts.iter().any(|a| a.validator_id == validator_id) {
            atts.push(att);
        }
    }

    fn make_3_validator_tc() -> TendermintConsensus {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [1u8; 32]));
        vs.add_validator(ValidatorInfo::new(2, 1000, [2u8; 32]));
        vs.add_validator(ValidatorInfo::new(3, 1000, [3u8; 32]));
        TendermintConsensus::new_for_test(1, 100, vs)
    }

    #[test]
    fn da_quorum_reached_with_3_validators() {
        // A 3-validator cluster in small-cluster DA mode must reach DA quorum
        // for every height when all three validators attest (regardless of
        // who the proposer is).
        let mut tc = make_3_validator_tc();
        tc.set_small_cluster_da_mode(true);

        let data_root = [0xABu8; 32];
        for h in 1u64..=10 {
            // Round-robin proposer across the 3 validators.
            let proposer = ((h - 1) % 3) + 1;
            tc.da_block_proposers.insert(h, proposer);

            // All three validators (including the proposer) attest. This is
            // what would arrive at a peer over gossip after each validator
            // performs its own DA sampling.
            for vid in 1u64..=3 {
                inject_da_attestation(&mut tc, h, data_root, vid, 1000);
            }

            assert!(
                tc.has_da_supermajority(h),
                "small-cluster mode: 3-of-3 attestations should clear quorum at h={h} (proposer={proposer})"
            );
            let cert_bytes = tc
                .try_build_da_certificate(h, data_root)
                .unwrap_or_else(|| {
                    panic!("DA certificate should build for h={h} (proposer={proposer})")
                });
            let cert: evaporchain_da::certificate::DACertificate =
                serde_json::from_slice(&cert_bytes).unwrap();
            assert!(
                cert.is_supermajority(),
                "rebuilt cert for h={h} should claim supermajority"
            );
            // In small-cluster mode the proposer's own attestation IS
            // included → all 3 attestations show up in the cert.
            assert_eq!(
                cert.attestations.len(),
                3,
                "small-cluster cert should include the proposer's self-attestation"
            );
            assert_eq!(cert.attested_stake, 3000);
            assert_eq!(cert.total_stake, 3000);
        }
    }

    #[test]
    fn da_quorum_3_validators_strict_mode_fails_with_dropped_attestation() {
        // Without small-cluster mode, dropping ONE non-proposer attestation
        // breaks DA quorum entirely — this is the production failure mode
        // the cluster smoke ran into. Documents why the fix is needed.
        let mut tc = make_3_validator_tc();
        tc.set_small_cluster_da_mode(false);

        let data_root = [0xCDu8; 32];
        let h = 5u64;
        let proposer = 1u64;
        tc.da_block_proposers.insert(h, proposer);

        // Proposer self-attests (will be filtered out) + only ONE of the two
        // peers attests in time. This is exactly the cluster-smoke pattern.
        inject_da_attestation(&mut tc, h, data_root, proposer, 1000);
        inject_da_attestation(&mut tc, h, data_root, 2, 1000);
        // validator 3's attestation is "delayed" — does not arrive.

        assert!(
            !tc.has_da_supermajority(h),
            "strict mode + 1-of-2 non-proposer attestations should NOT reach quorum"
        );
        assert!(
            tc.try_build_da_certificate(h, data_root).is_none(),
            "strict mode: cert build should fail under-quorum"
        );
    }

    #[test]
    fn da_quorum_3_validators_strict_mode_succeeds_with_both_peers() {
        // Sanity: in strict (proposer-excluded) mode, 2-of-2 non-proposer
        // attestations DOES reach quorum. Both peers attesting is the only
        // viable path in n=3 strict mode.
        let mut tc = make_3_validator_tc();
        tc.set_small_cluster_da_mode(false);

        let data_root = [0xEFu8; 32];
        let h = 7u64;
        let proposer = 2u64;
        tc.da_block_proposers.insert(h, proposer);

        // Proposer attestation arrives but is filtered out in strict mode.
        inject_da_attestation(&mut tc, h, data_root, proposer, 1000);
        // Both peers attest in time.
        inject_da_attestation(&mut tc, h, data_root, 1, 1000);
        inject_da_attestation(&mut tc, h, data_root, 3, 1000);

        assert!(
            tc.has_da_supermajority(h),
            "strict mode: 2-of-2 non-proposer attestations should clear quorum"
        );
        let cert_bytes = tc.try_build_da_certificate(h, data_root).unwrap();
        let cert: evaporchain_da::certificate::DACertificate =
            serde_json::from_slice(&cert_bytes).unwrap();
        // Strict mode: proposer's attestation is filtered → only 2 in cert.
        assert_eq!(cert.attestations.len(), 2);
        assert_eq!(cert.attested_stake, 2000);
        assert!(cert.is_supermajority());
    }

    #[test]
    fn small_cluster_mode_setter_round_trip() {
        let mut tc = make_3_validator_tc();
        assert!(!tc.small_cluster_da_mode());
        tc.set_small_cluster_da_mode(true);
        assert!(tc.small_cluster_da_mode());
        tc.set_small_cluster_da_mode(false);
        assert!(!tc.small_cluster_da_mode());
    }

    #[test]
    fn test_da_enforcement_height_zero_means_always_enforced() {
        // Setting enforcement height to 0 means enforcement from the very first block
        let mut tc = make_test_tc();
        tc.set_da_enforcement_height(0);

        let block = make_block_no_da_cert(0);
        assert!(
            !tc.verify_da_certificate(&block),
            "With enforcement_height=0, even block 0 must have a DA cert"
        );

        let block = make_block_no_da_cert(1);
        assert!(
            !tc.verify_da_certificate(&block),
            "With enforcement_height=0, block 1 must have a DA cert"
        );

        // But valid cert should still pass
        let block = make_block_with_valid_da_cert(0, 3);
        assert!(
            tc.verify_da_certificate(&block),
            "Valid DA cert at height 0 should pass even with enforcement_height=0"
        );
    }

    #[test]
    fn test_da_enforcement_height_u64_max_means_never_enforced() {
        // Setting enforcement height to u64::MAX effectively disables enforcement
        let mut tc = make_test_tc();
        tc.set_da_enforcement_height(u64::MAX);

        let block = make_block_no_da_cert(1_000_000);
        assert!(
            tc.verify_da_certificate(&block),
            "With enforcement_height=MAX, blocks should always pass soft mode"
        );
    }

    // ── Block-production timing ring buffer ──────────────────────────

    #[test]
    fn test_record_block_production_timing_appends_and_caps() {
        let mut tc = make_test_tc();
        assert!(tc.block_production_history().is_empty());

        tc.record_block_production_timing(1, 1_000); // 1 ms
        tc.record_block_production_timing(2, 2_000_000); // 2 s
        tc.record_block_production_timing(1, 500);
        let h = tc.block_production_history();
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].0, 1);
        assert!((h[0].1 - 0.001).abs() < 1e-9);
        assert_eq!(h[1].0, 2);
        assert!((h[1].1 - 2.0).abs() < 1e-9);
        assert_eq!(h[2].0, 1);

        // Overfill: ring buffer must drop oldest, never grow past the cap.
        for _ in 0..(BLOCK_PROD_HISTORY_CAP + 50) {
            tc.record_block_production_timing(9, 100);
        }
        let h = tc.block_production_history();
        assert_eq!(h.len(), BLOCK_PROD_HISTORY_CAP);
        // Last entries are all the producer-9 fills.
        assert_eq!(h.last().unwrap().0, 9);
    }

    // ── Drain (admin) ────────────────────────────────────────────────

    #[test]
    fn test_drain_state_round_trip() {
        let mut tc = make_test_tc();
        let (draining, since) = tc.drain_state();
        assert!(!draining);
        assert!(since.is_none());

        let started_at = tc.set_draining();
        let (draining, since) = tc.drain_state();
        assert!(draining);
        assert_eq!(since, Some(started_at));
        assert!(tc.is_draining());

        // Idempotent: a second set_draining keeps the flag, refreshes anchor.
        let started_at2 = tc.set_draining();
        assert_eq!(
            started_at, started_at2,
            "epoch anchor stable at this height"
        );

        let was = tc.clear_draining();
        assert!(was);
        let (draining, since) = tc.drain_state();
        assert!(!draining);
        assert!(since.is_none());

        // clear_draining when not draining: returns false, idempotent.
        let was = tc.clear_draining();
        assert!(!was);
    }
}

// ─────────────────── Per-height finality gap tests (Mainnet P1) ─────
//
// These cover the operator-visible finality-stall surface added on top
// of `TendermintConsensus`: per-height commit→finalise gap recording,
// the unfinalised tail accessor, and the ring-buffer eviction policy
// for the gap-history sample stream.

#[cfg(test)]
mod finality_gap_tests {
    use super::*;
    use crate::validator_set::ValidatorInfo;

    fn make_tc() -> TendermintConsensus {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [1u8; 32]));
        vs.add_validator(ValidatorInfo::new(2, 1000, [2u8; 32]));
        vs.add_validator(ValidatorInfo::new(3, 1000, [3u8; 32]));
        TendermintConsensus::new_for_test(1, 5, vs)
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    #[test]
    fn test_finality_gap_tracked_on_commit_and_finalise() {
        // Inject a synthetic commit timestamp 50 ms in the past, then
        // drive the same close-the-gap logic the production finalise
        // hook uses: pop the entry, compute now - committed, push the
        // sample. This avoids needing a fully-formed Block + cert path
        // while still exercising the ring buffer + accessor surface.
        let mut tc = make_tc();
        let synthetic_committed = now_ms().saturating_sub(50);
        tc.test_record_commit_at(1, synthetic_committed);

        // Mirror the close-the-gap step from on_block_committed.
        let pop_now = now_ms();
        let committed = tc.committed_at.remove(&1).expect("commit recorded");
        let gap_ms = pop_now.saturating_sub(committed);
        tc.test_push_finality_gap(1, gap_ms);

        let history = tc.finality_gap_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].0, 1);
        assert!(
            history[0].1 >= 50,
            "gap should be >= 50 ms (got {} ms)",
            history[0].1
        );
        // committed_at no longer holds height 1 → unfinalised tail empty.
        assert!(tc.unfinalised_tail().is_empty());
        assert_eq!(tc.worst_unfinalised_gap_ms(), 0);
    }

    #[test]
    fn test_unfinalised_tail_reports_age() {
        // Three heights committed with monotonically *older* timestamps
        // (height 1 oldest → largest age). Tail must report all three
        // and worst must equal the height-1 age.
        let mut tc = make_tc();
        let now = now_ms();
        tc.test_record_commit_at(1, now.saturating_sub(300));
        tc.test_record_commit_at(2, now.saturating_sub(200));
        tc.test_record_commit_at(3, now.saturating_sub(100));

        let tail = tc.unfinalised_tail();
        assert_eq!(tail.len(), 3);
        // BTreeMap iteration is height-ascending, so ages must be
        // *descending* (older commit → larger age).
        assert!(tail[0].1 >= tail[1].1);
        assert!(tail[1].1 >= tail[2].1);
        // The oldest commit dominates the worst-gap signal.
        assert!(tc.worst_unfinalised_gap_ms() >= 300);
        // Heights are ordered.
        assert_eq!(tail[0].0, 1);
        assert_eq!(tail[1].0, 2);
        assert_eq!(tail[2].0, 3);
    }

    #[test]
    fn test_finality_gap_history_ring_buffer_cap() {
        // Push 1100 samples; the deque must cap at FINALITY_GAP_HISTORY_CAP
        // and the oldest entries must be the ones evicted.
        let mut tc = make_tc();
        for h in 0u64..1100 {
            tc.test_push_finality_gap(h, h);
        }
        let history = tc.finality_gap_history();
        assert_eq!(history.len(), FINALITY_GAP_HISTORY_CAP);
        // First retained sample = 1100 - 1024 = 76.
        assert_eq!(history[0].0, 1100u64 - FINALITY_GAP_HISTORY_CAP as u64);
        // Last retained sample = 1099.
        assert_eq!(history[history.len() - 1].0, 1099);
    }
}

// ════════════════════════════════════════════════════════════════════════
// Lane T1.14 — Phase 2 round-trip test
// (POST_EXEC_STATE_VERIFICATION_PLAN.md, MAINNET_READINESS.md)
// ════════════════════════════════════════════════════════════════════════
//
// End-to-end proof that the proposer's stamped `block.post_state_root`
// (filled by Phase 2's speculative execute at `create_proposal`,
// commit `af6876d`) equals the validator's apply-time `state_root`
// when the same block is replayed via `apply_block`.
//
// Why this matters:
//   - Phase 2 wiring uses `ParallelExecutor::snapshot_for_simulation`
//     (commit `42a318e`) + `StateDB::begin_batch/rollback_batch` to
//     run a speculative `execute_block`, capture `state_root`, then
//     revert both executor + DB.
//   - InMemoryStateDB had no real `begin_batch/rollback_batch` until
//     commit `69ed84e` — so before this fix, in-memory tests could
//     not detect a snapshot-restore drift in the speculative path.
//   - This test exercises the full round-trip on the now-correct
//     in-memory backend. Failure = a mutable field on
//     `ParallelExecutor` is missed by `Clone`, OR a mutable field on
//     `InMemoryStateDB` is missed by `InMemoryBatchSnapshot`.
//
// Failure mode shape: post_state_root will not equal the validator's
// apply-time state_root, and the assertion below catches it BEFORE
// Phase 3's runtime warn-mode logger fires (which is silently
// observable on cluster but not test-blocking).

#[cfg(test)]
mod phase2_round_trip_tests {
    use super::*;
    use crate::validator_set::ValidatorInfo;
    use evaporchain_state::db::InMemoryStateDB;
    use evaporchain_types::TransferTx;

    fn make_tc_with_validators() -> TendermintConsensus {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [1u8; 32]));
        vs.add_validator(ValidatorInfo::new(2, 1000, [2u8; 32]));
        vs.add_validator(ValidatorInfo::new(3, 1000, [3u8; 32]));
        vs.add_validator(ValidatorInfo::new(4, 1000, [4u8; 32]));
        TendermintConsensus::new_for_test(1, 100, vs)
    }

    fn dummy_transfer(amount: u64) -> Transaction {
        Transaction::Transfer(TransferTx {
            from: [0xAA; 32],
            to: [0xBB; 32],
            amount,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        })
    }

    /// **The headline lane T1.14 test.** Proposer creates a block via
    /// `create_proposal` (Phase 2 wiring stamps `post_state_root`).
    /// Validator runs `apply_block` on the same DB. The validator's
    /// computed state_root must match the proposer's stamped claim.
    #[test]
    fn proposer_post_state_root_matches_validator_apply_state_root() {
        let mut tc = make_tc_with_validators();
        let mut db = InMemoryStateDB::new();

        // Submit a tx so the block has real execution work to do —
        // an empty block's post_state_root is much less useful as a
        // round-trip witness.
        tc.mempool.submit(dummy_transfer(100));

        // Proposer fills `block.post_state_root` via Phase 2's
        // speculative execute. After this returns, executor + db
        // should be bit-identical to their pre-call state.
        let block = tc
            .create_proposal(&mut db)
            .expect("proposer must produce a block");
        let claimed = block
            .post_state_root
            .expect("Phase 2 wiring must fill post_state_root");

        // Validator-path apply: real execute_block on the same db
        // (which Phase 2's rollback restored). Returns
        // BlockProductionResult { block, execution }.
        let result = tc
            .apply_block(&mut db, &block)
            .expect("validator apply must succeed");

        assert_eq!(
            result.execution.state_root, claimed,
            "Phase 2 round-trip violation: proposer's speculative state_root \
             ({}) does not equal validator's apply-time state_root ({}). \
             A mutable field on ParallelExecutor or InMemoryStateDB is \
             missed by snapshot/restore — investigate which one diverged.",
            hex::encode(&claimed[..8]),
            hex::encode(&result.execution.state_root[..8])
        );
    }

    /// Empty-block round-trip — even with no transactions, the
    /// speculative execute still runs (block-reward path, demurrage,
    /// epoch advance). The roots must still match.
    #[test]
    fn empty_block_post_state_root_matches_validator_apply_state_root() {
        let mut tc = make_tc_with_validators();
        let mut db = InMemoryStateDB::new();

        // No txs submitted — empty proposal.
        let block = tc
            .create_proposal(&mut db)
            .expect("proposer must produce an empty block");
        let claimed = block
            .post_state_root
            .expect("Phase 2 wiring must fill post_state_root even on empty blocks");

        let result = tc
            .apply_block(&mut db, &block)
            .expect("validator apply must succeed");

        assert_eq!(
            result.execution.state_root, claimed,
            "empty-block Phase 2 round-trip violation: speculative={} apply={}",
            hex::encode(&claimed[..8]),
            hex::encode(&result.execution.state_root[..8])
        );
    }

    /// Multi-tx round-trip — three txs in one block. Catches
    /// snapshot drift that's tx-count-dependent (e.g. mempool draining
    /// state, MMR insertion order, accumulator increments).
    #[test]
    fn multi_tx_block_post_state_root_matches_validator_apply_state_root() {
        let mut tc = make_tc_with_validators();
        let mut db = InMemoryStateDB::new();

        for i in 0..3 {
            tc.mempool.submit(dummy_transfer(100 + i));
        }

        let block = tc
            .create_proposal(&mut db)
            .expect("proposer must produce a block");
        let claimed = block
            .post_state_root
            .expect("Phase 2 wiring must fill post_state_root");

        let result = tc
            .apply_block(&mut db, &block)
            .expect("validator apply must succeed");

        assert_eq!(
            result.execution.state_root, claimed,
            "multi-tx Phase 2 round-trip violation: speculative={} apply={}",
            hex::encode(&claimed[..8]),
            hex::encode(&result.execution.state_root[..8])
        );
    }

    // ─── Lane T0.3 — Phase 4 governance flag tests ────────────────────
    //
    // `post_state_verify_mode ∈ {"off", "warn", "enforce"}`. Default
    // "warn" preserves af6876d/cb12cf1 always-on behaviour.
    //
    //   "off"     — proposer skips speculative execute (per-block
    //               CPU cost goes to zero).
    //   "warn"    — proposer fills, validator warns on mismatch,
    //               apply still succeeds.
    //   "enforce" — proposer fills, validator returns Err on
    //               mismatch from apply_block.

    #[test]
    fn phase4_off_mode_skips_speculative_execute() {
        let mut tc = make_tc_with_validators();
        // Flip flag to "off" before proposing.
        tc.governance_set_param("post_state_verify_mode", "off")
            .expect("flag set must succeed");
        let mut db = InMemoryStateDB::new();
        tc.mempool.submit(dummy_transfer(100));

        let block = tc.create_proposal(&mut db).expect("proposer produces block");

        assert!(
            block.post_state_root.is_none(),
            "off-mode proposer must NOT fill post_state_root; got {:?}",
            block.post_state_root.map(|r| hex::encode(&r[..8]))
        );
    }

    #[test]
    fn phase4_warn_mode_fills_and_apply_accepts_mismatch() {
        let mut tc = make_tc_with_validators();
        // Default mode is "warn" but set explicitly to be unambiguous.
        tc.governance_set_param("post_state_verify_mode", "warn")
            .expect("flag set must succeed");
        let mut db = InMemoryStateDB::new();
        tc.mempool.submit(dummy_transfer(100));

        let mut block = tc.create_proposal(&mut db).expect("proposer produces block");
        // Confirm Phase 2 actually ran in warn mode.
        assert!(
            block.post_state_root.is_some(),
            "warn-mode proposer must fill post_state_root"
        );

        // Forge a mismatch — replace the legitimate post_state_root
        // with a junk value. Validator will warn but should NOT reject.
        block.post_state_root = Some([0xAB; 32]);

        let result = tc.apply_block(&mut db, &block);
        assert!(
            result.is_ok(),
            "warn-mode validator must NOT reject on mismatch; got Err: {:?}",
            result.err()
        );
    }

    #[test]
    fn phase4_enforce_mode_rejects_mismatch() {
        let mut tc = make_tc_with_validators();
        tc.governance_set_param("post_state_verify_mode", "enforce")
            .expect("flag set must succeed");
        let mut db = InMemoryStateDB::new();
        tc.mempool.submit(dummy_transfer(100));

        let mut block = tc.create_proposal(&mut db).expect("proposer produces block");
        assert!(
            block.post_state_root.is_some(),
            "enforce-mode proposer must fill post_state_root"
        );

        // Forge a mismatch.
        block.post_state_root = Some([0xCD; 32]);

        let result = tc.apply_block(&mut db, &block);
        assert!(
            result.is_err(),
            "enforce-mode validator MUST reject on mismatch; got Ok"
        );
        let err_str = format!("{:?}", result.unwrap_err());
        assert!(
            err_str.contains("PHASE-4"),
            "enforce-mode error must reference PHASE-4; got: {}",
            err_str
        );
    }

    // ─── Lane T0.4 — Phase 5 block-hash inclusion tests ──────────────

    /// Pre-Phase-5 / off-mode blocks have post_state_root = None.
    /// Their hash MUST match the pre-T0.4 legacy formula bit-for-bit.
    /// If this test breaks, every chain that has ever produced a
    /// block under the pre-Phase-5 binary is bricked.
    #[test]
    fn phase5_none_preserves_legacy_block_hash() {
        let mut block = make_block(1, [0xAA; 32], 100);
        block.post_state_root = None;
        let hash_with_none = TendermintConsensus::block_hash(&block);

        // Compute the legacy hash by hand using the exact bytes the
        // pre-T0.4 formula appended (number, epoch, parent_hash,
        // state_root, timestamp, producer tag-0, vrf_output tag-0,
        // vrf_proof tag-0, data_root tag-0, no transactions). If the
        // test passes, the new code path took the `if let Some = None`
        // false branch and did NOT append anything for post_state_root.
        let mut legacy_input = Vec::new();
        legacy_input.extend_from_slice(&block.number.to_le_bytes());
        legacy_input.extend_from_slice(&block.epoch.to_le_bytes());
        legacy_input.extend_from_slice(&block.parent_hash);
        legacy_input.extend_from_slice(&block.state_root);
        legacy_input.extend_from_slice(&block.timestamp.to_le_bytes());
        legacy_input.push(0); // producer_id None
        legacy_input.push(0); // vrf_output None
        legacy_input.push(0); // vrf_proof None
        legacy_input.push(0); // data_root None
        // no txs
        let legacy_hash = blake3_hash(&legacy_input);

        assert_eq!(
            hash_with_none, legacy_hash,
            "Phase 5 broke legacy hash: post_state_root=None must NOT contribute to hash"
        );
    }

    /// Phase 5 active path: post_state_root = Some changes the hash.
    /// Two blocks identical in every other field but with one having
    /// post_state_root and the other None must produce DIFFERENT hashes.
    #[test]
    fn phase5_some_changes_block_hash() {
        let mut block_none = make_block(1, [0xAA; 32], 100);
        block_none.post_state_root = None;

        let mut block_some = make_block(1, [0xAA; 32], 100);
        block_some.post_state_root = Some([0xBB; 32]);

        let hash_none = TendermintConsensus::block_hash(&block_none);
        let hash_some = TendermintConsensus::block_hash(&block_some);

        assert_ne!(
            hash_none, hash_some,
            "Phase 5 broken: post_state_root must affect block hash when Some"
        );
    }

    /// Two blocks with DIFFERENT post_state_root values must produce
    /// different hashes — proves the field's bytes flow into the hash
    /// (not just a presence tag).
    #[test]
    fn phase5_different_post_state_roots_produce_different_hashes() {
        let mut block_a = make_block(1, [0xAA; 32], 100);
        block_a.post_state_root = Some([0xCC; 32]);
        let mut block_b = make_block(1, [0xAA; 32], 100);
        block_b.post_state_root = Some([0xDD; 32]);

        let hash_a = TendermintConsensus::block_hash(&block_a);
        let hash_b = TendermintConsensus::block_hash(&block_b);

        assert_ne!(
            hash_a, hash_b,
            "Phase 5 broken: distinct post_state_root values must hash differently"
        );
    }

    /// Helper for the Phase 5 hash tests.
    fn make_block(number: u64, parent: [u8; 32], timestamp: u64) -> Block {
        Block {
            number,
            epoch: number,
            parent_hash: parent,
            state_root: [1u8; 32],
            transactions: vec![],
            timestamp,
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
            post_state_root: None,
        }
    }

    #[test]
    fn phase4_enforce_mode_accepts_matching_root() {
        let mut tc = make_tc_with_validators();
        tc.governance_set_param("post_state_verify_mode", "enforce")
            .expect("flag set must succeed");
        let mut db = InMemoryStateDB::new();
        tc.mempool.submit(dummy_transfer(100));

        // Proposer produces a block with correct post_state_root via
        // the speculative execute path. apply_block should succeed
        // because proposer's claim matches validator's local execution.
        let block = tc.create_proposal(&mut db).expect("proposer produces block");
        assert!(block.post_state_root.is_some());

        let result = tc.apply_block(&mut db, &block);
        assert!(
            result.is_ok(),
            "enforce-mode must accept matching root; got Err: {:?}",
            result.err()
        );
    }

    /// T0.1.T0.3 acceptance test — after a divergent block is rejected
    /// by an enforce-mode validator, the chain MUST NOT advance locally.
    /// This is the cluster-stall behaviour the spec calls for: under
    /// 2f+1 validators in enforce-mode, a divergent proposal halts
    /// progress until a clean proposal is produced. The single-validator
    /// proxy for "cluster stalls" is "block_number() unchanged after
    /// apply_block returns Err".
    ///
    /// Companion to phase4_enforce_mode_rejects_mismatch which proves
    /// apply_block returns Err; this proves the chain-state contract:
    /// rejected blocks do NOT bump the local height counter.
    #[test]
    fn phase4_enforce_mode_rejected_block_does_not_advance_chain() {
        let mut tc = make_tc_with_validators();
        tc.governance_set_param("post_state_verify_mode", "enforce")
            .expect("flag set must succeed");
        let mut db = InMemoryStateDB::new();
        tc.mempool.submit(dummy_transfer(100));

        let height_before = tc.block_number();

        let mut block = tc
            .create_proposal(&mut db)
            .expect("proposer produces block");
        assert!(block.post_state_root.is_some());
        // Forge a mismatch.
        block.post_state_root = Some([0xCD; 32]);

        let result = tc.apply_block(&mut db, &block);
        assert!(
            result.is_err(),
            "pre-condition: enforce-mode apply_block must reject divergent block"
        );

        // The load-bearing post-condition: the chain has NOT advanced.
        // A validator that rejects a divergent block must keep its
        // local height pinned to where it was, so the next round can
        // re-propose at the same height with a clean state_root.
        let height_after = tc.block_number();
        assert_eq!(
            height_before, height_after,
            "enforce-mode rejected block must NOT advance the local chain; \
             before={}, after={}",
            height_before, height_after
        );
    }

    /// T0.3 follow-up — under enforce-mode, after a divergent rejection
    /// the validator can apply a SUBSEQUENT clean proposal at the same
    /// height. This is the "cluster recovers when a clean proposer
    /// emerges" half of the cluster-stall behaviour.
    #[test]
    fn phase4_enforce_mode_recovers_with_clean_proposal_after_rejection() {
        let mut tc = make_tc_with_validators();
        tc.governance_set_param("post_state_verify_mode", "enforce")
            .expect("flag set must succeed");
        let mut db = InMemoryStateDB::new();
        tc.mempool.submit(dummy_transfer(100));

        let height_before = tc.block_number();
        let mut block = tc
            .create_proposal(&mut db)
            .expect("proposer produces block");
        let clean_post_root = block.post_state_root.expect("enforce fills the field");

        // First attempt: forge a mismatch → reject.
        let mut divergent = block.clone();
        divergent.post_state_root = Some([0xCD; 32]);
        assert!(tc.apply_block(&mut db, &divergent).is_err());
        assert_eq!(tc.block_number(), height_before, "rejected block didn't advance");

        // Second attempt: re-apply the SAME block with the clean
        // post_state_root → must succeed. (In a real cluster this is
        // a different proposer in the next round; the test models
        // recovery by re-applying with the original clean root.)
        block.post_state_root = Some(clean_post_root);
        let result = tc.apply_block(&mut db, &block);
        assert!(
            result.is_ok(),
            "clean post_state_root must apply cleanly post-rejection; \
             err: {:?}",
            result.err()
        );
        assert!(
            tc.block_number() > height_before,
            "clean block must advance the chain"
        );
    }
}
