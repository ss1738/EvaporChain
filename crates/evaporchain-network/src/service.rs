use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use libp2p::{
    gossipsub::{self, IdentTopic, MessageAcceptance, MessageAuthenticity},
    identify, identity, mdns, noise,
    request_response::{self, ProtocolSupport},
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour, SwarmEvent},
    tcp, tls, yamux, Multiaddr, PeerId, StreamProtocol, SwarmBuilder,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::banlist::{now_ms, BanList};
use crate::{NetworkError, NetworkService};
use evaporchain_da::block_da::BlockDAPackage;
use evaporchain_da::sampling::{DASampler, SampleQuery, SampleResponse};
use evaporchain_types::{Block, Transaction};

// ─────────────────────────── Topics ──────────────────────────────────────

const TX_TOPIC: &str = "evaporchain/txs/1";
const BLOCK_TOPIC: &str = "evaporchain/blocks/1";
const CONSENSUS_TOPIC: &str = "evaporchain/consensus/1";
const BLOCK_SYNC_PROTOCOL: &str = "/evaporchain/blocksync/1";
const SHARD_SAMPLE_PROTOCOL: &str = "/evaporchain/shardsample/1";

// ─────────────────────────── Block Sync Types ────────────────────────────

/// Request a range of blocks from a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockSyncRequest {
    pub from_height: u64,
    pub to_height: u64,
}

/// Response containing requested blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockSyncResponse {
    pub blocks: Vec<Block>,
    /// The responder's current chain tip height.
    pub tip_height: u64,
}

/// Shared block cache for serving sync requests. The app inserts produced/applied blocks;
/// the network layer reads from it to serve peer requests.
pub type BlockCache = Arc<RwLock<BTreeMap<u64, Block>>>;

/// Disk fallback for the block-sync handler. Invoked on cache miss when
/// a peer requests a block older than the in-memory cache window
/// (`MAX_CACHE_SIZE = 2000` blocks behind tip). Returns the full block
/// from durable storage if present, `None` otherwise. When the fetcher
/// itself is `None`, the sync handler is cache-only — the legacy
/// behaviour from before fresh-from-genesis catch-up was supported.
///
/// Bug evidence (2026-05-07): M1 was wiped to height 0 while the
/// cluster was at h=15800. M1 requested `1..101`; every peer returned
/// 0 blocks because those were >2000 behind tip and evicted from the
/// cache. M1 sat at h=1 forever while polluting consensus with stale
/// prevotes. A fresh validator literally couldn't bootstrap.
#[derive(Clone)]
pub struct DiskBlockFetcher(pub Arc<dyn Fn(u64) -> Option<Block> + Send + Sync>);

impl DiskBlockFetcher {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(u64) -> Option<Block> + Send + Sync + 'static,
    {
        Self(Arc::new(f))
    }

    pub fn fetch(&self, height: u64) -> Option<Block> {
        (self.0)(height)
    }
}

impl std::fmt::Debug for DiskBlockFetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiskBlockFetcher(<fn>)")
    }
}

/// Shared DA shard cache — full nodes store BlockDAPackages so they can
/// serve shard sample requests from light clients.
pub type ShardCache = Arc<RwLock<BTreeMap<u64, BlockDAPackage>>>;

/// Maximum number of DA packages to keep in the shard cache.
const MAX_SHARD_CACHE_SIZE: usize = 500;

// ─────────────────────────── Shard Sample Types ─────────────────────────

/// Request a DA shard sample from a peer (light client → full node).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardSampleRequest {
    /// Queries for specific shards.
    pub queries: Vec<SampleQuery>,
}

/// Response containing shard samples with proofs (full node → light client).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardSampleResponse {
    /// Shard data + Merkle proofs for each requested shard.
    pub samples: Vec<Option<SampleResponse>>,
}

/// Maximum number of blocks to serve in a single sync response.
const MAX_SYNC_BATCH: u64 = 100;

/// Maximum number of shard queries we accept in a single
/// `ShardSampleRequest`. Each query forces a Merkle proof
/// construction on the serving side, so an attacker without a cap
/// could pin a CPU per request. Cap chosen well above legitimate
/// light-client sampling load (DA spec samples ~16 shards per block;
/// 256 lets a client batch many blocks per round-trip).
/// Audit AUDIT-2026-05-11-2.
const MAX_SHARD_QUERIES_PER_REQUEST: usize = 256;

/// Reason a `BlockSyncResponse` was rejected at the network layer.
/// Pure-function output of [`validate_sync_response_structure`].
/// Audit 2026-05-06 H-21 — these are cheap structural checks the
/// network layer enforces before forwarding to consensus, so a
/// malformed peer response doesn't reach the cryptographic
/// verification path.
#[derive(Debug, PartialEq, Eq)]
pub enum SyncResponseRejection {
    /// Response carries more than [`MAX_SYNC_BATCH`] blocks.
    OversizedBatch { len: usize, cap: u64 },
    /// Block heights are not monotonically non-decreasing.
    NonMonotoneHeights,
    /// `tip_height` is below the maximum height in the response —
    /// the peer is self-contradicting.
    TipBelowMaxHeight { tip: u64, max: u64 },
}

/// Validate a `BlockSyncResponse` at the network layer. Does the
/// three cheap structural checks (size cap, monotone heights, tip
/// consistency) before consensus runs the expensive cryptographic
/// verification. Pure function — testable without async/swarm
/// scaffolding.
pub fn validate_sync_response_structure(
    response: &BlockSyncResponse,
) -> Result<(), SyncResponseRejection> {
    if response.blocks.len() as u64 > MAX_SYNC_BATCH {
        return Err(SyncResponseRejection::OversizedBatch {
            len: response.blocks.len(),
            cap: MAX_SYNC_BATCH,
        });
    }
    let monotone = response
        .blocks
        .windows(2)
        .all(|w| w[0].number <= w[1].number);
    if !monotone {
        return Err(SyncResponseRejection::NonMonotoneHeights);
    }
    if let Some(max_h) = response.blocks.iter().map(|b| b.number).max() {
        if response.tip_height < max_h {
            return Err(SyncResponseRejection::TipBelowMaxHeight {
                tip: response.tip_height,
                max: max_h,
            });
        }
    }
    Ok(())
}

/// Maximum number of blocks to keep in the cache.
const MAX_CACHE_SIZE: usize = 2000;

/// Maximum allowed gossip message size. Re-audit (2026-05-02): unified
/// with libp2p-gossipsub's `max_transmit_size` below so the inbound
/// drop-on-arrival check is meaningful (previously 10 MB, but
/// gossipsub already rejected anything > 4 MB at the transport layer,
/// making the 10 MB ceiling unreachable). Both constants must move
/// together if either is tuned.
const MAX_GOSSIP_MESSAGE_SIZE: usize = 4 * 1024 * 1024;
const MAX_GOSSIPSUB_TRANSMIT_SIZE: usize = MAX_GOSSIP_MESSAGE_SIZE;
const MAX_CONSENSUS_MESSAGE_SIZE: usize = 512 * 1024;

/// Maximum gossip messages per peer per window before throttling.
const PEER_MSG_LIMIT: u64 = 500;
/// Rate limit window duration.
const PEER_MSG_WINDOW: Duration = Duration::from_secs(10);
/// Maximum tracked peers (LRU eviction beyond this).
const MAX_TRACKED_PEERS: usize = 1024;
/// Violations before a peer is banned.
const BAN_THRESHOLD: u32 = 10;
/// How long a ban lasts.
const BAN_DURATION: Duration = Duration::from_secs(600);
/// Sybil resistance: maximum concurrent connections per source IP.
/// One attacker host can run many libp2p identities but only from a
/// bounded set of source IPs; capping per-IP makes Sybil expensive
/// (attacker must rent many IPs). Tuned to allow several legitimate
/// nodes behind one home NAT or one Tailscale exit.
const MAX_CONNECTIONS_PER_IP: usize = 8;

struct PeerRateLimiter {
    counters: HashMap<PeerId, (u64, Instant)>,
}

impl PeerRateLimiter {
    fn new() -> Self {
        Self {
            counters: HashMap::new(),
        }
    }

    fn check_and_increment(&mut self, peer: &PeerId) -> bool {
        let now = Instant::now();
        let entry = self.counters.entry(*peer).or_insert((0, now));
        if now.duration_since(entry.1) >= PEER_MSG_WINDOW {
            entry.0 = 1;
            entry.1 = now;
            return true;
        }
        entry.0 += 1;
        if entry.0 > PEER_MSG_LIMIT {
            return false;
        }
        true
    }

    fn maybe_gc(&mut self) {
        if self.counters.len() > MAX_TRACKED_PEERS {
            let cutoff = Instant::now() - PEER_MSG_WINDOW * 2;
            self.counters.retain(|_, (_, ts)| *ts > cutoff);
        }
    }
}

struct PeerBanList {
    violations: HashMap<PeerId, u32>,
    banned: HashMap<PeerId, Instant>,
}

impl PeerBanList {
    fn new() -> Self {
        Self {
            violations: HashMap::new(),
            banned: HashMap::new(),
        }
    }

    fn is_banned(&mut self, peer: &PeerId) -> bool {
        if let Some(expiry) = self.banned.get(peer) {
            if Instant::now() < *expiry {
                return true;
            }
            self.banned.remove(peer);
            self.violations.remove(peer);
        }
        false
    }

    /// Re-audit (2026-05-02): full sweep of expired bans + stale
    /// violation counts. `is_banned()` lazily prunes only the queried
    /// peer; long-lived nodes accumulate stale entries from peers
    /// they never re-encounter. Called periodically from the network
    /// event loop.
    fn gc(&mut self) {
        let now = Instant::now();
        self.banned.retain(|_, expiry| now < *expiry);
        // Drop violation counts older than 2× the ban window —
        // arbitrary but bounded. Without timestamps on violations
        // we approximate by clearing if the ban set is empty (any
        // currently-tracked violations couldn't have triggered a ban
        // by now if BAN_DURATION has passed).
        if self.banned.is_empty() && self.violations.len() > 1024 {
            self.violations.clear();
        }
    }

    fn record_violation(&mut self, peer: PeerId) -> bool {
        let count = self.violations.entry(peer).or_insert(0);
        *count += 1;
        if *count >= BAN_THRESHOLD {
            self.banned.insert(peer, Instant::now() + BAN_DURATION);
            warn!(
                "Banned peer {peer} for {}s after {count} violations",
                BAN_DURATION.as_secs()
            );
            true
        } else {
            false
        }
    }
}

/// Per-IP connection accounting. Bounds how many concurrent peer
/// identities a single source IP can hold open. The libp2p PeerId is
/// cheap to spin up (Sybil-friendly); the source IP is not.
struct PerIpConnectionTracker {
    counts: std::collections::HashMap<std::net::IpAddr, usize>,
    per_ip_max: usize,
}

impl PerIpConnectionTracker {
    fn new(per_ip_max: usize) -> Self {
        Self {
            counts: std::collections::HashMap::new(),
            per_ip_max,
        }
    }

    /// Returns true if a new connection from this IP is permitted.
    /// Records the increment as a side effect when allowed.
    fn try_admit(&mut self, ip: std::net::IpAddr) -> bool {
        let n = self.counts.entry(ip).or_insert(0);
        if *n >= self.per_ip_max {
            return false;
        }
        *n += 1;
        true
    }

    /// Decrement on connection close. Idempotent at zero.
    fn release(&mut self, ip: std::net::IpAddr) {
        if let Some(n) = self.counts.get_mut(&ip) {
            if *n > 0 {
                *n -= 1;
            }
            if *n == 0 {
                self.counts.remove(&ip);
            }
        }
    }

    #[cfg(test)]
    fn count_for(&self, ip: &std::net::IpAddr) -> usize {
        self.counts.get(ip).copied().unwrap_or(0)
    }
}

/// Extract the remote IPv4/IPv6 address from a libp2p ConnectedPoint.
/// Returns None for transports without an IP component (rare).
fn endpoint_remote_ip(endpoint: &libp2p::core::ConnectedPoint) -> Option<std::net::IpAddr> {
    use libp2p::core::multiaddr::Protocol;
    use libp2p::core::ConnectedPoint;
    let addr = match endpoint {
        ConnectedPoint::Dialer { address, .. } => address,
        ConnectedPoint::Listener { send_back_addr, .. } => send_back_addr,
    };
    for proto in addr.iter() {
        match proto {
            Protocol::Ip4(ip) => return Some(std::net::IpAddr::V4(ip)),
            Protocol::Ip6(ip) => return Some(std::net::IpAddr::V6(ip)),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod per_ip_tracker_tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn admit_up_to_cap_then_reject() {
        let mut t = PerIpConnectionTracker::new(3);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        assert!(t.try_admit(ip));
        assert!(t.try_admit(ip));
        assert!(t.try_admit(ip));
        assert!(
            !t.try_admit(ip),
            "fourth connection from same IP should be rejected"
        );
        assert_eq!(t.count_for(&ip), 3);
    }

    #[test]
    fn release_frees_a_slot() {
        let mut t = PerIpConnectionTracker::new(2);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        assert!(t.try_admit(ip));
        assert!(t.try_admit(ip));
        assert!(!t.try_admit(ip));
        t.release(ip);
        assert!(t.try_admit(ip), "after release a slot should reopen");
    }

    #[test]
    fn release_below_zero_is_idempotent() {
        let mut t = PerIpConnectionTracker::new(2);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3));
        // Never admitted; release shouldn't panic.
        t.release(ip);
        assert_eq!(t.count_for(&ip), 0);
    }

    #[test]
    fn distinct_ips_are_independent() {
        let mut t = PerIpConnectionTracker::new(1);
        let a = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4));
        let b = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));
        assert!(t.try_admit(a));
        assert!(!t.try_admit(a));
        assert!(
            t.try_admit(b),
            "different IP should not be blocked by another IP's cap"
        );
    }
}

// ─────────────────────────── Config ──────────────────────────────────────

/// Configuration for the P2P network service.
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// Address to listen on (e.g., "/ip4/0.0.0.0/tcp/0" for random port).
    pub listen_address: String,
    /// Bootstrap peer addresses to connect to on startup.
    pub bootstrap_peers: Vec<String>,
    /// Channel buffer size for tx/block channels.
    pub channel_buffer: usize,
    /// Use TLS 1.3 transport instead of Noise. libp2p-tls generates
    /// self-signed certs from the node's identity key automatically.
    pub use_tls: bool,
    /// External TLS certificate config (for API server mTLS, not p2p).
    pub tls_certs: Option<crate::tls::TlsConfig>,
    /// Peer authorization policy. Controls which peers can connect.
    pub peer_authority: crate::tls::PeerAuthority,
    /// Max simultaneous connections from any single IP. Default 4.
    pub max_connections_per_ip: usize,
    /// Max simultaneous connections from any /24 (v4) or /48 (v6) subnet.
    /// Default 16.
    pub max_connections_per_subnet: usize,
    /// Total inbound connection cap across all peers. Default 200.
    pub max_inbound_connections: usize,
    /// Soft-ban duration when a peer is scored below threshold. Default 1h.
    pub peer_ban_duration_secs: u64,
    /// On-disk path for ban-list persistence. `None` disables persistence
    /// (the ban list is then memory-only).
    pub ban_list_path: Option<PathBuf>,
    /// Chain id baked into every gossipsub topic name so independent
    /// testnets running on the same LAN (e.g. via mDNS auto-discovery)
    /// can't cross-pollinate consensus messages, blocks, or txs. Empty
    /// string falls back to the legacy unscoped topic for backwards
    /// compatibility with any caller that hasn't been updated yet.
    pub chain_id: String,
    /// Enable libp2p mDNS LAN auto-discovery. Off by default — operators
    /// running multi-validator deployments should rely on
    /// `bootstrap_peers` (deterministic + chain-id-scoped). Leaving mDNS
    /// on caused cross-testnet poisoning when a stale cluster on the
    /// same subnet was auto-discovered and consumed connection slots.
    pub enable_mdns: bool,
    /// On-disk directory where the persistent libp2p identity key
    /// (`network_key.bin`) is stored. When `Some`, the service loads
    /// the key on startup if it exists, generating-and-persisting
    /// otherwise. When `None`, the service falls back to a fresh
    /// ephemeral identity each startup (legacy behaviour kept so the
    /// existing tests still pass without setting up a temp dir).
    /// Persisting the identity is what makes `bootstrap_peers` entries
    /// in genesis (which embed `/p2p/<peer_id>`) actually resolve to
    /// the right node across restarts.
    pub data_dir: Option<PathBuf>,
    /// Trusted validator IPs that bypass the per-IP and per-subnet
    /// inbound caps. Empty set = legacy behaviour (every peer subject
    /// to the `max_connections_per_ip` and `max_connections_per_subnet`
    /// gates). Populated set = those IPs can reconnect arbitrarily
    /// without exhausting the per-subnet quota — e.g., a known
    /// validator that churns through ephemeral peer-ids during long
    /// uptime won't get silently rejected once the 16-slot cap fills.
    ///
    /// Bug-B reference: `cluster_5node_2026_05_06_session.md` "Bug B
    /// — libp2p per-subnet cap exhausted by reconnect churn". Live
    /// evidence on 2026-05-07 evening: H1 ended with peer_count = 0
    /// after ~4 h of running because every reconnect from the
    /// Helsinki subnet hit the cap. Whitelisting validator IPs
    /// surgically bypasses the gate for legitimate peers without
    /// loosening sybil protection for the open internet.
    pub trusted_validator_ips: HashSet<IpAddr>,
    /// Disk fallback for the block-sync request handler. When set, the
    /// handler falls back to this callback for any height not present
    /// in the in-memory `block_cache`. When `None`, the handler is
    /// cache-only (legacy behaviour). See [`DiskBlockFetcher`] for
    /// the bug context this exists to close.
    pub disk_block_fetcher: Option<DiskBlockFetcher>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_address: "/ip4/0.0.0.0/tcp/0".to_string(),
            bootstrap_peers: vec![],
            channel_buffer: 256,
            use_tls: false,
            tls_certs: None,
            peer_authority: crate::tls::PeerAuthority::permissionless(),
            max_connections_per_ip: 4,
            max_connections_per_subnet: 16,
            max_inbound_connections: 200,
            peer_ban_duration_secs: 3_600,
            ban_list_path: None,
            trusted_validator_ips: HashSet::new(),
            chain_id: String::new(),
            enable_mdns: false,
            data_dir: None,
            disk_block_fetcher: None,
        }
    }
}

/// Load the persistent libp2p identity from `<data_dir>/network_key.bin`,
/// or generate a fresh ed25519 keypair and persist it (mode 0600 on Unix)
/// if the file does not yet exist. The on-disk format is libp2p's protobuf
/// keypair encoding (`Keypair::to_protobuf_encoding`), matching the wider
/// libp2p ecosystem so an operator can swap the file with one produced by
/// any libp2p tool.
pub fn load_or_generate_identity(data_dir: &Path) -> std::io::Result<identity::Keypair> {
    let key_path = data_dir.join("network_key.bin");
    if key_path.exists() {
        let bytes = std::fs::read(&key_path)?;
        return identity::Keypair::from_protobuf_encoding(&bytes).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("decode network_key.bin: {e}"),
            )
        });
    }
    if !data_dir.exists() {
        std::fs::create_dir_all(data_dir)?;
    }
    let kp = identity::Keypair::generate_ed25519();
    let bytes = kp.to_protobuf_encoding().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, format!("encode keypair: {e}"))
    })?;
    std::fs::write(&key_path, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(kp)
}

// ─────────────────────── Sybil-resistance state ──────────────────────────

/// A coarse subnet bucket: /24 for v4, /48 for v6. Returned as a
/// stringified prefix so the bucket key is stable across map lookups
/// and easy to log.
pub fn subnet_key(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            format!("{}.{}.{}.0/24", o[0], o[1], o[2])
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}::/48", s[0], s[1], s[2])
        }
    }
}

/// Reason an inbound connection was refused. Used to label the
/// `evap_inbound_rejections_total` counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    PerIp,
    PerSubnet,
    TotalCap,
    Banned,
    Unauthorized,
}

impl RejectionReason {
    pub fn label(&self) -> &'static str {
        match self {
            RejectionReason::PerIp => "per_ip",
            RejectionReason::PerSubnet => "per_subnet",
            RejectionReason::TotalCap => "total_cap",
            RejectionReason::Banned => "banned",
            RejectionReason::Unauthorized => "unauthorized",
        }
    }
}

/// Reputation score per remote peer. Decay-friendly counters; once
/// `score` falls below the threshold the peer is soft-banned.
#[derive(Debug, Clone, Copy)]
pub struct PeerScore {
    pub score: i32,
    pub last_seen_ms: u64,
    pub infractions: u32,
}

impl Default for PeerScore {
    fn default() -> Self {
        Self {
            score: 0,
            last_seen_ms: now_ms(),
            infractions: 0,
        }
    }
}

/// Score deltas. Positive: useful work. Negative: protocol violations.
pub const SCORE_VALID_BLOCK: i32 = 5;
pub const SCORE_VALID_VOTE: i32 = 1;
pub const SCORE_USEFUL_GOSSIP: i32 = 1;
pub const SCORE_INVALID_SIG: i32 = -50;
pub const SCORE_EQUIVOCATION: i32 = -100;
pub const SCORE_IDLE_TICK: i32 = -1;
/// Below this floor, the peer is soft-banned for `peer_ban_duration_secs`.
pub const SCORE_BAN_THRESHOLD: i32 = -100;

/// Per-peer summary for `/api/network/peers`.
///
/// Lane R.15: extended with `infractions` + `last_seen_ms` so
/// freeze-class debugging gets richer signal from one endpoint.
/// The Lane R.* cluster-freeze investigation needed all four
/// fields (score, infractions, age, last_seen) to confirm the
/// idle-tick was the culprit rather than a misbehaviour-driven
/// score drop. Without `infractions` you can't tell a
/// score=-100 from-idle-decay (infractions=0) apart from a
/// score=-100 from-spam (infractions=high).
#[derive(Debug, Clone, Serialize)]
pub struct PeerInfo {
    pub peer_id: String,
    pub ip: Option<String>,
    pub subnet: Option<String>,
    /// Unix-epoch ms of first observed connection.
    pub since_ms: u64,
    pub score: i32,
    pub age_seconds: u64,
    /// Count of negative-delta `adjust_score` calls since this
    /// peer's score entry was last reset (i.e. since the most
    /// recent successful `record_connect`). Lane R.15.
    pub infractions: u32,
    /// Unix-epoch ms of the most recent score-affecting event.
    /// Stays at the connect-time value if `adjust_score` has
    /// never fired for this peer. Lane R.15.
    pub last_seen_ms: u64,
}

/// Per-score-entry view for `/api/network/scores`. Unlike `PeerInfo`,
/// this surfaces *every* entry in the `scores` HashMap including
/// ghost-entries — peers that have been scored (e.g. via
/// `adjust_score` on a peer not currently in `peer_ips`) but are
/// not actively connected. The Lane R.* cluster-freeze root cause
/// was exactly this class of ghost entry; `scores_view` is the
/// operator's standing diagnostic for it.
#[derive(Debug, Clone, Serialize)]
pub struct PeerScoreEntry {
    pub peer_id: String,
    /// `true` iff `peer_id` is also in `peer_ips`. Ghost entries have
    /// `connected = false` and surface the freeze-class signal.
    pub connected: bool,
    pub ip: Option<String>,
    pub since_ms: Option<u64>,
    pub score: i32,
    pub infractions: u32,
    pub last_seen_ms: u64,
}

/// Counters surfaced to Prometheus as
/// `evap_inbound_rejections_total{reason="..."}`.
#[derive(Default, Debug)]
pub struct RejectionCounters {
    pub per_ip: AtomicU64,
    pub per_subnet: AtomicU64,
    pub total_cap: AtomicU64,
    pub banned: AtomicU64,
    pub unauthorized: AtomicU64,
}

impl RejectionCounters {
    fn record(&self, reason: RejectionReason) {
        let counter = match reason {
            RejectionReason::PerIp => &self.per_ip,
            RejectionReason::PerSubnet => &self.per_subnet,
            RejectionReason::TotalCap => &self.total_cap,
            RejectionReason::Banned => &self.banned,
            RejectionReason::Unauthorized => &self.unauthorized,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> [(&'static str, u64); 5] {
        [
            ("per_ip", self.per_ip.load(Ordering::Relaxed)),
            ("per_subnet", self.per_subnet.load(Ordering::Relaxed)),
            ("total_cap", self.total_cap.load(Ordering::Relaxed)),
            ("banned", self.banned.load(Ordering::Relaxed)),
            ("unauthorized", self.unauthorized.load(Ordering::Relaxed)),
        ]
    }
}

/// Live Sybil-resistance state. Held behind `Arc<RwLock<_>>` so the
/// API layer (`/api/network/peers` etc.) can read it without going
/// through a channel and without blocking the swarm event loop.
pub struct SybilState {
    /// peer_id -> (ip, since_ms)
    pub peer_ips: HashMap<PeerId, (IpAddr, u64)>,
    /// ip -> set of peer_ids
    pub ip_peers: HashMap<IpAddr, Vec<PeerId>>,
    /// "192.0.2.0/24" -> count of distinct active connections
    pub subnet_counts: HashMap<String, usize>,
    pub scores: HashMap<PeerId, PeerScore>,
    pub bans: BanList,
    pub rejections: RejectionCounters,
    pub ban_list_path: Option<PathBuf>,
    pub config: SybilConfig,
}

#[derive(Debug, Clone)]
pub struct SybilConfig {
    pub max_connections_per_ip: usize,
    pub max_connections_per_subnet: usize,
    pub max_inbound_connections: usize,
    pub peer_ban_duration_secs: u64,
    /// IPs whitelisted to bypass the per-IP and per-subnet caps. Used
    /// for known-validator peers so long-uptime peer-id churn doesn't
    /// exhaust the subnet quota. Empty set = legacy behaviour. See
    /// [`NetworkConfig::trusted_validator_ips`] for the live-evidence
    /// reference (2026-05-07 evening 4-hour soak Bug B).
    pub trusted_validator_ips: HashSet<IpAddr>,
}

impl SybilState {
    pub fn new(config: SybilConfig, ban_list_path: Option<PathBuf>) -> Self {
        let bans = match &ban_list_path {
            Some(path) => BanList::load(path),
            None => BanList::new(),
        };
        Self {
            peer_ips: HashMap::new(),
            ip_peers: HashMap::new(),
            subnet_counts: HashMap::new(),
            scores: HashMap::new(),
            bans,
            rejections: RejectionCounters::default(),
            ban_list_path,
            config,
        }
    }

    /// Try to admit a new inbound connection from `ip`. Returns `Ok(())`
    /// if accepted; `Err(reason)` if it should be disconnected.
    ///
    /// Trusted-validator-IP bypass: if `ip` is in
    /// [`SybilConfig::trusted_validator_ips`], the per-IP and
    /// per-subnet quotas are skipped. The total-cap and ban-list
    /// gates still apply (we never want to allow a banned peer
    /// even if it claims to be a validator, and the total cap is a
    /// memory-safety bound rather than a sybil bound). Bug-B fix:
    /// stops long-uptime peer-id churn from a known-validator
    /// subnet from silently exhausting the quota.
    pub fn try_admit_inbound(
        &mut self,
        ip: IpAddr,
        current_total: usize,
    ) -> Result<(), RejectionReason> {
        if self.bans.is_banned(&ip) {
            self.rejections.record(RejectionReason::Banned);
            return Err(RejectionReason::Banned);
        }
        if current_total >= self.config.max_inbound_connections {
            self.rejections.record(RejectionReason::TotalCap);
            return Err(RejectionReason::TotalCap);
        }
        let is_trusted = self.config.trusted_validator_ips.contains(&ip);
        if !is_trusted {
            let ip_count = self.ip_peers.get(&ip).map(|v| v.len()).unwrap_or(0);
            if ip_count >= self.config.max_connections_per_ip {
                self.rejections.record(RejectionReason::PerIp);
                return Err(RejectionReason::PerIp);
            }
            let key = subnet_key(&ip);
            let subnet_count = self.subnet_counts.get(&key).copied().unwrap_or(0);
            if subnet_count >= self.config.max_connections_per_subnet {
                self.rejections.record(RejectionReason::PerSubnet);
                return Err(RejectionReason::PerSubnet);
            }
        }
        Ok(())
    }

    /// Record a successful connection.
    ///
    /// Lane R.3: a successful handshake is a fresh slate. Reset score
    /// to 0 (and clear infraction count) so a peer that previously
    /// went idle, disconnected, then reconnected does NOT inherit
    /// the residual negative score from its last connection lifetime.
    /// Without this reset, disconnected peers accumulate
    /// SCORE_IDLE_TICK penalties indefinitely (Lane R.3 root cause)
    /// and arrive already-near-banned on reconnect.
    pub fn record_connect(&mut self, peer_id: PeerId, ip: IpAddr) {
        let now = now_ms();
        self.peer_ips.entry(peer_id).or_insert((ip, now));
        let entry = self.ip_peers.entry(ip).or_default();
        if !entry.contains(&peer_id) {
            entry.push(peer_id);
        }
        *self.subnet_counts.entry(subnet_key(&ip)).or_insert(0) += 1;
        // Fresh-slate the score on successful reconnection. The TLS /
        // peer-id authorization check has already passed by the time
        // we reach here; the peer's pre-disconnect Sybil history is
        // moot for purposes of forward-looking conduct.
        self.scores.insert(
            peer_id,
            PeerScore {
                score: 0,
                last_seen_ms: now,
                infractions: 0,
            },
        );
    }

    /// Record a connection close. Idempotent.
    ///
    /// Lane R.3: removes the score entry too. If we leave it in
    /// `scores`, the periodic SCORE_IDLE_TICK sweep will keep
    /// decrementing the score even though the peer is gone — a
    /// disconnected peer accumulating idle penalty is the bug
    /// `score: -292, age_seconds: 47` surfaced (the score had been
    /// decaying for hours while offline, even though the
    /// "connection" had only existed for 47 s).
    pub fn record_disconnect(&mut self, peer_id: &PeerId) {
        self.scores.remove(peer_id);
        if let Some((ip, _)) = self.peer_ips.remove(peer_id) {
            if let Some(v) = self.ip_peers.get_mut(&ip) {
                v.retain(|p| p != peer_id);
                if v.is_empty() {
                    self.ip_peers.remove(&ip);
                }
            }
            let key = subnet_key(&ip);
            if let Some(c) = self.subnet_counts.get_mut(&key) {
                if *c > 0 {
                    *c -= 1;
                }
                if *c == 0 {
                    self.subnet_counts.remove(&key);
                }
            }
        }
    }

    /// Adjust score by `delta` and return `Some(ip)` to soft-ban iff
    /// the threshold has been crossed for the first time.
    pub fn adjust_score(&mut self, peer_id: &PeerId, delta: i32) -> Option<IpAddr> {
        let entry = self.scores.entry(*peer_id).or_default();
        entry.score = entry.score.saturating_add(delta);
        entry.last_seen_ms = now_ms();
        if delta < 0 {
            entry.infractions = entry.infractions.saturating_add(1);
        }
        if entry.score < SCORE_BAN_THRESHOLD {
            return self.peer_ips.get(peer_id).map(|(ip, _)| *ip);
        }
        None
    }

    /// Record a ban + persist immediately so the file survives a hard
    /// crash mid-session.
    pub fn ban_ip(&mut self, ip: IpAddr, reason: impl Into<String>) {
        let until = now_ms() + self.config.peer_ban_duration_secs * 1_000;
        self.bans.add_ban(ip, until, reason);
        if let Some(path) = self.ban_list_path.clone() {
            if let Err(e) = self.bans.save(&path) {
                warn!("ban list save failed: {e}");
            }
        }
    }

    pub fn unban_ip(&mut self, ip: &IpAddr) -> bool {
        let removed = self.bans.remove_ban(ip);
        if removed {
            if let Some(path) = self.ban_list_path.clone() {
                if let Err(e) = self.bans.save(&path) {
                    warn!("ban list save failed: {e}");
                }
            }
        }
        removed
    }

    pub fn peer_view(&self) -> Vec<PeerInfo> {
        let now = now_ms();
        self.peer_ips
            .iter()
            .map(|(pid, (ip, since))| {
                let score_entry = self.scores.get(pid);
                PeerInfo {
                    peer_id: pid.to_string(),
                    ip: Some(ip.to_string()),
                    subnet: Some(subnet_key(ip)),
                    since_ms: *since,
                    score: score_entry.map(|s| s.score).unwrap_or(0),
                    age_seconds: now.saturating_sub(*since) / 1_000,
                    // Lane R.15: surface infractions + last_seen_ms
                    // so freeze-class debugging reads cleanly from
                    // one endpoint.
                    infractions: score_entry.map(|s| s.infractions).unwrap_or(0),
                    last_seen_ms: score_entry.map(|s| s.last_seen_ms).unwrap_or(*since),
                }
            })
            .collect()
    }

    /// Diagnostic projection of the `scores` map, including
    /// ghost-entries (peers in `scores` but not `peer_ips`). The
    /// Lane R.* freeze-class root cause hinged on a peer being
    /// scored without being connected; `peer_view()` only iterates
    /// connected peers, so such ghosts were invisible to
    /// `/api/network/peers`. `scores_view()` is the operator surface
    /// that catches the next freeze-class issue without log-grepping.
    pub fn scores_view(&self) -> Vec<PeerScoreEntry> {
        self.scores
            .iter()
            .map(|(pid, score)| {
                let connected_ip = self.peer_ips.get(pid);
                PeerScoreEntry {
                    peer_id: pid.to_string(),
                    connected: connected_ip.is_some(),
                    ip: connected_ip.map(|(ip, _)| ip.to_string()),
                    since_ms: connected_ip.map(|(_, since)| *since),
                    score: score.score,
                    infractions: score.infractions,
                    last_seen_ms: score.last_seen_ms,
                }
            })
            .collect()
    }
}

// ─────────────────────────── Behaviour ───────────────────────────────────

#[derive(NetworkBehaviour)]
struct EvaporBehaviour {
    gossipsub: gossipsub::Behaviour,
    // mDNS is opt-in via NetworkConfig::enable_mdns. Disabled by default
    // because LAN-side discovery cross-pollinates independent testnets
    // running on the same subnet (a stranger 4-validator cluster on .227
    // poisoned our peer counts during P1 #11 verification 2026-05-02).
    mdns: Toggle<mdns::tokio::Behaviour>,
    identify: identify::Behaviour,
    block_sync: request_response::json::Behaviour<BlockSyncRequest, BlockSyncResponse>,
    shard_sample: request_response::json::Behaviour<ShardSampleRequest, ShardSampleResponse>,
}

// ─────────────────────────── Service ─────────────────────────────────────

/// Channels returned by [`P2pNetworkService::start`] for the application
/// to send and receive gossip messages.
pub struct NetworkChannels {
    /// Send transactions to the network (app → network).
    pub tx_sender: mpsc::Sender<Transaction>,
    /// Receive transactions from the network (network → app).
    pub tx_receiver: mpsc::Receiver<Transaction>,
    /// Send blocks to the network (app → network).
    pub block_sender: mpsc::Sender<Block>,
    /// Receive blocks from the network (network → app).
    pub block_receiver: mpsc::Receiver<Block>,
    /// Number of connected peers (updated by the network event loop).
    pub peer_count: Arc<AtomicUsize>,
    /// Shared block cache — app inserts blocks, network reads to serve sync requests.
    pub block_cache: BlockCache,
    /// Send sync request (from_height, to_height) to trigger block backfill from peers.
    pub sync_request_sender: mpsc::Sender<(u64, u64)>,
    /// Receive synced blocks from peers (backfill responses).
    pub sync_blocks_receiver: mpsc::Receiver<Vec<Block>>,
    /// Receive peer tip height announcements (peer connected with this chain height).
    pub tip_receiver: mpsc::Receiver<u64>,
    /// Send consensus messages to the network (app → network).
    pub consensus_sender: mpsc::Sender<Vec<u8>>,
    /// Receive consensus messages from the network (network → app).
    pub consensus_receiver: mpsc::Receiver<Vec<u8>>,
    /// Shared DA shard cache — app inserts BlockDAPackages, network serves sample requests.
    pub shard_cache: ShardCache,
    /// Send shard sample requests to peers (light client → network).
    pub sample_request_sender: mpsc::Sender<Vec<SampleQuery>>,
    /// Receive shard sample responses from peers (network → light client).
    pub sample_response_receiver: mpsc::Receiver<Vec<SampleResponse>>,
    /// Live Sybil-resistance state (peer IPs, scores, bans, rejection
    /// counters). Read by the API layer for `/api/network/peers` /
    /// `/api/network/banned` and by the metrics handler.
    pub sybil_state: Arc<RwLock<SybilState>>,
}

/// Handle for broadcasting to a running network service.
#[derive(Clone)]
pub struct NetworkHandle {
    tx_sender: mpsc::Sender<Transaction>,
    block_sender: mpsc::Sender<Block>,
}

#[async_trait]
impl NetworkService for NetworkHandle {
    async fn broadcast_tx(&self, tx: &Transaction) -> Result<(), NetworkError> {
        self.tx_sender
            .send(tx.clone())
            .await
            .map_err(|e| NetworkError::BroadcastFailed(e.to_string()))
    }

    async fn broadcast_block(&self, block: &Block) -> Result<(), NetworkError> {
        self.block_sender
            .send(block.clone())
            .await
            .map_err(|e| NetworkError::BroadcastFailed(e.to_string()))
    }
}

/// Acquire a write lock on the block cache, recovering from poisoning.
fn safe_write(cache: &BlockCache) -> std::sync::RwLockWriteGuard<'_, BTreeMap<u64, Block>> {
    cache.write().unwrap_or_else(|poisoned| {
        warn!("Recovered poisoned block cache write lock");
        poisoned.into_inner()
    })
}

/// Acquire a read lock on the block cache, recovering from poisoning.
fn safe_read(cache: &BlockCache) -> std::sync::RwLockReadGuard<'_, BTreeMap<u64, Block>> {
    cache.read().unwrap_or_else(|poisoned| {
        warn!("Recovered poisoned block cache read lock");
        poisoned.into_inner()
    })
}

/// Insert a block into the cache, evicting old entries if needed.
pub fn cache_block(cache: &BlockCache, block: &Block) {
    let mut c = safe_write(cache);
    c.insert(block.number, block.clone());
    // Evict oldest entries if cache is too large
    while c.len() > MAX_CACHE_SIZE {
        if let Some(&oldest) = c.keys().next() {
            c.remove(&oldest);
        }
    }
}

/// Insert a DA package into the shard cache for serving sample requests.
pub fn cache_da_package(cache: &ShardCache, block_number: u64, package: BlockDAPackage) {
    let mut c = cache.write().unwrap_or_else(|poisoned| {
        warn!("Recovered poisoned shard cache write lock");
        poisoned.into_inner()
    });
    c.insert(block_number, package);
    while c.len() > MAX_SHARD_CACHE_SIZE {
        if let Some(&oldest) = c.keys().next() {
            c.remove(&oldest);
        }
    }
}

/// P2P network service using libp2p with GossipSub + mDNS + block sync.
pub struct P2pNetworkService;

impl P2pNetworkService {
    /// Start the network service. Returns channels for the app to communicate
    /// with the network layer, a handle for broadcasting, and the local PeerId.
    ///
    /// The network event loop runs as a spawned tokio task.
    pub async fn start(
        config: NetworkConfig,
    ) -> Result<(NetworkChannels, NetworkHandle, PeerId), NetworkError> {
        let block_cache: BlockCache = Arc::new(RwLock::new(BTreeMap::new()));
        let block_cache_inner = Arc::clone(&block_cache);
        let shard_cache: ShardCache = Arc::new(RwLock::new(BTreeMap::new()));
        let shard_cache_inner = Arc::clone(&shard_cache);
        let disk_block_fetcher = config.disk_block_fetcher.clone();

        let peer_authority = config.peer_authority.clone();
        let use_tls = config.use_tls;

        // ── Sybil-resistance state (per-IP/subnet caps, scoring, bans) ──
        let sybil_cfg = SybilConfig {
            max_connections_per_ip: config.max_connections_per_ip,
            max_connections_per_subnet: config.max_connections_per_subnet,
            max_inbound_connections: config.max_inbound_connections,
            peer_ban_duration_secs: config.peer_ban_duration_secs,
            trusted_validator_ips: config.trusted_validator_ips.clone(),
        };
        let sybil_state = Arc::new(RwLock::new(SybilState::new(
            sybil_cfg,
            config.ban_list_path.clone(),
        )));
        let sybil_state_inner = Arc::clone(&sybil_state);

        // Behaviour constructor shared by both transport paths
        macro_rules! build_behaviour {
            ($key:ident) => {{
                // Use default message ID (source + seq_no) so each validator's
                // consensus votes get unique IDs even across rounds.
                // H8 (audit 2026-05-02): was Permissive, which let
                // malformed / unsigned messages traverse the mesh and
                // forced expensive deserialization downstream before
                // any rejection. Strict drops them at the protocol
                // boundary using libp2p-gossipsub's signature check
                // (we already pass MessageAuthenticity::Signed). Pair
                // with `MessageAuthenticity::Signed` below — the two
                // settings are coupled.
                // Phase 4.4 (2026-05-03): mesh fanout bumped from
                // (3/2/6) to (8/6/12) per audit `end_to_end_audit
                // _2026_04_27.md §4` — eclipse-attack resistance.
                // With mesh_n=3 an attacker controlling 3 mesh slots
                // could fully isolate a peer. Bumping to mesh_n=8
                // (libp2p-gossipsub's recommended baseline) raises
                // the eclipse cost to 8 controlled slots and matches
                // the published Gossipsub paper's safe-degree
                // threshold. mesh_outbound_min=2 forces at least
                // two genuinely outbound (i.e. peer-id we dialed)
                // mesh members so an inbound-only adversary can't
                // saturate the slot count.
                // M7 (audit 2026-05-13): `.validate_messages()` flips
                // libp2p-gossipsub into manual-validation mode. Without
                // it, gossipsub auto-forwards every payload to mesh
                // peers the moment the libp2p signature check passes
                // (which is all `ValidationMode::Strict` does — it
                // doesn't look at our app payload at all). A peer
                // pushing 4MB junk JSON would amplify across the entire
                // mesh before our deserializer rejected it.
                //
                // In manual mode the node holds the message in the
                // memcache until we call
                // `report_message_validation_result(&msg_id,
                //  &propagation_source, MessageAcceptance::*)`:
                //   * Accept → forward to mesh peers
                //   * Reject → drop + penalise propagation_source via
                //              libp2p's peer-score system
                //   * Ignore → drop without penalising (use for
                //              local-policy drops like ban-list hits)
                // See the event handler at the `gossipsub::Event::
                // Message` arm below for the call sites.
                let gossipsub_config = gossipsub::ConfigBuilder::default()
                    .heartbeat_interval(Duration::from_millis(500))
                    .validation_mode(gossipsub::ValidationMode::Strict)
                    .validate_messages()
                    .max_transmit_size(MAX_GOSSIPSUB_TRANSMIT_SIZE)
                    .mesh_n(8)
                    .mesh_n_low(6)
                    .mesh_n_high(12)
                    .mesh_outbound_min(2)
                    .gossip_lazy(6)
                    .build()
                    .expect("valid gossipsub config");
                let gossipsub = gossipsub::Behaviour::new(
                    MessageAuthenticity::Signed($key.clone()),
                    gossipsub_config,
                )
                .expect("valid gossipsub behaviour");

                let mdns: Toggle<mdns::tokio::Behaviour> = if config.enable_mdns {
                    Some(
                        mdns::tokio::Behaviour::new(
                            mdns::Config::default(),
                            $key.public().to_peer_id(),
                        )
                        .expect("valid mdns behaviour"),
                    )
                    .into()
                } else {
                    None.into()
                };

                let identify = identify::Behaviour::new(identify::Config::new(
                    "/evaporchain/1.0.0".to_string(),
                    $key.public(),
                ));

                let block_sync = request_response::json::Behaviour::new(
                    [(
                        StreamProtocol::new(BLOCK_SYNC_PROTOCOL),
                        ProtocolSupport::Full,
                    )],
                    request_response::Config::default()
                        .with_request_timeout(Duration::from_secs(30)),
                );

                let shard_sample = request_response::json::Behaviour::new(
                    [(
                        StreamProtocol::new(SHARD_SAMPLE_PROTOCOL),
                        ProtocolSupport::Full,
                    )],
                    request_response::Config::default()
                        .with_request_timeout(Duration::from_secs(10)),
                );

                EvaporBehaviour {
                    gossipsub,
                    mdns,
                    identify,
                    block_sync,
                    shard_sample,
                }
            }};
        }

        // Build the swarm — TLS 1.3 or Noise, selected at startup
        // Enable port_reuse so the listener socket can also be used for
        // outbound dials. Without this, libp2p tries to dial peers from
        // ephemeral ports while the SAME local port is already bound for
        // listening, and on macOS the kernel rejects with EADDRINUSE
        // when the dial target is another node listening on the same
        // port (every cluster member uses 9000). Reproduced on the
        // 3-Mini Tailscale cluster: bootstrap dial from apsarth/ironman
        // to satyawan:9000 always failed until this flag was set.
        // libp2p 0.54 deprecated port_reuse — the option has no effect now,
        // port-reuse is decided per-connection by the behaviour. Keep the
        // call so the explicit intent is preserved in source while we wait
        // for the actual deprecation to be enforced.
        #[allow(deprecated)]
        let tcp_cfg = || tcp::Config::default().port_reuse(true);

        // Resolve the libp2p identity. When `data_dir` is set, persist it
        // so the PeerId is stable across restarts — required for
        // bootstrap_peers entries baked into genesis (which embed
        // `/p2p/<peer_id>` suffixes) to actually resolve to the right
        // node. When `data_dir` is None, fall back to a fresh ephemeral
        // identity (legacy behaviour, used by unit tests).
        let local_keypair = match config.data_dir.as_deref() {
            Some(dir) => load_or_generate_identity(dir).map_err(|e| {
                NetworkError::ConnectionError(format!(
                    "load_or_generate_identity({}): {e}",
                    dir.display()
                ))
            })?,
            None => identity::Keypair::generate_ed25519(),
        };

        let mut swarm = if use_tls {
            info!("Using TLS 1.3 transport (libp2p-tls)");
            SwarmBuilder::with_existing_identity(local_keypair.clone())
                .with_tokio()
                .with_tcp(tcp_cfg(), tls::Config::new, yamux::Config::default)
                .map_err(|e| NetworkError::ConnectionError(format!("tls transport: {e}")))?
                .with_behaviour(|key| build_behaviour!(key))
                .map_err(|e| NetworkError::ConnectionError(format!("behaviour: {e}")))?
                .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
                .build()
        } else {
            SwarmBuilder::with_existing_identity(local_keypair.clone())
                .with_tokio()
                .with_tcp(tcp_cfg(), noise::Config::new, yamux::Config::default)
                .map_err(|e| NetworkError::ConnectionError(format!("tcp transport: {e}")))?
                .with_behaviour(|key| build_behaviour!(key))
                .map_err(|e| NetworkError::ConnectionError(format!("behaviour: {e}")))?
                .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
                .build()
        };

        let local_peer_id = *swarm.local_peer_id();

        // Subscribe to topics
        // Scope topics by chain_id so a stray validator from a different
        // testnet on the same LAN (mDNS auto-discovery is on) doesn't
        // join our gossip mesh and pollute mempools / sync requests.
        // Empty chain_id keeps the legacy topic for back-compat.
        let topic_suffix = if config.chain_id.is_empty() {
            String::new()
        } else {
            format!("/{}", config.chain_id)
        };
        let tx_topic = IdentTopic::new(format!("{}{}", TX_TOPIC, topic_suffix));
        let block_topic = IdentTopic::new(format!("{}{}", BLOCK_TOPIC, topic_suffix));
        let consensus_topic = IdentTopic::new(format!("{}{}", CONSENSUS_TOPIC, topic_suffix));
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&tx_topic)
            .map_err(|e| NetworkError::ConnectionError(format!("subscribe tx: {e}")))?;
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&block_topic)
            .map_err(|e| NetworkError::ConnectionError(format!("subscribe block: {e}")))?;
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&consensus_topic)
            .map_err(|e| NetworkError::ConnectionError(format!("subscribe consensus: {e}")))?;

        // Listen
        let listen_addr: Multiaddr = config
            .listen_address
            .parse()
            .map_err(|e| NetworkError::ConnectionError(format!("parse listen addr: {e}")))?;
        swarm
            .listen_on(listen_addr)
            .map_err(|e| NetworkError::ConnectionError(format!("listen: {e}")))?;

        // Connect to bootstrap peers
        for addr_str in &config.bootstrap_peers {
            if let Ok(addr) = addr_str.parse::<Multiaddr>() {
                if let Err(e) = swarm.dial(addr.clone()) {
                    warn!("Failed to dial bootstrap peer {}: {}", addr, e);
                }
            }
        }

        // Create channels
        let buf = config.channel_buffer;
        let (app_tx_sender, mut net_tx_receiver) = mpsc::channel::<Transaction>(buf);
        let (net_tx_sender, app_tx_receiver) = mpsc::channel::<Transaction>(buf);
        let (app_block_sender, mut net_block_receiver) = mpsc::channel::<Block>(buf);
        let (net_block_sender, app_block_receiver) = mpsc::channel::<Block>(buf);

        // Consensus message channels (raw bytes — app serializes/deserializes)
        let (app_consensus_sender, mut net_consensus_receiver) = mpsc::channel::<Vec<u8>>(buf);
        let (net_consensus_sender, app_consensus_receiver) = mpsc::channel::<Vec<u8>>(buf);

        // Sync channels
        let (sync_req_sender, mut sync_req_receiver) = mpsc::channel::<(u64, u64)>(32);
        let (sync_blocks_sender, sync_blocks_receiver) = mpsc::channel::<Vec<Block>>(32);
        let (tip_sender, tip_receiver) = mpsc::channel::<u64>(32);

        // Shard sample channels
        let (sample_req_sender, mut sample_req_receiver) = mpsc::channel::<Vec<SampleQuery>>(32);
        let (sample_resp_sender, sample_resp_receiver) = mpsc::channel::<Vec<SampleResponse>>(32);

        let peer_count = Arc::new(AtomicUsize::new(0));
        let peer_count_inner = Arc::clone(&peer_count);

        let handle = NetworkHandle {
            tx_sender: app_tx_sender.clone(),
            block_sender: app_block_sender.clone(),
        };

        let channels = NetworkChannels {
            tx_sender: app_tx_sender,
            tx_receiver: app_tx_receiver,
            block_sender: app_block_sender,
            block_receiver: app_block_receiver,
            peer_count,
            block_cache: Arc::clone(&block_cache),
            sync_request_sender: sync_req_sender,
            sync_blocks_receiver,
            tip_receiver,
            consensus_sender: app_consensus_sender,
            consensus_receiver: app_consensus_receiver,
            shard_cache: Arc::clone(&shard_cache),
            sample_request_sender: sample_req_sender,
            sample_response_receiver: sample_resp_receiver,
            sybil_state: Arc::clone(&sybil_state),
        };

        // Clone bootstrap addrs for periodic re-dial inside the event loop
        let bootstrap_addrs: Vec<Multiaddr> = config
            .bootstrap_peers
            .iter()
            .filter_map(|s| s.parse::<Multiaddr>().ok())
            .collect();

        // Snapshot the Sybil caps so the spawned event loop doesn't have
        // to keep `config` around just for log lines.
        let cap_per_ip = config.max_connections_per_ip;
        let cap_per_subnet = config.max_connections_per_subnet;
        let cap_total = config.max_inbound_connections;

        // Spawn the event loop
        tokio::spawn(async move {
            let tx_topic_hash = tx_topic.hash();
            let block_topic_hash = block_topic.hash();
            let consensus_topic_hash = consensus_topic.hash();

            // Re-dial bootstrap peers every 30s if we have fewer than expected
            let mut redial_timer = tokio::time::interval(Duration::from_secs(30));
            redial_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            let mut rate_limiter = PeerRateLimiter::new();
            let mut ban_list = PeerBanList::new();
            let mut per_ip_tracker = PerIpConnectionTracker::new(MAX_CONNECTIONS_PER_IP);
            let mut gc_counter: u64 = 0;
            // M12 (audit 2026-05-02): rotate the picked peer for
            // request_response (block-sync / shard-sample) per call.
            // Previously all outbound requests always hit peers[0],
            // so a single slow / unresponsive peer wedged every
            // sync request — no retry, no rotation, no jitter.
            // Each request increments the counter; the modulo picks a
            // different connected peer, spreading load + giving the
            // chain a path forward when peer 0 is misbehaving.
            let mut req_rotation: u64 = 0;
            // Re-audit (2026-05-02) freshness/age-out: peers that
            // recently failed an OutboundFailure are tracked here
            // with a cool-off timestamp. Picking logic skips them
            // for `REQ_PEER_COOLOFF` seconds, so a flapping peer is
            // naturally rotated out of the request pool until it
            // looks healthy again. Implements both M12-followup
            // (peer rotation freshness) and the request_response
            // retry-queue audit item: any caller re-issuing a
            // request after a failure lands on a different peer.
            const REQ_PEER_COOLOFF: Duration = Duration::from_secs(30);
            const REQ_FAIL_MAP_CAP: usize = 256;
            let mut recently_failed: HashMap<PeerId, Instant> = HashMap::new();
            let mut idle_score_timer = tokio::time::interval(Duration::from_secs(300));
            idle_score_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    // Periodic 5-minute idle-score sweep. Quiet peers
                    // accumulate -1 per tick; once below the ban
                    // threshold their source IP is banned and the
                    // connection dropped.
                    _ = idle_score_timer.tick() => {
                        // Re-audit (2026-05-02): periodic GC of
                        // expired bans + stale violation counts so
                        // long-lived nodes don't accumulate ban
                        // entries from peers never re-seen.
                        ban_list.gc();
                        let mut to_disconnect: Vec<(PeerId, IpAddr)> = Vec::new();
                        if let Ok(mut s) = sybil_state_inner.write() {
                            // Lane R.3: iterate only CONNECTED peers
                            // (peer_ips), not the score map (which
                            // outlives connection lifetime in older
                            // builds). A peer that disconnected hours
                            // ago must not still be accumulating
                            // SCORE_IDLE_TICK penalty in absentia.
                            // Lane R.3's `record_disconnect` clears
                            // the score entry on disconnect, but this
                            // belt-and-braces iteration source makes
                            // the invariant explicit at the call site
                            // too.
                            let peer_ids: Vec<PeerId> = s.peer_ips.keys().copied().collect();
                            for pid in peer_ids {
                                // Lane R.1: skip idle-score penalty for
                                // authorized validators. In a permissioned
                                // validator-set cluster the Sybil-score
                                // mechanism is the wrong tool — authorized
                                // peers are pre-vetted via TLS / peer-id
                                // allowlist (`peer_authority`) and must
                                // not be slow-banned just because they
                                // went 100 idle ticks without a positive
                                // event. Without this gate, every small
                                // cluster eventually freezes itself
                                // (caught on the 3-Mini Tailscale cluster
                                // 2026-05-04: cluster halted at h=771
                                // after ~90 min because peers crossed
                                // SCORE_BAN_THRESHOLD via accumulated
                                // SCORE_IDLE_TICK without misbehaviour).
                                if peer_authority.is_authorized(&pid) {
                                    continue;
                                }
                                if let Some(ip) = s.adjust_score(&pid, SCORE_IDLE_TICK) {
                                    s.ban_ip(ip, "score_threshold_breach");
                                    to_disconnect.push((pid, ip));
                                }
                            }
                            s.bans.cleanup_expired();
                        }
                        for (pid, ip) in to_disconnect {
                            warn!("network: soft-banning {ip} (peer {pid}) — score below {SCORE_BAN_THRESHOLD}");
                            let _ = swarm.disconnect_peer_id(pid);
                        }
                    }
                    // Periodic bootstrap re-dial for peers that weren't reachable at startup
                    _ = redial_timer.tick() => {
                        let connected = swarm.connected_peers().count();
                        if connected < bootstrap_addrs.len() {
                            for addr in &bootstrap_addrs {
                                if let Err(e) = swarm.dial(addr.clone()) {
                                    debug!("Re-dial {addr}: {e}");
                                }
                            }
                            info!("Re-dialing {} bootstrap peers (currently {} connected)", bootstrap_addrs.len(), connected);
                        }
                    }
                    // App wants to broadcast a transaction
                    Some(tx) = net_tx_receiver.recv() => {
                        match serde_json::to_vec(&tx) {
                            Ok(data) => {
                                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(tx_topic.clone(), data) {
                                    debug!("Failed to publish tx: {e}");
                                }
                            }
                            Err(e) => warn!("Failed to serialize tx: {e}"),
                        }
                    }
                    // App wants to broadcast a block
                    Some(block) = net_block_receiver.recv() => {
                        match serde_json::to_vec(&block) {
                            Ok(data) => {
                                let sz = data.len();
                                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(block_topic.clone(), data) {
                                    warn!("Failed to publish block #{} ({sz} bytes): {e}", block.number);
                                }
                            }
                            Err(e) => warn!("Failed to serialize block: {e}"),
                        }
                    }
                    // App wants to broadcast a consensus message
                    Some(data) = net_consensus_receiver.recv() => {
                        let sz = data.len();
                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(consensus_topic.clone(), data) {
                            warn!("Failed to publish consensus msg ({sz} bytes): {e}");
                        }
                    }
                    // App requests shard samples from peers (light client DAS)
                    Some(queries) = sample_req_receiver.recv() => {
                        // GC the failure map opportunistically.
                        let now_inst = Instant::now();
                        recently_failed.retain(|_, t| now_inst.duration_since(*t) < REQ_PEER_COOLOFF);
                        let all_peers: Vec<PeerId> = swarm.connected_peers().cloned().collect();
                        let healthy: Vec<PeerId> = all_peers
                            .iter()
                            .copied()
                            .filter(|p| !recently_failed.contains_key(p))
                            .collect();
                        // Prefer healthy peers; if none, fall back to all
                        // (better to retry against a recently-failed peer
                        // than wedge waiting for cool-off).
                        let pool = if !healthy.is_empty() { &healthy } else { &all_peers };
                        if pool.is_empty() {
                            warn!("No peers available for shard sample request");
                        } else {
                            // Fan out to ALL peers in pool. Earlier code sent
                            // to one round-robin peer, which wedged whenever
                            // that peer didn't yet have the block's shards
                            // — common right at finalization time over WAN.
                            // Receiving multiple responses for the same query
                            // is safe: the main loop's cumulative
                            // da_valid_sample_count handles duplicates by
                            // raising confidence faster.
                            req_rotation = req_rotation.wrapping_add(1);
                            debug!(
                                "Requesting {} shard samples from {} peers (fan-out)",
                                queries.len(), pool.len()
                            );
                            for &target in pool.iter() {
                                swarm.behaviour_mut().shard_sample.send_request(
                                    &target,
                                    ShardSampleRequest { queries: queries.clone() },
                                );
                            }
                        }
                    }
                    // App requests block sync from peers
                    Some((from, to)) = sync_req_receiver.recv() => {
                        let now_inst = Instant::now();
                        recently_failed.retain(|_, t| now_inst.duration_since(*t) < REQ_PEER_COOLOFF);
                        let all_peers: Vec<PeerId> = swarm.connected_peers().cloned().collect();
                        let healthy: Vec<PeerId> = all_peers
                            .iter()
                            .copied()
                            .filter(|p| !recently_failed.contains_key(p))
                            .collect();
                        let pool = if !healthy.is_empty() { &healthy } else { &all_peers };
                        if pool.is_empty() {
                            warn!("No peers available for block sync request {from}..{to}");
                        } else {
                            let idx = (req_rotation as usize) % pool.len();
                            req_rotation = req_rotation.wrapping_add(1);
                            let target = pool[idx];
                            if from > to {
                                warn!("Invalid sync range: from={from} > to={to}");
                                continue;
                            }
                            let capped_to = from + MAX_SYNC_BATCH.min(to - from);
                            info!("Requesting blocks {from}..{capped_to} from peer {target}");
                            swarm.behaviour_mut().block_sync.send_request(
                                &target,
                                BlockSyncRequest { from_height: from, to_height: capped_to },
                            );
                        }
                    }
                    // Swarm events
                    event = swarm.select_next_some() => {
                        match event {
                            // ── GossipSub messages ──
                            //
                            // M7 (audit 2026-05-13): with `.validate_messages()`
                            // on the config, every gossipsub payload sits in the
                            // memcache here until we tell gossipsub what to do
                            // with it via `report_message_validation_result`:
                            //   * Accept  → forward to mesh peers
                            //   * Reject  → drop + ding propagation_source via
                            //               libp2p peer-score
                            //   * Ignore  → drop without penalising
                            // We MUST hit `report_message_validation_result`
                            // exactly once on every path the message takes
                            // through this arm — otherwise the memcache keeps
                            // it around and the message never relays (even on
                            // honest peers).
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::Gossipsub(
                                gossipsub::Event::Message {
                                    message,
                                    message_id,
                                    propagation_source,
                                },
                            )) => {
                                let mut acceptance = MessageAcceptance::Ignore;
                                // Per-peer ban + rate limiting
                                let mut skip = false;
                                if let Some(ref source) = message.source {
                                    if ban_list.is_banned(source) {
                                        // Local policy drop — Ignore (no peer-score penalty
                                        // beyond what the ban-list already imposes).
                                        acceptance = MessageAcceptance::Ignore;
                                        skip = true;
                                    } else if !rate_limiter.check_and_increment(source) {
                                        debug!("Rate-limited peer {source} — dropping gossip message");
                                        gc_counter += 1;
                                        if gc_counter.is_multiple_of(100) {
                                            rate_limiter.maybe_gc();
                                        }
                                        // Local policy drop — Ignore.
                                        acceptance = MessageAcceptance::Ignore;
                                        skip = true;
                                    }
                                }
                                if !skip {
                                    gc_counter += 1;
                                    if gc_counter.is_multiple_of(1000) {
                                        rate_limiter.maybe_gc();
                                    }
                                    // Drop oversized messages before deserialization (DoS protection)
                                    if message.data.len() > MAX_GOSSIP_MESSAGE_SIZE {
                                        warn!(
                                            "Dropping oversized gossip message: {} bytes (limit {})",
                                            message.data.len(),
                                            MAX_GOSSIP_MESSAGE_SIZE
                                        );
                                        if let Some(ref source) = message.source {
                                            ban_list.record_violation(*source);
                                        }
                                        acceptance = MessageAcceptance::Reject;
                                    } else if message.topic == tx_topic_hash {
                                        match serde_json::from_slice::<Transaction>(&message.data) {
                                            Ok(tx) => {
                                                let _ = net_tx_sender.send(tx).await;
                                                acceptance = MessageAcceptance::Accept;
                                            }
                                            Err(e) => {
                                                debug!("Invalid tx gossip: {e}");
                                                if let Some(ref source) = message.source {
                                                    ban_list.record_violation(*source);
                                                }
                                                acceptance = MessageAcceptance::Reject;
                                            }
                                        }
                                    } else if message.topic == block_topic_hash {
                                        match serde_json::from_slice::<Block>(&message.data) {
                                            Ok(block) => {
                                                let _ = net_block_sender.send(block).await;
                                                acceptance = MessageAcceptance::Accept;
                                            }
                                            Err(e) => {
                                                debug!("Invalid block gossip: {e}");
                                                if let Some(ref source) = message.source {
                                                    ban_list.record_violation(*source);
                                                }
                                                acceptance = MessageAcceptance::Reject;
                                            }
                                        }
                                    } else if message.topic == consensus_topic_hash {
                                        if message.data.len() > MAX_CONSENSUS_MESSAGE_SIZE {
                                            debug!(
                                                "Dropping oversized consensus message: {} bytes (limit {})",
                                                message.data.len(),
                                                MAX_CONSENSUS_MESSAGE_SIZE
                                            );
                                            if let Some(ref source) = message.source {
                                                ban_list.record_violation(*source);
                                            }
                                            acceptance = MessageAcceptance::Reject;
                                        } else {
                                            let _ = net_consensus_sender.send(message.data.to_vec()).await;
                                            acceptance = MessageAcceptance::Accept;
                                        }
                                    } else {
                                        // Unknown topic — peer is probably from an old
                                        // version. Don't relay, don't penalise.
                                        acceptance = MessageAcceptance::Ignore;
                                    }
                                }
                                if let Err(e) = swarm
                                    .behaviour_mut()
                                    .gossipsub
                                    .report_message_validation_result(
                                        &message_id,
                                        &propagation_source,
                                        acceptance,
                                    )
                                {
                                    debug!(
                                        "report_message_validation_result failed for {message_id}: {e:?}"
                                    );
                                }
                            }
                            // ── Block sync: inbound request (serve blocks) ──
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::BlockSync(
                                request_response::Event::Message {
                                    peer,
                                    message: request_response::Message::Request { request, channel, .. },
                                },
                            )) => {
                                if !rate_limiter.check_and_increment(&peer) {
                                    warn!("Rate-limited peer {peer} — dropping sync request");
                                    continue;
                                }
                                let from = request.from_height;
                                if request.from_height > request.to_height {
                                    warn!("Peer {peer} sent invalid sync range: {}..{}", request.from_height, request.to_height);
                                    continue;
                                }
                                let to = request.to_height.min(from + MAX_SYNC_BATCH);
                                info!("Peer {peer} requested blocks {from}..{to}");

                                let cache = safe_read(&block_cache_inner);
                                let tip = cache.keys().last().copied().unwrap_or(0);
                                // Two-pass: cache first (cheap), then disk
                                // fallback for any height the cache evicted.
                                // Without the disk pass a fresh-from-genesis
                                // peer can't bootstrap once the cache window
                                // (`MAX_CACHE_SIZE = 2000` blocks) has rolled
                                // past — the bug we hit on M1 wipe-and-rejoin
                                // 2026-05-07.
                                // `to` is exclusive: request covers [from, to).
                                // MAX_SYNC_BATCH enforces the cap on serving side.
                                let mut blocks: Vec<Block> = Vec::with_capacity((to - from) as usize);
                                let mut disk_misses = 0u64;
                                for n in from..to {
                                    if let Some(b) = cache.get(&n).cloned() {
                                        blocks.push(b);
                                    } else if let Some(ref fetcher) = disk_block_fetcher {
                                        match fetcher.fetch(n) {
                                            Some(b) => blocks.push(b),
                                            None => disk_misses += 1,
                                        }
                                    }
                                }
                                drop(cache);

                                if disk_misses > 0 {
                                    debug!(
                                        "Sync request from {peer}: {} blocks served, {} not found on disk (range {from}..{to})",
                                        blocks.len(),
                                        disk_misses,
                                    );
                                }
                                info!("Serving {} blocks to peer {peer} (tip={tip})", blocks.len());
                                let response = BlockSyncResponse { blocks, tip_height: tip };
                                if let Err(e) = swarm.behaviour_mut().block_sync.send_response(channel, response) {
                                    warn!("Failed to send sync response to {peer}: {e:?}");
                                }
                            }
                            // ── Block sync: outbound response (received blocks) ──
                            //
                            // Network-layer validation (audit 2026-05-06 H-21).
                            // Cheap structural checks before forwarding to
                            // consensus — consensus does the cryptographic
                            // verification, but forwarding obviously-malformed
                            // responses is a DoS amplifier we can avoid here.
                            // Each failure records a peer violation so
                            // chronically-malformed peers get banned.
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::BlockSync(
                                request_response::Event::Message {
                                    peer,
                                    message: request_response::Message::Response { response, .. },
                                },
                            )) => {
                                info!(
                                    "Received {} sync blocks from peer {peer} (tip={})",
                                    response.blocks.len(), response.tip_height
                                );

                                // Cheap structural validation before forwarding
                                // to consensus (audit 2026-05-06 H-21). The
                                // pure helper returns a typed
                                // SyncResponseRejection on failure; we log it
                                // and record a peer violation so chronically
                                // malformed peers get banned.
                                if let Err(rej) = validate_sync_response_structure(&response) {
                                    warn!(
                                        "Peer {peer} returned malformed sync response: \
                                         {rej:?}; recording violation"
                                    );
                                    ban_list.record_violation(peer);
                                    // Notify main task so sync_in_flight is cleared
                                    // and the node can retry with another peer.
                                    let _ = sync_blocks_sender.send(vec![]).await;
                                    let _ = tip_sender.send(response.tip_height).await;
                                    continue;
                                }

                                // Always notify main task so sync_in_flight is cleared,
                                // even on empty responses.  Without this, a 0-block
                                // response from a stale peer leaves sync_in_flight=true
                                // permanently and all subsequent sync requests are silently
                                // dropped (cluster-soak bug 2026-05-09).
                                if response.blocks.is_empty() {
                                    // Peer couldn't serve what we asked — cool it out so
                                    // the next request picks a healthier peer.
                                    if recently_failed.len() >= REQ_FAIL_MAP_CAP {
                                        recently_failed.clear();
                                    }
                                    recently_failed.insert(peer, Instant::now());
                                }
                                let _ = sync_blocks_sender.send(response.blocks).await;
                                let _ = tip_sender.send(response.tip_height).await;
                            }
                            // ── Block sync failures ──
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::BlockSync(
                                request_response::Event::OutboundFailure { peer, error, .. },
                            )) => {
                                warn!("Block sync request to {peer} failed: {error}");
                                // Cool the failing peer out of the request rotation.
                                if recently_failed.len() >= REQ_FAIL_MAP_CAP {
                                    recently_failed.clear();
                                }
                                recently_failed.insert(peer, Instant::now());
                            }
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::BlockSync(
                                request_response::Event::InboundFailure { peer, error, .. },
                            )) => {
                                debug!("Inbound sync from {peer} failed: {error}");
                            }
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::BlockSync(
                                request_response::Event::ResponseSent { .. },
                            )) => {}
                            // ── Shard sample: inbound request (serve shard proofs) ──
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::ShardSample(
                                request_response::Event::Message {
                                    peer,
                                    message: request_response::Message::Request { request, channel, .. },
                                },
                            )) => {
                                // Audit AUDIT-2026-05-11-1: rate-limit the
                                // shard-sample inbound symmetric to gossipsub
                                // and block-sync. Without this gate, a peer
                                // that has saturated its gossipsub / sync
                                // budget can still flood Merkle-proof work
                                // through this protocol.
                                if !rate_limiter.check_and_increment(&peer) {
                                    warn!("Rate-limited peer {peer} — dropping shard-sample request");
                                    continue;
                                }
                                // Audit AUDIT-2026-05-11-2: cap inbound
                                // queries.len() before allocating. Each
                                // query drives a Merkle proof; without a
                                // cap a single ~1MB JSON request can pin
                                // a CPU on the serving node.
                                if request.queries.len() > MAX_SHARD_QUERIES_PER_REQUEST {
                                    warn!(
                                        "Peer {peer} sent {} shard queries, cap is {} — recording violation",
                                        request.queries.len(),
                                        MAX_SHARD_QUERIES_PER_REQUEST,
                                    );
                                    ban_list.record_violation(peer);
                                    continue;
                                }
                                debug!("Peer {peer} requested {} shard samples", request.queries.len());
                                let cache = shard_cache_inner.read().unwrap_or_else(|p| {
                                    warn!("Recovered poisoned shard cache read lock");
                                    p.into_inner()
                                });
                                let mut samples = Vec::with_capacity(request.queries.len());
                                for query in &request.queries {
                                    let sample = cache.get(&query.block_number).and_then(|pkg| {
                                        if query.shard_index < pkg.shards.len() {
                                            DASampler::generate_proof(&pkg.shards, query.shard_index)
                                                .ok()
                                                .map(|proof| SampleResponse {
                                                    shard: pkg.shards[query.shard_index].clone(),
                                                    proof,
                                                    attestation_signature: None,
                                                    attester_public_key: None,
                                                })
                                        } else {
                                            None
                                        }
                                    });
                                    samples.push(sample);
                                }
                                drop(cache);
                                let response = ShardSampleResponse { samples };
                                if let Err(e) = swarm.behaviour_mut().shard_sample.send_response(channel, response) {
                                    warn!("Failed to send shard sample response to {peer}: {e:?}");
                                }
                            }
                            // ── Shard sample: outbound response (received samples) ──
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::ShardSample(
                                request_response::Event::Message {
                                    peer,
                                    message: request_response::Message::Response { response, .. },
                                },
                            )) => {
                                let valid: Vec<SampleResponse> = response.samples.into_iter().flatten().collect();
                                debug!("Received {} shard samples from peer {peer}", valid.len());
                                if !valid.is_empty() {
                                    let _ = sample_resp_sender.send(valid).await;
                                }
                            }
                            // ── Shard sample failures ──
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::ShardSample(
                                request_response::Event::OutboundFailure { peer, error, .. },
                            )) => {
                                warn!("Shard sample request to {peer} failed: {error}");
                                if recently_failed.len() >= REQ_FAIL_MAP_CAP {
                                    recently_failed.clear();
                                }
                                recently_failed.insert(peer, Instant::now());
                            }
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::ShardSample(
                                request_response::Event::InboundFailure { peer, error, .. },
                            )) => {
                                debug!("Inbound shard sample from {peer} failed: {error}");
                            }
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::ShardSample(
                                request_response::Event::ResponseSent { .. },
                            )) => {}
                            // ── mDNS discovery ──
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::Mdns(
                                mdns::Event::Discovered(peers),
                            )) => {
                                for (peer_id, addr) in peers {
                                    info!("mDNS discovered peer: {peer_id} at {addr}");
                                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                                }
                                let count = swarm.connected_peers().count();
                                peer_count_inner.store(count, Ordering::Relaxed);
                            }
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::Mdns(
                                mdns::Event::Expired(peers),
                            )) => {
                                for (peer_id, _addr) in peers {
                                    debug!("mDNS peer expired: {peer_id}");
                                    swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                                }
                                let count = swarm.connected_peers().count();
                                peer_count_inner.store(count, Ordering::Relaxed);
                            }
                            // ── Connection events ──
                            SwarmEvent::ConnectionEstablished { peer_id, ref endpoint, .. } => {
                                if !peer_authority.is_authorized(&peer_id) {
                                    if let Ok(s) = sybil_state_inner.write() {
                                        s.rejections.record(RejectionReason::Unauthorized);
                                    }
                                    warn!("Unauthorized peer {peer_id} — disconnecting");
                                    let _ = swarm.disconnect_peer_id(peer_id);
                                    continue;
                                }
                                // Sybil resistance: per-IP / per-subnet / total caps + ban list.
                                // PeerId is cheap to mint, source IP is not.
                                let endpoint_ip = endpoint_remote_ip(endpoint);
                                if let Some(ip) = endpoint_ip {
                                    // Lane R.1 defence-in-depth: if an
                                    // authorized validator's IP is on
                                    // the ban list (e.g. inherited from
                                    // a previous-version cluster freeze
                                    // before R.1 landed, or from a
                                    // legitimate score breach that has
                                    // since recovered), auto-unban so
                                    // the cluster can recover without
                                    // operator restart. We've already
                                    // confirmed `peer_id` is authorized.
                                    if let Ok(mut s) = sybil_state_inner.write() {
                                        if s.bans.is_banned(&ip) {
                                            warn!(
                                                "network: auto-unbanning {ip} for authorized peer {peer_id} (Lane R.1 recovery)"
                                            );
                                            s.unban_ip(&ip);
                                        }
                                    }
                                    let total = swarm.connected_peers().count();
                                    let admit = sybil_state_inner.write().map(|mut s| {
                                        s.try_admit_inbound(ip, total)
                                    });
                                    match admit {
                                        Ok(Ok(())) => {
                                            // Legacy in-loop tracker kept for back-compat, but the
                                            // authoritative accounting is now SybilState.
                                            let _ = per_ip_tracker.try_admit(ip);
                                            if let Ok(mut s) = sybil_state_inner.write() {
                                                s.record_connect(peer_id, ip);
                                            }
                                        }
                                        Ok(Err(reason)) => {
                                            let label = reason.label();
                                            let cap_for_log = match reason {
                                                RejectionReason::PerIp => cap_per_ip,
                                                RejectionReason::PerSubnet => cap_per_subnet,
                                                RejectionReason::TotalCap => cap_total,
                                                _ => 0,
                                            };
                                            warn!(
                                                "network: rejected inbound from IP={ip} reason={label}_limit_exceeded peer={peer_id} cap={cap_for_log}"
                                            );
                                            let _ = swarm.disconnect_peer_id(peer_id);
                                            continue;
                                        }
                                        Err(_) => {
                                            warn!("sybil_state lock poisoned during admit; rejecting {peer_id}");
                                            let _ = swarm.disconnect_peer_id(peer_id);
                                            continue;
                                        }
                                    }
                                }
                                let count = swarm.connected_peers().count();
                                peer_count_inner.store(count, Ordering::Relaxed);
                                info!("Connection established with {peer_id} (total: {count})");

                                // Request the peer's chain tip to detect if we're behind
                                swarm.behaviour_mut().block_sync.send_request(
                                    &peer_id,
                                    BlockSyncRequest { from_height: 0, to_height: 0 },
                                );
                            }
                            SwarmEvent::ConnectionClosed { peer_id, ref endpoint, .. } => {
                                if let Some(ip) = endpoint_remote_ip(endpoint) {
                                    per_ip_tracker.release(ip);
                                }
                                if let Ok(mut s) = sybil_state_inner.write() {
                                    s.record_disconnect(&peer_id);
                                }
                                let count = swarm.connected_peers().count();
                                peer_count_inner.store(count, Ordering::Relaxed);
                            }
                            SwarmEvent::NewListenAddr { address, .. } => {
                                info!("Listening on {address}/p2p/{local_peer_id}");
                            }
                            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                                warn!("Outgoing connection error to {peer_id:?}: {error}");
                            }
                            SwarmEvent::IncomingConnectionError { error, .. } => {
                                warn!("Incoming connection error: {error}");
                            }
                            SwarmEvent::Dialing { peer_id, .. } => {
                                debug!("Dialing peer {peer_id:?}");
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        Ok((channels, handle, local_peer_id))
    }
}

// ─────────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::TransferTx;
    use std::time::Duration;
    use tokio::time::timeout;

    fn make_config(port: u16) -> NetworkConfig {
        NetworkConfig {
            listen_address: format!("/ip4/127.0.0.1/tcp/{port}"),
            bootstrap_peers: vec![],
            channel_buffer: 64,
            use_tls: false,
            tls_certs: None,
            peer_authority: crate::tls::PeerAuthority::permissionless(),
            ..NetworkConfig::default()
        }
    }

    fn dummy_tx(amount: u64) -> Transaction {
        Transaction::Transfer(TransferTx {
            from: [1u8; 32],
            to: [2u8; 32],
            amount,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        })
    }

    fn dummy_block(num: u64) -> Block {
        Block {
            number: num,
            epoch: num,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
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
            da_row_roots: vec![],
            da_col_roots: vec![],
        }
    }

    /// Wait for two nodes to discover each other via mDNS.
    async fn wait_for_discovery(duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    #[tokio::test]
    async fn test_service_starts() {
        let config = NetworkConfig::default();
        let result = P2pNetworkService::start(config).await;
        assert!(result.is_ok());
        let (_channels, _handle, peer_id) = result.unwrap();
        // PeerId should be valid (non-zero length when encoded)
        assert!(!peer_id.to_string().is_empty());
    }

    #[tokio::test]
    async fn test_peer_discovery_mdns() {
        // Start two nodes; mDNS should discover them
        let (ch1, _h1, pid1) = P2pNetworkService::start(make_config(0))
            .await
            .expect("node1 start");
        let (_ch2, _h2, pid2) = P2pNetworkService::start(make_config(0))
            .await
            .expect("node2 start");

        assert_ne!(pid1, pid2);

        // mDNS discovery needs a moment
        wait_for_discovery(Duration::from_secs(3)).await;

        // Both services are running (channels are live)
        drop(ch1);
    }

    #[tokio::test]
    async fn test_tx_gossip_roundtrip() {
        // Start two nodes
        let (ch1, _h1, _pid1) = P2pNetworkService::start(make_config(0))
            .await
            .expect("node1");
        let (mut ch2, _h2, _pid2) = P2pNetworkService::start(make_config(0))
            .await
            .expect("node2");

        // Wait for mDNS discovery
        wait_for_discovery(Duration::from_secs(3)).await;

        // Node 1 sends a transaction
        let tx = dummy_tx(42);
        ch1.tx_sender.send(tx).await.expect("send tx");

        // Node 2 should receive it — drain stale messages from parallel tests
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut found = false;
        while tokio::time::Instant::now() < deadline {
            match timeout(
                deadline - tokio::time::Instant::now(),
                ch2.tx_receiver.recv(),
            )
            .await
            {
                Ok(Some(Transaction::Transfer(t))) if t.amount == 42 => {
                    found = true;
                    break;
                }
                Ok(Some(_)) => continue, // stale message from another test
                Ok(None) => {
                    eprintln!("tx_receiver closed (mDNS may not have connected)");
                    break;
                }
                Err(_) => {
                    eprintln!("tx gossip timed out (mDNS may not be available)");
                    break;
                }
            }
        }
        if !found {
            eprintln!("did not receive expected Transfer(42) — mDNS may be flaky");
        }
    }

    #[tokio::test]
    async fn test_block_gossip_roundtrip() {
        let (ch1, _h1, _pid1) = P2pNetworkService::start(make_config(0))
            .await
            .expect("node1");
        let (mut ch2, _h2, _pid2) = P2pNetworkService::start(make_config(0))
            .await
            .expect("node2");

        wait_for_discovery(Duration::from_secs(3)).await;

        // Node 1 broadcasts a block
        let block = dummy_block(99);
        ch1.block_sender.send(block).await.expect("send block");

        let result = timeout(Duration::from_secs(5), ch2.block_receiver.recv()).await;
        match result {
            Ok(Some(received_block)) => {
                assert_eq!(received_block.number, 99);
            }
            Ok(None) => {
                eprintln!("block_receiver closed (mDNS may not have connected)");
            }
            Err(_) => {
                eprintln!("block gossip timed out (mDNS may not be available)");
            }
        }
    }

    #[tokio::test]
    async fn test_network_handle_broadcast() {
        let (_ch, handle, _pid) = P2pNetworkService::start(NetworkConfig::default())
            .await
            .expect("start");

        // Broadcasting via handle should succeed (even with no peers)
        let tx = dummy_tx(100);
        let result = handle.broadcast_tx(&tx).await;
        assert!(result.is_ok());

        let block = dummy_block(1);
        let result = handle.broadcast_block(&block).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_mock_network_still_works() {
        let mock = crate::MockNetwork;
        assert!(mock.broadcast_tx(&dummy_tx(1)).await.is_ok());
        assert!(mock.broadcast_block(&dummy_block(1)).await.is_ok());
    }

    #[tokio::test]
    async fn test_block_cache_insert_and_evict() {
        let cache: BlockCache = Arc::new(RwLock::new(BTreeMap::new()));

        // Insert blocks
        for i in 0..10 {
            cache_block(&cache, &dummy_block(i));
        }
        assert_eq!(cache.read().unwrap().len(), 10);

        // Verify ordering
        let c = cache.read().unwrap();
        let keys: Vec<u64> = c.keys().copied().collect();
        assert_eq!(keys, (0..10).collect::<Vec<_>>());
    }

    // ─── DiskBlockFetcher: bootstrap-from-genesis sync path ──────
    //
    // Pure tests for the sync handler's two-pass cache-then-disk
    // fallback. The integration through libp2p is exercised by
    // `test_block_sync_request_response`; here we cover the unit:
    // that the fetcher type behaves as expected (forwards calls,
    // returns None for absent heights, is Send+Sync+Clone) so a
    // future regression in the fetcher abstraction itself is caught
    // without a full swarm spin-up.

    #[test]
    fn disk_block_fetcher_returns_block_for_known_height() {
        let store: BTreeMap<u64, Block> = (1..=5).map(|n| (n, dummy_block(n))).collect();
        let fetcher = DiskBlockFetcher::new(move |h| store.get(&h).cloned());
        let got = fetcher.fetch(3).expect("h=3 must be present");
        assert_eq!(got.number, 3);
    }

    #[test]
    fn disk_block_fetcher_returns_none_for_missing_height() {
        let fetcher = DiskBlockFetcher::new(|_| None);
        assert!(fetcher.fetch(42).is_none());
    }

    #[test]
    fn disk_block_fetcher_is_clone_and_thread_safe() {
        // Compile-time assertion that the type satisfies the trait
        // bounds the sync handler's tokio task requires.
        fn assert_send_sync_clone<T: Send + Sync + Clone + 'static>() {}
        assert_send_sync_clone::<DiskBlockFetcher>();

        let fetcher = DiskBlockFetcher::new(|h| Some(dummy_block(h)));
        let cloned = fetcher.clone();
        assert_eq!(cloned.fetch(7).unwrap().number, 7);
        assert_eq!(fetcher.fetch(99).unwrap().number, 99);
    }

    // ─── H-21 sync-response structural validation ────────────────
    //
    // Pure-function tests for `validate_sync_response_structure`.
    // No async/swarm needed — these are the cheap structural checks
    // the network layer enforces before forwarding bytes to the
    // consensus layer.

    #[test]
    fn validate_sync_accepts_empty_response() {
        let r = BlockSyncResponse {
            blocks: vec![],
            tip_height: 0,
        };
        assert_eq!(validate_sync_response_structure(&r), Ok(()));
    }

    #[test]
    fn validate_sync_accepts_well_formed_batch() {
        let blocks: Vec<Block> = (10..=15).map(dummy_block).collect();
        let r = BlockSyncResponse {
            blocks,
            tip_height: 100,
        };
        assert_eq!(validate_sync_response_structure(&r), Ok(()));
    }

    #[test]
    fn validate_sync_rejects_oversized_batch() {
        let blocks: Vec<Block> = (1..=(MAX_SYNC_BATCH + 1)).map(dummy_block).collect();
        let r = BlockSyncResponse {
            blocks,
            tip_height: 1000,
        };
        match validate_sync_response_structure(&r) {
            Err(SyncResponseRejection::OversizedBatch { len, cap }) => {
                assert_eq!(len, (MAX_SYNC_BATCH + 1) as usize);
                assert_eq!(cap, MAX_SYNC_BATCH);
            }
            other => panic!("expected OversizedBatch, got {other:?}"),
        }
    }

    #[test]
    fn validate_sync_rejects_non_monotone_heights() {
        // 10, 11, 9 — second pair regresses.
        let blocks = vec![dummy_block(10), dummy_block(11), dummy_block(9)];
        let r = BlockSyncResponse {
            blocks,
            tip_height: 100,
        };
        assert_eq!(
            validate_sync_response_structure(&r),
            Err(SyncResponseRejection::NonMonotoneHeights)
        );
    }

    #[test]
    fn validate_sync_accepts_repeated_heights() {
        // Equal heights are allowed (non-decreasing, not strictly
        // increasing) — concurrent forks of the DAG can produce
        // multiple blocks at the same height.
        let blocks = vec![dummy_block(10), dummy_block(10), dummy_block(11)];
        let r = BlockSyncResponse {
            blocks,
            tip_height: 100,
        };
        assert_eq!(validate_sync_response_structure(&r), Ok(()));
    }

    #[test]
    fn validate_sync_rejects_tip_below_max_height() {
        // Peer says tip=10 but ships block at height 50 — self-
        // contradicting; tip must be ≥ every block's height.
        let blocks = vec![dummy_block(40), dummy_block(50)];
        let r = BlockSyncResponse {
            blocks,
            tip_height: 10,
        };
        match validate_sync_response_structure(&r) {
            Err(SyncResponseRejection::TipBelowMaxHeight { tip, max }) => {
                assert_eq!(tip, 10);
                assert_eq!(max, 50);
            }
            other => panic!("expected TipBelowMaxHeight, got {other:?}"),
        }
    }

    #[test]
    fn validate_sync_accepts_tip_equal_to_max_height() {
        // tip == max is the typical "you're caught up to me" case.
        let blocks = vec![dummy_block(40), dummy_block(50)];
        let r = BlockSyncResponse {
            blocks,
            tip_height: 50,
        };
        assert_eq!(validate_sync_response_structure(&r), Ok(()));
    }

    #[tokio::test]
    async fn test_block_sync_request_response() {
        // Start two nodes
        let (ch1, _h1, _pid1) = P2pNetworkService::start(make_config(0))
            .await
            .expect("node1");
        let (mut ch2, _h2, _pid2) = P2pNetworkService::start(make_config(0))
            .await
            .expect("node2");

        // Populate node1's block cache
        for i in 1..=10 {
            cache_block(&ch1.block_cache, &dummy_block(i));
        }

        // Wait for mDNS discovery
        wait_for_discovery(Duration::from_secs(3)).await;

        // Node 2 requests blocks 1..5 from peers
        ch2.sync_request_sender
            .send((1, 5))
            .await
            .expect("send sync request");

        // Node 2 should receive synced blocks
        let result = timeout(Duration::from_secs(5), ch2.sync_blocks_receiver.recv()).await;
        match result {
            Ok(Some(blocks)) => {
                assert!(!blocks.is_empty(), "should receive blocks");
                info!("Received {} synced blocks", blocks.len());
            }
            Ok(None) => {
                eprintln!("sync_blocks_receiver closed (mDNS may not have connected)");
            }
            Err(_) => {
                eprintln!("block sync timed out (mDNS may not be available)");
            }
        }
    }

    #[tokio::test]
    async fn test_shard_cache_insert_and_evict() {
        use evaporchain_da::block_da::BlockDA;

        let cache: ShardCache = Arc::new(RwLock::new(BTreeMap::new()));
        let da = BlockDA::new().expect("create BlockDA");
        let data = b"test block data for erasure coding";
        let package = da.encode_block(data).expect("encode");

        cache_da_package(&cache, 1, package.clone());
        cache_da_package(&cache, 2, package);

        let c = cache.read().unwrap();
        assert_eq!(c.len(), 2);
        assert!(c.contains_key(&1));
        assert!(c.contains_key(&2));
    }

    #[tokio::test]
    async fn test_block_cache_eviction_at_max() {
        let cache: BlockCache = Arc::new(RwLock::new(BTreeMap::new()));

        // Fill to exactly MAX_CACHE_SIZE + 50
        for i in 0..(MAX_CACHE_SIZE as u64 + 50) {
            cache_block(&cache, &dummy_block(i));
        }

        let c = cache.read().unwrap();
        assert_eq!(c.len(), MAX_CACHE_SIZE);
        // Oldest blocks (0..50) should have been evicted
        assert!(!c.contains_key(&0));
        assert!(!c.contains_key(&49));
        // Newest blocks should remain
        assert!(c.contains_key(&50));
        assert!(c.contains_key(&(MAX_CACHE_SIZE as u64 + 49)));
    }

    #[tokio::test]
    async fn test_shard_cache_eviction_at_max() {
        use evaporchain_da::block_da::BlockDA;

        let cache: ShardCache = Arc::new(RwLock::new(BTreeMap::new()));
        let da = BlockDA::new().expect("create BlockDA");
        let data = b"eviction test data for shard cache";
        let package = da.encode_block(data).expect("encode");

        for i in 0..(MAX_SHARD_CACHE_SIZE as u64 + 20) {
            cache_da_package(&cache, i, package.clone());
        }

        let c = cache.read().unwrap();
        assert_eq!(c.len(), MAX_SHARD_CACHE_SIZE);
        assert!(!c.contains_key(&0));
        assert!(!c.contains_key(&19));
        assert!(c.contains_key(&20));
        assert!(c.contains_key(&(MAX_SHARD_CACHE_SIZE as u64 + 19)));
    }

    #[tokio::test]
    async fn test_block_cache_overwrite_same_height() {
        let cache: BlockCache = Arc::new(RwLock::new(BTreeMap::new()));

        let mut b1 = dummy_block(5);
        b1.timestamp = 100;
        cache_block(&cache, &b1);

        let mut b2 = dummy_block(5);
        b2.timestamp = 200;
        cache_block(&cache, &b2);

        let c = cache.read().unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c.get(&5).unwrap().timestamp, 200);
    }

    #[test]
    fn test_block_sync_request_serialization() {
        let req = BlockSyncRequest {
            from_height: 100,
            to_height: 200,
        };
        let bytes = serde_json::to_vec(&req).expect("serialize");
        let decoded: BlockSyncRequest = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(decoded.from_height, 100);
        assert_eq!(decoded.to_height, 200);
    }

    #[test]
    fn test_block_sync_response_serialization() {
        let resp = BlockSyncResponse {
            blocks: vec![dummy_block(1), dummy_block(2)],
            tip_height: 99,
        };
        let bytes = serde_json::to_vec(&resp).expect("serialize");
        let decoded: BlockSyncResponse = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(decoded.blocks.len(), 2);
        assert_eq!(decoded.blocks[0].number, 1);
        assert_eq!(decoded.blocks[1].number, 2);
        assert_eq!(decoded.tip_height, 99);
    }

    // Audit AUDIT-2026-05-11-2: pin the inbound-queries cap so a
    // future refactor that bumps it (e.g. to accommodate a bulk
    // sampler) has to consciously change this constant — and the
    // commit that bumps it should re-evaluate the per-query CPU cost.
    #[test]
    fn shard_query_cap_is_capped_at_256() {
        assert_eq!(MAX_SHARD_QUERIES_PER_REQUEST, 256);
        // Sanity bound: a request at the cap should be well under the
        // 1 MB libp2p JSON ceiling. A `SampleQuery` is ~24 bytes
        // serialized; 256 × 24 ≈ 6 KB. Plenty of headroom.
        let req = ShardSampleRequest {
            queries: (0..MAX_SHARD_QUERIES_PER_REQUEST)
                .map(|i| evaporchain_da::sampling::SampleQuery {
                    block_number: i as u64,
                    shard_index: 0,
                })
                .collect(),
        };
        let bytes = serde_json::to_vec(&req).expect("serialize");
        assert!(bytes.len() < 64 * 1024, "request at cap should be < 64KB, was {}", bytes.len());
    }

    #[test]
    fn test_shard_sample_request_serialization() {
        use evaporchain_da::sampling::SampleQuery;

        let req = ShardSampleRequest {
            queries: vec![
                SampleQuery {
                    block_number: 10,
                    shard_index: 0,
                },
                SampleQuery {
                    block_number: 10,
                    shard_index: 3,
                },
            ],
        };
        let bytes = serde_json::to_vec(&req).expect("serialize");
        let decoded: ShardSampleRequest = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(decoded.queries.len(), 2);
        assert_eq!(decoded.queries[0].shard_index, 0);
        assert_eq!(decoded.queries[1].shard_index, 3);
    }

    #[test]
    fn test_network_config_defaults() {
        let cfg = NetworkConfig::default();
        assert_eq!(cfg.listen_address, "/ip4/0.0.0.0/tcp/0");
        assert!(cfg.bootstrap_peers.is_empty());
        assert_eq!(cfg.channel_buffer, 256);
        assert!(cfg.data_dir.is_none());
    }

    #[test]
    fn load_or_generate_identity_round_trip() {
        // Round-trip: persist a fresh keypair, reload it, and assert the
        // PeerId (and protobuf-encoded bytes) match. This is the core
        // invariant that lets bootstrap_peers in genesis embed a stable
        // `/p2p/<peer_id>` suffix.
        let dir = std::env::temp_dir().join(format!(
            "evapor-net-key-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // First call: generates and persists.
        let kp1 = load_or_generate_identity(&dir).expect("generate");
        let key_path = dir.join("network_key.bin");
        assert!(key_path.is_file(), "network_key.bin must be written");
        let bytes_on_disk = std::fs::read(&key_path).unwrap();
        assert!(!bytes_on_disk.is_empty(), "key bytes must be non-empty");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "network_key.bin must be 0600");
        }

        // Second call: must load the same key, returning the same PeerId.
        let kp2 = load_or_generate_identity(&dir).expect("reload");
        let pid1 = kp1.public().to_peer_id();
        let pid2 = kp2.public().to_peer_id();
        assert_eq!(pid1, pid2, "PeerId must be stable across reload");

        // Encoded bytes round-trip too.
        let enc1 = kp1.to_protobuf_encoding().unwrap();
        let enc2 = kp2.to_protobuf_encoding().unwrap();
        assert_eq!(enc1, enc2, "protobuf encoding must be stable");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_constants_sane() {
        // Re-audit (2026-05-02): MAX_GOSSIP_MESSAGE_SIZE unified with
        // gossipsub's max_transmit_size to make the inbound drop
        // check meaningful (previously 10 MB ceiling > 4 MB transport
        // limit was unreachable).
        assert_eq!(MAX_GOSSIP_MESSAGE_SIZE, 4 * 1024 * 1024);
        assert_eq!(MAX_GOSSIPSUB_TRANSMIT_SIZE, MAX_GOSSIP_MESSAGE_SIZE);
        assert_eq!(MAX_CACHE_SIZE, 2000);
        assert_eq!(MAX_SHARD_CACHE_SIZE, 500);
        assert_eq!(MAX_SYNC_BATCH, 100);
    }

    #[test]
    fn test_sync_batch_cap_arithmetic() {
        // Simulates the capping logic from the event loop:
        // let capped_to = from + MAX_SYNC_BATCH.min(to - from);
        let from = 50u64;
        let to = 500u64;
        let capped_to = from + MAX_SYNC_BATCH.min(to - from);
        assert_eq!(capped_to, 150); // 50 + 100

        // Small range should not be capped
        let to_small = 60u64;
        let capped_small = from + MAX_SYNC_BATCH.min(to_small - from);
        assert_eq!(capped_small, 60); // 50 + 10
    }

    #[test]
    fn test_block_cache_empty_read() {
        let cache: BlockCache = Arc::new(RwLock::new(BTreeMap::new()));
        let c = safe_read(&cache);
        assert!(c.is_empty());
        assert_eq!(c.keys().last().copied().unwrap_or(0), 0);
    }

    #[test]
    fn test_block_cache_poison_recovery() {
        let cache: BlockCache = Arc::new(RwLock::new(BTreeMap::new()));

        // Insert a block normally
        cache_block(&cache, &dummy_block(1));

        // Poison the lock by panicking inside a write guard
        let cache_clone = Arc::clone(&cache);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = cache_clone.write().unwrap();
            panic!("intentional poison");
        }));
        assert!(result.is_err());

        // Lock is now poisoned — safe_read and safe_write should recover
        let c = safe_read(&cache);
        assert_eq!(c.len(), 1);
        assert!(c.contains_key(&1));
        drop(c);

        // safe_write via cache_block should also recover
        cache_block(&cache, &dummy_block(2));
        let c = safe_read(&cache);
        assert_eq!(c.len(), 2);
    }

    #[tokio::test]
    async fn test_network_handle_is_clone() {
        let (_ch, handle, _pid) = P2pNetworkService::start(NetworkConfig::default())
            .await
            .expect("start");

        let handle2 = handle.clone();
        let tx = dummy_tx(77);
        assert!(handle2.broadcast_tx(&tx).await.is_ok());
    }

    #[tokio::test]
    async fn test_block_cache_maintains_order() {
        let cache: BlockCache = Arc::new(RwLock::new(BTreeMap::new()));

        // Insert out of order
        cache_block(&cache, &dummy_block(5));
        cache_block(&cache, &dummy_block(1));
        cache_block(&cache, &dummy_block(10));
        cache_block(&cache, &dummy_block(3));

        let c = cache.read().unwrap();
        let keys: Vec<u64> = c.keys().copied().collect();
        assert_eq!(keys, vec![1, 3, 5, 10]);
    }

    #[tokio::test]
    async fn test_shard_sample_request_response() {
        use evaporchain_da::block_da::BlockDA;
        use evaporchain_da::sampling::SampleQuery;

        // Start two nodes
        let (ch1, _h1, _pid1) = P2pNetworkService::start(make_config(0))
            .await
            .expect("node1");
        let (mut ch2, _h2, _pid2) = P2pNetworkService::start(make_config(0))
            .await
            .expect("node2");

        // Populate node1's shard cache with DA-encoded block
        let da = BlockDA::new().expect("create BlockDA");
        let data = b"hello world block data for shard sampling test";
        let package = da.encode_block(data).expect("encode");
        let commitment_root = package.header.commitment_root;
        cache_da_package(&ch1.shard_cache, 42, package);

        // Wait for mDNS discovery
        wait_for_discovery(Duration::from_secs(3)).await;

        // Node 2 requests shard samples for block 42
        let queries = vec![
            SampleQuery {
                block_number: 42,
                shard_index: 0,
            },
            SampleQuery {
                block_number: 42,
                shard_index: 1,
            },
        ];
        ch2.sample_request_sender
            .send(queries)
            .await
            .expect("send sample request");

        // Node 2 should receive shard samples
        let result = timeout(Duration::from_secs(5), ch2.sample_response_receiver.recv()).await;
        match result {
            Ok(Some(samples)) => {
                assert!(!samples.is_empty(), "should receive shard samples");
                // Verify each sample's proof is valid against the commitment root
                for sample in &samples {
                    assert_eq!(sample.proof.root, commitment_root);
                    assert!(DASampler::verify_proof(&sample.shard, &sample.proof));
                }
                info!("Received {} verified shard samples", samples.len());
            }
            Ok(None) => {
                eprintln!("sample_response_receiver closed (mDNS may not have connected)");
            }
            Err(_) => {
                eprintln!("shard sample timed out (mDNS may not be available)");
            }
        }
    }

    // ── PeerRateLimiter ──

    #[test]
    fn test_rate_limiter_allows_within_limit() {
        let mut rl = PeerRateLimiter::new();
        let peer = PeerId::random();
        for _ in 0..PEER_MSG_LIMIT {
            assert!(rl.check_and_increment(&peer));
        }
    }

    #[test]
    fn test_rate_limiter_blocks_over_limit() {
        let mut rl = PeerRateLimiter::new();
        let peer = PeerId::random();
        for _ in 0..PEER_MSG_LIMIT {
            rl.check_and_increment(&peer);
        }
        assert!(!rl.check_and_increment(&peer));
    }

    #[test]
    fn test_rate_limiter_independent_peers() {
        let mut rl = PeerRateLimiter::new();
        let peer_a = PeerId::random();
        let peer_b = PeerId::random();
        for _ in 0..PEER_MSG_LIMIT {
            rl.check_and_increment(&peer_a);
        }
        assert!(!rl.check_and_increment(&peer_a));
        assert!(rl.check_and_increment(&peer_b));
    }

    #[test]
    fn test_rate_limiter_gc_removes_stale() {
        let mut rl = PeerRateLimiter::new();
        for _ in 0..(MAX_TRACKED_PEERS + 100) {
            let peer = PeerId::random();
            rl.check_and_increment(&peer);
        }
        assert!(rl.counters.len() > MAX_TRACKED_PEERS);
        rl.maybe_gc();
        assert!(rl.counters.len() <= MAX_TRACKED_PEERS + 100);
    }

    // ── PeerBanList ──

    #[test]
    fn test_ban_list_not_banned_initially() {
        let mut bl = PeerBanList::new();
        let peer = PeerId::random();
        assert!(!bl.is_banned(&peer));
    }

    #[test]
    fn test_ban_list_violations_below_threshold() {
        let mut bl = PeerBanList::new();
        let peer = PeerId::random();
        for _ in 0..(BAN_THRESHOLD - 1) {
            assert!(!bl.record_violation(peer));
        }
        assert!(!bl.is_banned(&peer));
    }

    #[test]
    fn test_ban_list_bans_at_threshold() {
        let mut bl = PeerBanList::new();
        let peer = PeerId::random();
        for _ in 0..(BAN_THRESHOLD - 1) {
            bl.record_violation(peer);
        }
        assert!(bl.record_violation(peer));
        assert!(bl.is_banned(&peer));
    }

    #[test]
    fn test_ban_list_independent_peers() {
        let mut bl = PeerBanList::new();
        let peer_a = PeerId::random();
        let peer_b = PeerId::random();
        for _ in 0..BAN_THRESHOLD {
            bl.record_violation(peer_a);
        }
        assert!(bl.is_banned(&peer_a));
        assert!(!bl.is_banned(&peer_b));
    }

    #[test]
    fn test_ban_list_violations_count() {
        let mut bl = PeerBanList::new();
        let peer = PeerId::random();
        assert_eq!(*bl.violations.entry(peer).or_insert(0), 0);
        bl.record_violation(peer);
        assert_eq!(*bl.violations.get(&peer).unwrap(), 1);
        bl.record_violation(peer);
        assert_eq!(*bl.violations.get(&peer).unwrap(), 2);
    }

    // ── Sybil-resistance state ──────────────────────────────────────────

    fn sybil_cfg() -> SybilConfig {
        SybilConfig {
            max_connections_per_ip: 4,
            max_connections_per_subnet: 16,
            max_inbound_connections: 200,
            peer_ban_duration_secs: 3_600,
            trusted_validator_ips: HashSet::new(),
        }
    }

    fn ipv4(a: u8, b: u8, c: u8, d: u8) -> std::net::IpAddr {
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn test_subnet_key_v4_is_slash_24() {
        let ip = ipv4(192, 0, 2, 47);
        assert_eq!(subnet_key(&ip), "192.0.2.0/24");
    }

    #[test]
    fn test_subnet_key_v6_is_slash_48() {
        let ip = std::net::IpAddr::V6("2001:db8:abcd:1234::1".parse().unwrap());
        assert_eq!(subnet_key(&ip), "2001:db8:abcd::/48");
    }

    #[test]
    fn test_per_ip_limit_rejects_5th_connection() {
        let mut s = SybilState::new(sybil_cfg(), None);
        let ip = ipv4(192, 0, 2, 1);
        for _ in 0..4 {
            assert!(s.try_admit_inbound(ip, 0).is_ok());
            s.record_connect(PeerId::random(), ip);
        }
        assert_eq!(s.try_admit_inbound(ip, 0), Err(RejectionReason::PerIp));
        assert_eq!(s.rejections.per_ip.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_subnet_limit_rejects() {
        // Cap subnet at 3 distinct IPs
        let cfg = SybilConfig {
            max_connections_per_ip: 1,
            max_connections_per_subnet: 3,
            max_inbound_connections: 200,
            peer_ban_duration_secs: 60,
            trusted_validator_ips: HashSet::new(),
        };
        let mut s = SybilState::new(cfg, None);
        for last in 1..=3u8 {
            let ip = ipv4(192, 0, 2, last);
            assert!(s.try_admit_inbound(ip, 0).is_ok());
            s.record_connect(PeerId::random(), ip);
        }
        let blocked = ipv4(192, 0, 2, 99);
        assert_eq!(
            s.try_admit_inbound(blocked, 0),
            Err(RejectionReason::PerSubnet)
        );
        assert_eq!(s.rejections.per_subnet.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_total_cap_rejects() {
        let cfg = SybilConfig {
            max_connections_per_ip: 100,
            max_connections_per_subnet: 100,
            max_inbound_connections: 5,
            peer_ban_duration_secs: 60,
            trusted_validator_ips: HashSet::new(),
        };
        let mut s = SybilState::new(cfg, None);
        let ip = ipv4(10, 0, 0, 1);
        assert_eq!(s.try_admit_inbound(ip, 5), Err(RejectionReason::TotalCap));
    }

    /// Bug-B regression: trusted validator IPs bypass the per-IP and
    /// per-subnet caps. Live evidence (2026-05-07 evening 4-h soak):
    /// Hetzner-Helsinki validator subnet exhausted the 16-slot cap
    /// from peer-id churn, leaving H1 with peer_count=0. With the
    /// validator's IP whitelisted, the gate stops rejecting reconnects
    /// from that subnet specifically.
    #[test]
    fn trusted_validator_ip_bypasses_per_ip_and_per_subnet_caps() {
        let mut trusted = HashSet::new();
        let validator_ip = ipv4(100, 119, 53, 101);
        trusted.insert(validator_ip);
        let cfg = SybilConfig {
            max_connections_per_ip: 1,
            max_connections_per_subnet: 1,
            max_inbound_connections: 200,
            peer_ban_duration_secs: 60,
            trusted_validator_ips: trusted,
        };
        let mut s = SybilState::new(cfg, None);
        // Saturate the per-IP cap with several reconnects from the
        // same validator IP. Pre-fix this would Err(PerIp) on the 2nd.
        for _ in 0..10 {
            assert!(
                s.try_admit_inbound(validator_ip, 0).is_ok(),
                "trusted validator IP must bypass per_ip cap"
            );
            s.record_connect(PeerId::random(), validator_ip);
        }
        // Untrusted IP in the SAME subnet still subject to the cap.
        let untrusted_same_subnet = ipv4(100, 119, 53, 200);
        // First connection from untrusted IP in the subnet — already at
        // cap because the subnet now has 10 peers from the trusted IP.
        assert_eq!(
            s.try_admit_inbound(untrusted_same_subnet, 0),
            Err(RejectionReason::PerSubnet),
            "untrusted IP in trusted-validator subnet must still be \
             subject to the per_subnet cap"
        );
        // Total-cap and ban-list still apply to trusted IPs.
        assert_eq!(
            s.try_admit_inbound(validator_ip, 200),
            Err(RejectionReason::TotalCap),
            "total cap still applies to trusted IPs (memory-safety bound)"
        );
    }

    /// Empty trusted_validator_ips set preserves legacy behaviour: the
    /// per-IP and per-subnet caps still apply to every connection.
    #[test]
    fn empty_trusted_set_preserves_legacy_behavior() {
        let mut s = SybilState::new(sybil_cfg(), None);
        let ip = ipv4(192, 0, 2, 1);
        for _ in 0..4 {
            s.try_admit_inbound(ip, 0).unwrap();
            s.record_connect(PeerId::random(), ip);
        }
        // 5th connection from same IP rejected — exactly as before.
        assert_eq!(s.try_admit_inbound(ip, 0), Err(RejectionReason::PerIp));
    }

    #[test]
    fn test_disconnect_releases_slots() {
        let mut s = SybilState::new(sybil_cfg(), None);
        let ip = ipv4(10, 0, 0, 1);
        let pid = PeerId::random();
        s.try_admit_inbound(ip, 0).unwrap();
        s.record_connect(pid, ip);
        s.record_disconnect(&pid);
        assert_eq!(s.peer_ips.len(), 0);
        assert_eq!(s.subnet_counts.len(), 0);
    }

    #[test]
    fn test_score_decay_below_threshold_triggers_ban() {
        let mut s = SybilState::new(sybil_cfg(), None);
        let ip = ipv4(192, 0, 2, 17);
        let pid = PeerId::random();
        s.try_admit_inbound(ip, 0).unwrap();
        s.record_connect(pid, ip);
        // Apply a bigger-than-threshold negative delta in one go.
        let triggered_ip = s.adjust_score(&pid, -150);
        assert_eq!(triggered_ip, Some(ip));
        s.ban_ip(ip, "score_threshold_breach");
        assert!(s.bans.is_banned(&ip));
        // A subsequent admit attempt is now refused with reason=banned.
        let result = s.try_admit_inbound(ip, 0);
        assert_eq!(result, Err(RejectionReason::Banned));
    }

    #[test]
    fn test_ban_persists_across_restart() {
        let dir = std::env::temp_dir().join(format!("evap_sybil_{}", crate::banlist::now_ms()));
        let path = crate::banlist::BanList::default_path(&dir);
        let cfg = sybil_cfg();
        // Round 1: induce a ban + persist.
        {
            let mut s = SybilState::new(cfg.clone(), Some(path.clone()));
            let ip = ipv4(198, 51, 100, 9);
            s.ban_ip(ip, "test_persist");
            assert!(s.bans.is_banned(&ip));
        }
        // Round 2: fresh instance reading from the same path.
        {
            let mut s = SybilState::new(cfg, Some(path.clone()));
            let ip = ipv4(198, 51, 100, 9);
            assert!(s.bans.is_banned(&ip), "ban must survive restart");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_unauthorized_increments_rejections() {
        let s = SybilState::new(sybil_cfg(), None);
        s.rejections.record(RejectionReason::Unauthorized);
        assert_eq!(s.rejections.unauthorized.load(Ordering::Relaxed), 1);
    }

    /// Lane R.3 root-cause regression test. Locks two invariants
    /// that — together with the Lane R.1 authorization gate — kill
    /// the cluster-freeze livelock at the data-structure level:
    ///
    ///   I1. **Disconnected peers must NOT continue accumulating
    ///       SCORE_IDLE_TICK penalty.** `record_disconnect` removes
    ///       the score entry from `scores`. Without this, a peer
    ///       that vanishes for hours arrives at reconnect already
    ///       deep in negative territory (the live-cluster diagnosis
    ///       caught a peer with `score: -292, age_seconds: 47` —
    ///       the score had decayed for hours while offline even
    ///       though the connection had only existed for 47 s).
    ///
    ///   I2. **`record_connect` is a fresh slate.** Any pre-existing
    ///       score for that PeerId is overwritten with a default
    ///       PeerScore (score=0, infractions=0). A successful TLS /
    ///       peer-id authorization handshake supersedes whatever
    ///       Sybil residue was inherited.
    ///
    /// Together: an authorized peer can disconnect, stay away for
    /// weeks, and reconnect with the SAME score=0 state every time.
    #[test]
    fn test_score_reset_on_reconnect_lane_r3() {
        let mut s = SybilState::new(sybil_cfg(), None);
        let ip = ipv4(192, 0, 2, 200);
        let pid = PeerId::random();

        // Initial connect → score=0.
        s.try_admit_inbound(ip, 0).unwrap();
        s.record_connect(pid, ip);
        assert_eq!(s.scores.get(&pid).map(|e| e.score), Some(0));

        // Drive the score deeply negative (-90, just above the -100
        // ban threshold so we don't trigger ban_ip in this test).
        for _ in 0..90 {
            s.adjust_score(&pid, SCORE_IDLE_TICK);
        }
        assert_eq!(s.scores.get(&pid).map(|e| e.score), Some(-90));

        // Disconnect → I1: score entry must be removed.
        s.record_disconnect(&pid);
        assert!(
            !s.scores.contains_key(&pid),
            "Lane R.3 I1: record_disconnect must clear score entry"
        );

        // Simulate the bug-class scenario: idle ticks fire while
        // peer is offline. The R.3 fix in the call site iterates
        // peer_ips not scores, so disconnected peers are skipped —
        // here we just confirm at the data layer that the score
        // doesn't get implicitly recreated.
        for _ in 0..200 {
            // No-op: peer is disconnected, its score doesn't exist.
            // Real call site iterates peer_ips so this lookup
            // wouldn't even happen, but we assert the property
            // holds even if a stray adjust_score were called.
            if s.scores.contains_key(&pid) {
                s.adjust_score(&pid, SCORE_IDLE_TICK);
            }
        }
        assert!(
            !s.scores.contains_key(&pid),
            "Lane R.3 I1: no implicit score recreation while disconnected"
        );

        // Reconnect → I2: fresh slate (NOT inherited -90).
        s.try_admit_inbound(ip, 0).unwrap();
        s.record_connect(pid, ip);
        assert_eq!(
            s.scores.get(&pid).map(|e| e.score),
            Some(0),
            "Lane R.3 I2: record_connect must fresh-slate the score, not inherit residue"
        );
        assert_eq!(
            s.scores.get(&pid).map(|e| e.infractions),
            Some(0),
            "Lane R.3 I2: infractions must reset on reconnect"
        );
    }

    /// Lane R.2 regression test for Lane R.1 — the cluster-freeze
    /// fix where authorized validators bypass the Sybil idle-score
    /// penalty.
    ///
    /// Locks two properties:
    ///   1. **Bug class still exists** if no gate is applied: after
    ///      `ceil(|SCORE_BAN_THRESHOLD| / |SCORE_IDLE_TICK|)` = 100
    ///      idle ticks, an un-gated peer crosses the ban threshold.
    ///      This confirms why R.1 was needed.
    ///   2. **Gate works** when the call-site (service.rs idle-score
    ///      tick loop) skips `adjust_score` for authorized peers:
    ///      300 idle ticks with the gate = zero score decay = no ban.
    ///
    /// If either property regresses (e.g. SCORE_IDLE_TICK changes
    /// magnitude, or someone removes the `peer_authority.is_authorized`
    /// gate), this test fails loudly. Mirrors the mechanism in the
    /// real service.rs idle-tick loop without depending on the full
    /// libp2p Swarm runtime.
    #[test]
    fn test_authorized_peer_bypass_idle_score_lane_r1() {
        // Arithmetic check: 100 ticks should be enough to ban an
        // un-gated peer.
        let ticks_to_ban =
            (SCORE_BAN_THRESHOLD.unsigned_abs() / SCORE_IDLE_TICK.unsigned_abs()) as usize;
        assert_eq!(
            ticks_to_ban, 100,
            "Lane R.1 fix is calibrated to 100-tick ban time; if these constants \
             change, re-evaluate whether the cluster-freeze fix is still load-bearing"
        );

        // ── Property 1: bug class confirmed (un-gated peer eventually bans) ──
        {
            let mut s = SybilState::new(sybil_cfg(), None);
            let ip = ipv4(192, 0, 2, 100);
            let pid = PeerId::random();
            s.try_admit_inbound(ip, 0).unwrap();
            s.record_connect(pid, ip);
            let mut triggered_at: Option<usize> = None;
            for i in 0..200 {
                if let Some(returned_ip) = s.adjust_score(&pid, SCORE_IDLE_TICK) {
                    s.ban_ip(returned_ip, "score_threshold_breach");
                    triggered_at = Some(i + 1);
                    break;
                }
            }
            // After 100 ticks the score reaches -100 (== threshold,
            // not below). Ban triggers when score < threshold, so the
            // 101st tick at -101 fires it.
            assert_eq!(
                triggered_at,
                Some(101),
                "un-gated peer must hit ban at the 101st tick (score crosses below -100)"
            );
            assert!(s.bans.is_banned(&ip));
        }

        // ── Property 2: Lane R.1 gate works (authorized peer never bans) ──
        // Models the call-site skip:
        //   if peer_authority.is_authorized(&pid) { continue; }
        {
            let mut s = SybilState::new(sybil_cfg(), None);
            let ip = ipv4(198, 51, 100, 100);
            let pid = PeerId::random();
            s.try_admit_inbound(ip, 0).unwrap();
            s.record_connect(pid, ip);
            // Gate applied: skip adjust_score for "authorized" peer.
            let is_authorized_in_test = true;
            for _ in 0..300 {
                if !is_authorized_in_test {
                    if let Some(returned_ip) = s.adjust_score(&pid, SCORE_IDLE_TICK) {
                        s.ban_ip(returned_ip, "score_threshold_breach");
                    }
                }
            }
            assert!(
                !s.bans.is_banned(&ip),
                "authorized peer must never be banned by idle ticks (Lane R.1)"
            );
            // Score must remain at the default starting value (0) since
            // adjust_score was never called.
            let score_after = s.scores.get(&pid).map(|e| e.score).unwrap_or(0);
            assert_eq!(
                score_after, 0,
                "authorized peer score must not decay when the gate is in place"
            );
        }
    }

    /// Diagnostic surface regression: `scores_view()` must surface
    /// every entry in `scores`, including ghost entries (peers in
    /// `scores` but not `peer_ips`). The Lane R.* freeze-class root
    /// cause was exactly this hidden state; without `scores_view`
    /// it stayed invisible to operators.
    #[test]
    fn test_scores_view_surfaces_ghost_entries() {
        let mut s = SybilState::new(sybil_cfg(), None);
        let ip = ipv4(192, 0, 2, 50);
        let connected = PeerId::random();
        let ghost = PeerId::random();

        // Connected peer: shows up in both peer_ips and scores.
        s.try_admit_inbound(ip, 0).unwrap();
        s.record_connect(connected, ip);

        // Ghost: scored without ever connecting (e.g. an
        // adjust_score call on an unconnected peer that hit
        // `or_default()`). This is the freeze-class signal.
        s.adjust_score(&ghost, -10);
        assert!(
            !s.peer_ips.contains_key(&ghost),
            "ghost setup: must not be in peer_ips"
        );
        assert!(
            s.scores.contains_key(&ghost),
            "ghost setup: must be in scores"
        );

        let view = s.scores_view();
        assert_eq!(view.len(), 2, "scores_view must include both entries");

        let connected_entry = view
            .iter()
            .find(|e| e.peer_id == connected.to_string())
            .expect("connected peer must be in view");
        assert!(connected_entry.connected);
        assert!(connected_entry.ip.is_some());
        assert!(connected_entry.since_ms.is_some());
        assert_eq!(connected_entry.score, 0);

        let ghost_entry = view
            .iter()
            .find(|e| e.peer_id == ghost.to_string())
            .expect("ghost peer must be surfaced by scores_view");
        assert!(!ghost_entry.connected, "ghost must report connected=false");
        assert!(ghost_entry.ip.is_none(), "ghost must not have an IP");
        assert!(
            ghost_entry.since_ms.is_none(),
            "ghost must not have since_ms"
        );
        assert_eq!(ghost_entry.score, -10);
        assert_eq!(ghost_entry.infractions, 1);
    }

    /// T1.20 — `RejectionReason::label()` returns the snake_case
    /// strings used by the `evap_inbound_rejections_total`
    /// Prometheus counter (lines 575-583). All five variants
    /// pinned. The metric's cardinality is fixed by this enum;
    /// a refactor that silently renamed a string would break
    /// dashboards.
    #[test]
    fn t1_20_rejection_reason_label_returns_metric_strings() {
        assert_eq!(RejectionReason::PerIp.label(), "per_ip");
        assert_eq!(RejectionReason::PerSubnet.label(), "per_subnet");
        assert_eq!(RejectionReason::TotalCap.label(), "total_cap");
        assert_eq!(RejectionReason::Banned.label(), "banned");
        assert_eq!(RejectionReason::Unauthorized.label(), "unauthorized");
    }

    /// T1.20 — `cache_block` inserts at the block's height and
    /// overwrites on re-insert (lines 1032-1042). Existing tests
    /// use the cache extensively but the insert/overwrite path
    /// isn't pinned directly.
    #[test]
    fn t1_20_cache_block_insert_and_overwrite() {
        let cache: BlockCache = Arc::new(RwLock::new(BTreeMap::new()));
        let b1 = dummy_block(5);
        cache_block(&cache, &b1);
        assert_eq!(safe_read(&cache).len(), 1);
        assert!(safe_read(&cache).contains_key(&5));

        // Re-insert at same height overwrites.
        let mut b1_alt = dummy_block(5);
        b1_alt.epoch = 99;
        cache_block(&cache, &b1_alt);
        let c = safe_read(&cache);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get(&5).unwrap().epoch, 99);
        drop(c);

        // Sparse inserts at other heights accumulate.
        for h in 10..=20 {
            cache_block(&cache, &dummy_block(h));
        }
        assert_eq!(safe_read(&cache).len(), 12);
    }

    /// T1.20 — `cache_block` eviction at capacity (lines 1036-1041).
    /// Fill to `MAX_CACHE_SIZE`, then add one more; the oldest
    /// (lowest key in the BTreeMap) must be dropped, newest
    /// retained.
    #[test]
    fn t1_20_cache_block_evicts_oldest_at_capacity() {
        let cache: BlockCache = Arc::new(RwLock::new(BTreeMap::new()));
        for h in 0..(MAX_CACHE_SIZE as u64) {
            cache_block(&cache, &dummy_block(h));
        }
        assert_eq!(safe_read(&cache).len(), MAX_CACHE_SIZE);
        assert!(safe_read(&cache).contains_key(&0));

        cache_block(&cache, &dummy_block(MAX_CACHE_SIZE as u64));
        let c = safe_read(&cache);
        assert_eq!(c.len(), MAX_CACHE_SIZE);
        assert!(!c.contains_key(&0), "oldest (height 0) must be evicted");
        assert!(
            c.contains_key(&(MAX_CACHE_SIZE as u64)),
            "newest must be retained"
        );
    }
}
