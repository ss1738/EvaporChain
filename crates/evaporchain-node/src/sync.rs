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
}

impl SyncServer {
    pub fn new() -> Self {
        Self {
            provider: SnapshotProvider::new(),
            local_height: 0,
        }
    }

    /// Bootstrap the provider from the most recent persisted snapshot.
    pub fn load_from_store(&mut self, chain_store: &ChainStore) {
        if let Some((height, state_root, data)) = chain_store.load_latest_snapshot() {
            self.provider.create_snapshot(height, height / 10, state_root, &data);
            self.local_height = height;
            tracing::info!(height, size = data.len(), "Sync server loaded snapshot from disk");
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
        self.provider.handle_request(msg, self.local_height)
    }

    pub fn set_height(&mut self, height: u64) {
        self.local_height = height;
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

    let result =
        SnapshotApplier::apply(db, &snapshot).map_err(|e| format!("apply: {}", e))?;

    tracing::info!(
        height = snapshot.header.block_height,
        accounts = result.accounts_restored,
        objects = result.objects_restored,
        elapsed_ms = result.elapsed_ms,
        "Sync snapshot applied"
    );

    Ok((snapshot.header.block_height, snapshot.header.epoch))
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

        let meta = server.handle_request(&SyncMessage::SnapshotMetadataRequest {
            height: 100,
        });
        assert!(meta.is_some());
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
        assert!(SyncClient::needs_sync(0, 2000));
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
