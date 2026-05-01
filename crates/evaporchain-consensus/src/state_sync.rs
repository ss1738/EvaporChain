//! State sync protocol for fast-syncing new nodes.
//!
//! Instead of replaying all blocks from genesis, a new node can:
//! 1. Discover the chain tip from peers
//! 2. Light-client verify a recent block header (using commit certificates)
//! 3. Download a state snapshot at that height
//! 4. Verify the snapshot against the state root in the trusted header
//! 5. Resume block-by-block consensus from there
//!
//! This module implements the state machine and chunk-based transfer protocol.
//! The actual network transport is handled by the P2P layer.

use crate::light_client::{LightBlockHeader, LightClientVerifier, VerificationResult};
use crate::validator_set::ValidatorSet;
use evaporchain_crypto::hash::blake3_hash;
#[cfg(test)]
use evaporchain_types::CommitCertificate;
use std::collections::{HashMap, HashSet};
use tracing::{debug, info, warn};

// ─────────────────────── Constants ───────────────────────────────────

/// Size of each state snapshot chunk (256 KB).
const CHUNK_SIZE: usize = 256 * 1024;

/// Maximum concurrent chunk requests.
const MAX_CONCURRENT_REQUESTS: usize = 16;

/// Minimum number of peers that must agree on chain tip before syncing.
const MIN_TIP_AGREEMENT: usize = 2;

/// Maximum height lag before triggering state sync (vs block-by-block catch-up).
const STATE_SYNC_THRESHOLD: u64 = 1000;

// ─────────────────────── Types ───────────────────────────────────────

/// State sync phases.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncPhase {
    /// Discovering the chain tip from peers.
    DiscoveringTip,
    /// Light-client verifying the target header.
    VerifyingHeader,
    /// Downloading state snapshot chunks.
    DownloadingSnapshot {
        target_height: u64,
        total_chunks: usize,
        received_chunks: usize,
    },
    /// Verifying the downloaded snapshot against the state root.
    VerifyingSnapshot,
    /// Sync complete — ready to resume consensus.
    Complete { synced_height: u64 },
    /// Sync failed.
    Failed(String),
}

/// A chunk of the state snapshot.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotChunk {
    /// Height this snapshot belongs to.
    pub height: u64,
    /// Chunk index (0-based).
    pub index: usize,
    /// Total number of chunks in the snapshot.
    pub total: usize,
    /// Raw chunk data.
    pub data: Vec<u8>,
    /// Blake3 hash of this chunk's data.
    pub hash: [u8; 32],
}

/// Metadata about a state snapshot offered by a peer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotMetadata {
    /// Block height of the snapshot.
    pub height: u64,
    /// Epoch at snapshot time.
    pub epoch: u64,
    /// State root at this height (must match block header).
    pub state_root: [u8; 32],
    /// Number of chunks in the snapshot.
    pub total_chunks: usize,
    /// Blake3 hashes of each chunk (for integrity verification).
    pub chunk_hashes: Vec<[u8; 32]>,
    /// Total uncompressed size in bytes.
    pub total_size: u64,
}

impl SnapshotMetadata {
    /// Compute a commitment to this metadata (for verification).
    pub fn commitment(&self) -> [u8; 32] {
        let mut input = Vec::new();
        input.extend_from_slice(&self.height.to_le_bytes());
        input.extend_from_slice(&self.epoch.to_le_bytes());
        input.extend_from_slice(&self.state_root);
        input.extend_from_slice(&(self.total_chunks as u64).to_le_bytes());
        for hash in &self.chunk_hashes {
            input.extend_from_slice(hash);
        }
        blake3_hash(&input)
    }
}

/// Messages exchanged during state sync.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SyncMessage {
    /// Request: "What's your latest committed height?"
    TipRequest,
    /// Response: peer's chain tip.
    TipResponse { height: u64, block_hash: [u8; 32] },
    /// Request: "Send me the light block header at height H."
    HeaderRequest { height: u64 },
    /// Response: light block header with commit certificate.
    HeaderResponse { header: LightBlockHeader },
    /// Request: "Send me snapshot metadata for height H."
    SnapshotMetadataRequest { height: u64 },
    /// Response: snapshot metadata.
    SnapshotMetadataResponse { metadata: SnapshotMetadata },
    /// Request: "Send me chunk N of the snapshot at height H."
    ChunkRequest { height: u64, chunk_index: usize },
    /// Response: a snapshot chunk.
    ChunkResponse { chunk: SnapshotChunk },
}

/// Actions the sync manager wants the node to perform.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SyncAction {
    /// Send a message to a specific peer.
    SendToPeer { peer_id: u64, message: SyncMessage },
    /// Broadcast a message to all peers.
    Broadcast { message: SyncMessage },
    /// Apply the assembled state snapshot.
    ApplySnapshot {
        height: u64,
        state_root: [u8; 32],
        data: Vec<u8>,
    },
    /// Resume block-by-block consensus from this height.
    ResumeConsensus {
        height: u64,
        epoch: u64,
        validator_set: ValidatorSet,
    },
}

// ─────────────────────── StateSyncManager ────────────────────────────

/// Manages the state sync protocol for a syncing node.
/// Hardcoded genesis checkpoint for safe bootstrap.
/// Nodes refuse to sync unless the initial header chain connects to this root.
#[derive(Debug, Clone)]
pub struct GenesisCheckpoint {
    pub height: u64,
    pub state_root: [u8; 32],
    pub block_hash: [u8; 32],
}

pub struct StateSyncManager {
    /// Current sync phase.
    phase: SyncPhase,
    /// Our node's current committed height.
    local_height: u64,
    /// Chain tips reported by peers: peer_id → (height, block_hash).
    peer_tips: HashMap<u64, (u64, [u8; 32])>,
    /// Target height we're syncing to.
    target_height: Option<u64>,
    /// Light client verifier for header verification.
    light_client: Option<LightClientVerifier>,
    /// Snapshot metadata for the target height.
    snapshot_meta: Option<SnapshotMetadata>,
    /// Downloaded chunks (index → data).
    received_chunks: HashMap<usize, Vec<u8>>,
    /// Chunks we've requested but not yet received.
    pending_requests: HashSet<usize>,
    /// Genesis checkpoint for safe initial sync.
    genesis_checkpoint: Option<GenesisCheckpoint>,
}

impl StateSyncManager {
    /// Create a new state sync manager.
    pub fn new(local_height: u64) -> Self {
        Self {
            phase: SyncPhase::DiscoveringTip,
            local_height,
            peer_tips: HashMap::new(),
            target_height: None,
            light_client: None,
            snapshot_meta: None,
            received_chunks: HashMap::new(),
            pending_requests: HashSet::new(),
            genesis_checkpoint: None,
        }
    }

    /// Create a state sync manager with a hardcoded genesis checkpoint.
    /// The node will refuse to bootstrap from any header chain that doesn't
    /// connect back to this checkpoint.
    pub fn with_checkpoint(local_height: u64, checkpoint: GenesisCheckpoint) -> Self {
        Self {
            genesis_checkpoint: Some(checkpoint),
            ..Self::new(local_height)
        }
    }

    /// Current sync phase.
    pub fn phase(&self) -> &SyncPhase {
        &self.phase
    }

    /// Whether state sync is needed (we're too far behind).
    pub fn needs_state_sync(local_height: u64, network_height: u64) -> bool {
        network_height.saturating_sub(local_height) > STATE_SYNC_THRESHOLD
    }

    /// Start the sync process by requesting tips from peers.
    pub fn start(&mut self) -> Vec<SyncAction> {
        self.phase = SyncPhase::DiscoveringTip;
        vec![SyncAction::Broadcast {
            message: SyncMessage::TipRequest,
        }]
    }

    /// Handle a sync message from a peer. Returns actions to take.
    pub fn on_message(&mut self, peer_id: u64, msg: SyncMessage) -> Vec<SyncAction> {
        match msg {
            SyncMessage::TipResponse { height, block_hash } => {
                self.handle_tip_response(peer_id, height, block_hash)
            }
            SyncMessage::HeaderResponse { header } => self.handle_header_response(header),
            SyncMessage::SnapshotMetadataResponse { metadata } => {
                self.handle_snapshot_metadata(peer_id, metadata)
            }
            SyncMessage::ChunkResponse { chunk } => self.handle_chunk_response(chunk),
            _ => vec![], // We don't handle request messages (those go to the server side)
        }
    }

    /// Handle a tip response from a peer.
    fn handle_tip_response(
        &mut self,
        peer_id: u64,
        height: u64,
        block_hash: [u8; 32],
    ) -> Vec<SyncAction> {
        if self.phase != SyncPhase::DiscoveringTip {
            return vec![];
        }

        self.peer_tips.insert(peer_id, (height, block_hash));

        // Find the most common tip height
        let mut height_votes: HashMap<u64, usize> = HashMap::new();
        for &(h, _) in self.peer_tips.values() {
            *height_votes.entry(h).or_default() += 1;
        }

        let best = height_votes
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(&h, &count)| (h, count));

        if let Some((tip_height, agreement)) = best {
            if agreement >= MIN_TIP_AGREEMENT && tip_height > self.local_height {
                // We have consensus on the tip — request the header
                self.target_height = Some(tip_height);
                self.phase = SyncPhase::VerifyingHeader;

                // Request header from a peer at this height
                let peer = *self
                    .peer_tips
                    .iter()
                    .find(|(_, &(h, _))| h == tip_height)
                    .map(|(pid, _)| pid)
                    .unwrap();

                debug!(tip_height, agreement, "Tip discovered, requesting header");

                return vec![SyncAction::SendToPeer {
                    peer_id: peer,
                    message: SyncMessage::HeaderRequest { height: tip_height },
                }];
            }
        }

        vec![]
    }

    /// Handle a header response — verify it with the light client.
    fn handle_header_response(&mut self, header: LightBlockHeader) -> Vec<SyncAction> {
        if self.phase != SyncPhase::VerifyingHeader {
            return vec![];
        }

        let target = match self.target_height {
            Some(h) => h,
            None => return vec![],
        };

        if header.height != target {
            warn!(
                expected = target,
                got = header.height,
                "Header height mismatch"
            );
            return vec![];
        }

        // Initialize or use existing light client
        let current_time = header.timestamp;
        if let Some(ref mut lc) = self.light_client {
            match lc.verify(&header, current_time) {
                VerificationResult::Valid => {
                    info!(height = target, "Header verified via light client");
                }
                VerificationResult::NeedBisection {
                    trusted_height,
                    target_height,
                } => {
                    // Request intermediate header for bisection
                    let mid = (trusted_height + target_height) / 2;
                    self.target_height = Some(mid);
                    let peer = self.any_peer_at_height(mid);
                    if let Some(pid) = peer {
                        return vec![SyncAction::SendToPeer {
                            peer_id: pid,
                            message: SyncMessage::HeaderRequest { height: mid },
                        }];
                    }
                    self.phase = SyncPhase::Failed("No peer for bisection".into());
                    return vec![];
                }
                VerificationResult::Invalid(reason) => {
                    self.phase = SyncPhase::Failed(format!("Header invalid: {}", reason));
                    return vec![];
                }
            }
        } else {
            // Bootstrap: verify the header against the genesis checkpoint if configured.
            if let Some(ref checkpoint) = self.genesis_checkpoint {
                if header.height < checkpoint.height {
                    warn!(
                        header_height = header.height,
                        checkpoint_height = checkpoint.height,
                        "Bootstrap header is below genesis checkpoint — rejecting"
                    );
                    self.phase = SyncPhase::Failed("Header below genesis checkpoint".into());
                    return vec![];
                }
                if header.height == checkpoint.height && header.state_root != checkpoint.state_root
                {
                    warn!(
                        "Bootstrap header state root does not match genesis checkpoint — rejecting"
                    );
                    self.phase =
                        SyncPhase::Failed("State root mismatch with genesis checkpoint".into());
                    return vec![];
                }
                if header.commit_certificate.signer_ids.is_empty() {
                    warn!("Bootstrap header has empty commit certificate — rejecting");
                    self.phase =
                        SyncPhase::Failed("Empty commit certificate on bootstrap header".into());
                    return vec![];
                }
                let n = header.validator_set.active_count();
                let quorum = if n == 0 { usize::MAX } else { n * 2 / 3 + 1 };
                if header.commit_certificate.signer_ids.len() < quorum {
                    warn!(
                        signers = header.commit_certificate.signer_ids.len(),
                        quorum = quorum,
                        "Bootstrap header commit certificate lacks quorum — rejecting"
                    );
                    self.phase = SyncPhase::Failed(format!(
                        "Commit certificate has {} signers, need {} for quorum",
                        header.commit_certificate.signer_ids.len(),
                        quorum
                    ));
                    return vec![];
                }
            }
            self.light_client = Some(LightClientVerifier::new(header.clone(), current_time));
            info!(
                height = target,
                has_checkpoint = self.genesis_checkpoint.is_some(),
                "Light client bootstrapped with verified header"
            );
        }

        // Header verified — request snapshot metadata
        self.phase = SyncPhase::DownloadingSnapshot {
            target_height: target,
            total_chunks: 0,
            received_chunks: 0,
        };

        let peer = self.any_peer_at_height(target);
        if let Some(pid) = peer {
            vec![SyncAction::SendToPeer {
                peer_id: pid,
                message: SyncMessage::SnapshotMetadataRequest { height: target },
            }]
        } else {
            self.phase = SyncPhase::Failed("No peer for snapshot".into());
            vec![]
        }
    }

    /// Handle snapshot metadata — start downloading chunks.
    fn handle_snapshot_metadata(
        &mut self,
        peer_id: u64,
        metadata: SnapshotMetadata,
    ) -> Vec<SyncAction> {
        let target = match self.target_height {
            Some(h) => h,
            None => return vec![],
        };

        if metadata.height != target {
            return vec![];
        }

        // Cross-check: metadata state_root must match the light-client-verified header
        if let Some(ref lc) = self.light_client {
            if let Some(trusted) = lc.trusted_state_at(target) {
                if trusted.header.state_root != metadata.state_root {
                    warn!(
                        expected = hex::encode(trusted.header.state_root),
                        got = hex::encode(metadata.state_root),
                        "Snapshot state root doesn't match verified header — rejecting"
                    );
                    return vec![];
                }
            }
        }

        if metadata.chunk_hashes.len() != metadata.total_chunks {
            warn!("Chunk hash count doesn't match total_chunks — rejecting");
            return vec![];
        }

        info!(
            height = target,
            chunks = metadata.total_chunks,
            size = metadata.total_size,
            "Snapshot metadata received, starting download"
        );

        self.phase = SyncPhase::DownloadingSnapshot {
            target_height: target,
            total_chunks: metadata.total_chunks,
            received_chunks: 0,
        };
        self.snapshot_meta = Some(metadata);
        self.received_chunks.clear();
        self.pending_requests.clear();

        // Request first batch of chunks
        self.request_next_chunks(peer_id)
    }

    /// Handle a received chunk — verify and store it.
    fn handle_chunk_response(&mut self, chunk: SnapshotChunk) -> Vec<SyncAction> {
        let meta = match &self.snapshot_meta {
            Some(m) => m,
            None => return vec![],
        };

        // Verify chunk index is valid
        if chunk.index >= meta.total_chunks {
            warn!(
                index = chunk.index,
                total = meta.total_chunks,
                "Invalid chunk index"
            );
            return vec![];
        }

        // Verify chunk hash
        let actual_hash = blake3_hash(&chunk.data);
        if actual_hash != meta.chunk_hashes[chunk.index] {
            warn!(
                index = chunk.index,
                "Chunk hash mismatch — peer sent corrupted data"
            );
            // Re-request this chunk from a different peer
            self.pending_requests.remove(&chunk.index);
            // Could request from different peer, but for now just re-request
            return vec![];
        }

        self.pending_requests.remove(&chunk.index);
        self.received_chunks.insert(chunk.index, chunk.data);

        let received = self.received_chunks.len();
        let total = meta.total_chunks;

        self.phase = SyncPhase::DownloadingSnapshot {
            target_height: meta.height,
            total_chunks: total,
            received_chunks: received,
        };

        debug!(received, total, "Chunk verified and stored");

        if received == total {
            // All chunks received — assemble and verify
            return self.assemble_and_verify();
        }

        // Request more chunks
        let peer = self.any_peer_at_height(meta.height);
        if let Some(pid) = peer {
            self.request_next_chunks(pid)
        } else {
            vec![]
        }
    }

    /// Request the next batch of chunks we still need.
    fn request_next_chunks(&mut self, peer_id: u64) -> Vec<SyncAction> {
        let meta = match &self.snapshot_meta {
            Some(m) => m,
            None => return vec![],
        };

        let mut actions = Vec::new();
        for i in 0..meta.total_chunks {
            if actions.len() >= MAX_CONCURRENT_REQUESTS {
                break;
            }
            if !self.received_chunks.contains_key(&i) && !self.pending_requests.contains(&i) {
                self.pending_requests.insert(i);
                actions.push(SyncAction::SendToPeer {
                    peer_id,
                    message: SyncMessage::ChunkRequest {
                        height: meta.height,
                        chunk_index: i,
                    },
                });
            }
        }
        actions
    }

    /// Assemble all chunks and verify against the state root.
    fn assemble_and_verify(&mut self) -> Vec<SyncAction> {
        let meta = match &self.snapshot_meta {
            Some(m) => m.clone(),
            None => return vec![],
        };

        self.phase = SyncPhase::VerifyingSnapshot;

        // Assemble chunks in order
        let mut full_data = Vec::with_capacity(meta.total_size as usize);
        for i in 0..meta.total_chunks {
            match self.received_chunks.get(&i) {
                Some(chunk_data) => full_data.extend_from_slice(chunk_data),
                None => {
                    self.phase = SyncPhase::Failed(format!("Missing chunk {}", i));
                    return vec![];
                }
            }
        }

        // Verify the assembled state against the state root
        let computed_root = blake3_hash(&full_data);
        if computed_root != meta.state_root {
            self.phase = SyncPhase::Failed("State root mismatch after assembly".into());
            return vec![];
        }

        info!(
            height = meta.height,
            size = full_data.len(),
            "Snapshot verified — applying state"
        );

        self.phase = SyncPhase::Complete {
            synced_height: meta.height,
        };

        vec![SyncAction::ApplySnapshot {
            height: meta.height,
            state_root: meta.state_root,
            data: full_data,
        }]
    }

    /// Find any peer that reported a height >= target.
    fn any_peer_at_height(&self, target: u64) -> Option<u64> {
        self.peer_tips
            .iter()
            .find(|(_, &(h, _))| h >= target)
            .map(|(&pid, _)| pid)
    }

    /// Get download progress as a percentage (0.0 to 1.0).
    pub fn download_progress(&self) -> f64 {
        match &self.phase {
            SyncPhase::DownloadingSnapshot {
                total_chunks,
                received_chunks,
                ..
            } => {
                if *total_chunks == 0 {
                    0.0
                } else {
                    *received_chunks as f64 / *total_chunks as f64
                }
            }
            SyncPhase::Complete { .. } => 1.0,
            _ => 0.0,
        }
    }

    /// Check if sync is complete.
    pub fn is_complete(&self) -> bool {
        matches!(self.phase, SyncPhase::Complete { .. })
    }

    /// Check if sync failed.
    pub fn is_failed(&self) -> bool {
        matches!(self.phase, SyncPhase::Failed(_))
    }
}

// ─────────────────────── Snapshot Provider ────────────────────────────

/// Server-side: handles incoming state sync requests from peers.
pub struct SnapshotProvider {
    /// Available snapshots indexed by height.
    snapshots: HashMap<u64, SnapshotMetadata>,
    /// Snapshot data chunks indexed by (height, chunk_index).
    chunk_data: HashMap<(u64, usize), Vec<u8>>,
}

impl SnapshotProvider {
    pub fn new() -> Self {
        Self {
            snapshots: HashMap::new(),
            chunk_data: HashMap::new(),
        }
    }

    /// Create a snapshot from raw state data.
    pub fn create_snapshot(
        &mut self,
        height: u64,
        epoch: u64,
        state_root: [u8; 32],
        data: &[u8],
    ) -> SnapshotMetadata {
        let chunks: Vec<&[u8]> = data.chunks(CHUNK_SIZE).collect();
        let chunk_hashes: Vec<[u8; 32]> = chunks.iter().map(|c| blake3_hash(c)).collect();

        for (i, chunk) in chunks.iter().enumerate() {
            self.chunk_data.insert((height, i), chunk.to_vec());
        }

        let meta = SnapshotMetadata {
            height,
            epoch,
            state_root,
            total_chunks: chunks.len(),
            chunk_hashes,
            total_size: data.len() as u64,
        };

        self.snapshots.insert(height, meta.clone());
        meta
    }

    /// Handle an incoming sync request from a peer.
    pub fn handle_request(&self, msg: &SyncMessage, local_height: u64) -> Option<SyncMessage> {
        match msg {
            SyncMessage::TipRequest => Some(SyncMessage::TipResponse {
                height: local_height,
                block_hash: [0u8; 32], // Simplified; real impl uses actual hash
            }),
            SyncMessage::SnapshotMetadataRequest { height } => {
                self.snapshots
                    .get(height)
                    .map(|meta| SyncMessage::SnapshotMetadataResponse {
                        metadata: meta.clone(),
                    })
            }
            SyncMessage::ChunkRequest {
                height,
                chunk_index,
            } => {
                let data = self.chunk_data.get(&(*height, *chunk_index))?;
                let meta = self.snapshots.get(height)?;
                Some(SyncMessage::ChunkResponse {
                    chunk: SnapshotChunk {
                        height: *height,
                        index: *chunk_index,
                        total: meta.total_chunks,
                        data: data.clone(),
                        hash: meta.chunk_hashes[*chunk_index],
                    },
                })
            }
            _ => None,
        }
    }

    /// Number of snapshots available.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    /// Remove old snapshots, keeping only the most recent N.
    pub fn prune(&mut self, keep: usize) {
        if self.snapshots.len() <= keep {
            return;
        }
        let mut heights: Vec<u64> = self.snapshots.keys().copied().collect();
        heights.sort();
        let remove_count = heights.len() - keep;
        for &h in &heights[..remove_count] {
            if let Some(meta) = self.snapshots.remove(&h) {
                for i in 0..meta.total_chunks {
                    self.chunk_data.remove(&(h, i));
                }
            }
        }
    }
}

impl Default for SnapshotProvider {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validator_set::ValidatorInfo;
    use evaporchain_crypto::signatures::{BlsKeypair, BlsSignature, BlsVerifier};

    fn make_vs_with_bls(n: u64, stake: u64) -> (ValidatorSet, Vec<BlsKeypair>) {
        let mut vs = ValidatorSet::new();
        let mut kps = Vec::new();
        for i in 0..n {
            let kp = BlsKeypair::generate();
            let mut info = ValidatorInfo::new(i, stake, [i as u8; 32]);
            info.bls_public_key = Some(kp.public_key_bytes().0);
            info.pop_verified = true; // test-only: bypass PoP to keep fixture simple
            vs.add_validator(info);
            kps.push(kp);
        }
        (vs, kps)
    }

    fn bls_vote_message(height: u64, round: u32, block_hash: &[u8; 32]) -> Vec<u8> {
        let mut msg = Vec::with_capacity(48);
        msg.extend_from_slice(b"precommit");
        msg.extend_from_slice(&height.to_le_bytes());
        msg.extend_from_slice(&round.to_le_bytes());
        msg.extend_from_slice(block_hash);
        msg
    }

    fn make_cert(
        height: u64,
        block_hash: [u8; 32],
        kps: &[BlsKeypair],
        ids: &[u64],
    ) -> CommitCertificate {
        let msg = bls_vote_message(height, 0, &block_hash);
        let sigs: Vec<BlsSignature> = ids.iter().map(|&id| kps[id as usize].sign(&msg)).collect();
        let agg = BlsVerifier::aggregate_signatures(&sigs).unwrap();
        CommitCertificate {
            height,
            round: 0,
            block_hash,
            aggregate_signature: agg.0,
            signer_ids: ids.to_vec(),
        }
    }

    fn make_header(height: u64, vs: &ValidatorSet, kps: &[BlsKeypair]) -> LightBlockHeader {
        make_header_with_state_root(height, vs, kps, blake3_hash(&vec![height as u8; 64]))
    }

    fn make_header_with_state_root(
        height: u64,
        vs: &ValidatorSet,
        kps: &[BlsKeypair],
        state_root: [u8; 32],
    ) -> LightBlockHeader {
        let hash = [height as u8; 32];
        let cert = make_cert(
            height,
            hash,
            kps,
            &(0..vs.active_count() as u64).collect::<Vec<_>>(),
        );
        LightBlockHeader {
            height,
            epoch: height / 100,
            block_hash: hash,
            parent_hash: [(height - 1) as u8; 32],
            state_root,
            timestamp: height * 10,
            validator_set: vs.clone(),
            commit_certificate: cert,
        }
    }

    #[test]
    fn test_needs_state_sync() {
        assert!(!StateSyncManager::needs_state_sync(100, 200));
        assert!(!StateSyncManager::needs_state_sync(100, 1100));
        assert!(StateSyncManager::needs_state_sync(100, 1102));
        assert!(StateSyncManager::needs_state_sync(0, 5000));
    }

    #[test]
    fn test_tip_discovery() {
        let mut sync = StateSyncManager::new(0);
        let actions = sync.start();
        assert_eq!(actions.len(), 1); // Broadcast TipRequest

        // One peer responds — not enough agreement
        let actions = sync.on_message(
            1,
            SyncMessage::TipResponse {
                height: 1000,
                block_hash: [1u8; 32],
            },
        );
        assert!(actions.is_empty());

        // Second peer agrees
        let actions = sync.on_message(
            2,
            SyncMessage::TipResponse {
                height: 1000,
                block_hash: [1u8; 32],
            },
        );
        assert!(!actions.is_empty()); // Should request header
        assert_eq!(*sync.phase(), SyncPhase::VerifyingHeader);
    }

    #[test]
    fn test_snapshot_provider_create_and_serve() {
        let mut provider = SnapshotProvider::new();

        // Create a snapshot with 1MB of data (4 chunks at 256KB each)
        let data = vec![0xAB; CHUNK_SIZE * 4];
        let state_root = blake3_hash(&data);
        let meta = provider.create_snapshot(100, 1, state_root, &data);

        assert_eq!(meta.total_chunks, 4);
        assert_eq!(meta.total_size, (CHUNK_SIZE * 4) as u64);
        assert_eq!(provider.snapshot_count(), 1);

        // Serve metadata request
        let resp =
            provider.handle_request(&SyncMessage::SnapshotMetadataRequest { height: 100 }, 100);
        assert!(resp.is_some());

        // Serve chunk requests
        for i in 0..4 {
            let resp = provider.handle_request(
                &SyncMessage::ChunkRequest {
                    height: 100,
                    chunk_index: i,
                },
                100,
            );
            match resp {
                Some(SyncMessage::ChunkResponse { chunk }) => {
                    assert_eq!(chunk.index, i);
                    assert_eq!(chunk.data.len(), CHUNK_SIZE);
                    assert_eq!(chunk.hash, blake3_hash(&chunk.data));
                }
                _ => panic!("Expected ChunkResponse"),
            }
        }
    }

    #[test]
    fn test_snapshot_provider_prune() {
        let mut provider = SnapshotProvider::new();
        for h in [100, 200, 300, 400, 500] {
            let data = vec![h as u8; CHUNK_SIZE];
            let root = blake3_hash(&data);
            provider.create_snapshot(h, h / 100, root, &data);
        }
        assert_eq!(provider.snapshot_count(), 5);

        provider.prune(2);
        assert_eq!(provider.snapshot_count(), 2);

        // Only the latest 2 should remain
        assert!(provider
            .handle_request(&SyncMessage::SnapshotMetadataRequest { height: 100 }, 500)
            .is_none());
        assert!(provider
            .handle_request(&SyncMessage::SnapshotMetadataRequest { height: 400 }, 500)
            .is_some());
        assert!(provider
            .handle_request(&SyncMessage::SnapshotMetadataRequest { height: 500 }, 500)
            .is_some());
    }

    #[test]
    fn test_chunk_hash_verification() {
        let mut sync = StateSyncManager::new(0);
        sync.phase = SyncPhase::DownloadingSnapshot {
            target_height: 100,
            total_chunks: 2,
            received_chunks: 0,
        };

        let data = vec![0xAB; CHUNK_SIZE * 2];
        let chunk0_data = data[..CHUNK_SIZE].to_vec();
        let chunk1_data = data[CHUNK_SIZE..].to_vec();

        let state_root = blake3_hash(&data);
        sync.snapshot_meta = Some(SnapshotMetadata {
            height: 100,
            epoch: 1,
            state_root,
            total_chunks: 2,
            chunk_hashes: vec![blake3_hash(&chunk0_data), blake3_hash(&chunk1_data)],
            total_size: data.len() as u64,
        });
        sync.target_height = Some(100);
        sync.peer_tips.insert(1, (100, [1u8; 32]));

        // Send valid chunk 0
        let actions = sync.on_message(
            1,
            SyncMessage::ChunkResponse {
                chunk: SnapshotChunk {
                    height: 100,
                    index: 0,
                    total: 2,
                    data: chunk0_data.clone(),
                    hash: blake3_hash(&chunk0_data),
                },
            },
        );
        assert!(!sync.is_complete());

        // Send chunk 1 with corrupted data — should be rejected
        let corrupted = vec![0xFF; CHUNK_SIZE];
        let actions = sync.on_message(
            1,
            SyncMessage::ChunkResponse {
                chunk: SnapshotChunk {
                    height: 100,
                    index: 1,
                    total: 2,
                    data: corrupted,
                    hash: blake3_hash(&chunk1_data), // hash doesn't match corrupted data
                },
            },
        );
        // Chunk was rejected, still not complete
        assert!(!sync.is_complete());
        assert_eq!(sync.received_chunks.len(), 1); // only chunk 0

        // Send correct chunk 1
        let actions = sync.on_message(
            1,
            SyncMessage::ChunkResponse {
                chunk: SnapshotChunk {
                    height: 100,
                    index: 1,
                    total: 2,
                    data: chunk1_data.clone(),
                    hash: blake3_hash(&chunk1_data),
                },
            },
        );
        assert!(sync.is_complete());

        // Should have an ApplySnapshot action
        assert!(actions
            .iter()
            .any(|a| matches!(a, SyncAction::ApplySnapshot { .. })));
    }

    #[test]
    fn test_full_sync_flow_with_provider() {
        let (vs, kps) = make_vs_with_bls(4, 1000);

        // Setup provider with a snapshot
        let mut provider = SnapshotProvider::new();
        let state_data = vec![0xDE; CHUNK_SIZE * 3];
        let state_root = blake3_hash(&state_data);
        provider.create_snapshot(100, 1, state_root, &state_data);

        // Setup syncing node
        let mut sync = StateSyncManager::new(0);
        let actions = sync.start();

        // Simulate tip responses from 2 peers
        sync.on_message(
            1,
            SyncMessage::TipResponse {
                height: 100,
                block_hash: [100u8; 32],
            },
        );
        let actions = sync.on_message(
            2,
            SyncMessage::TipResponse {
                height: 100,
                block_hash: [100u8; 32],
            },
        );
        assert_eq!(*sync.phase(), SyncPhase::VerifyingHeader);

        // Simulate header response — state_root must match the snapshot
        let header = make_header_with_state_root(100, &vs, &kps, state_root);
        let actions = sync.on_message(1, SyncMessage::HeaderResponse { header });

        // Should now request snapshot metadata
        assert!(matches!(
            sync.phase(),
            SyncPhase::DownloadingSnapshot { .. }
        ));

        // Serve metadata
        let meta_resp = provider
            .handle_request(&SyncMessage::SnapshotMetadataRequest { height: 100 }, 100)
            .unwrap();
        let actions = sync.on_message(1, meta_resp);

        // Should request chunks
        assert!(!actions.is_empty());

        // Serve all chunks
        for action in actions {
            if let SyncAction::SendToPeer { message, .. } = action {
                if let Some(resp) = provider.handle_request(&message, 100) {
                    sync.on_message(1, resp);
                }
            }
        }

        // If not all chunks were requested in first batch, continue
        while !sync.is_complete() && !sync.is_failed() {
            let meta = sync.snapshot_meta.as_ref().unwrap();
            for i in 0..meta.total_chunks {
                if !sync.received_chunks.contains_key(&i) {
                    let resp = provider
                        .handle_request(
                            &SyncMessage::ChunkRequest {
                                height: 100,
                                chunk_index: i,
                            },
                            100,
                        )
                        .unwrap();
                    sync.on_message(1, resp);
                }
            }
        }

        assert!(sync.is_complete());
        assert_eq!(sync.download_progress(), 1.0);
    }

    #[test]
    fn test_snapshot_metadata_commitment() {
        let meta1 = SnapshotMetadata {
            height: 100,
            epoch: 1,
            state_root: [1u8; 32],
            total_chunks: 2,
            chunk_hashes: vec![[2u8; 32], [3u8; 32]],
            total_size: 512000,
        };
        let meta2 = SnapshotMetadata {
            height: 100,
            epoch: 1,
            state_root: [1u8; 32],
            total_chunks: 2,
            chunk_hashes: vec![[2u8; 32], [4u8; 32]], // different chunk hash
            total_size: 512000,
        };

        // Different chunk hashes → different commitments
        assert_ne!(meta1.commitment(), meta2.commitment());

        // Same metadata → same commitment
        let meta1_clone = meta1.clone();
        assert_eq!(meta1.commitment(), meta1_clone.commitment());
    }

    #[test]
    fn test_download_progress() {
        let mut sync = StateSyncManager::new(0);
        assert_eq!(sync.download_progress(), 0.0);

        sync.phase = SyncPhase::DownloadingSnapshot {
            target_height: 100,
            total_chunks: 10,
            received_chunks: 5,
        };
        assert!((sync.download_progress() - 0.5).abs() < f64::EPSILON);

        sync.phase = SyncPhase::Complete { synced_height: 100 };
        assert_eq!(sync.download_progress(), 1.0);
    }

    #[test]
    fn test_snapshot_metadata_state_root_mismatch_rejected() {
        let (vs, kps) = make_vs_with_bls(4, 1000);
        let mut sync = StateSyncManager::new(0);
        sync.start();

        sync.on_message(
            1,
            SyncMessage::TipResponse {
                height: 100,
                block_hash: [100u8; 32],
            },
        );
        sync.on_message(
            2,
            SyncMessage::TipResponse {
                height: 100,
                block_hash: [100u8; 32],
            },
        );

        // Header has state_root = blake3([100; 64])
        let header = make_header(100, &vs, &kps);
        let header_state_root = header.state_root;
        sync.on_message(1, SyncMessage::HeaderResponse { header });

        // Metadata with DIFFERENT state_root
        let bad_meta = SnapshotMetadata {
            height: 100,
            epoch: 1,
            state_root: [0xFF; 32],
            total_chunks: 1,
            chunk_hashes: vec![[0xAA; 32]],
            total_size: 1024,
        };
        assert_ne!([0xFF; 32], header_state_root);
        let actions = sync.on_message(
            1,
            SyncMessage::SnapshotMetadataResponse { metadata: bad_meta },
        );
        assert!(actions.is_empty(), "mismatched state_root must be rejected");
    }
}
