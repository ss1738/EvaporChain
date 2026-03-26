//! Tendermint-style BFT consensus engine for EvaporChain.
//!
//! Implements a simplified Tendermint consensus protocol:
//!   NewRound → Propose → Prevote → Precommit → Commit
//!
//! - Round-robin proposer weighted by stake + health (via ValidatorSet)
//! - 2f+1 votes required for progression (tolerates f = (n-1)/3 failures)
//! - Timeout-based round advancement when proposer is offline
//! - Nil votes for safety (lock on first valid proposal)

use crate::mempool::Mempool;
use crate::validator_set::{ValidatorInfo, ValidatorSet};
use crate::{BlockProductionResult, ConsensusError};

use evaporchain_crypto::hash::blake3_hash;
use evaporchain_execution::fees::PidFeeController;
use evaporchain_execution::{ExecutionEngine, SimpleExecutor};
use evaporchain_state::db::StateDB;
use evaporchain_types::{Block, Epoch, Transaction};

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

// ─────────────────────── Configuration ───────────────────────────────────

/// Default timeout for each consensus phase.
const PROPOSE_TIMEOUT_MS: u64 = 3000;
const PREVOTE_TIMEOUT_MS: u64 = 2000;
const PRECOMMIT_TIMEOUT_MS: u64 = 2000;

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
    },
    /// Validator precommits to a block hash (or None for nil precommit).
    Precommit {
        height: u64,
        round: u32,
        block_hash: Option<[u8; 32]>,
        validator_id: u64,
    },
}

impl ConsensusMessage {
    pub fn height(&self) -> u64 {
        match self {
            Self::Proposal { height, .. } => *height,
            Self::Prevote { height, .. } => *height,
            Self::Precommit { height, .. } => *height,
        }
    }

    pub fn round(&self) -> u32 {
        match self {
            Self::Proposal { round, .. } => *round,
            Self::Prevote { round, .. } => *round,
            Self::Precommit { round, .. } => *round,
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
    executor: SimpleExecutor,
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
}

impl TendermintConsensus {
    /// Create a new Tendermint consensus engine.
    pub fn new(my_id: u64, grace_period: u64, validator_set: ValidatorSet) -> Self {
        let block_gas_limit = 500_000;
        Self {
            my_id,
            height: 1, // Start at height 1 (genesis is 0)
            epoch: 0,
            parent_hash: [0u8; 32],
            executor: SimpleExecutor::new_production(
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
        // 2f+1 where f = (n-1)/3
        (n * 2 + 2) / 3
    }

    /// Who is the proposer for the current height/round?
    fn proposer_for_round(&self, height: u64, round: u32) -> Option<&ValidatorInfo> {
        // Mix round into epoch for leader selection diversity
        let virtual_epoch = height.wrapping_mul(100).wrapping_add(round as u64);
        self.validator_set.leader_for_epoch(virtual_epoch)
    }

    /// Am I the proposer for the current height/round?
    fn am_i_proposer(&self) -> bool {
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
        for tx in &block.transactions {
            input.extend_from_slice(&serde_json::to_vec(tx).unwrap_or_default());
        }
        blake3_hash(&input)
    }

    // ──────────────── Core State Machine ────────────────────────────────

    /// Called on every tick. Returns actions the node should perform.
    /// This is the main driver of the consensus state machine.
    pub fn tick(&mut self, db: &mut dyn StateDB) -> Vec<ConsensusAction> {
        let mut actions = Vec::new();

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
                        let prevote = ConsensusMessage::Prevote {
                            height: self.height,
                            round: self.round_state.round,
                            block_hash: Some(hash),
                            validator_id: self.my_id,
                        };
                        actions.push(ConsensusAction::BroadcastMessage(prevote));
                        self.round_state.phase = Phase::Prevote;
                        self.round_state.phase_start = Instant::now();
                    }
                }

                // Timeout: move to prevote with nil
                if self.round_state.phase_start.elapsed() > self.propose_timeout {
                    if !self.round_state.prevoted {
                        self.round_state.prevoted = true;
                        let prevote = ConsensusMessage::Prevote {
                            height: self.height,
                            round: self.round_state.round,
                            block_hash: None, // nil vote
                            validator_id: self.my_id,
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

                        let precommit = ConsensusMessage::Precommit {
                            height: self.height,
                            round: self.round_state.round,
                            block_hash: hash,
                            validator_id: self.my_id,
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
                        let precommit = ConsensusMessage::Precommit {
                            height: self.height,
                            round: self.round_state.round,
                            block_hash: None,
                            validator_id: self.my_id,
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
                if let Some(Some(_hash)) = self.check_precommit_quorum() {
                    // 2f+1 precommits for a block → commit!
                    if let Some(block) = self.round_state.proposed_block.take() {
                        self.round_state.phase = Phase::Commit;
                        actions.push(ConsensusAction::CommitBlock(block));
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
                // Waiting for commit to be applied externally
            }
        }

        actions
    }

    /// Process an incoming consensus message. Returns actions to perform.
    pub fn on_message(&mut self, msg: ConsensusMessage) -> Vec<ConsensusAction> {
        let mut actions = Vec::new();

        // Ignore messages for old heights
        if msg.height() < self.height {
            return actions;
        }

        // Ignore messages for future heights (we'll catch up via block sync)
        if msg.height() > self.height {
            debug!(
                msg_height = msg.height(),
                local_height = self.height,
                "Ignoring future consensus message"
            );
            return actions;
        }

        // Ignore messages for old rounds
        if msg.round() < self.round_state.round {
            return actions;
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
                    debug!("Proposal parent hash mismatch — ignoring");
                    return actions;
                }

                let hash = Self::block_hash(&block);
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
                    };
                    actions.push(ConsensusAction::BroadcastMessage(prevote));
                    self.round_state.phase = Phase::Prevote;
                    self.round_state.phase_start = Instant::now();
                }
            }

            ConsensusMessage::Prevote {
                height: _,
                round,
                block_hash,
                validator_id,
            } => {
                if round == self.round_state.round {
                    self.round_state.prevotes.insert(validator_id, block_hash);
                }
            }

            ConsensusMessage::Precommit {
                height: _,
                round,
                block_hash,
                validator_id,
            } => {
                if round == self.round_state.round {
                    self.round_state.precommits.insert(validator_id, block_hash);

                    // Check if we can commit now
                    if let Some(Some(_)) = self.check_precommit_quorum() {
                        if let Some(block) = self.round_state.proposed_block.take() {
                            self.round_state.phase = Phase::Commit;
                            actions.push(ConsensusAction::CommitBlock(block));
                        }
                    }
                }
            }
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
        self.height += 1;

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

    /// Apply a block received from block sync (not through consensus).
    /// Used for catch-up when joining the network.
    pub fn apply_block(
        &mut self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<BlockProductionResult, ConsensusError> {
        let execution = self
            .executor
            .execute_block(db, block)
            .map_err(|e| ConsensusError::ExecutionFailed(e.to_string()))?;

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
            .map_err(|e| ConsensusError::ExecutionFailed(e.to_string()))?;

        Ok(BlockProductionResult {
            block: block.clone(),
            execution,
        })
    }

    // ──────────────── Internal Helpers ───────────────────────────────────

    /// Create a block proposal from the current mempool.
    fn create_proposal(&mut self, _db: &mut dyn StateDB) -> Option<Block> {
        let next_epoch = self.epoch + 1;
        let txs: Vec<Transaction> = self.mempool.drain();

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let block = Block {
            number: self.height,
            epoch: next_epoch,
            parent_hash: self.parent_hash,
            state_root: [0u8; 32], // Will be filled after execution
            transactions: txs,
            timestamp,
            producer_id: Some(self.my_id),
        };

        info!(
            height = self.height,
            round = self.round_state.round,
            txs = block.transactions.len(),
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
        let next_round = self.round_state.round + 1;
        if next_round >= MAX_ROUNDS_PER_HEIGHT {
            warn!(
                height = self.height,
                "Max rounds reached — forcing empty block commit"
            );
            // Force an empty block to make progress
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let block = Block {
                number: self.height,
                epoch: self.epoch + 1,
                parent_hash: self.parent_hash,
                state_root: [0u8; 32],
                transactions: vec![],
                timestamp,
                producer_id: Some(self.my_id),
            };
            self.round_state = RoundState::new(0);
            self.round_state.phase = Phase::Commit;
            self.round_state.proposed_block = Some(block);
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
        TendermintConsensus::new(my_id, 5, make_validator_set(ids))
    }

    #[test]
    fn test_quorum_size() {
        // 1 validator: quorum = 1
        let tc = make_consensus(1, &[1]);
        assert_eq!(tc.quorum_size(), 1);

        // 3 validators: quorum = 2 (tolerates 1 failure)
        let tc = make_consensus(1, &[1, 2, 3]);
        assert_eq!(tc.quorum_size(), 2);

        // 4 validators: quorum = 3
        let tc = make_consensus(1, &[1, 2, 3, 4]);
        assert_eq!(tc.quorum_size(), 3);

        // 7 validators: quorum = 5
        let tc = make_consensus(1, &[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(tc.quorum_size(), 5);
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
}
