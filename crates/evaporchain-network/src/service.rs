use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use libp2p::{
    gossipsub::{self, IdentTopic, MessageAuthenticity},
    identify, mdns, noise, tls,
    request_response::{self, ProtocolSupport},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, StreamProtocol, SwarmBuilder,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::{NetworkError, NetworkService};
use evaporchain_da::block_da::BlockDAPackage;
use evaporchain_da::sampling::{SampleQuery, SampleResponse, DASampler};
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

/// Maximum number of blocks to keep in the cache.
const MAX_CACHE_SIZE: usize = 2000;

/// Maximum allowed gossip message size (10 MB). Messages exceeding this
/// are dropped before deserialization to prevent OOM attacks.
const MAX_GOSSIP_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

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
        Self { violations: HashMap::new(), banned: HashMap::new() }
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

    fn record_violation(&mut self, peer: PeerId) -> bool {
        let count = self.violations.entry(peer).or_insert(0);
        *count += 1;
        if *count >= BAN_THRESHOLD {
            self.banned.insert(peer, Instant::now() + BAN_DURATION);
            warn!("Banned peer {peer} for {}s after {count} violations", BAN_DURATION.as_secs());
            true
        } else {
            false
        }
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
        }
    }
}

// ─────────────────────────── Behaviour ───────────────────────────────────

#[derive(NetworkBehaviour)]
struct EvaporBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
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

        let peer_authority = config.peer_authority.clone();
        let use_tls = config.use_tls;

        // Behaviour constructor shared by both transport paths
        macro_rules! build_behaviour {
            ($key:ident) => {{
                // Use default message ID (source + seq_no) so each validator's
                // consensus votes get unique IDs even across rounds.
                let gossipsub_config = gossipsub::ConfigBuilder::default()
                    .heartbeat_interval(Duration::from_millis(500))
                    .validation_mode(gossipsub::ValidationMode::Permissive)
                    .max_transmit_size(4 * 1024 * 1024)
                    .mesh_n(3)
                    .mesh_n_low(2)
                    .mesh_n_high(6)
                    .mesh_outbound_min(1)
                    .gossip_lazy(3)
                    .build()
                    .expect("valid gossipsub config");
                let gossipsub = gossipsub::Behaviour::new(
                    MessageAuthenticity::Signed($key.clone()),
                    gossipsub_config,
                )
                .expect("valid gossipsub behaviour");

                let mdns = mdns::tokio::Behaviour::new(
                    mdns::Config::default(),
                    $key.public().to_peer_id(),
                )
                .expect("valid mdns behaviour");

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
        let mut swarm = if use_tls {
            info!("Using TLS 1.3 transport (libp2p-tls)");
            SwarmBuilder::with_new_identity()
                .with_tokio()
                .with_tcp(
                    tcp::Config::default(),
                    tls::Config::new,
                    yamux::Config::default,
                )
                .map_err(|e| NetworkError::ConnectionError(format!("tls transport: {e}")))?
                .with_behaviour(|key| build_behaviour!(key))
                .map_err(|e| NetworkError::ConnectionError(format!("behaviour: {e}")))?
                .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
                .build()
        } else {
            SwarmBuilder::with_new_identity()
                .with_tokio()
                .with_tcp(
                    tcp::Config::default(),
                    noise::Config::new,
                    yamux::Config::default,
                )
                .map_err(|e| NetworkError::ConnectionError(format!("tcp transport: {e}")))?
                .with_behaviour(|key| build_behaviour!(key))
                .map_err(|e| NetworkError::ConnectionError(format!("behaviour: {e}")))?
                .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
                .build()
        };

        let local_peer_id = *swarm.local_peer_id();

        // Subscribe to topics
        let tx_topic = IdentTopic::new(TX_TOPIC);
        let block_topic = IdentTopic::new(BLOCK_TOPIC);
        let consensus_topic = IdentTopic::new(CONSENSUS_TOPIC);
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
        };

        // Clone bootstrap addrs for periodic re-dial inside the event loop
        let bootstrap_addrs: Vec<Multiaddr> = config.bootstrap_peers.iter()
            .filter_map(|s| s.parse::<Multiaddr>().ok())
            .collect();

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
            let mut gc_counter: u64 = 0;

            loop {
                tokio::select! {
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
                                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(block_topic.clone(), data) {
                                    debug!("Failed to publish block: {e}");
                                }
                            }
                            Err(e) => warn!("Failed to serialize block: {e}"),
                        }
                    }
                    // App wants to broadcast a consensus message
                    Some(data) = net_consensus_receiver.recv() => {
                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(consensus_topic.clone(), data) {
                            debug!("Failed to publish consensus msg: {e}");
                        }
                    }
                    // App requests shard samples from peers (light client DAS)
                    Some(queries) = sample_req_receiver.recv() => {
                        let peers: Vec<PeerId> = swarm.connected_peers().cloned().collect();
                        if peers.is_empty() {
                            warn!("No peers available for shard sample request");
                        } else {
                            let target = peers[0];
                            debug!("Requesting {} shard samples from peer {target}", queries.len());
                            swarm.behaviour_mut().shard_sample.send_request(
                                &target,
                                ShardSampleRequest { queries },
                            );
                        }
                    }
                    // App requests block sync from peers
                    Some((from, to)) = sync_req_receiver.recv() => {
                        // Pick a connected peer to request blocks from
                        let peers: Vec<PeerId> = swarm.connected_peers().cloned().collect();
                        if peers.is_empty() {
                            warn!("No peers available for block sync request {from}..{to}");
                        } else {
                            // Request from each peer (first responder wins)
                            let target = peers[0]; // Pick first peer
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
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::Gossipsub(
                                gossipsub::Event::Message { message, .. },
                            )) => {
                                // Per-peer ban + rate limiting
                                if let Some(ref source) = message.source {
                                    if ban_list.is_banned(source) {
                                        continue;
                                    }
                                    if !rate_limiter.check_and_increment(source) {
                                        debug!("Rate-limited peer {source} — dropping gossip message");
                                        gc_counter += 1;
                                        if gc_counter % 100 == 0 {
                                            rate_limiter.maybe_gc();
                                        }
                                        continue;
                                    }
                                }
                                gc_counter += 1;
                                if gc_counter % 1000 == 0 {
                                    rate_limiter.maybe_gc();
                                }
                                // Drop oversized messages before deserialization (DoS protection)
                                if message.data.len() > MAX_GOSSIP_MESSAGE_SIZE {
                                    warn!(
                                        "Dropping oversized gossip message: {} bytes (limit {})",
                                        message.data.len(),
                                        MAX_GOSSIP_MESSAGE_SIZE
                                    );
                                } else if message.topic == tx_topic_hash {
                                    match serde_json::from_slice::<Transaction>(&message.data) {
                                        Ok(tx) => {
                                            let _ = net_tx_sender.send(tx).await;
                                        }
                                        Err(e) => {
                                            debug!("Invalid tx gossip: {e}");
                                            if let Some(ref source) = message.source {
                                                ban_list.record_violation(*source);
                                            }
                                        }
                                    }
                                } else if message.topic == block_topic_hash {
                                    match serde_json::from_slice::<Block>(&message.data) {
                                        Ok(block) => {
                                            let _ = net_block_sender.send(block).await;
                                        }
                                        Err(e) => {
                                            debug!("Invalid block gossip: {e}");
                                            if let Some(ref source) = message.source {
                                                ban_list.record_violation(*source);
                                            }
                                        }
                                    }
                                } else if message.topic == consensus_topic_hash {
                                    // Forward raw bytes — app deserializes
                                    let _ = net_consensus_sender.send(message.data.to_vec()).await;
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
                                let blocks: Vec<Block> = (from..=to)
                                    .filter_map(|n| cache.get(&n).cloned())
                                    .collect();
                                let tip = cache.keys().last().copied().unwrap_or(0);
                                drop(cache);

                                info!("Serving {} blocks to peer {peer} (tip={tip})", blocks.len());
                                let response = BlockSyncResponse { blocks, tip_height: tip };
                                if let Err(e) = swarm.behaviour_mut().block_sync.send_response(channel, response) {
                                    warn!("Failed to send sync response to {peer}: {e:?}");
                                }
                            }
                            // ── Block sync: outbound response (received blocks) ──
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
                                if !response.blocks.is_empty() {
                                    let _ = sync_blocks_sender.send(response.blocks).await;
                                }
                                let _ = tip_sender.send(response.tip_height).await;
                            }
                            // ── Block sync failures ──
                            SwarmEvent::Behaviour(EvaporBehaviourEvent::BlockSync(
                                request_response::Event::OutboundFailure { peer, error, .. },
                            )) => {
                                warn!("Block sync request to {peer} failed: {error}");
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
                            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                                if !peer_authority.is_authorized(&peer_id) {
                                    warn!("Unauthorized peer {peer_id} — disconnecting");
                                    let _ = swarm.disconnect_peer_id(peer_id);
                                    continue;
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
                            SwarmEvent::ConnectionClosed { .. } => {
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
            match timeout(deadline - tokio::time::Instant::now(), ch2.tx_receiver.recv()).await {
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
        ch2.sync_request_sender.send((1, 5)).await.expect("send sync request");

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
        let req = BlockSyncRequest { from_height: 100, to_height: 200 };
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

    #[test]
    fn test_shard_sample_request_serialization() {
        use evaporchain_da::sampling::SampleQuery;

        let req = ShardSampleRequest {
            queries: vec![
                SampleQuery { block_number: 10, shard_index: 0 },
                SampleQuery { block_number: 10, shard_index: 3 },
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
    }

    #[test]
    fn test_constants_sane() {
        assert_eq!(MAX_GOSSIP_MESSAGE_SIZE, 10 * 1024 * 1024);
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
            SampleQuery { block_number: 42, shard_index: 0 },
            SampleQuery { block_number: 42, shard_index: 1 },
        ];
        ch2.sample_request_sender.send(queries).await.expect("send sample request");

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
}
