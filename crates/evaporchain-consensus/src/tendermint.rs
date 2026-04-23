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
use evaporchain_da::block_da::BlockDA;
use evaporchain_da::namespace::{NamespaceMerkleTree, NamespacedBlob};
use crate::finality::FinalityTracker;
use crate::mempool::Mempool;
use crate::validator_set::{ValidatorInfo, ValidatorSet, EpochTransitionManager, ValidatorSetChange};
use crate::{BlockProductionResult, ConsensusError};

use evaporchain_crypto::hash::blake3_hash;
use evaporchain_crypto::signatures::{BlsKeypair, BlsPublicKey, BlsSignature, BlsVerifier};
use evaporchain_crypto::vrf::{RandomnessBeacon, VrfKeypair, VrfOutput, VrfProof, leader_vrf_input, vrf_verify};
use evaporchain_execution::fees::PidFeeController;
use evaporchain_execution::ExecutionEngine;
use evaporchain_execution::parallel::ParallelExecutor;
use evaporchain_state::db::StateDB;
use evaporchain_da::erasure2d::ErasureEncoder2D;
use evaporchain_da::commitments::RowColumnCommitments;
use evaporchain_types::{Block, CommitCertificate, Epoch, Transaction};

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
const PROPOSE_TIMEOUT_MS: u64 = 8000;
const PREVOTE_TIMEOUT_MS: u64 = 4000;
const PRECOMMIT_TIMEOUT_MS: u64 = 4000;

/// Maximum rounds before forcing commit (prevents livelock).
const MAX_ROUNDS_PER_HEIGHT: u32 = 10;

// ─────────────────────── Consensus Messages ─────────────────────────────

/// Messages exchanged between validators during consensus.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl ConsensusMessage {
    pub fn height(&self) -> u64 {
        match self {
            Self::Proposal { height, .. } => *height,
            Self::Prevote { height, .. } => *height,
            Self::Precommit { height, .. } => *height,
            Self::KeyAnnounce { .. } => 0,
            Self::DAAttestation { block_number, .. } => *block_number,
        }
    }

    pub fn round(&self) -> u32 {
        match self {
            Self::Proposal { round, .. } => *round,
            Self::Prevote { round, .. } => *round,
            Self::Precommit { round, .. } => *round,
            Self::KeyAnnounce { .. } => 0,
            Self::DAAttestation { .. } => 0,
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
#[derive(Debug)]
struct RoundState {
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
}

// ─────────────────────── TendermintConsensus ─────────────────────────────

/// Tendermint-style BFT consensus engine.
pub struct TendermintConsensus {
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
    proposals_seen: HashMap<(u64, u32), Vec<(u64, [u8; 32])>>,
    /// Tracks consecutive missed proposals per validator.
    /// Reset to 0 when the validator successfully produces a block.
    missed_proposals: HashMap<u64, u64>,
    /// Weak subjectivity checkpoint: (height, state_root) pairs.
    /// Validators refuse to reorg past the most recent checkpoint.
    weak_subjectivity_checkpoints: Vec<(u64, [u8; 32])>,
    /// Interval between weak subjectivity checkpoints (in blocks).
    checkpoint_interval: u64,
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
}

impl TendermintConsensus {
    /// Create a new Tendermint consensus engine.
    pub fn new(my_id: u64, grace_period: u64, validator_set: ValidatorSet) -> Self {
        Self::new_with_gas_limit(my_id, grace_period, validator_set, 500_000)
    }

    /// Create with a custom block gas limit (for high-throughput mode).
    pub fn new_with_gas_limit(my_id: u64, grace_period: u64, validator_set: ValidatorSet, block_gas_limit: u64) -> Self {
        let block_gas_limit = block_gas_limit;
        Self {
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
            missed_proposals: HashMap::new(),
            weak_subjectivity_checkpoints: Vec::new(),
            checkpoint_interval: 1000, // Checkpoint every 1000 blocks
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
        }
    }

    /// Set the proof verifier for validating Nova IVC proofs on proposed blocks.
    pub fn set_proof_verifier(&mut self, verifier: Box<dyn ProofVerifier>, genesis_state_root: [u8; 32]) {
        self.proof_verifier = Some(verifier);
        self.genesis_state_root = genesis_state_root;
    }

    /// Set the anchor hash provider for rule-based consensus enforcement.
    pub fn set_anchor_provider(&mut self, provider: Box<dyn AnchorHashProvider>) {
        self.anchor_provider = Some(provider);
    }

    /// Set the BLS keypair for this validator (enables aggregate signatures).
    pub fn set_bls_keypair(&mut self, keypair: BlsKeypair) {
        // Also register our own BLS public key in the validator set
        let pk_bytes = keypair.public_key_bytes().0.clone();
        if let Some(vi) = self.validator_set.get_mut(self.my_id) {
            vi.bls_public_key = Some(pk_bytes);
        }
        self.bls_keypair = Some(keypair);
    }

    /// Generate a KeyAnnounce message for broadcasting our BLS public key
    /// along with a proof-of-possession (prevents rogue-key attacks).
    pub fn make_key_announce(&self) -> Option<ConsensusMessage> {
        self.bls_keypair.as_ref().map(|kp| ConsensusMessage::KeyAnnounce {
            validator_id: self.my_id,
            bls_public_key: kp.public_key_bytes().0.clone(),
            proof_of_possession: kp.proof_of_possession().0.clone(),
        })
    }

    /// Set the VRF keypair for this validator (enables VRF-based leader election).
    pub fn set_vrf_keypair(&mut self, keypair: VrfKeypair) {
        self.vrf_keypair = Some(keypair);
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

    /// Submit a reveal nonce for a previously committed encrypted transaction.
    /// The nonce will be used at the next block production to decrypt and include the tx.
    pub fn submit_reveal(&mut self, commitment: [u8; 32], nonce: [u8; 32]) {
        debug!(
            commitment = hex::encode(commitment),
            "Reveal nonce submitted for encrypted tx"
        );
        self.pending_reveals.push((commitment, nonce));
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
    pub fn new_for_test(my_id: u64, grace_period: u64, validator_set: ValidatorSet) -> Self {
        Self {
            my_id,
            height: 1,
            epoch: 0,
            parent_hash: [0u8; 32],
            executor: ParallelExecutor::new_for_test(grace_period),
            mempool: Mempool::new(),
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
            missed_proposals: HashMap::new(),
            weak_subjectivity_checkpoints: Vec::new(),
            checkpoint_interval: 1000,
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
        }
    }

    /// Restore state after a restart.
    pub fn restore_state(&mut self, block_number: u64, epoch: Epoch, parent_hash: [u8; 32]) {
        self.height = block_number + 1;
        self.epoch = epoch;
        self.parent_hash = parent_hash;
        self.round_state = RoundState::new(0);
        self.locked_block = None;
        self.locked_round = None;
        self.valid_block = None;
        self.valid_round = None;
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn validator_set(&self) -> &ValidatorSet {
        &self.validator_set
    }

    pub fn block_number(&self) -> u64 {
        self.height.saturating_sub(1)
    }

    pub fn round(&self) -> u32 {
        self.round_state.round
    }

    pub fn phase(&self) -> Phase {
        self.round_state.phase
    }

    /// Number of validators needed for a 2f+1 quorum.
    fn quorum_size(&self) -> usize {
        let n = self.validator_set.len();
        // Strict >2/3 majority: floor(2n/3) + 1
        n * 2 / 3 + 1
    }

    /// Who is the proposer for the current height/round?
    fn proposer_for_round(&self, height: u64, round: u32) -> Option<&ValidatorInfo> {
        // Mix round into epoch for leader selection diversity
        let virtual_epoch = height.wrapping_mul(100).wrapping_add(round as u64);
        self.validator_set.leader_for_epoch(virtual_epoch)
    }

    /// Am I the proposer for the current height/round?
    pub fn am_i_proposer(&self) -> bool {
        self.proposer_for_round(self.height, self.round_state.round)
            .map_or(false, |v| v.id == self.my_id)
    }

    /// Compute the hash of a block for voting purposes.
    fn block_hash(block: &Block) -> [u8; 32] {
        let mut input = Vec::new();
        input.extend_from_slice(&block.number.to_le_bytes());
        input.extend_from_slice(&block.epoch.to_le_bytes());
        input.extend_from_slice(&block.parent_hash);
        input.extend_from_slice(&block.state_root);
        input.extend_from_slice(&block.timestamp.to_le_bytes());
        // Include VRF output in block hash (commits randomness to the block).
        if let Some(ref vrf_out) = block.vrf_output {
            input.extend_from_slice(vrf_out);
        }
        // Include state function commitment hash (Rule-Based Consensus).
        if let Some(ref sfc) = block.state_function_commitment {
            input.extend_from_slice(&sfc.commitment_hash);
        }
        for tx in &block.transactions {
            input.extend_from_slice(
                &serde_json::to_vec(tx).expect("transaction serialization must not fail"),
            );
        }
        blake3_hash(&input)
    }

    // ──────────────── Core State Machine ────────────────────────────────

    /// Called on every tick. Returns actions the node should perform.
    /// This is the main driver of the consensus state machine.
    pub fn tick(&mut self, db: &mut dyn StateDB) -> Vec<ConsensusAction> {
        let mut actions = Vec::new();

        // Re-broadcast BLS KeyAnnounce every 50 blocks so late-joining peers get our key
        if self.height > 0 && self.height % 50 == 0 && self.round_state.phase == Phase::Propose && self.round_state.round == 0 {
            if let Some(msg) = self.make_key_announce() {
                actions.push(ConsensusAction::BroadcastMessage(msg));
            }
        }

        match self.round_state.phase {
            Phase::Propose => {
                // If I'm the proposer and haven't proposed yet, propose
                if self.am_i_proposer() && self.round_state.proposed_block.is_none() {
                    if let Some(proposal) = self.create_proposal(db) {
                        let msg = ConsensusMessage::Proposal {
                            height: self.height,
                            round: self.round_state.round,
                            block: proposal.clone(),
                            proposer_id: self.my_id,
                        };
                        self.round_state.proposed_block = Some(proposal.clone());
                        self.round_state.proposed_hash = Some(Self::block_hash(&proposal));
                        actions.push(ConsensusAction::BroadcastMessage(msg));

                        // Self-prevote for our own proposal
                        let hash = Self::block_hash(&proposal);
                        self.round_state.prevotes.insert(self.my_id, Some(hash));
                        self.round_state.prevoted = true;
                        let vote_hash = Some(hash);
                        let bls_sig = self.bls_sign_vote(self.height, self.round_state.round, &vote_hash, "prevote");
                        if let Some(ref sig) = bls_sig {
                            self.round_state.prevote_bls_sigs.insert(self.my_id, sig.clone());
                        }
                        let prevote = ConsensusMessage::Prevote {
                            height: self.height,
                            round: self.round_state.round,
                            block_hash: vote_hash,
                            validator_id: self.my_id,
                            bls_signature: bls_sig,
                        };
                        actions.push(ConsensusAction::BroadcastMessage(prevote));

                        // Proposer DA self-attestation: sample our own block's data
                        self.da_block_proposers.insert(proposal.number, self.my_id);
                        if let Some(att_msg) = self.perform_da_sampling(&proposal) {
                            actions.push(ConsensusAction::BroadcastMessage(att_msg));
                        }

                        self.round_state.phase = Phase::Prevote;
                        self.round_state.phase_start = Instant::now();
                    }
                }

                // Timeout: move to prevote with nil
                if self.round_state.phase_start.elapsed() > self.propose_timeout {
                    if !self.round_state.prevoted {
                        self.round_state.prevoted = true;
                        let nil_hash: Option<[u8; 32]> = None;
                        let bls_sig = self.bls_sign_vote(self.height, self.round_state.round, &nil_hash, "prevote");
                        if let Some(ref sig) = bls_sig {
                            self.round_state.prevote_bls_sigs.insert(self.my_id, sig.clone());
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

                        // Lock on this block
                        if hash.is_some() {
                            self.locked_block = self.round_state.proposed_block.clone();
                            self.locked_round = Some(self.round_state.round);
                            self.valid_block = self.round_state.proposed_block.clone();
                            self.valid_round = Some(self.round_state.round);
                        }

                        let bls_sig = self.bls_sign_vote(self.height, self.round_state.round, &hash, "precommit");
                        if let Some(ref sig) = bls_sig {
                            self.round_state.precommit_bls_sigs.insert(self.my_id, sig.clone());
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
                        let bls_sig = self.bls_sign_vote(self.height, self.round_state.round, &nil_hash, "precommit");
                        if let Some(ref sig) = bls_sig {
                            self.round_state.precommit_bls_sigs.insert(self.my_id, sig.clone());
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
                    // 2f+1 precommits for a block → commit!
                    if let Some(mut block) = self.round_state.proposed_block.take() {
                        // Verify the proposed block matches the quorum hash
                        let block_hash = Self::block_hash(&block);
                        if block_hash != hash {
                            warn!(
                                height = self.height,
                                round = self.round_state.round,
                                "Precommit quorum hash mismatch — our block differs from network consensus"
                            );
                            // Return txs to mempool since our block won't be committed
                            for tx in block.transactions.iter().rev() {
                                self.mempool.submit_priority(tx.clone());
                            }
                            // We don't have the correct block — request sync
                            actions.push(ConsensusAction::RequestSync(self.height, self.height + 1));
                        } else {
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
        if let ConsensusMessage::KeyAnnounce { validator_id, ref bls_public_key, ref proof_of_possession } = msg {
            if bls_public_key.len() != 48 {
                warn!(validator_id, len = bls_public_key.len(), "Invalid BLS key length (expected 48)");
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
                if vi.bls_public_key.is_none() || vi.bls_public_key.as_ref() != Some(bls_public_key) {
                    vi.bls_public_key = Some(bls_public_key.clone());
                    vi.bls_pop = if proof_of_possession.is_empty() { None } else { Some(proof_of_possession.clone()) };
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
            block_number, data_root, validator_id, samples_verified, stake, ref signature, ref public_key,
        } = msg {
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
                    "DA attestation received"
                );
            }
            return actions;
        }

        // Ignore messages for old heights
        if msg.height() < self.height {
            return actions;
        }

        // If we receive a message for a future height, we are behind — request sync
        if msg.height() > self.height {
            if msg.height() > self.height + 1 {
                tracing::warn!(
                    local_height = self.height,
                    msg_height = msg.height(),
                    "Behind by {} blocks — requesting sync",
                    msg.height() - self.height
                );
                actions.push(ConsensusAction::RequestSync(self.height + 1, msg.height().saturating_sub(1)));
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
        }

        match msg {
            ConsensusMessage::Proposal {
                height,
                round,
                block,
                proposer_id,
            } => {
                // Verify proposer is legitimate for this round
                let expected_proposer = self
                    .proposer_for_round(height, round)
                    .map(|v| v.id);
                if expected_proposer != Some(proposer_id) {
                    warn!(
                        expected = ?expected_proposer,
                        got = proposer_id,
                        "Invalid proposer for height={} round={}",
                        height, round
                    );
                    return actions;
                }

                // Verify block connects to our chain
                if block.parent_hash != self.parent_hash {
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

                let hash = Self::block_hash(&block);

                // ── Equivocation Detection ──
                // Track proposals per (height, round). If the same proposer sends
                // two different block hashes for the same slot, slash them.
                let key = (height, round);
                let entry = self.proposals_seen.entry(key).or_default();
                let already_proposed = entry.iter().find(|(id, _)| *id == proposer_id);
                if let Some((_, prev_hash)) = already_proposed {
                    if *prev_hash != hash {
                        // EQUIVOCATION: same proposer, same slot, different block!
                        let slashed = self.validator_set.slash_equivocation(proposer_id);
                        warn!(
                            validator = proposer_id,
                            slashed_amount = slashed,
                            height = height,
                            round = round,
                            "SLASHED for equivocation (double-signing)"
                        );
                        return actions; // Reject the equivocating proposal
                    }
                } else {
                    entry.push((proposer_id, hash));
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
                            if proposed_anchor != local_anchor {
                                warn!(
                                    height = height,
                                    round = round,
                                    proposer = proposer_id,
                                    local = %hex::encode(&local_anchor[..8]),
                                    proposed = %hex::encode(&proposed_anchor[..8]),
                                    "Rejected proposal: anchor hash mismatch"
                                );
                                return actions;
                            }
                            debug!(height = height, "Anchor hash verified on proposal");
                        }
                    }
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

                self.round_state.proposed_block = Some(block);
                self.round_state.proposed_hash = Some(hash);

                // Send prevote if we haven't already
                if !self.round_state.prevoted {
                    self.round_state.prevoted = true;

                    // If locked on a different block, vote nil
                    let vote_hash = if let (Some(ref locked), Some(lr)) =
                        (&self.locked_block, self.locked_round)
                    {
                        let locked_hash = Self::block_hash(locked);
                        if locked_hash == hash || lr < round {
                            Some(hash)
                        } else {
                            None // locked on different block
                        }
                    } else {
                        Some(hash) // not locked, vote for proposal
                    };

                    self.round_state.prevotes.insert(self.my_id, vote_hash);
                    let prevote = ConsensusMessage::Prevote {
                        height: self.height,
                        round: self.round_state.round,
                        block_hash: vote_hash,
                        validator_id: self.my_id,
                            bls_signature: None,
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
                height: _,
                round,
                block_hash,
                validator_id,
                bls_signature,
            } => {
                if round == self.round_state.round {
                    // ── Vote Equivocation Detection ──
                    if let Some(&existing_hash) = self.round_state.prevotes.get(&validator_id) {
                        if existing_hash != block_hash {
                            let slashed = self.validator_set.slash_equivocation(validator_id);
                            warn!(
                                validator = validator_id,
                                slashed_amount = slashed,
                                height = self.height,
                                round = round,
                                "SLASHED for prevote equivocation (double-voting)"
                            );
                            return actions; // Reject the conflicting prevote
                        }
                    }
                    self.round_state.prevotes.insert(validator_id, block_hash);
                    if let Some(sig) = bls_signature {
                        self.round_state.prevote_bls_sigs.insert(validator_id, sig);
                    }
                }
            }

            ConsensusMessage::Precommit {
                height: _,
                round,
                block_hash,
                validator_id,
                bls_signature,
            } => {
                if round == self.round_state.round {
                    // ── Vote Equivocation Detection ──
                    if let Some(&existing_hash) = self.round_state.precommits.get(&validator_id) {
                        if existing_hash != block_hash {
                            let slashed = self.validator_set.slash_equivocation(validator_id);
                            warn!(
                                validator = validator_id,
                                slashed_amount = slashed,
                                height = self.height,
                                round = round,
                                "SLASHED for precommit equivocation (double-voting)"
                            );
                            return actions; // Reject the conflicting precommit
                        }
                    }
                    self.round_state.precommits.insert(validator_id, block_hash);
                    if let Some(sig) = bls_signature {
                        self.round_state.precommit_bls_sigs.insert(validator_id, sig);
                    }

                    // Check if we can commit now
                    if let Some(Some(hash)) = self.check_precommit_quorum() {
                        if let Some(mut block) = self.round_state.proposed_block.take() {
                            let block_hash = Self::block_hash(&block);
                            if block_hash != hash {
                                for tx in block.transactions.iter().rev() {
                                    self.mempool.submit_priority(tx.clone());
                                }
                                actions.push(ConsensusAction::RequestSync(self.height, self.height + 1));
                            } else {
                                if block.commit_certificate.is_none() {
                                    block.commit_certificate = self.try_build_commit_certificate(hash);
                                }
                                self.round_state.phase = Phase::Commit;
                                actions.push(ConsensusAction::CommitBlock(block));
                            }
                        }
                    }
                }
            }
            // KeyAnnounce and DAAttestation are handled before height filters — unreachable here
            ConsensusMessage::KeyAnnounce { .. } => {}
            ConsensusMessage::DAAttestation { .. } => {}
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
        // Update validator health
        if let Some(producer_id) = block.producer_id {
            self.validator_set
                .update_health_score(producer_id, objects_evaporated);
            // Reset missed-proposal counter for successful producer
            self.missed_proposals.insert(producer_id, 0);
        }
        self.validator_set.decay_health_scores();

        // Derive parent hash for next block
        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(&block.number.to_le_bytes());
        hash_input.extend_from_slice(&block.epoch.to_le_bytes());
        hash_input.extend_from_slice(&state_root);
        hash_input.extend_from_slice(&block.parent_hash);
        self.parent_hash = blake3_hash(&hash_input);

        self.epoch = block.epoch;
        self.committed_heights.insert(self.height);
        if let Some(pid) = block.producer_id {
            self.da_block_proposers.insert(block.number, pid);
        }
        self.height += 1;

        // Advance randomness beacon with this block's VRF output.
        if let Some(ref vrf_out) = block.vrf_output {
            self.randomness_beacon.ingest(block.number, vrf_out);
        }

        // ── Weak Subjectivity Checkpoint ──
        // Periodically snapshot (height, state_root) so nodes refuse to reorg
        // past this point. Prevents long-range attacks.
        if block.number > 0 && block.number % self.checkpoint_interval == 0 {
            self.weak_subjectivity_checkpoints
                .push((block.number, state_root));
            info!(
                height = block.number,
                state_root = hex::encode(state_root),
                total_checkpoints = self.weak_subjectivity_checkpoints.len(),
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
                    self.epoch_manager.queue_change(
                        ValidatorSetChange::Join(info),
                        block.epoch,
                    );
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
            let block_hash = self.parent_hash; // Hash we just computed above
            let total_stake = self.validator_set.total_stake();
            let signing_stake = cert.signer_ids.iter()
                .filter_map(|id| self.validator_set.get_validator(*id))
                .map(|v| v.stake)
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
        }

        // ── DA Attestation Round ──
        // Start a new DA attestation round if the block has a data_root.
        // Validators will sample shards and submit attestations.
        if let Some(data_root) = block.data_root {
            let total_stake = self.validator_set.total_stake();
            self.da_attestation.start_round(block.number, data_root, total_stake);

            // If we have a BLS keypair, create our own attestation immediately
            if let Some(ref bls_kp) = self.bls_keypair {
                if let Some(my_validator) = self.validator_set.get_validator(self.my_id) {
                    let att = self.da_attestation.create_own_attestation(
                        self.my_id,
                        my_validator.stake,
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

        // Reset round state for new height
        self.round_state = RoundState::new(0);
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
    /// Returns false if the block tries to reorg past a checkpoint.
    pub fn check_weak_subjectivity(&self, block: &Block) -> bool {
        if let Some(&(cp_height, _cp_root)) = self.weak_subjectivity_checkpoints.last() {
            if block.number <= cp_height {
                warn!(
                    block = block.number,
                    checkpoint = cp_height,
                    "Block rejected: violates weak subjectivity checkpoint"
                );
                return false;
            }
        }
        true
    }

    /// Get all weak subjectivity checkpoints.
    pub fn checkpoints(&self) -> &[(u64, [u8; 32])] {
        &self.weak_subjectivity_checkpoints
    }

    /// Load checkpoints from persistent storage (on restart).
    pub fn load_checkpoints(&mut self, checkpoints: Vec<(u64, [u8; 32])>) {
        self.weak_subjectivity_checkpoints = checkpoints;
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
            return Err(ConsensusError::ExecutionFailed(
                format!(
                    "Block {} violates weak subjectivity checkpoint",
                    block.number
                ),
            ));
        }

        let execution = self
            .executor
            .execute_block(db, block)
            .map_err(|e: evaporchain_execution::ExecutionError| ConsensusError::ExecutionFailed(e.to_string()))?;

        self.on_block_committed(block, execution.state_root, execution.objects_evaporated);

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
        let execution = self
            .executor
            .execute_block(db, block)
            .map_err(|e: evaporchain_execution::ExecutionError| ConsensusError::ExecutionFailed(e.to_string()))?;

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

    // ──────────────── Internal Helpers ───────────────────────────────────

    /// Create a block proposal from the current mempool.
    /// Caps transactions per block to keep proposals under gossipsub size limits.
    fn create_proposal(&mut self, _db: &mut dyn StateDB) -> Option<Block> {
        const MAX_TXS_PER_BLOCK: usize = 50;
        let next_epoch = self.epoch + 1;

        // Process encrypted mempool reveals first (MEV-protected txs get priority)
        let reveals: Vec<([u8; 32], [u8; 32])> = self.pending_reveals.drain(..).collect();
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

        // Fill remaining capacity from plain mempool
        let remaining = MAX_TXS_PER_BLOCK.saturating_sub(txs.len());
        if remaining > 0 {
            txs.extend(self.mempool.take(remaining));
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

        // Compute DA commitment (data_root) over transaction data
        let data_root = if !txs.is_empty() {
            match serde_json::to_vec(&txs) {
                Ok(tx_bytes) => {
                    match BlockDA::new() {
                        Ok(da) => {
                            match da.encode_block(&tx_bytes) {
                                Ok(package) => {
                                    debug!(
                                        height = self.height,
                                        shards = package.shards.len(),
                                        data_bytes = tx_bytes.len(),
                                        "DA erasure-coded block data"
                                    );
                                    Some(package.header.commitment_root)
                                }
                                Err(e) => {
                                    warn!("DA encoding failed: {e} — block produced without data_root");
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            warn!("DA init failed: {e} — block produced without data_root");
                            None
                        }
                    }
                }
                Err(e) => {
                    warn!("TX serialization failed: {e} — block produced without data_root");
                    None
                }
            }
        } else {
            None // Empty blocks have no data_root
        };

        // Build namespace Merkle tree for blob commitments
        let blob_commitments = if !txs.is_empty() {
            let namespaced_blobs: Vec<NamespacedBlob> = txs.iter().map(|tx| {
                let (ns_id, data) = match tx {
                    Transaction::Blob(blob_tx) => {
                        (blob_tx.namespace_id, blob_tx.data.clone())
                    }
                    _ => {
                        // Non-blob txs go into namespace 0 (core transactions)
                        let data = serde_json::to_vec(tx).unwrap_or_default();
                        (0u64, data)
                    }
                };
                let mut namespace = [0u8; 8];
                namespace.copy_from_slice(&ns_id.to_be_bytes());
                NamespacedBlob { namespace, data }
            }).collect();
            let nmt = NamespaceMerkleTree::from_blobs(&namespaced_blobs);
            nmt.blob_commitment_hashes()
        } else {
            vec![]
        };

        let anchor_hash = self.anchor_provider.as_ref()
            .and_then(|p| p.anchor_hash_for_height(self.height));

        // Attach DA certificate from the previous block if supermajority was reached
        // (certificates are built asynchronously as attestations arrive from peers)
        let da_certificate = self.try_attach_pending_da_certificate();

        let block = Block {
            number: self.height,
            epoch: next_epoch,
            parent_hash: self.parent_hash,
            state_root: [0u8; 32], // Will be filled after execution
            transactions: txs,
            timestamp,
            producer_id: Some(self.my_id),
            vrf_output: vrf_out,
            vrf_proof: vrf_prf,
            data_root,
            blob_commitments,
            da_certificate,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None, // Filled after execution by the node
            da_row_roots: vec![],
            da_col_roots: vec![],
        };

        info!(
            height = self.height,
            round = self.round_state.round,
            txs = block.transactions.len(),
            has_data_root = block.data_root.is_some(),
            "Created proposal"
        );

        Some(block)
    }

    /// Check if any block hash has 2f+1 prevotes.
    /// Returns Some(Some(hash)) if quorum for a block, Some(None) if quorum for nil.
    fn check_prevote_quorum(&self) -> Option<Option<[u8; 32]>> {
        let quorum = self.quorum_size();

        // Count votes per hash
        let mut hash_counts: HashMap<Option<[u8; 32]>, usize> = HashMap::new();
        for (_, hash) in &self.round_state.prevotes {
            *hash_counts.entry(*hash).or_insert(0) += 1;
        }

        // Check if any hash (including nil) has quorum
        for (hash, count) in &hash_counts {
            if *count >= quorum {
                return Some(*hash);
            }
        }

        None
    }

    /// Check if any block hash has 2f+1 precommits.
    fn check_precommit_quorum(&self) -> Option<Option<[u8; 32]>> {
        let quorum = self.quorum_size();

        let mut hash_counts: HashMap<Option<[u8; 32]>, usize> = HashMap::new();
        for (_, hash) in &self.round_state.precommits {
            *hash_counts.entry(*hash).or_insert(0) += 1;
        }

        for (hash, count) in &hash_counts {
            if *count >= quorum {
                return Some(*hash);
            }
        }

        None
    }

    /// Move to the next round within the same height.
    fn advance_round(&mut self) {
        // ── Downtime Detection ──
        // If no proposal was received this round, the expected proposer missed.
        // Track consecutive misses and slash after threshold.
        if self.round_state.proposed_block.is_none() {
            if let Some(expected) = self.proposer_for_round(self.height, self.round_state.round) {
                let expected_id = expected.id;
                let misses = self.missed_proposals.entry(expected_id).or_insert(0);
                *misses += 1;
                let total_misses = *misses;

                if total_misses >= 3 {
                    let slashed = self.validator_set.slash_downtime(expected_id, total_misses);
                    warn!(
                        validator = expected_id,
                        missed_blocks = total_misses,
                        slashed_amount = slashed,
                        "SLASHED for downtime (missed proposals)"
                    );
                    // Reset counter after slashing (jailed at 3+)
                    self.missed_proposals.insert(expected_id, 0);
                } else {
                    debug!(
                        validator = expected_id,
                        missed_blocks = total_misses,
                        "Proposer missed round (slash at 3)"
                    );
                }
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
            return;
        }

        info!(
            height = self.height,
            from_round = self.round_state.round,
            to_round = next_round,
            "Advancing to next round"
        );
        self.round_state = RoundState::new(next_round);
    }

    /// Get current proposer info for display.
    pub fn current_proposer(&self) -> Option<&ValidatorInfo> {
        self.proposer_for_round(self.height, self.round_state.round)
    }

    // ──────────────── BLS Aggregate Signatures ─────────────────────────

    /// Construct the canonical message to BLS-sign for a vote.
    fn bls_vote_message(height: u64, round: u32, block_hash: &Option<[u8; 32]>, phase: &str) -> Vec<u8> {
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
    fn bls_sign_vote(&self, height: u64, round: u32, block_hash: &Option<[u8; 32]>, phase: &str) -> Option<Vec<u8>> {
        self.bls_keypair.as_ref().map(|kp| {
            let msg = Self::bls_vote_message(height, round, block_hash, phase);
            kp.sign(&msg).0
        })
    }

    /// Try to build a CommitCertificate from collected BLS precommit signatures.
    fn try_build_commit_certificate(&self, block_hash: [u8; 32]) -> Option<CommitCertificate> {
        let quorum = self.quorum_size();
        let mut signer_ids = Vec::new();
        let mut sigs = Vec::new();

        for (vid, vote_hash) in &self.round_state.precommits {
            if *vote_hash == Some(block_hash) {
                if let Some(sig_bytes) = self.round_state.precommit_bls_sigs.get(vid) {
                    signer_ids.push(*vid);
                    sigs.push(BlsSignature(sig_bytes.clone()));
                }
            }
        }

        if sigs.len() < quorum {
            return None;
        }

        let agg_sig = BlsVerifier::aggregate_signatures(&sigs)?;
        Some(CommitCertificate {
            height: self.height,
            round: self.round_state.round,
            block_hash,
            aggregate_signature: agg_sig.0,
            signer_ids,
        })
    }

    /// Verify a commit certificate against the current validator set.
    pub fn verify_commit_certificate(&self, cert: &CommitCertificate) -> bool {
        let quorum = self.quorum_size();
        if cert.signer_ids.len() < quorum {
            return false;
        }

        let mut pks = Vec::new();
        for &vid in &cert.signer_ids {
            if let Some(validator) = self.validator_set.get(vid) {
                if let Some(ref bls_pk_bytes) = validator.bls_public_key {
                    pks.push(BlsPublicKey(bls_pk_bytes.clone()));
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }

        let msg = Self::bls_vote_message(cert.height, cert.round, &Some(cert.block_hash), "precommit");
        let agg_sig = BlsSignature(cert.aggregate_signature.clone());
        BlsVerifier::aggregate_verify(&msg, &agg_sig, &pks)
    }

    /// Create a DA attestation message for a committed block.
    /// Returns None if this validator has no BLS keypair.
    pub fn make_da_attestation(&self, block_number: u64, data_root: [u8; 32], shards_verified: u32) -> Option<ConsensusMessage> {
        let kp = self.bls_keypair.as_ref()?;
        let stake = self.validator_set.get(self.my_id)
            .map(|v| v.stake)
            .unwrap_or(0);
        let att = evaporchain_da::certificate::create_attestation(
            block_number, &data_root, self.my_id, shards_verified, stake, kp,
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

    /// Try to build a DA certificate from collected attestations for a block.
    /// Returns serialized certificate bytes if supermajority is reached.
    pub fn try_build_da_certificate(&mut self, block_number: u64, data_root: [u8; 32]) -> Option<Vec<u8>> {
        let atts = self.da_attestations.get(&block_number)?;
        let proposer = self.da_block_proposers.get(&block_number).copied();
        let total_stake = self.validator_set.total_stake();
        let mut builder = evaporchain_da::certificate::CertificateBuilder::new(
            block_number, data_root, total_stake,
        );
        for att in atts {
            // Exclude the block proposer — they cannot attest to their own block's DA
            if Some(att.validator_id) == proposer {
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
                if atts.is_empty() { continue; }
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
    /// The validator re-encodes the block's transaction data, verifies the data_root
    /// matches the proposer's commitment, then samples random shards and verifies
    /// their Merkle proofs. If all checks pass, a signed DA attestation is produced.
    pub fn perform_da_sampling(&self, block: &Block) -> Option<ConsensusMessage> {
        let data_root = block.data_root?;

        let tx_bytes = serde_json::to_vec(&block.transactions).ok()?;
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
        let queries = BlockDA::generate_sample_queries(
            block.number,
            &package.header,
            num_samples,
            &seed,
        );

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
            "DA sampling passed"
        );

        self.make_da_attestation(block.number, data_root, verified)
    }

    /// Verify a DA certificate included in a received block.
    /// Returns true if the certificate is valid or absent (absent = not yet enforced).
    pub fn verify_da_certificate(&self, block: &Block) -> bool {
        let cert_bytes = match &block.da_certificate {
            Some(bytes) => bytes,
            None => return true, // DA certificate not yet mandatory
        };
        let cert: evaporchain_da::certificate::DACertificate = match serde_json::from_slice(cert_bytes) {
            Ok(c) => c,
            Err(_) => {
                warn!(block = block.number, "DA certificate deserialization failed");
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

    #[test]
    fn test_full_consensus_round_single_validator() {
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
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
        let mut validators: Vec<TendermintConsensus> = ids
            .iter()
            .map(|&id| make_consensus(id, ids))
            .collect();

        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
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
            let current_msgs: Vec<_> = messages.drain(..).collect();
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
            matches!(a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote {
                    block_hash: None, ..
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
            ConsensusAction::BroadcastMessage(msg @ ConsensusMessage::Proposal { .. }) => Some(msg.clone()),
            _ => None,
        });
        assert!(proposal.is_some(), "Proposer should create a proposal");

        // Deliver proposal to validator 2 (not the proposer)
        let non_proposer_idx = ids.iter().position(|&id| id != proposer_id).unwrap();
        let actions = nodes[non_proposer_idx].on_message(proposal.clone().unwrap());

        // Validator 2 should send a prevote
        let prevote = actions.iter().find_map(|a| match a {
            ConsensusAction::BroadcastMessage(msg @ ConsensusMessage::Prevote { .. }) => Some(msg.clone()),
            _ => None,
        });
        assert!(prevote.is_some(), "Should generate a prevote");

        // Deliver the same prevote to proposer TWICE
        let actions1 = nodes[proposer_idx].on_message(prevote.clone().unwrap());
        let actions2 = nodes[proposer_idx].on_message(prevote.unwrap());

        // The second delivery shouldn't cause different behavior than if it hadn't happened
        // (the vote is already recorded, so it's a no-op)
        // We just verify no crash and no duplicate commit
        let commits1 = actions1.iter().filter(|a| matches!(a, ConsensusAction::CommitBlock(_))).count();
        let commits2 = actions2.iter().filter(|a| matches!(a, ConsensusAction::CommitBlock(_))).count();
        // Should not commit from duplicate votes alone
        assert!(commits1 + commits2 <= 1, "Duplicate votes should not cause multiple commits");
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
        };

        let fake_proposal = ConsensusMessage::Proposal {
            height: 1,
            round: 0,
            block,
            proposer_id: *wrong_id,
        };

        let actions = tc.on_message(fake_proposal);
        // Should not generate a prevote for a wrong proposer's block
        let prevotes: Vec<_> = actions.iter().filter(|a| matches!(a, ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. }))).collect();
        assert!(prevotes.is_empty(), "Should not prevote for wrong proposer");
    }

    #[test]
    fn test_consensus_liveness_with_timeouts() {
        // Even with no messages, consensus should advance rounds via timeouts
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);
        let mut db = InMemoryStateDB::new();


        let initial_round = tc.round();

        // Simulate timeout-driven advancement:
        // Each tick checks elapsed time, so we set phase_start to the past
        // AFTER the tick resets it, then tick again to trigger the timeout.
        for _ in 0..20 {
            // Set phase_start far in the past to trigger timeout
            tc.round_state.phase_start = std::time::Instant::now() - std::time::Duration::from_secs(10);
            // Now tick — this should detect the timeout and advance
            tc.tick(&mut db);
        }

        // Round should have advanced (timeout-driven round rotation)
        assert!(tc.round() > initial_round, "Timeouts should advance rounds: was {} now {}", initial_round, tc.round());
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

        // Set BLS keypair for this node
        let my_idx = ids.iter().position(|&id| id == my_id).unwrap();
        // Generate a new keypair for this node (can't move from vec)
        // Instead, we'll use from_secret_bytes to reconstruct
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
        assert_ne!(msg1, msg3, "Different hash should produce different message");

        let msg4 = TendermintConsensus::bls_vote_message(10, 0, &Some([1u8; 32]), "precommit");
        assert_ne!(msg1, msg4, "Different phase should produce different message");
    }

    #[test]
    fn test_bls_sign_vote_with_keypair() {
        let mut tc = make_consensus(1, &[1, 2, 3, 4]);

        // Without BLS keypair, should return None
        assert!(tc.bls_sign_vote(1, 0, &Some([1u8; 32]), "prevote").is_none());

        // With BLS keypair, should return Some
        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);
        assert!(tc.bls_sign_vote(1, 0, &Some([1u8; 32]), "prevote").is_some());
    }

    #[test]
    fn test_bls_prevotes_include_signatures() {
        let mut db = InMemoryStateDB::new();
        let mut tc = make_bls_consensus(1, &[1]);

        // Single validator with BLS — tick should produce prevote with BLS sig
        let actions = tc.tick(&mut db);
        let has_bls_prevote = actions.iter().any(|a| {
            matches!(a,
                ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote {
                    bls_signature: Some(_), ..
                })
            )
        });
        assert!(has_bls_prevote, "Prevote should include BLS signature when keypair is set");
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
            .enumerate()
            .map(|(i, &id)| {
                let mut tc = TendermintConsensus::new_for_test(id, 5, vs.clone());
                // We need to generate new keypairs since we can't clone BlsKeypair
                let kp = BlsKeypair::generate();
                tc.validator_set.get_mut(id).unwrap().bls_public_key = Some(kp.public_key_bytes().0);
                // Also update in all other nodes' validator sets
                tc.set_bls_keypair(kp);
                tc
            })
            .collect();

        // Synchronize BLS public keys across all nodes
        let pks: Vec<(u64, Vec<u8>)> = nodes.iter().map(|n| {
            let pk = n.validator_set.get(n.my_id).unwrap().bls_public_key.clone().unwrap();
            (n.my_id, pk)
        }).collect();
        for node in &mut nodes {
            for (id, pk) in &pks {
                node.validator_set.get_mut(*id).unwrap().bls_public_key = Some(pk.clone());
            }
        }

        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
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
            let current_msgs: Vec<_> = messages.drain(..).collect();
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
        assert!(cert.signer_ids.len() >= 3, "Certificate should have >= quorum signers");
        assert!(!cert.aggregate_signature.is_empty(), "Aggregate signature should not be empty");

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
            let current_msgs: Vec<_> = messages.drain(..).collect();
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

        assert!(!committed_blocks.is_empty(), "Non-BLS consensus should still work");
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
        };

        let msg = ConsensusMessage::Proposal {
            height: 1,
            round: 0,
            block,
            proposer_id,
        };

        let actions = tc.on_message(msg);
        // Should generate a prevote (proof accepted)
        let has_prevote = actions.iter().any(|a| matches!(a, ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. })));
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
        };

        let msg = ConsensusMessage::Proposal {
            height: 1,
            round: 0,
            block,
            proposer_id,
        };

        let actions = tc.on_message(msg);
        // Should NOT generate a prevote (proof rejected)
        let has_prevote = actions.iter().any(|a| matches!(a, ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. })));
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
        };

        let msg = ConsensusMessage::Proposal {
            height: 1,
            round: 0,
            block,
            proposer_id,
        };

        let actions = tc.on_message(msg);
        let has_prevote = actions.iter().any(|a| matches!(a, ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. })));
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
                tc.validator_set
                    .get_mut(id)
                    .unwrap()
                    .bls_public_key = Some(kp.public_key_bytes().0);
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
                node.validator_set
                    .get_mut(*id)
                    .unwrap()
                    .bls_public_key = Some(pk.clone());
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
            let current: Vec<_> = messages.drain(..).collect();
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
        let has_blob = block.transactions.iter().any(|tx| {
            matches!(tx, Transaction::Blob(_))
        });
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
        use evaporchain_da::erasure2d::ErasureEncoder2D;
        use evaporchain_da::commitments::{RowColumnCommitments, generate_2d_queries};
        use evaporchain_da::certificate::{CertificateBuilder, create_attestation};

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
            1,     // block_number
            data_root,
            num_validators * 1000, // total_stake
        );

        for vid in 1..=num_validators {
            let seed = blake3::hash(&vid.to_le_bytes());
            let queries = generate_2d_queries(1, matrix.extended_dim(), num_samples, seed.as_bytes());

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
            assert!(all_valid, "All sampled cells should verify for validator {}", vid);

            // Create BLS attestation
            let kp = BlsKeypair::generate();
            let attestation = create_attestation(
                1,         // block_number
                &data_root,
                vid,
                num_samples as u32,
                1000,      // stake
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
        assert!(
            cert.is_supermajority(),
            "4/4 validators = supermajority"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Test 6: Full end-to-end — consensus + DA + BLS + multi-height
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_full_e2e_consensus_da_bls() {
        use evaporchain_da::erasure2d::ErasureEncoder2D;
        use evaporchain_da::commitments::RowColumnCommitments;
        use evaporchain_da::certificate::{CertificateBuilder, create_attestation};

        let ids = &[1u64, 2, 3, 4];
        let mut nodes = make_bls_network(ids);
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 10_000_000,
            nonce: 0,
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
            let current: Vec<_> = messages.drain(..).collect();
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
        let bls_prevotes = all_messages.iter().filter(|m| {
            matches!(m, ConsensusMessage::Prevote { bls_signature: Some(_), .. })
        }).count();

        let bls_precommits = all_messages.iter().filter(|m| {
            matches!(m, ConsensusMessage::Precommit { bls_signature: Some(_), .. })
        }).count();

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
    use evaporchain_crypto::vrf::{VrfKeypair, VrfOutput, VrfProof, leader_vrf_input, vrf_verify, vrf_leader_check};
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
                tc.validator_set
                    .get_mut(id)
                    .unwrap()
                    .bls_public_key = Some(bls_kp.public_key_bytes().0);
                tc.set_bls_keypair(bls_kp);
                // Set VRF keypair
                let vrf_kp = VrfKeypair::generate();
                tc.validator_set
                    .get_mut(id)
                    .unwrap()
                    .vrf_public_key = Some(vrf_kp.public_key_bytes());
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
        });

        // Tick to get the proposer to create a block
        let mut proposal = None;
        for v in nodes.iter_mut() {
            for a in v.tick(&mut db) {
                if let ConsensusAction::BroadcastMessage(
                    ConsensusMessage::Proposal { ref block, .. }
                ) = a
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
        assert!(
            block.vrf_proof.is_some(),
            "Block should contain VRF proof"
        );

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
            let current: Vec<_> = messages.drain(..).collect();
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

        assert!(!committed.is_empty(), "VRF-enabled network should reach consensus");
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
            producer_id: Some(proposer_id),
            vrf_output: Some([0xAA; 32]),    // Fake VRF output
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
            matches!(a, ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. }))
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
            if !committed.is_empty() { break; }
            let current: Vec<_> = messages.drain(..).collect();
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
                if !committed.is_empty() { break; }
                let current: Vec<_> = messages.drain(..).collect();
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
            assert!(block.vrf_output.is_some(), "Height {} should have VRF output", height);
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
    use crate::validator_set::{ValidatorInfo, ValidatorSet, EpochTransitionManager};
    use evaporchain_types::{Transaction, ValidatorStakeTx, ValidatorExitTx, Block};

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
        tc.submit_reveal(commitment, nonce);

        let (_, _, reveals) = tc.mempool_stats();
        assert_eq!(reveals, 1);
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
        assert_eq!(reveals, 0);   // reveals consumed
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

        // Submit 60 plain txs (over MAX_TXS_PER_BLOCK = 50)
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
        // Should be capped at 50 (5 revealed + 45 plain)
        assert_eq!(block.transactions.len(), 50);
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
        assert!(block.data_root.is_some(), "block with txs should have data_root");

        // Verify the data_root is a valid commitment
        let data_root = block.data_root.unwrap();
        assert_ne!(data_root, [0u8; 32], "data_root should not be all zeros");
    }

    #[test]
    fn test_empty_proposal_has_no_data_root() {
        let mut tc = make_test_tc();
        let mut db = InMemoryStateDB::new();

        // No transactions in mempool
        let block = tc.create_proposal(&mut db).unwrap();
        assert_eq!(block.transactions.len(), 0);
        assert!(block.data_root.is_none(), "empty block should have no data_root");
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
                "shard {} should verify against data_root", i
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
            assert_ne!(*commitment, [0u8; 32], "blob_commitment[{i}] should not be zero");
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
        assert!(actions.is_empty(), "equivocating prevote should be rejected");
        // Validator 2 should be jailed
        let v = tc.validator_set.get(2).unwrap();
        assert!(v.jailed, "equivocating validator should be jailed");
        assert_eq!(v.total_slashed, 100); // 10% of 1000
        assert_eq!(v.stake, 900);
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

        assert!(actions.is_empty(), "equivocating precommit should be rejected");
        let v = tc.validator_set.get(3).unwrap();
        assert!(v.jailed);
        assert_eq!(v.total_slashed, 100);
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
        let v = tc.validator_set.get(4).unwrap();
        assert!(v.jailed);
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
                assert_ne!(leader.id, 2, "Jailed validator should not lead at epoch {}", epoch);
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
    fn test_da_sampling_on_proposal_with_txs() {
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        tc.mempool.submit(dummy_transfer(42));
        let block = tc.create_proposal(&mut db).unwrap();
        assert!(block.data_root.is_some(), "Block with txs should have data_root");

        let att = tc.perform_da_sampling(&block);
        assert!(att.is_some(), "DA sampling should produce an attestation for a valid block");

        if let Some(ConsensusMessage::DAAttestation { block_number, samples_verified, .. }) = att {
            assert_eq!(block_number, block.number);
            assert!(samples_verified >= 4, "Should verify at least 4 samples");
        } else {
            panic!("Expected DAAttestation message");
        }
    }

    #[test]
    fn test_da_sampling_empty_block_returns_none() {
        let mut tc = make_proposer_tc();
        let mut db = InMemoryStateDB::new();

        let kp = BlsKeypair::generate();
        tc.set_bls_keypair(kp);

        let block = tc.create_proposal(&mut db).unwrap();
        assert!(block.data_root.is_none(), "Empty block should have no data_root");

        let att = tc.perform_da_sampling(&block);
        assert!(att.is_none(), "No attestation for empty blocks");
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
        let has_proposal = actions.iter().any(|a| matches!(a, ConsensusAction::BroadcastMessage(ConsensusMessage::Proposal { .. })));
        let has_prevote = actions.iter().any(|a| matches!(a, ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. })));
        let has_da_att = actions.iter().any(|a| matches!(a, ConsensusAction::BroadcastMessage(ConsensusMessage::DAAttestation { .. })));

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
            block: block,
            proposer_id,
        };
        let actions = tc_receiver.on_message(proposal_msg);

        let has_prevote = actions.iter().any(|a| matches!(a, ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { block_hash: Some(_), .. })));
        let has_da_att = actions.iter().any(|a| matches!(a, ConsensusAction::BroadcastMessage(ConsensusMessage::DAAttestation { .. })));

        assert!(has_prevote, "Validator should prevote for valid proposal");
        assert!(has_da_att, "Validator should broadcast DA attestation after sampling");
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
}
