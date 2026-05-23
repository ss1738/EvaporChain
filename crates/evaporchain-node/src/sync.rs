//! State sync integration — bridges consensus state_sync, state snapshots,
//! and node persistence for fast-syncing new/recovering nodes.
//!
//! Two roles:
//! - **Server**: serves state snapshots to syncing peers via SnapshotProvider
//! - **Client**: restores state from local snapshots on startup, or syncs
//!   from peers when too far behind (via StateSyncManager)

use evaporchain_consensus::state_sync::{
    SnapshotProvider, StateSyncManager, SyncAction, SyncMessage, SyncPhase,
};
use evaporchain_state::db::StateDB;
use evaporchain_state::snapshot::{deserialize_snapshot, SnapshotApplier};

use crate::persistence::ChainStore;

/// Server-side sync service: serves snapshots to peers.
pub struct SyncServer {
    provider: SnapshotProvider,
    local_height: u64,
    /// Canonical hash of the block at `local_height` — populated via
    /// [`Self::set_tip`] after each block commit so that `TipResponse`
    /// returns the real hash instead of `[0u8; 32]`. AUDIT_2026_05_06
    /// H-21: peers depended on this to verify a responder's tip claim;
    /// the placeholder zero made tip-discovery cryptographically blind.
    local_block_hash: [u8; 32],
}

impl SyncServer {
    pub fn new() -> Self {
        Self {
            provider: SnapshotProvider::new(),
            local_height: 0,
            local_block_hash: [0u8; 32],
        }
    }

    /// Bootstrap the provider from the most recent persisted snapshot.
    pub fn load_from_store(&mut self, chain_store: &ChainStore) {
        if let Some((height, state_root, data)) = chain_store.load_latest_snapshot() {
            self.provider
                .create_snapshot(height, height / 10, state_root, &data);
            self.local_height = height;
            tracing::info!(
                height,
                size = data.len(),
                "Sync server loaded snapshot from disk"
            );
        }
    }

    /// Register a freshly created snapshot (called every 100 blocks).
    pub fn register_snapshot(
        &mut self,
        height: u64,
        epoch: u64,
        state_root: [u8; 32],
        data: &[u8],
    ) {
        self.provider
            .create_snapshot(height, epoch, state_root, data);
        self.local_height = height;
        self.provider.prune(3);
    }

    /// Handle an incoming sync request from a peer.
    pub fn handle_request(&self, msg: &SyncMessage) -> Option<SyncMessage> {
        let kind = match msg {
            SyncMessage::TipRequest => "TipRequest",
            SyncMessage::HeaderRequest { .. } => "HeaderRequest",
            SyncMessage::SnapshotMetadataRequest { .. } => "SnapshotMetadataRequest",
            SyncMessage::ChunkRequest { .. } => "ChunkRequest",
            _ => "non-request",
        };
        let resp = self
            .provider
            .handle_request(msg, self.local_height, self.local_block_hash);
        let resp_kind = match &resp {
            Some(SyncMessage::TipResponse { .. }) => "TipResponse",
            Some(SyncMessage::HeaderResponse { .. }) => "HeaderResponse",
            Some(SyncMessage::SnapshotMetadataResponse { .. }) => "SnapshotMetadataResponse",
            Some(SyncMessage::ChunkResponse { .. }) => "ChunkResponse",
            Some(_) => "other",
            None => "no-response",
        };
        tracing::info!(
            req = kind,
            resp = resp_kind,
            local_height = self.local_height,
            snapshot_count = self.provider.snapshot_count(),
            "STATE-SYNC server handle_request"
        );
        resp
    }

    /// Update tip height alone — leaves `local_block_hash` at its prior
    /// value. Use this on bootstrap (`load_from_store`) where the hash
    /// is not yet known; production-path callers committing a real block
    /// should use [`Self::set_tip`] instead so the TipResponse carries
    /// the real hash.
    pub fn set_height(&mut self, height: u64) {
        self.local_height = height;
    }

    /// Update both height and canonical block hash atomically. Called
    /// after each block commit with the value of
    /// `TendermintConsensus::block_hash(&block)` so peers asking
    /// "what's your tip?" get a verifiable answer instead of the
    /// `[0u8; 32]` placeholder.
    pub fn set_tip(&mut self, height: u64, block_hash: [u8; 32]) {
        self.local_height = height;
        self.local_block_hash = block_hash;
    }

    pub fn snapshot_count(&self) -> usize {
        self.provider.snapshot_count()
    }
}

/// Client-side sync: manages state download from peers.
pub struct SyncClient {
    manager: StateSyncManager,
}

impl SyncClient {
    pub fn new(local_height: u64) -> Self {
        Self {
            manager: StateSyncManager::new(local_height),
        }
    }

    pub fn needs_sync(local_height: u64, network_height: u64) -> bool {
        StateSyncManager::needs_state_sync(local_height, network_height)
    }

    pub fn start(&mut self) -> Vec<SyncAction> {
        self.manager.start()
    }

    pub fn on_message(&mut self, peer_id: u64, msg: SyncMessage) -> Vec<SyncAction> {
        self.manager.on_message(peer_id, msg)
    }

    pub fn phase(&self) -> &SyncPhase {
        self.manager.phase()
    }

    pub fn is_complete(&self) -> bool {
        self.manager.is_complete()
    }

    pub fn progress(&self) -> f64 {
        self.manager.download_progress()
    }
}

/// Try to restore state from a locally persisted snapshot.
/// Used on startup when the state DB is empty but a snapshot exists in
/// the chain store (e.g., from a previous run or a received sync).
///
/// Returns `(height, epoch, state_root)` on success.
pub fn try_restore_from_snapshot(
    db: &mut dyn StateDB,
    chain_store: &ChainStore,
    node_tag: &str,
) -> Option<(u64, u64, [u8; 32])> {
    let (height, state_root, data) = chain_store.load_latest_snapshot()?;

    eprintln!(
        "{} \x1b[1;33mFound snapshot at height {} ({} bytes), restoring...\x1b[0m",
        node_tag,
        height,
        data.len()
    );

    let snapshot = match deserialize_snapshot(&data) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "{} \x1b[31mFailed to deserialize snapshot: {}\x1b[0m",
                node_tag, e
            );
            return None;
        }
    };

    if snapshot.header.state_root != state_root {
        eprintln!(
            "{} \x1b[31mSnapshot state root mismatch with stored metadata\x1b[0m",
            node_tag
        );
        return None;
    }

    match SnapshotApplier::apply(db, &snapshot) {
        Ok(result) => {
            eprintln!(
                "{} \x1b[1;32mState restored from snapshot\x1b[0m — height={}, {} accounts, {} objects, {} ghosts ({}ms)",
                node_tag,
                snapshot.header.block_height,
                result.accounts_restored,
                result.objects_restored,
                result.ghosts_restored,
                result.elapsed_ms,
            );
            Some((
                snapshot.header.block_height,
                snapshot.header.epoch,
                result.state_root,
            ))
        }
        Err(e) => {
            eprintln!(
                "{} \x1b[31mFailed to apply snapshot: {}\x1b[0m",
                node_tag, e
            );
            None
        }
    }
}

/// Apply a snapshot received via state sync (from ApplySnapshot action).
/// Deserializes the raw bytes and applies to the state database.
pub fn apply_sync_snapshot(
    db: &mut dyn StateDB,
    data: &[u8],
    expected_root: [u8; 32],
) -> Result<(u64, u64), String> {
    let snapshot = deserialize_snapshot(data).map_err(|e| format!("deserialize: {}", e))?;

    if snapshot.header.state_root != expected_root {
        return Err(format!(
            "state root mismatch: expected {}, got {}",
            hex::encode(expected_root),
            hex::encode(snapshot.header.state_root)
        ));
    }

    let result = SnapshotApplier::apply(db, &snapshot).map_err(|e| format!("apply: {}", e))?;

    tracing::info!(
        height = snapshot.header.block_height,
        accounts = result.accounts_restored,
        objects = result.objects_restored,
        elapsed_ms = result.elapsed_ms,
        "Sync snapshot applied"
    );

    Ok((snapshot.header.block_height, snapshot.header.epoch))
}

// ─────────────── Fast-sync from peer snapshot endpoint ───────────────────
//
// Bootstrap a brand-new node by downloading the most recent
// SnapshotFile blob from a single peer's
// `/api/snapshot/{latest,download/:height}`, verifying its integrity
// hash, and applying it to the local state DB. Tendermint normal-sync
// then advances from `block_height + 1` onwards — i.e. the snapshot
// short-circuits replay of every block from genesis.
//
// Trust model: same as weak-subjectivity / state-sync today. The peer
// is trusted to serve a finalized snapshot for the chain
// (`chain_id` mismatch is a hard reject). The integrity_hash check
// catches any corruption in flight.

/// Outcome of a successful fast-sync. The caller restores Tendermint
/// state from these fields and then engages normal-sync from
/// `block_height + 1` onwards.
#[derive(Debug, Clone)]
pub struct FastSyncOutcome {
    pub chain_id: String,
    pub block_height: u64,
    pub epoch: u64,
    pub state_root: [u8; 32],
    pub parent_hash: [u8; 32],
    pub bell_reading: Option<evaporchain_state::SnapshotBellReading>,
    pub validator_set: evaporchain_state::ValidatorSetSnapshot,
}

/// Fetch + verify a SnapshotFile from `peer_url` (no state-DB
/// mutation). Returns the parsed `SnapshotFile`; caller is responsible
/// for `.apply_to(&mut db)` while holding the DB lock. Splitting the
/// fetch/verify and apply phases keeps the std::sync DB Mutex out of
/// the await chain.
pub async fn fetch_snapshot_blob_from_peer(
    peer_url: &str,
    expected_chain_id: &str,
) -> Result<evaporchain_state::SnapshotFile, String> {
    let peer = peer_url.trim_end_matches('/');

    // 1. Discover latest snapshot height + verify chain_id.
    let latest_url = format!("{}/api/snapshot/latest", peer);
    let latest = reqwest::get(&latest_url)
        .await
        .map_err(|e| format!("GET {}: {}", latest_url, e))?;
    if !latest.status().is_success() {
        return Err(format!("GET {} returned {}", latest_url, latest.status()));
    }
    let latest_json: serde_json::Value = latest
        .json()
        .await
        .map_err(|e| format!("parse {}: {}", latest_url, e))?;
    if !latest_json
        .get("available")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err(format!("peer {} reports no snapshot available", peer));
    }
    let height = latest_json
        .get("block_height")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "missing block_height in /api/snapshot/latest".to_string())?;
    let peer_chain = latest_json
        .get("chain_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if peer_chain != expected_chain_id {
        return Err(format!(
            "chain_id mismatch: peer={}, local={}",
            peer_chain, expected_chain_id
        ));
    }

    // 2. Download the blob.
    let dl_url = format!("{}/api/snapshot/download/{}", peer, height);
    let dl = reqwest::get(&dl_url)
        .await
        .map_err(|e| format!("GET {}: {}", dl_url, e))?;
    if !dl.status().is_success() {
        return Err(format!("GET {} returned {}", dl_url, dl.status()));
    }
    let bytes = dl
        .bytes()
        .await
        .map_err(|e| format!("read body {}: {}", dl_url, e))?;

    // 3. Verify integrity hash + quorum certificate + double-check
    //    chain_id from the blob itself (the metadata endpoint is
    //    informational only).
    //
    // Audit NET-1 (2026-05-18): fast-sync is a TRUST-BOOTSTRAP hot path.
    // The lenient `from_bytes` accepts any self-consistent blob (the
    // integrity hash is attacker-recomputable), so a single malicious
    // `--fast-sync` peer could seed a forged validator-set/balances.
    // `from_bytes_strict` additionally requires a valid 2f+1 quorum
    // certificate over the snapshot (T0.8 sub-task 2). `from_bytes`
    // stays for tooling / pre-cert legacy snapshots only, NOT this path.
    let file = evaporchain_state::SnapshotFile::from_bytes_strict(&bytes)
        .map_err(|e| format!("verify: {}", e))?;
    if file.chain_id != expected_chain_id {
        return Err(format!(
            "blob chain_id mismatch: blob={}, local={}",
            file.chain_id, expected_chain_id
        ));
    }

    Ok(file)
}

/// Convenience wrapper: fetch + verify + apply in one call. Used in
/// integration tests and tooling — production node startup splits the
/// phases (see `main.rs` fast-sync block) to keep the DB Mutex out of
/// the .await chain.
pub async fn fast_sync_from_peer(
    db: &mut dyn StateDB,
    peer_url: &str,
    expected_chain_id: &str,
) -> Result<FastSyncOutcome, String> {
    let file = fetch_snapshot_blob_from_peer(peer_url, expected_chain_id).await?;
    let _ = file.apply_to(db).map_err(|e| format!("apply: {}", e))?;
    Ok(FastSyncOutcome {
        chain_id: file.chain_id.clone(),
        block_height: file.block_height,
        epoch: file.epoch,
        state_root: file.state_root,
        parent_hash: file.parent_hash,
        bell_reading: file.bell_reading.clone(),
        validator_set: file.validator_set.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_consensus::state_sync::SyncMessage;
    use evaporchain_state::InMemoryStateDB;

    #[test]
    fn test_sync_server_basic() {
        let mut server = SyncServer::new();
        let data = vec![1u8; 1024];
        let root = evaporchain_crypto::hash::blake3_hash(&data);
        server.register_snapshot(100, 10, root, &data);
        assert_eq!(server.snapshot_count(), 1);
        server.set_height(100);

        let tip = server.handle_request(&SyncMessage::TipRequest);
        assert!(tip.is_some());
        if let Some(SyncMessage::TipResponse { height, .. }) = tip {
            assert_eq!(height, 100);
        }

        let meta = server.handle_request(&SyncMessage::SnapshotMetadataRequest { height: 100 });
        assert!(meta.is_some());
    }

    /// H-21 close: `set_tip` plumbs the canonical block hash through to
    /// `TipResponse.block_hash`. Pre-fix, the server returned the
    /// `[0u8; 32]` placeholder regardless of local state, leaving peer
    /// tip-claim verification cryptographically blind. This test pins
    /// the fix.
    #[test]
    fn set_tip_propagates_block_hash_to_tip_response() {
        let mut server = SyncServer::new();
        let canonical_hash: [u8; 32] = [0xAB; 32];
        server.set_tip(42, canonical_hash);

        let resp = server.handle_request(&SyncMessage::TipRequest);
        match resp {
            Some(SyncMessage::TipResponse { height, block_hash }) => {
                assert_eq!(height, 42, "height must mirror set_tip arg");
                assert_eq!(
                    block_hash, canonical_hash,
                    "block_hash must mirror set_tip arg, NOT the [0u8; 32] placeholder"
                );
            }
            other => panic!("expected TipResponse; got {other:?}"),
        }
    }

    /// Default state (no `set_tip` called) returns zero hash — backward-
    /// compatible with the pre-fix behaviour. `set_height` is documented
    /// as not touching the hash; this test pins that contract.
    #[test]
    fn set_height_alone_leaves_block_hash_zero() {
        let mut server = SyncServer::new();
        server.set_height(7);
        let resp = server.handle_request(&SyncMessage::TipRequest);
        match resp {
            Some(SyncMessage::TipResponse { height, block_hash }) => {
                assert_eq!(height, 7);
                assert_eq!(
                    block_hash, [0u8; 32],
                    "set_height must NOT touch block_hash — that's set_tip's job"
                );
            }
            other => panic!("expected TipResponse; got {other:?}"),
        }
    }

    #[test]
    fn test_sync_server_prunes_old() {
        let mut server = SyncServer::new();
        for i in 1..=5u64 {
            let data = vec![i as u8; 512];
            let root = evaporchain_crypto::hash::blake3_hash(&data);
            server.register_snapshot(i * 100, i * 10, root, &data);
        }
        assert_eq!(server.snapshot_count(), 3);
    }

    #[test]
    fn test_sync_server_chunk_request() {
        let mut server = SyncServer::new();
        let data = vec![42u8; 1024];
        let root = evaporchain_crypto::hash::blake3_hash(&data);
        server.register_snapshot(200, 20, root, &data);

        let chunk_resp = server.handle_request(&SyncMessage::ChunkRequest {
            height: 200,
            chunk_index: 0,
        });
        assert!(chunk_resp.is_some());
        if let Some(SyncMessage::ChunkResponse { chunk }) = chunk_resp {
            assert_eq!(chunk.height, 200);
            assert_eq!(chunk.index, 0);
            assert_eq!(chunk.data, data);
        }
    }

    #[test]
    fn test_sync_client_needs_sync() {
        // SyncClient::needs_sync delegates to StateSyncManager::needs_state_sync,
        // whose threshold was raised from 1000 → 50_000 in `b063b0b`. Pre-bump
        // these assertions used 2000 / 1500 / 1000 to straddle the boundary.
        // Post-bump we use 100_000 / 1500 / 1000 to preserve the same shape:
        // way-behind / close / equal.
        assert!(SyncClient::needs_sync(0, 100_000));
        assert!(!SyncClient::needs_sync(1000, 1500));
        assert!(!SyncClient::needs_sync(1000, 1000));
    }

    #[test]
    fn test_apply_sync_snapshot_wrong_root() {
        let mut db = InMemoryStateDB::new();
        let data = vec![0u8; 100];
        let wrong_root = [0xFFu8; 32];
        let result = apply_sync_snapshot(&mut db, &data, wrong_root);
        assert!(result.is_err());
    }
}
