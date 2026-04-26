//! State sync / catch-up mechanism for nodes that fall behind.
//!
//! Allows a lagging node to request and apply missing blocks from peers,
//! validating the chain as it catches up.

use evaporchain_types::Block;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const MAX_BATCH_SIZE: u64 = 100;

/// State of the sync process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    Synced,
    Syncing {
        target_height: u64,
        current_height: u64,
    },
    Stalled,
}

/// Request a range of blocks from a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockRequest {
    pub from_height: u64,
    pub to_height: u64,
    pub requester_id: u64,
}

/// Response with requested blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockResponse {
    pub blocks: Vec<Block>,
    pub responder_id: u64,
    pub has_more: bool,
}

/// Manages state synchronization for a node that has fallen behind.
pub struct StateSyncManager {
    local_height: u64,
    local_tip_hash: [u8; 32],
    state: SyncState,
    peer_heights: HashMap<u64, u64>,
    pending_request: Option<BlockRequest>,
    stall_counter: u32,
    max_stall_retries: u32,
}

impl StateSyncManager {
    pub fn new(local_height: u64) -> Self {
        Self {
            local_height,
            local_tip_hash: [0u8; 32],
            state: SyncState::Synced,
            peer_heights: HashMap::new(),
            pending_request: None,
            stall_counter: 0,
            max_stall_retries: 5,
        }
    }

    pub fn set_local_tip_hash(&mut self, hash: [u8; 32]) {
        self.local_tip_hash = hash;
    }

    pub fn state(&self) -> &SyncState {
        &self.state
    }

    pub fn local_height(&self) -> u64 {
        self.local_height
    }

    pub fn update_peer_height(&mut self, peer_id: u64, height: u64) {
        self.peer_heights.insert(peer_id, height);
        self.check_sync_needed();
    }

    pub fn remove_peer(&mut self, peer_id: u64) {
        self.peer_heights.remove(&peer_id);
    }

    fn best_peer_height(&self) -> u64 {
        self.peer_heights.values().copied().max().unwrap_or(0)
    }

    fn check_sync_needed(&mut self) {
        let best = self.best_peer_height();
        if best > self.local_height + 1 {
            self.state = SyncState::Syncing {
                target_height: best,
                current_height: self.local_height,
            };
        }
    }

    /// Generate the next block request if we need to sync.
    pub fn next_request(&mut self) -> Option<BlockRequest> {
        if self.pending_request.is_some() {
            return None;
        }

        let target = match &self.state {
            SyncState::Syncing { target_height, .. } => *target_height,
            _ => return None,
        };

        let from = self.local_height + 1;
        let to = (from + MAX_BATCH_SIZE - 1).min(target);
        if from > to {
            self.state = SyncState::Synced;
            return None;
        }

        // Pick the peer with the highest known height
        let best_peer = self
            .peer_heights
            .iter()
            .max_by_key(|(_, h)| *h)
            .map(|(id, _)| *id);

        let req = BlockRequest {
            from_height: from,
            to_height: to,
            requester_id: best_peer.unwrap_or(0),
        };
        self.pending_request = Some(req.clone());
        Some(req)
    }

    /// Validate and apply a batch of blocks received from a peer.
    /// Returns the blocks that passed validation (in order), or an error.
    pub fn on_block_response(
        &mut self,
        response: BlockResponse,
    ) -> Result<Vec<Block>, SyncError> {
        self.pending_request = None;

        if response.blocks.is_empty() {
            self.stall_counter += 1;
            if self.stall_counter >= self.max_stall_retries {
                self.state = SyncState::Stalled;
                return Err(SyncError::Stalled);
            }
            return Ok(vec![]);
        }

        self.stall_counter = 0;

        // Validate chain continuity
        let mut validated = Vec::with_capacity(response.blocks.len());
        let mut expected_height = self.local_height + 1;
        let mut prev_hash = self.local_tip_hash;

        for block in &response.blocks {
            if block.number != expected_height {
                return Err(SyncError::HeightGap {
                    expected: expected_height,
                    got: block.number,
                });
            }

            if prev_hash != [0u8; 32] && block.parent_hash != prev_hash {
                return Err(SyncError::ParentHashMismatch {
                    height: block.number,
                });
            }

            prev_hash = compute_block_hash(block);
            expected_height += 1;
            validated.push(block.clone());
        }

        // Update local height and tip hash
        if !validated.is_empty() {
            self.local_tip_hash = prev_hash;
        }
        if let Some(last) = validated.last() {
            self.local_height = last.number;
        }

        // Check if sync is complete
        let target = match &self.state {
            SyncState::Syncing { target_height, .. } => *target_height,
            _ => self.local_height,
        };

        if self.local_height >= target {
            self.state = SyncState::Synced;
        } else {
            self.state = SyncState::Syncing {
                target_height: target,
                current_height: self.local_height,
            };
        }

        Ok(validated)
    }

    /// Handle a block request from another node.
    /// The caller provides a closure to fetch blocks from local storage.
    pub fn handle_request<F>(
        &self,
        request: &BlockRequest,
        get_block: F,
    ) -> BlockResponse
    where
        F: Fn(u64) -> Option<Block>,
    {
        let from = request.from_height;
        let to = request.to_height.min(from + MAX_BATCH_SIZE - 1);

        let mut blocks = Vec::new();
        for h in from..=to {
            if h > self.local_height {
                break;
            }
            if let Some(block) = get_block(h) {
                blocks.push(block);
            } else {
                break;
            }
        }

        let has_more = to < self.local_height;
        BlockResponse {
            blocks,
            responder_id: 0,
            has_more,
        }
    }

    pub fn set_local_height(&mut self, height: u64) {
        self.local_height = height;
    }

    pub fn is_syncing(&self) -> bool {
        matches!(self.state, SyncState::Syncing { .. })
    }
}

fn compute_block_hash(block: &Block) -> [u8; 32] {
    use blake3::Hasher;
    let mut hasher = Hasher::new();
    hasher.update(&block.number.to_le_bytes());
    hasher.update(&block.parent_hash);
    hasher.update(&block.state_root);
    hasher.update(&block.epoch.to_le_bytes());
    hasher.update(&block.timestamp.to_le_bytes());
    let hash = hasher.finalize();
    *hash.as_bytes()
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("height gap: expected {expected}, got {got}")]
    HeightGap { expected: u64, got: u64 },
    #[error("parent hash mismatch at height {height}")]
    ParentHashMismatch { height: u64 },
    #[error("sync stalled after max retries")]
    Stalled,
    #[error("invalid block: {0}")]
    InvalidBlock(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_block_chain(start: u64, count: u64) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut prev_hash = [0u8; 32];

        for i in 0..count {
            let height = start + i;
            let block = Block {
                number: height,
                epoch: height,
                parent_hash: prev_hash,
                state_root: [1u8; 32],
                transactions: vec![],
                timestamp: 1000 + height,
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
            };
            prev_hash = compute_block_hash(&block);
            blocks.push(block);
        }
        blocks
    }

    #[test]
    fn test_sync_detects_lag() {
        let mut mgr = StateSyncManager::new(0);
        assert_eq!(*mgr.state(), SyncState::Synced);

        mgr.update_peer_height(1, 100);
        assert!(matches!(
            mgr.state(),
            SyncState::Syncing {
                target_height: 100,
                ..
            }
        ));
    }

    #[test]
    fn test_sync_generates_requests() {
        let mut mgr = StateSyncManager::new(0);
        mgr.update_peer_height(1, 200);

        let req = mgr.next_request().unwrap();
        assert_eq!(req.from_height, 1);
        assert_eq!(req.to_height, 100); // MAX_BATCH_SIZE

        // Should not generate another while pending
        assert!(mgr.next_request().is_none());
    }

    #[test]
    fn test_sync_processes_blocks() {
        let mut mgr = StateSyncManager::new(0);
        mgr.update_peer_height(1, 10);

        let _req = mgr.next_request();
        let blocks = make_block_chain(1, 10);
        let response = BlockResponse {
            blocks,
            responder_id: 1,
            has_more: false,
        };

        let validated = mgr.on_block_response(response).unwrap();
        assert_eq!(validated.len(), 10);
        assert_eq!(mgr.local_height(), 10);
        assert_eq!(*mgr.state(), SyncState::Synced);
    }

    #[test]
    fn test_sync_rejects_height_gap() {
        let mut mgr = StateSyncManager::new(0);
        mgr.update_peer_height(1, 10);
        let _req = mgr.next_request();

        let mut blocks = make_block_chain(1, 5);
        blocks[2].number = 99; // gap

        let response = BlockResponse {
            blocks,
            responder_id: 1,
            has_more: false,
        };

        let result = mgr.on_block_response(response);
        assert!(result.is_err());
    }

    #[test]
    fn test_sync_handles_empty_response() {
        let mut mgr = StateSyncManager::new(0);
        mgr.update_peer_height(1, 10);
        let _req = mgr.next_request();

        for _ in 0..4 {
            let response = BlockResponse {
                blocks: vec![],
                responder_id: 1,
                has_more: false,
            };
            mgr.pending_request = None;
            let _ = mgr.on_block_response(response);
        }
        assert_ne!(*mgr.state(), SyncState::Stalled);

        // 5th empty response triggers stall
        mgr.pending_request = None;
        let response = BlockResponse {
            blocks: vec![],
            responder_id: 1,
            has_more: false,
        };
        let result = mgr.on_block_response(response);
        assert!(result.is_err());
        assert_eq!(*mgr.state(), SyncState::Stalled);
    }

    #[test]
    fn test_sync_multi_batch() {
        let mut mgr = StateSyncManager::new(0);
        mgr.update_peer_height(1, 250);

        // First batch: blocks 1-100
        let _req = mgr.next_request().unwrap();
        let blocks = make_block_chain(1, 100);
        let response = BlockResponse {
            blocks,
            responder_id: 1,
            has_more: true,
        };
        let validated = mgr.on_block_response(response).unwrap();
        assert_eq!(validated.len(), 100);
        assert_eq!(mgr.local_height(), 100);
        assert!(mgr.is_syncing());

        // Second batch: blocks 101-200
        let req = mgr.next_request().unwrap();
        assert_eq!(req.from_height, 101);
    }

    #[test]
    fn test_handle_request_serves_blocks() {
        let blocks = make_block_chain(1, 50);
        let block_map: HashMap<u64, Block> = blocks.iter().map(|b| (b.number, b.clone())).collect();

        let mgr = StateSyncManager::new(50);
        let request = BlockRequest {
            from_height: 10,
            to_height: 20,
            requester_id: 2,
        };

        let response = mgr.handle_request(&request, |h| block_map.get(&h).cloned());
        assert_eq!(response.blocks.len(), 11); // 10..=20
        assert!(response.has_more);
    }

    #[test]
    fn test_sync_rejects_parent_hash_mismatch() {
        let mut mgr = StateSyncManager::new(0);
        mgr.update_peer_height(1, 10);
        let _req = mgr.next_request();

        let mut blocks = make_block_chain(1, 5);
        blocks[2].parent_hash = [0xFFu8; 32]; // tampered

        let response = BlockResponse {
            blocks,
            responder_id: 1,
            has_more: false,
        };

        let result = mgr.on_block_response(response);
        assert!(result.is_err());
        match result.unwrap_err() {
            SyncError::ParentHashMismatch { height } => assert_eq!(height, 3),
            other => panic!("Expected ParentHashMismatch, got {:?}", other),
        }
    }

    #[test]
    fn test_peer_removal() {
        let mut mgr = StateSyncManager::new(0);
        mgr.update_peer_height(1, 100);
        mgr.update_peer_height(2, 200);
        assert_eq!(mgr.best_peer_height(), 200);

        mgr.remove_peer(2);
        assert_eq!(mgr.best_peer_height(), 100);
    }
}
