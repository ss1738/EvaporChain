use async_trait::async_trait;
use evaporchain_types::{Block, Transaction};
use thiserror::Error;

pub mod banlist;
pub mod service;
pub mod tls;

pub use banlist::{now_ms, BanEntry, BanList};
pub use libp2p::PeerId;
pub use service::{
    cache_da_package, load_or_generate_identity, subnet_key, NetworkConfig, P2pNetworkService,
    PeerInfo, PeerScore, RejectionReason, ShardCache, SybilState,
};
pub use tls::{PeerAuthority, TlsConfig};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "evaporchain-network's BanList is the chain's
    /// Sybil-resistance backstop. Bans are keyed by source IP (peer
    /// IDs are free; IPs are not). Adding a longer expiry for an
    /// existing ban extends it; manual unban succeeds; expired bans
    /// auto-prune on `is_banned` so the file cannot accumulate."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let mut bl = BanList::new();
        let ip: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let now = now_ms();

        // Fresh ban → is_banned returns true.
        bl.add_ban(ip, now + 60_000, "score_threshold_breach");
        assert!(bl.is_banned(&ip));

        // Re-adding a SHORTER expiry must NOT shorten the ban.
        bl.add_ban(ip, now + 1_000, "manual");
        let active = bl.active_bans();
        assert!(active.iter().any(|e| e.ip == ip && e.until_ms >= now + 60_000));

        // Re-adding a LONGER expiry extends the ban.
        bl.add_ban(ip, now + 120_000, "extended");
        let active2 = bl.active_bans();
        assert!(active2.iter().any(|e| e.ip == ip && e.until_ms >= now + 120_000));

        // Manual remove drops the entry.
        assert!(bl.remove_ban(&ip));
        assert!(!bl.is_banned(&ip));
        assert!(!bl.remove_ban(&ip), "second remove must report no-op");

        // Already-expired ban auto-prunes on is_banned (now-1).
        let stale: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 99));
        bl.add_ban(stale, now.saturating_sub(1), "stale");
        assert!(!bl.is_banned(&stale), "expired ban must not register");
        // After lookup, entry is pruned from the map.
        assert!(bl.active_bans().iter().all(|e| e.ip != stale));
    }
}

/// Errors that can occur in the network layer.
#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("broadcast failed: {0}")]
    BroadcastFailed(String),
    #[error("connection error: {0}")]
    ConnectionError(String),
}

/// Trait for P2P network services.
#[async_trait]
pub trait NetworkService: Send + Sync {
    /// Broadcast a transaction to the network.
    async fn broadcast_tx(&self, tx: &Transaction) -> Result<(), NetworkError>;
    /// Broadcast a block to the network.
    async fn broadcast_block(&self, block: &Block) -> Result<(), NetworkError>;
}

/// Mock network that logs operations.
pub struct MockNetwork;

#[async_trait]
impl NetworkService for MockNetwork {
    async fn broadcast_tx(&self, _tx: &Transaction) -> Result<(), NetworkError> {
        tracing::info!("Mock: transaction broadcast");
        Ok(())
    }

    async fn broadcast_block(&self, _block: &Block) -> Result<(), NetworkError> {
        tracing::info!("Mock: block broadcast");
        Ok(())
    }
}
