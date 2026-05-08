//! Paymaster sponsorship service for EvaporChain UserOpTxs.
//!
//! Day 2 of the multi-token-gas Option B build (see
//! `docs/MULTI_TOKEN_GAS_OPTIONS.md`). The chain-side enforcement of
//! paymaster sponsorship landed in Day 1A+1B (commits `dc89531` +
//! `3ccf4f7`). This crate provides the off-chain side: a long-running
//! service that takes a half-built `UserOpTx` from a wallet and stamps
//! the paymaster's sponsorship signature, so the wallet can submit it
//! to the chain with the paymaster paying gas.
//!
//! ## What this crate is, and is NOT
//!
//! IS:
//!
//! - The cryptographic + nonce-counter side of paymaster sponsorship.
//! - A simple HTTP wrapper (in the `bin/server.rs` binary) so wallets
//!   can request sponsorship over the network.
//! - A library so future tooling (e.g. a paymaster integrated into a
//!   wallet, batched paymasters, smart-contract paymasters) can reuse
//!   the signing primitive.
//!
//! IS NOT (deferred to later sprints):
//!
//! - A token-payment collection layer. The user reimburses the
//!   paymaster in their preferred token via a separate flow that's a
//!   business arrangement between user and paymaster — not a chain
//!   primitive. The MVP service unconditionally signs every request.
//! - A price oracle. Post-MVP a paymaster will quote in USDC/ETH/etc
//!   and only sign once payment lands; the MVP signs everything.
//! - An on-chain submitter. The wallet (or a relayer) submits the
//!   signed `UserOpTx` to the chain. Decoupling lets paymaster
//!   operators avoid running their own full nodes.
//! - User-signature verification. The chain enforces the user's
//!   `Transaction::signature`; the paymaster trusts the wallet to get
//!   it right (a malformed user-signed UserOp simply gets rejected at
//!   `verify_tx_signature`, costing the paymaster the gas it just
//!   sponsored — which is paymaster-side risk, not consensus risk).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use evaporchain_crypto::signatures::{HybridKeypair, HybridVerifier, Signer, Verifier};
use evaporchain_types::{AccountAddress, Transaction, UserOpTx};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors surfaced by the paymaster signing path.
#[derive(Debug, Error)]
pub enum PaymasterError {
    #[error("UserOpTx must have paymaster set to this paymaster's address")]
    PaymasterAddressMismatch,
    #[error("UserOpTx already carries a paymaster_signature — refusing to overwrite")]
    AlreadySigned,
    #[error("paymaster_nonce file IO: {0}")]
    NonceIo(#[from] std::io::Error),
    #[error("paymaster_nonce file is malformed: {0}")]
    NonceParse(String),
    #[error("keypair file IO: {0}")]
    KeypairIo(String),
    /// Spam-signing hardening per `docs/runbooks/paymaster.md` §Threat:
    /// spam-signing. A malformed user signature on the inbound UserOp
    /// would cost the paymaster gas at execute time for a tx the chain
    /// rejects — so we pre-validate when `require_user_sig: true` (the
    /// production default).
    #[error("user signature missing or invalid (require_user_sig is enabled)")]
    InvalidUserSignature,
    /// Per-`UserOp.sender` token-bucket rate limit hit. The wallet
    /// should back off and retry; the paymaster will not sign another
    /// sponsorship for this sender until the bucket refills.
    #[error("per-sender rate limit exceeded for sender 0x{sender_hex}")]
    RateLimited { sender_hex: String },
    /// Audit-log IO error. Fail-closed: a sponsorship the operator
    /// can't audit is one they can't bill against, so we surface
    /// the failure rather than silently sponsoring without the line.
    #[error("audit log write failed: {0}")]
    AuditIo(String),
    /// The inbound UserOp's inner Transaction variant isn't in this
    /// paymaster's `allowed_inner_variants` set. Operator-side
    /// defense-in-depth above the chain's global whitelist.
    #[error("inner variant '{variant}' is not allowed by this paymaster")]
    InnerVariantNotAllowed { variant: String },
    /// `call_data` failed to decode as a `Transaction`. Reaches the
    /// paymaster only when `allowed_inner_variants` is enforced —
    /// otherwise the chain's `execute_user_op` would surface the
    /// same failure at execute time.
    #[error("call_data decode failed: {0}")]
    CallDataDecode(String),
}

/// Inner-`Transaction` variant — the operator-facing identity used
/// by `PaymasterConfig::allowed_inner_variants` and the
/// `--allow-inner` CLI flag. The paymaster decodes inbound
/// `UserOp.call_data` as a `Transaction`, projects to this enum,
/// and checks set membership BEFORE allocating a nonce or signing.
///
/// Naming follows the chain's existing `Transaction` variants
/// lower-cased + snake-cased; `--allow-inner=transfer,call_script`
/// maps cleanly. The set is intentionally narrower than the full
/// `Transaction` enum: only the variants the chain's V1 whitelist
/// permits as inner intents under a sponsored UserOp (per
/// `crates/evaporchain-execution/src/lib.rs:execute_user_op`
/// dispatch arm).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InnerVariant {
    Transfer,
    CallScript,
    CallContract,
}

impl InnerVariant {
    /// Tag a runtime `Transaction` enum value with this restricted
    /// variant tag. Returns `None` for variants the chain doesn't
    /// accept as a UserOp inner intent — paymaster rejects with
    /// `InnerVariantNotAllowed` either way (the inbound `call_data`
    /// would already fail at the chain level, but we surface the
    /// rejection earlier).
    pub fn from_transaction(tx: &Transaction) -> Option<Self> {
        match tx {
            Transaction::Transfer(_) => Some(Self::Transfer),
            Transaction::CallScript(_) => Some(Self::CallScript),
            Transaction::CallContract(_) => Some(Self::CallContract),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Transfer => "transfer",
            Self::CallScript => "call_script",
            Self::CallContract => "call_contract",
        }
    }

    /// Parse the operator-facing CLI form (lowercase + snake_case).
    pub fn parse_cli(s: &str) -> Option<Self> {
        match s {
            "transfer" => Some(Self::Transfer),
            "call_script" => Some(Self::CallScript),
            "call_contract" => Some(Self::CallContract),
            _ => None,
        }
    }
}

/// JSON request body POSTed to `/sponsor`. The wallet builds the body
/// of the `UserOpTx` (sender, sender_nonce, call_gas_limit, call_data,
/// optionally the user's own signature/public_key) and asks the
/// paymaster to fill in the sponsorship fields.
///
/// `paymaster` and `paymaster_nonce` are filled in by the service —
/// the wallet can pass them as a sanity hint, but the service
/// overrides them to its own canonical values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SponsorshipRequest {
    pub user_op: UserOpTx,
}

/// JSON response body. The returned `UserOpTx` is wire-ready for the
/// chain — submit via the node's `/api/tx` endpoint (or equivalent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SponsorshipResponse {
    pub user_op: UserOpTx,
    pub paymaster_address_hex: String,
    pub paymaster_nonce: u64,
}

/// JSON response body for `/info`. Wallets read this before
/// submitting a `/sponsor` request to discover (a) where to address
/// the sponsorship — `paymaster_address_hex` — (b) what nonce to
/// stamp — `next_paymaster_nonce` — (c) what chain to sign for —
/// `chain_id` — and (d) the operator's policy so the wallet can
/// fail a doomed request locally rather than burning a round-trip.
///
/// Wire shape backwards-compat: the policy fields all have
/// `#[serde(default)]` so a wallet built before Day 11 ignores
/// them, and a wallet built after Day 11 hitting an old paymaster
/// gets sensible defaults (`require_user_sig: false`, etc. — the
/// permissive baseline). Wallets that want to pre-validate against
/// the policy MUST handle the missing-field case as "unknown
/// policy; submit and see".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymasterInfo {
    pub paymaster_address_hex: String,
    pub next_paymaster_nonce: u64,
    pub chain_id: String,
    /// Day 7 hardening — strict mode rejects UserOps without a
    /// valid user signature.
    #[serde(default)]
    pub require_user_sig: bool,
    /// Day 7 hardening — per-sender token-bucket replenish rate.
    /// `0.0` means rate limiting is off.
    #[serde(default)]
    pub per_sender_rps: f64,
    /// Day 7 hardening — per-sender burst capacity.
    #[serde(default)]
    pub per_sender_burst: u32,
    /// Day 8 hardening — operator is keeping a billing-
    /// reconciliation audit log. The path itself is NOT exposed
    /// (operational hygiene); only whether the log exists.
    #[serde(default)]
    pub audit_log_enabled: bool,
    /// Day 10 — operator's inner-tx whitelist. `None` means the
    /// paymaster trusts the chain's global whitelist; `Some(set)`
    /// is a strict subset. Strings are the snake_case CLI form
    /// (`transfer`, `call_script`, `call_contract`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_inner_variants: Option<Vec<String>>,
    /// Day 12 — idempotency cache size. `0` means idempotency is
    /// disabled; the wallet's `Idempotency-Key` header (if sent)
    /// is ignored. Wallets that retry should read this and back
    /// off when 0.
    #[serde(default)]
    pub idempotency_max_keys: usize,
    /// Day 12 — idempotency cache TTL in seconds.
    #[serde(default)]
    pub idempotency_ttl_secs: u64,
}

/// Hardening knobs the operator can flip per deployment. Defaults
/// match the production-safe profile recommended in
/// `docs/runbooks/paymaster.md` §Threat: spam-signing — strict in
/// prod, relaxed for testnet / debug if desired.
#[derive(Debug, Clone)]
pub struct PaymasterConfig {
    /// Verify the user-side signature on the inbound `UserOpTx`
    /// before signing sponsorship. Closes the spam-drain attack
    /// where a bad-actor wallet floods `/sponsor` with malformed
    /// UserOps that the chain will reject (charging the paymaster
    /// gas for nothing). Default `true` — disable only for testnet
    /// experiments where wallets are still wiring their signing
    /// path.
    pub require_user_sig: bool,
    /// Per-`UserOp.sender` token-bucket replenish rate, in
    /// sponsorships per second. `0.0` disables the rate limiter
    /// (every request gets through). Default `5.0` — adequate for
    /// normal wallet behaviour, throttles a single account that
    /// tries to flood the paymaster.
    pub per_sender_rps: f64,
    /// Per-sender burst: max sponsorships a fresh sender can
    /// allocate before the bucket starts throttling. Default `10`.
    pub per_sender_burst: u32,
    /// Append-only JSON-lines audit log path. When set, every
    /// successful sponsorship writes one line:
    ///
    /// ```json
    /// {"ts_unix_ms":...,"sender":"0x..","paymaster_nonce":N,
    ///  "call_gas_limit":N,"call_data_hash":"...","chain_id":"..."}
    /// ```
    ///
    /// Used by operators for billing reconciliation (paymasters
    /// charging users in another token can match audit lines to
    /// off-chain payments). The log is append-only with fsync
    /// per-line so a crash never loses a sponsored entry. IO
    /// errors fail the sponsorship — there is no silent-drop
    /// path.
    ///
    /// `None` (default) disables audit logging.
    pub audit_log: Option<PathBuf>,
    /// Idempotency cache size. `0` disables idempotency entirely.
    /// Default: `1024`. Wallets that retry a sponsorship under the
    /// same `Idempotency-Key` get the cached response — same
    /// paymaster_nonce, same signature — instead of a fresh
    /// allocation. Prevents duplicate sponsorships on network blips.
    pub idempotency_max_keys: usize,
    /// Idempotency cache TTL in seconds. Default: `3600` (1h). After
    /// this, the same key is treated as a fresh request. Tune higher
    /// if wallet retries can span longer (e.g., user closes laptop
    /// mid-flight); lower if the paymaster's nonce horizon is short.
    pub idempotency_ttl_secs: u64,
    /// Operator-side narrowing of which inner-tx variants this
    /// paymaster will sponsor.
    ///
    /// - `None` (default): trust the chain — sponsor any inner
    ///   variant the chain accepts as a UserOp intent. Forwards-
    ///   compatible with chain whitelist expansion.
    /// - `Some(set)`: only sponsor inner variants in the set.
    ///   Reject everything else with `InnerVariantNotAllowed`
    ///   BEFORE allocating a sponsorship nonce. Useful for
    ///   specialized paymasters: e.g., a "Transfer-only" paymaster
    ///   that won't subsidize complex contract calls.
    ///
    /// Empty `call_data` (gas-only sponsorship) is always allowed
    /// regardless of this setting — there's no inner intent to
    /// classify.
    pub allowed_inner_variants: Option<Vec<InnerVariant>>,
}

impl Default for PaymasterConfig {
    fn default() -> Self {
        Self {
            require_user_sig: true,
            per_sender_rps: 5.0,
            per_sender_burst: 10,
            idempotency_max_keys: 1024,
            idempotency_ttl_secs: 3600,
            audit_log: None,
            allowed_inner_variants: None,
        }
    }
}

impl PaymasterConfig {
    /// Permissive profile for testnet / dev — disables both the
    /// user-sig pre-check and the rate limiter. Do NOT use in prod.
    /// Idempotency cache is left ENABLED (1024 keys, 1h TTL) since
    /// even testnet wallets benefit from retry-safety.
    pub fn permissive() -> Self {
        Self {
            require_user_sig: false,
            per_sender_rps: 0.0,
            per_sender_burst: 0,
            idempotency_max_keys: 1024,
            idempotency_ttl_secs: 3600,
            audit_log: None,
            allowed_inner_variants: None,
        }
    }
}

/// Token-bucket rate limiter, keyed by `UserOp.sender`. Buckets
/// refill at `per_sender_rps` tokens per second up to
/// `per_sender_burst`. A sponsor request consumes one token; if the
/// bucket is empty, the request is rejected.
///
/// Garbage collection: on every check, buckets that have been full
/// (i.e., idle) for `IDLE_GC_THRESHOLD` get pruned. Bounds the
/// HashMap size proportional to active senders, not historical
/// total.
struct RateLimiter {
    buckets: HashMap<AccountAddress, Bucket>,
    rps: f64,
    burst: u32,
    last_gc: Instant,
}

#[derive(Clone, Copy)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

const IDLE_GC_THRESHOLD: Duration = Duration::from_secs(600);
const GC_INTERVAL: Duration = Duration::from_secs(60);

impl RateLimiter {
    fn new(rps: f64, burst: u32) -> Self {
        Self {
            buckets: HashMap::new(),
            rps,
            burst,
            last_gc: Instant::now(),
        }
    }

    fn enabled(&self) -> bool {
        self.rps > 0.0 && self.burst > 0
    }

    /// Try to consume one token for `sender`. Returns `true` if
    /// allowed, `false` if rate-limited.
    fn try_consume(&mut self, sender: AccountAddress) -> bool {
        if !self.enabled() {
            return true;
        }
        let now = Instant::now();
        if now.duration_since(self.last_gc) >= GC_INTERVAL {
            self.gc(now);
            self.last_gc = now;
        }
        let burst = self.burst as f64;
        let bucket = self.buckets.entry(sender).or_insert(Bucket {
            tokens: burst,
            last_refill: now,
        });
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.rps).min(burst);
        bucket.last_refill = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn gc(&mut self, now: Instant) {
        let burst = self.burst as f64;
        self.buckets.retain(|_, b| {
            // A bucket that's idle long enough — i.e., its tokens
            // have refilled to the cap and it hasn't been touched —
            // is safe to drop. Re-creating it on next request gives
            // the sender a fresh-cap bucket, which matches what GC-
            // dropping then re-creating would produce anyway.
            let idle = now.duration_since(b.last_refill);
            idle < IDLE_GC_THRESHOLD || b.tokens < burst
        });
    }
}

/// Counters + gauges exposed via `GET /metrics` in the binary.
/// Atomic so concurrent sponsor calls increment without locking, and
/// the metrics endpoint reads without blocking either. All counters
/// are monotonic; gauges are sampled at read time.
///
/// Labels (Prometheus convention) live in the metric NAME — Rust
/// doesn't have label-aware counter aggregation in std, and we don't
/// want to pull in a full `prometheus` crate dep for a handful of
/// counters. The exposition format hand-written in
/// `Paymaster::prometheus_metrics()` translates these into labelled
/// `evaporchain_paymaster_sponsorships_total{status="..."}` lines.
#[derive(Debug)]
pub struct PaymasterMetrics {
    pub sponsorships_ok: AtomicU64,
    pub sponsorships_already_signed: AtomicU64,
    pub sponsorships_invalid_user_sig: AtomicU64,
    pub sponsorships_rate_limited: AtomicU64,
    pub sponsorships_nonce_io: AtomicU64,
    pub sponsorships_audit_io: AtomicU64,
    pub sponsorships_other: AtomicU64,
    /// Day 12 — count of idempotent replays (i.e., `/sponsor` calls
    /// where the wallet's `Idempotency-Key` matched a previous
    /// request and the paymaster returned the cached response
    /// instead of allocating a fresh nonce). High replay rate signals
    /// either flaky wallet → paymaster networking or a wallet bug
    /// retrying without backoff.
    pub sponsorships_idempotent_replay: AtomicU64,
    pub started_at: Instant,
}

impl Default for PaymasterMetrics {
    fn default() -> Self {
        Self {
            sponsorships_ok: AtomicU64::new(0),
            sponsorships_already_signed: AtomicU64::new(0),
            sponsorships_invalid_user_sig: AtomicU64::new(0),
            sponsorships_rate_limited: AtomicU64::new(0),
            sponsorships_nonce_io: AtomicU64::new(0),
            sponsorships_audit_io: AtomicU64::new(0),
            sponsorships_other: AtomicU64::new(0),
            sponsorships_idempotent_replay: AtomicU64::new(0),
            started_at: Instant::now(),
        }
    }
}

impl PaymasterMetrics {
    fn record(&self, outcome: Result<(), &PaymasterError>) {
        let counter = match outcome {
            Ok(()) => &self.sponsorships_ok,
            Err(PaymasterError::AlreadySigned) => &self.sponsorships_already_signed,
            Err(PaymasterError::InvalidUserSignature) => &self.sponsorships_invalid_user_sig,
            Err(PaymasterError::RateLimited { .. }) => &self.sponsorships_rate_limited,
            Err(PaymasterError::NonceIo(_)) | Err(PaymasterError::NonceParse(_)) => {
                &self.sponsorships_nonce_io
            }
            Err(PaymasterError::AuditIo(_)) => &self.sponsorships_audit_io,
            Err(_) => &self.sponsorships_other,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// LRU + TTL cache mapping wallet-supplied idempotency keys to the
/// SponsorshipResponse the paymaster originally returned. Bounded
/// by `max_keys` (oldest evicted on insertion when full); per-entry
/// TTL `ttl` (expired entries treated as cache miss on read).
///
/// Wallets opt in by sending an `Idempotency-Key: <hex>` header on
/// `/sponsor`. A retry under the same key gets the same response —
/// same paymaster_nonce, same signature — so the wallet doesn't end
/// up holding two distinct UserOps for what was logically one
/// sponsorship.
#[derive(Debug)]
struct IdempotencyCache {
    entries: HashMap<String, (SponsorshipResponse, Instant)>,
    insertion_order: VecDeque<String>,
    max_keys: usize,
    ttl: Duration,
}

impl IdempotencyCache {
    fn new(max_keys: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
            max_keys,
            ttl,
        }
    }

    fn enabled(&self) -> bool {
        self.max_keys > 0
    }

    fn get(&mut self, key: &str) -> Option<SponsorshipResponse> {
        if !self.enabled() {
            return None;
        }
        let now = Instant::now();
        let expired = self
            .entries
            .get(key)
            .map(|(_, ts)| now.duration_since(*ts) > self.ttl)
            .unwrap_or(false);
        if expired {
            self.entries.remove(key);
            return None;
        }
        self.entries.get(key).map(|(r, _)| r.clone())
    }

    fn insert(&mut self, key: String, response: SponsorshipResponse) {
        if !self.enabled() {
            return;
        }
        // If the key already exists (shouldn't, but defend), refresh
        // it without touching the LRU order — caller already saw a
        // miss path so we record this as a fresh-but-overwriting
        // event.
        if self.entries.contains_key(&key) {
            self.entries.insert(key, (response, Instant::now()));
            return;
        }
        if self.entries.len() >= self.max_keys {
            // Evict oldest. The deque holds insertion order; the
            // HashMap holds the values. They can briefly disagree if
            // a key was overwritten above, so loop until we evict an
            // entry that's actually still present.
            while let Some(oldest) = self.insertion_order.pop_front() {
                if self.entries.remove(&oldest).is_some() {
                    break;
                }
            }
        }
        self.insertion_order.push_back(key.clone());
        self.entries.insert(key, (response, Instant::now()));
    }
}

/// Outcome of a `sponsor_idempotent` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SponsorOutcome {
    /// First time we saw this idempotency key (or no key was
    /// provided). The paymaster allocated a fresh sponsorship
    /// nonce and signed; `user_op` was mutated in place to carry
    /// the new sponsorship fields.
    Fresh { paymaster_nonce: u64 },
    /// The supplied idempotency key matched a previous request.
    /// `user_op` was filled with the cached response — same
    /// paymaster_nonce, same signature — so a wallet retry gets
    /// the SAME wire-ready transaction it got the first time.
    Replay { paymaster_nonce: u64 },
}

impl SponsorOutcome {
    pub fn paymaster_nonce(&self) -> u64 {
        match self {
            Self::Fresh { paymaster_nonce } | Self::Replay { paymaster_nonce } => {
                *paymaster_nonce
            }
        }
    }
}

/// Long-lived state held by a paymaster service.
///
/// Wraps the keypair, derives the paymaster's account address from
/// `blake3(public_key_bytes)` (matching the chain's
/// `generate_address_from_pubkey`), and serialises sponsorship-nonce
/// allocations under a mutex so concurrent `/sponsor` requests get
/// distinct nonces.
pub struct Paymaster {
    keypair: HybridKeypair,
    address: AccountAddress,
    chain_id: String,
    nonce_state: Arc<Mutex<NonceState>>,
    config: PaymasterConfig,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    audit_log: Option<Arc<Mutex<AuditLogger>>>,
    idempotency: Arc<Mutex<IdempotencyCache>>,
    metrics: Arc<PaymasterMetrics>,
}

/// Append-only JSON-lines audit log writer. Held inside `Paymaster`
/// when `PaymasterConfig::audit_log` is set. The file handle is
/// opened once with `O_APPEND` at construction so concurrent writers
/// (across the same process) are kernel-serialised at write
/// boundaries; the mutex is for our own line-buffer staging only.
struct AuditLogger {
    file: std::fs::File,
}

#[derive(Debug, Serialize)]
struct AuditEntry<'a> {
    ts_unix_ms: u128,
    sender: String,
    paymaster_nonce: u64,
    call_gas_limit: u64,
    call_data_hash: String,
    chain_id: &'a str,
}

struct NonceState {
    next: u64,
    file: PathBuf,
}

impl Paymaster {
    /// Build a paymaster from an in-memory keypair with default
    /// hardening (`require_user_sig: true`, `per_sender_rps: 5.0`,
    /// `per_sender_burst: 10`). The caller is responsible for keeping
    /// the keypair file-permissioned — a leaked keypair lets anyone
    /// drain the paymaster's balance up to its on-chain
    /// nonce-allocation horizon.
    pub fn new(
        keypair: HybridKeypair,
        chain_id: impl Into<String>,
        nonce_file: impl Into<PathBuf>,
    ) -> Result<Self, PaymasterError> {
        Self::new_with_config(keypair, chain_id, nonce_file, PaymasterConfig::default())
    }

    /// Build a paymaster with explicit hardening config. Use
    /// `PaymasterConfig::permissive()` for testnet / dev profiles
    /// where wallets are still wiring their signing path. Production
    /// deployments should stick with `PaymasterConfig::default()`.
    pub fn new_with_config(
        keypair: HybridKeypair,
        chain_id: impl Into<String>,
        nonce_file: impl Into<PathBuf>,
        config: PaymasterConfig,
    ) -> Result<Self, PaymasterError> {
        let pk = keypair.public_key_bytes();
        let address: AccountAddress = *blake3::hash(&pk).as_bytes();
        let nonce_file: PathBuf = nonce_file.into();
        let next = load_nonce(&nonce_file)?;
        let rate_limiter =
            RateLimiter::new(config.per_sender_rps, config.per_sender_burst);
        let audit_log = if let Some(path) = config.audit_log.clone() {
            // Open with append + create. Each write is followed by
            // an explicit fsync to land the line on disk before the
            // sponsorship returns successfully.
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| {
                    PaymasterError::AuditIo(format!(
                        "open audit log {}: {e}",
                        path.display()
                    ))
                })?;
            Some(Arc::new(Mutex::new(AuditLogger { file })))
        } else {
            None
        };
        let idempotency = IdempotencyCache::new(
            config.idempotency_max_keys,
            Duration::from_secs(config.idempotency_ttl_secs),
        );
        Ok(Self {
            keypair,
            address,
            chain_id: chain_id.into(),
            nonce_state: Arc::new(Mutex::new(NonceState {
                next,
                file: nonce_file,
            })),
            config,
            rate_limiter: Arc::new(Mutex::new(rate_limiter)),
            audit_log,
            idempotency: Arc::new(Mutex::new(idempotency)),
            metrics: Arc::new(PaymasterMetrics::default()),
        })
    }

    /// Address derived from the paymaster's public key. This is what
    /// callers stamp into `UserOpTx::paymaster`.
    pub fn address(&self) -> AccountAddress {
        self.address
    }

    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }

    /// Current next-nonce — what would be assigned on the next call to
    /// `sponsor`. Read-only; useful for the `/info` endpoint and tests.
    pub fn next_paymaster_nonce(&self) -> u64 {
        self.nonce_state.lock().expect("nonce mutex").next
    }

    /// Stamp `paymaster`, `paymaster_nonce`, `paymaster_signature`,
    /// `paymaster_public_key` onto an in-flight `UserOpTx`. Returns
    /// the assigned nonce on success so callers can echo it back to
    /// the wallet.
    ///
    /// Concurrency: holds the nonce mutex across (allocate, persist,
    /// sign) so concurrent `/sponsor` requests get strictly
    /// monotonic nonces and the on-disk file matches the highest
    /// in-flight allocation. A crash mid-flight loses nothing —
    /// the nonce file is fsync'd before the in-memory counter
    /// advances, and the in-memory counter is never consumed twice.
    pub fn sponsor(&self, user_op: &mut UserOpTx) -> Result<u64, PaymasterError> {
        // Wrap the implementation so every outcome — Ok or Err of any
        // variant — increments exactly one metrics counter.
        let result = self.sponsor_inner(user_op);
        match &result {
            Ok(_) => self.metrics.record(Ok(())),
            Err(e) => self.metrics.record(Err(e)),
        }
        result
    }

    /// Idempotent variant of `sponsor`. If `idempotency_key` is `Some`
    /// and matches a cached response, returns the cached
    /// SponsorshipResponse fields (mutating `user_op` in place to
    /// match) — same paymaster_nonce, same signature. The wallet
    /// retries gracefully without burning a fresh nonce.
    ///
    /// Calling with `idempotency_key: None` is equivalent to calling
    /// `sponsor` directly: no cache lookup, no cache insert, fresh
    /// allocation always.
    ///
    /// Each call records exactly one metrics counter:
    ///   - cache hit → `sponsorships_idempotent_replay`
    ///   - cache miss + Ok → `sponsorships_ok`
    ///   - cache miss + Err(variant) → `sponsorships_<variant>`
    pub fn sponsor_idempotent(
        &self,
        idempotency_key: Option<&str>,
        user_op: &mut UserOpTx,
    ) -> Result<SponsorOutcome, PaymasterError> {
        // Cache lookup BEFORE any state mutation.
        if let Some(key) = idempotency_key {
            let mut cache = self.idempotency.lock().expect("idempotency mutex");
            if let Some(cached) = cache.get(key) {
                drop(cache);
                let nonce = cached.paymaster_nonce;
                *user_op = cached.user_op;
                self.metrics
                    .sponsorships_idempotent_replay
                    .fetch_add(1, Ordering::Relaxed);
                return Ok(SponsorOutcome::Replay {
                    paymaster_nonce: nonce,
                });
            }
        }

        // Cache miss (or no key) — run the normal sponsor path.
        let result = self.sponsor_inner(user_op);
        match &result {
            Ok(_) => self.metrics.record(Ok(())),
            Err(e) => self.metrics.record(Err(e)),
        }
        let nonce = result?;

        // Cache the response for future retries under this key.
        if let Some(key) = idempotency_key {
            let response = SponsorshipResponse {
                user_op: user_op.clone(),
                paymaster_address_hex: hex::encode(self.address),
                paymaster_nonce: nonce,
            };
            self.idempotency
                .lock()
                .expect("idempotency mutex")
                .insert(key.to_string(), response);
        }

        Ok(SponsorOutcome::Fresh {
            paymaster_nonce: nonce,
        })
    }

    fn sponsor_inner(&self, user_op: &mut UserOpTx) -> Result<u64, PaymasterError> {
        if user_op.paymaster_signature.is_some() {
            return Err(PaymasterError::AlreadySigned);
        }

        // Day 7 hardening — per-sender rate limit. Done BEFORE the
        // user-sig check + nonce allocation so a flooding sender
        // doesn't pay our verification CPU either, only their own
        // bucket arithmetic.
        {
            let mut limiter = self.rate_limiter.lock().expect("rate-limiter mutex");
            if !limiter.try_consume(user_op.sender) {
                return Err(PaymasterError::RateLimited {
                    sender_hex: hex::encode(user_op.sender),
                });
            }
        }

        // Stamp `paymaster` BEFORE the user-sig check so the canonical
        // message we verify against matches what the chain will
        // verify against post-sponsor. Overwriting any stale value
        // is intentional: a wallet that pre-stamped `paymaster` did
        // so to commit to THIS paymaster (its address); if the
        // pre-stamped value differs from `self.address`, the
        // overwrite invalidates the user sig and we reject below —
        // protecting against a wallet that signed for a different
        // paymaster. We do NOT stamp `paymaster_nonce` here; that's
        // allocated only after the user-sig check passes, so a
        // rejected request doesn't burn a nonce.
        user_op.paymaster = Some(self.address);

        // Day 7 hardening — pre-validate the user-side signature.
        // The chain rejects a UserOp with a bad user sig at execute
        // time, but by then the paymaster has already paid gas. We
        // verify here under the SAME canonical message the chain
        // uses (`Transaction::UserOp(user_op).signing_message(chain_id)`)
        // — a passing check means the chain will also accept.
        if self.config.require_user_sig {
            let sig = user_op
                .signature
                .as_deref()
                .ok_or(PaymasterError::InvalidUserSignature)?;
            let pk = user_op
                .public_key
                .as_deref()
                .ok_or(PaymasterError::InvalidUserSignature)?;
            let canonical = Transaction::UserOp(user_op.clone()).signing_message(&self.chain_id);
            if !HybridVerifier::verify(&canonical, sig, pk) {
                return Err(PaymasterError::InvalidUserSignature);
            }
        }

        // Day 10 — per-paymaster inner-tx whitelist. Operator narrows
        // which inner variants this paymaster will sponsor; default
        // (None) trusts the chain. Empty call_data is always allowed
        // — gas-only sponsorship has no inner intent to classify.
        if let Some(ref allowed) = self.config.allowed_inner_variants {
            if !user_op.call_data.is_empty() {
                let inner: Transaction = serde_json::from_slice(&user_op.call_data)
                    .map_err(|e| {
                        PaymasterError::CallDataDecode(format!("decode: {e}"))
                    })?;
                let variant = InnerVariant::from_transaction(&inner).ok_or_else(|| {
                    // Variant the chain doesn't accept as inner — the
                    // chain would reject this anyway, but surface
                    // earlier with a clearer message.
                    PaymasterError::InnerVariantNotAllowed {
                        variant: format!(
                            "{:?}",
                            std::mem::discriminant(&inner)
                        ),
                    }
                })?;
                if !allowed.contains(&variant) {
                    return Err(PaymasterError::InnerVariantNotAllowed {
                        variant: variant.as_str().to_string(),
                    });
                }
            }
        }

        let mut state = self.nonce_state.lock().expect("nonce mutex");
        let assigned = state.next;
        // Persist BEFORE advancing the in-memory counter and before
        // signing. If persist fails, we surface the IO error and the
        // counter does not advance — caller can retry.
        persist_nonce(&state.file, assigned + 1)?;
        state.next = assigned + 1;
        // Drop the lock before doing the (relatively expensive) sign
        // step? No — we want the nonce + signature pair to be
        // strictly serialised so a nonce is never assigned without a
        // matching signature being produced. Hold the lock.

        user_op.paymaster = Some(self.address);
        user_op.paymaster_nonce = Some(assigned);
        user_op.paymaster_public_key = Some(self.keypair.public_key_bytes());

        let payload = user_op
            .paymaster_sponsorship_payload(&self.chain_id)
            .expect("paymaster + paymaster_nonce both Some after stamping");
        user_op.paymaster_signature = Some(self.keypair.sign(&payload));

        // Day 8 — append audit line. Fail-closed: an unsponsorable
        // sponsor request would be a billing-reconciliation hole, so
        // we surface IO errors rather than silently dropping audit
        // lines. We hold the nonce mutex across this write so audit
        // line ordering matches paymaster_nonce ordering.
        if let Some(ref logger) = self.audit_log {
            let entry = AuditEntry {
                ts_unix_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0),
                sender: format!("0x{}", hex::encode(user_op.sender)),
                paymaster_nonce: assigned,
                call_gas_limit: user_op.call_gas_limit,
                call_data_hash: format!(
                    "0x{}",
                    hex::encode(blake3::hash(&user_op.call_data).as_bytes())
                ),
                chain_id: &self.chain_id,
            };
            let mut line = serde_json::to_string(&entry)
                .map_err(|e| PaymasterError::AuditIo(format!("serialize: {e}")))?;
            line.push('\n');
            let mut g = logger.lock().expect("audit log mutex");
            use std::io::Write;
            g.file
                .write_all(line.as_bytes())
                .map_err(|e| PaymasterError::AuditIo(format!("write: {e}")))?;
            g.file
                .sync_all()
                .map_err(|e| PaymasterError::AuditIo(format!("fsync: {e}")))?;
        }

        Ok(assigned)
    }

    /// Build a `PaymasterInfo` for the `/info` endpoint.
    pub fn info(&self) -> PaymasterInfo {
        PaymasterInfo {
            paymaster_address_hex: hex::encode(self.address),
            next_paymaster_nonce: self.next_paymaster_nonce(),
            chain_id: self.chain_id.clone(),
            require_user_sig: self.config.require_user_sig,
            per_sender_rps: self.config.per_sender_rps,
            per_sender_burst: self.config.per_sender_burst,
            audit_log_enabled: self.config.audit_log.is_some(),
            allowed_inner_variants: self.config.allowed_inner_variants.as_ref().map(|set| {
                set.iter().map(|v| v.as_str().to_string()).collect()
            }),
            idempotency_max_keys: self.config.idempotency_max_keys,
            idempotency_ttl_secs: self.config.idempotency_ttl_secs,
        }
    }

    /// Read-only handle to the metrics struct. Intended for tests +
    /// the `/metrics` HTTP handler in the binary.
    pub fn metrics(&self) -> &PaymasterMetrics {
        &self.metrics
    }

    /// Render Prometheus exposition format. Hand-written rather than
    /// pulled in via the `prometheus` crate — we have a small, stable
    /// metrics surface and the format is simple. Operators scrape
    /// this with `prometheus`, `vmagent`, `vector`, etc. via standard
    /// HTTP GET. Returns text/plain with `; version=0.0.4` content
    /// per the Prometheus exposition spec (the binary supplies that
    /// header).
    pub fn prometheus_metrics(&self) -> String {
        let m = &self.metrics;
        let now = Instant::now();
        let uptime = now.duration_since(m.started_at).as_secs();
        let next_nonce = self.next_paymaster_nonce();
        let active_senders = self
            .rate_limiter
            .lock()
            .map(|r| r.buckets.len() as u64)
            .unwrap_or(0);
        let load = |c: &AtomicU64| c.load(Ordering::Relaxed);

        let mut out = String::new();
        out.push_str(
            "# HELP evaporchain_paymaster_sponsorships_total \
             Number of /sponsor requests by outcome.\n\
             # TYPE evaporchain_paymaster_sponsorships_total counter\n",
        );
        let pairs = [
            ("ok", load(&m.sponsorships_ok)),
            ("already_signed", load(&m.sponsorships_already_signed)),
            ("invalid_user_sig", load(&m.sponsorships_invalid_user_sig)),
            ("rate_limited", load(&m.sponsorships_rate_limited)),
            ("nonce_io", load(&m.sponsorships_nonce_io)),
            ("audit_io", load(&m.sponsorships_audit_io)),
            ("other", load(&m.sponsorships_other)),
        ];
        for (status, count) in pairs {
            out.push_str(&format!(
                "evaporchain_paymaster_sponsorships_total{{status=\"{status}\"}} {count}\n"
            ));
        }
        out.push_str(
            "# HELP evaporchain_paymaster_next_nonce \
             Next sponsorship nonce that will be assigned.\n\
             # TYPE evaporchain_paymaster_next_nonce gauge\n",
        );
        out.push_str(&format!(
            "evaporchain_paymaster_next_nonce {next_nonce}\n"
        ));
        out.push_str(
            "# HELP evaporchain_paymaster_active_senders \
             Number of senders currently held in the rate-limiter \
             HashMap (active or in flight, before idle GC).\n\
             # TYPE evaporchain_paymaster_active_senders gauge\n",
        );
        out.push_str(&format!(
            "evaporchain_paymaster_active_senders {active_senders}\n"
        ));
        out.push_str(
            "# HELP evaporchain_paymaster_uptime_seconds \
             Process uptime in seconds since paymaster construction.\n\
             # TYPE evaporchain_paymaster_uptime_seconds gauge\n",
        );
        out.push_str(&format!(
            "evaporchain_paymaster_uptime_seconds {uptime}\n"
        ));
        out.push_str(
            "# HELP evaporchain_paymaster_idempotent_replays_total \
             Number of /sponsor calls served from the idempotency cache.\n\
             # TYPE evaporchain_paymaster_idempotent_replays_total counter\n",
        );
        out.push_str(&format!(
            "evaporchain_paymaster_idempotent_replays_total {}\n",
            load(&m.sponsorships_idempotent_replay)
        ));
        out
    }
}

fn load_nonce(path: &Path) -> Result<u64, PaymasterError> {
    match std::fs::read_to_string(path) {
        Ok(s) => s
            .trim()
            .parse::<u64>()
            .map_err(|e| PaymasterError::NonceParse(e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(PaymasterError::NonceIo(e)),
    }
}

/// Atomically persist `next` to `path` via tempfile + rename + fsync
/// of both file and parent directory.
fn persist_nonce(path: &Path, next: u64) -> Result<(), PaymasterError> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let tmp = parent.join(format!(
        ".paymaster_nonce.tmp.{}",
        std::process::id()
    ));
    {
        let mut f = std::fs::File::create(&tmp)?;
        writeln!(f, "{next}")?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    // Best-effort parent fsync — not all FS support open-on-dir;
    // ignore EISDIR / EPERM on platforms that don't.
    if let Ok(d) = std::fs::File::open(&parent) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Load a hybrid keypair from a JSON file. Convenience for the binary —
/// production deployments may load via a KMS / secret manager instead.
///
/// File format:
///
/// ```json
/// {
///   "ecdsa_secret_hex": "...",
///   "mldsa_public_hex": "...",
///   "mldsa_secret_hex": "..."
/// }
/// ```
///
/// ECDSA derives its public key from the secret. ML-DSA's pqc_dilithium
/// representation can't reconstruct the public from secret alone, so
/// both are stored.
pub fn load_keypair_from_file(path: &Path) -> anyhow::Result<HybridKeypair> {
    use evaporchain_crypto::signatures::{EcdsaKeypair, MlDsaKeypair};
    #[derive(Deserialize)]
    struct OnDisk {
        ecdsa_secret_hex: String,
        mldsa_public_hex: String,
        mldsa_secret_hex: String,
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read keypair file {}: {e}", path.display()))?;
    let parsed: OnDisk = serde_json::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("parse keypair file {}: {e}", path.display()))?;
    let ecdsa_sk = hex::decode(parsed.ecdsa_secret_hex)
        .map_err(|e| anyhow::anyhow!("ecdsa hex: {e}"))?;
    let mldsa_pk = hex::decode(parsed.mldsa_public_hex)
        .map_err(|e| anyhow::anyhow!("mldsa pk hex: {e}"))?;
    let mldsa_sk = hex::decode(parsed.mldsa_secret_hex)
        .map_err(|e| anyhow::anyhow!("mldsa sk hex: {e}"))?;
    let ecdsa = EcdsaKeypair::from_bytes(&ecdsa_sk)
        .map_err(|e| anyhow::anyhow!("ecdsa from bytes: {e}"))?;
    let mldsa = MlDsaKeypair::from_bytes(&mldsa_pk, &mldsa_sk)
        .map_err(|e| anyhow::anyhow!("mldsa from bytes: {e}"))?;
    Ok(HybridKeypair::from_parts(ecdsa, mldsa))
}

/// Generate a fresh keypair and write it to `path` in the same JSON
/// format `load_keypair_from_file` expects. Intended for first-run /
/// dev setups; production should use a KMS.
pub fn generate_keypair_to_file(path: &Path) -> anyhow::Result<HybridKeypair> {
    use std::io::Write;
    let kp = HybridKeypair::generate();
    let on_disk = serde_json::json!({
        "ecdsa_secret_hex": hex::encode(kp.ecdsa.secret_key_bytes()),
        "mldsa_public_hex": hex::encode(kp.mldsa.public_key()),
        "mldsa_secret_hex": hex::encode(kp.mldsa.secret_key()),
    });
    let mut f = std::fs::File::create(path)
        .map_err(|e| anyhow::anyhow!("create keypair file {}: {e}", path.display()))?;
    writeln!(f, "{}", serde_json::to_string_pretty(&on_disk)?)?;
    f.sync_all()?;
    Ok(kp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_crypto::signatures::{HybridVerifier, Verifier};
    use evaporchain_types::{Transaction, TransferTx};
    use tempfile::TempDir;

    fn fresh_paymaster(tmp: &TempDir, chain_id: &str) -> Paymaster {
        // Existing tests pre-date the Day 7 hardening — they exercise
        // nonce monotonicity, persistence, sponsor stamping, etc. and
        // don't construct user-signed UserOps. Use the permissive
        // profile so they still test what they intend to.
        let kp = HybridKeypair::generate();
        let nonce_file = tmp.path().join("paymaster_nonce");
        Paymaster::new_with_config(kp, chain_id, nonce_file, PaymasterConfig::permissive())
            .expect("paymaster")
    }

    #[test]
    fn sponsor_stamps_all_four_paymaster_fields() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pm = fresh_paymaster(&tmp, "test");
        let mut user_op = UserOpTx {
            sender: [1u8; 32],
            nonce: 0,
            call_data: vec![],
            call_gas_limit: 1000,
            paymaster: None,
            paymaster_nonce: None,
            paymaster_data: None,
            paymaster_signature: None,
            paymaster_public_key: None,
            signature: None,
            public_key: None,
        };
        let assigned = pm.sponsor(&mut user_op).expect("sponsor");
        assert_eq!(assigned, 0);
        assert_eq!(user_op.paymaster, Some(pm.address()));
        assert_eq!(user_op.paymaster_nonce, Some(0));
        assert!(user_op.paymaster_signature.is_some());
        assert!(user_op.paymaster_public_key.is_some());
    }

    #[test]
    fn sponsor_assigns_monotonic_nonces() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pm = fresh_paymaster(&tmp, "test");
        let mut a = blank_user_op();
        let mut b = blank_user_op();
        let mut c = blank_user_op();
        let na = pm.sponsor(&mut a).unwrap();
        let nb = pm.sponsor(&mut b).unwrap();
        let nc = pm.sponsor(&mut c).unwrap();
        assert_eq!((na, nb, nc), (0, 1, 2));
        assert_eq!(pm.next_paymaster_nonce(), 3);
    }

    #[test]
    fn nonce_persists_across_paymaster_restart() {
        let tmp = tempfile::TempDir::new().unwrap();
        let nonce_file = tmp.path().join("paymaster_nonce");
        let kp = HybridKeypair::generate();
        let pk_bytes = kp.public_key_bytes();
        // First instance: sponsor twice.
        {
            // Reload-only path needs the same keypair, but
            // `Paymaster::new` consumes it; we don't call `new` twice
            // with the same kp here — the persistence test only cares
            // about the nonce counter.
            let pm = Paymaster::new_with_config(
                kp,
                "test",
                &nonce_file,
                PaymasterConfig::permissive(),
            )
            .unwrap();
            pm.sponsor(&mut blank_user_op()).unwrap();
            pm.sponsor(&mut blank_user_op()).unwrap();
            assert_eq!(pm.next_paymaster_nonce(), 2);
        }
        // Second instance with a fresh keypair but the same nonce
        // file: counter resumes from 2.
        let kp2 = HybridKeypair::generate();
        // Sanity: regenerating gives a different pk (so it's a real
        // restart-with-new-key scenario).
        assert_ne!(pk_bytes, kp2.public_key_bytes());
        let pm2 = Paymaster::new_with_config(
            kp2,
            "test",
            &nonce_file,
            PaymasterConfig::permissive(),
        )
        .unwrap();
        assert_eq!(pm2.next_paymaster_nonce(), 2);
    }

    #[test]
    fn sponsorship_signature_verifies_under_chain_payload_rules() {
        // The chain's execute_user_op verifies sponsorship via
        // (a) blake3(pk) deriving to paymaster, and
        // (b) HybridVerifier::verify(payload, sig, pk) where payload
        //     is UserOpTx::paymaster_sponsorship_payload(chain_id).
        //
        // execute_user_op is private to evaporchain-execution, but the
        // binding it enforces is exactly the two checks above. If both
        // hold, the chain accepts.
        let tmp = TempDir::new().unwrap();
        let chain_id = "evaporchain-mainnet";
        let pm = fresh_paymaster(&tmp, chain_id);
        let pm_addr = pm.address();

        let inner = Transaction::Transfer(TransferTx {
            from: [1u8; 32],
            to: [9u8; 32],
            amount: 500,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let mut user_op = UserOpTx {
            sender: [1u8; 32],
            nonce: 0,
            call_data: serde_json::to_vec(&inner).unwrap(),
            call_gas_limit: 50_000,
            paymaster: None,
            paymaster_nonce: None,
            paymaster_data: None,
            paymaster_signature: None,
            paymaster_public_key: None,
            signature: None,
            public_key: None,
        };
        pm.sponsor(&mut user_op).unwrap();

        // (a) pk derives to paymaster address.
        let pk = user_op.paymaster_public_key.as_deref().unwrap();
        let derived: AccountAddress = *blake3::hash(pk).as_bytes();
        assert_eq!(derived, pm_addr);
        // (b) sig verifies under the canonical sponsorship payload.
        let payload = user_op
            .paymaster_sponsorship_payload(chain_id)
            .expect("payload available after sponsor");
        let sig = user_op.paymaster_signature.as_deref().unwrap();
        assert!(
            HybridVerifier::verify(&payload, sig, pk),
            "sponsorship sig must verify under chain rules"
        );
    }

    #[test]
    fn refuses_to_overwrite_existing_signature() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pm = fresh_paymaster(&tmp, "test");
        let mut user_op = blank_user_op();
        user_op.paymaster_signature = Some(vec![0u8; 32]);
        let r = pm.sponsor(&mut user_op);
        assert!(matches!(r, Err(PaymasterError::AlreadySigned)));
    }

    fn blank_user_op() -> UserOpTx {
        UserOpTx {
            sender: [1u8; 32],
            nonce: 0,
            call_data: vec![],
            call_gas_limit: 1000,
            paymaster: None,
            paymaster_nonce: None,
            paymaster_data: None,
            paymaster_signature: None,
            paymaster_public_key: None,
            signature: None,
            public_key: None,
        }
    }

    // ─── Day 7: spam-signing hardening tests ──────────────────────────

    /// Build a paymaster with default (strict) config — require_user_sig
    /// = true + per_sender_rps = 5.0 + per_sender_burst = 10.
    fn strict_paymaster(tmp: &TempDir, chain_id: &str) -> Paymaster {
        let kp = HybridKeypair::generate();
        let nonce_file = tmp.path().join("paymaster_nonce");
        Paymaster::new(kp, chain_id, nonce_file).expect("paymaster")
    }

    /// Build a UserOp with a valid user-side signature stamped over
    /// the canonical message the chain (and now the paymaster) check
    /// against. Returns (UserOp, user_keypair) so callers can mutate
    /// + re-sign for tampering tests.
    fn user_signed_user_op_for(
        pm: &Paymaster,
        sender_byte: u8,
    ) -> (UserOpTx, HybridKeypair) {
        let user_kp = HybridKeypair::generate();
        let sender: AccountAddress = *blake3::hash(&user_kp.public_key_bytes()).as_bytes();
        let _ = sender_byte; // sender derived from key; param kept for tests that may want a fixed slot
        let mut user_op = UserOpTx {
            sender,
            nonce: 0,
            call_data: vec![],
            call_gas_limit: 1000,
            // Wallet pre-stamps paymaster so the user sig commits to it;
            // the paymaster will overwrite this to its own address before
            // the user-sig check, so the value MUST already match.
            paymaster: Some(pm.address()),
            paymaster_nonce: None,
            paymaster_data: None,
            paymaster_signature: None,
            paymaster_public_key: None,
            signature: None,
            public_key: None,
        };
        let canonical = Transaction::UserOp(user_op.clone()).signing_message(pm.chain_id());
        user_op.signature = Some(user_kp.sign(&canonical));
        user_op.public_key = Some(user_kp.public_key_bytes());
        (user_op, user_kp)
    }

    #[test]
    fn strict_mode_rejects_userop_without_user_signature() {
        // require_user_sig = true; UserOp with signature = None gets
        // rejected before the paymaster spends its sponsorship sig.
        let tmp = TempDir::new().unwrap();
        let pm = strict_paymaster(&tmp, "test");
        let mut user_op = blank_user_op();
        let r = pm.sponsor(&mut user_op);
        assert!(matches!(r, Err(PaymasterError::InvalidUserSignature)));
        // Counter NOT advanced — rejected before nonce allocation.
        assert_eq!(pm.next_paymaster_nonce(), 0);
    }

    #[test]
    fn strict_mode_rejects_userop_with_invalid_user_signature() {
        // Wallet attempts to flood `/sponsor` with bad sigs; paymaster
        // catches it pre-allocation.
        let tmp = TempDir::new().unwrap();
        let pm = strict_paymaster(&tmp, "test");
        let mut user_op = blank_user_op();
        user_op.paymaster = Some(pm.address());
        user_op.signature = Some(vec![0u8; 4001]); // bogus
        user_op.public_key = Some(vec![0u8; 1986]); // wrong-shape pk
        let r = pm.sponsor(&mut user_op);
        assert!(matches!(r, Err(PaymasterError::InvalidUserSignature)));
        assert_eq!(pm.next_paymaster_nonce(), 0);
    }

    #[test]
    fn strict_mode_accepts_correctly_signed_userop() {
        let tmp = TempDir::new().unwrap();
        let pm = strict_paymaster(&tmp, "evaporchain-mainnet");
        let (mut user_op, _) = user_signed_user_op_for(&pm, 1);
        let assigned = pm.sponsor(&mut user_op).expect("strict-mode sponsor");
        assert_eq!(assigned, 0);
        assert!(user_op.paymaster_signature.is_some());
    }

    #[test]
    fn strict_mode_rejects_user_signed_for_different_paymaster() {
        // Wallet signs with paymaster = some-other-address, then
        // tries to redirect to us. Our overwrite + user-sig check
        // catches the redirect.
        let tmp = TempDir::new().unwrap();
        let pm = strict_paymaster(&tmp, "test");
        let user_kp = HybridKeypair::generate();
        let sender: AccountAddress = *blake3::hash(&user_kp.public_key_bytes()).as_bytes();
        let mut user_op = UserOpTx {
            sender,
            nonce: 0,
            call_data: vec![],
            call_gas_limit: 1000,
            paymaster: Some([0xAA; 32]), // ← signed for a different paymaster
            paymaster_nonce: None,
            paymaster_data: None,
            paymaster_signature: None,
            paymaster_public_key: None,
            signature: None,
            public_key: None,
        };
        let canonical = Transaction::UserOp(user_op.clone()).signing_message(pm.chain_id());
        user_op.signature = Some(user_kp.sign(&canonical));
        user_op.public_key = Some(user_kp.public_key_bytes());

        let r = pm.sponsor(&mut user_op);
        assert!(matches!(r, Err(PaymasterError::InvalidUserSignature)));
    }

    #[test]
    fn rate_limiter_throttles_after_burst_exceeded() {
        // Burst = 3, rps = 0 (no replenishment) — first 3 sponsors
        // succeed, the 4th hits RateLimited.
        let tmp = TempDir::new().unwrap();
        let kp = HybridKeypair::generate();
        let nonce_file = tmp.path().join("paymaster_nonce");
        let pm = Paymaster::new_with_config(
            kp,
            "test",
            nonce_file,
            PaymasterConfig {
                require_user_sig: false,
                per_sender_rps: 0.000_001, // effectively zero refill in test window
                per_sender_burst: 3,
                audit_log: None,
                allowed_inner_variants: None,
                idempotency_max_keys: 0,
                idempotency_ttl_secs: 0,
            },
        )
        .unwrap();

        for i in 0..3 {
            let mut uo = blank_user_op();
            pm.sponsor(&mut uo).expect("first 3 succeed");
            assert_eq!(pm.next_paymaster_nonce(), i + 1);
        }
        let mut uo4 = blank_user_op();
        let r = pm.sponsor(&mut uo4);
        assert!(matches!(r, Err(PaymasterError::RateLimited { .. })));
        // Counter NOT advanced — rejected before nonce allocation.
        assert_eq!(pm.next_paymaster_nonce(), 3);
    }

    #[test]
    fn rate_limiter_keys_per_sender() {
        // Rate limit hit on sender A doesn't affect sender B.
        let tmp = TempDir::new().unwrap();
        let kp = HybridKeypair::generate();
        let nonce_file = tmp.path().join("paymaster_nonce");
        let pm = Paymaster::new_with_config(
            kp,
            "test",
            nonce_file,
            PaymasterConfig {
                require_user_sig: false,
                per_sender_rps: 0.000_001,
                per_sender_burst: 1,
                audit_log: None,
                allowed_inner_variants: None,
                idempotency_max_keys: 0,
                idempotency_ttl_secs: 0,
            },
        )
        .unwrap();

        // Sender A gets one through, then is throttled.
        let mut a1 = blank_user_op();
        a1.sender = [0xAA; 32];
        pm.sponsor(&mut a1).unwrap();
        let mut a2 = blank_user_op();
        a2.sender = [0xAA; 32];
        let r = pm.sponsor(&mut a2);
        assert!(matches!(r, Err(PaymasterError::RateLimited { .. })));

        // Sender B has its own bucket — gets through.
        let mut b1 = blank_user_op();
        b1.sender = [0xBB; 32];
        pm.sponsor(&mut b1).expect("B's first sponsor allowed");
    }

    // ─── Day 8: audit log tests ───────────────────────────────────────

    #[test]
    fn audit_log_writes_one_line_per_successful_sponsor() {
        let tmp = TempDir::new().unwrap();
        let nonce_file = tmp.path().join("paymaster_nonce");
        let audit_file = tmp.path().join("audit.jsonl");
        let kp = HybridKeypair::generate();
        let pm = Paymaster::new_with_config(
            kp,
            "test",
            nonce_file,
            PaymasterConfig {
                require_user_sig: false,
                per_sender_rps: 0.0,
                per_sender_burst: 0,
                audit_log: Some(audit_file.clone()),
                allowed_inner_variants: None,
                idempotency_max_keys: 0,
                idempotency_ttl_secs: 0,
            },
        )
        .unwrap();

        for _ in 0..3 {
            let mut uo = blank_user_op();
            pm.sponsor(&mut uo).unwrap();
        }

        let contents = std::fs::read_to_string(&audit_file).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 3, "one line per sponsorship");
        for (i, line) in lines.iter().enumerate() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(v["paymaster_nonce"].as_u64().unwrap(), i as u64);
            assert_eq!(v["chain_id"], "test");
            assert!(v["sender"].as_str().unwrap().starts_with("0x"));
            assert!(v["call_data_hash"].as_str().unwrap().starts_with("0x"));
            assert!(v["ts_unix_ms"].as_u64().unwrap() > 0);
        }
    }

    #[test]
    fn audit_log_persists_across_restart() {
        // First instance writes 2 lines; second instance opens with
        // O_APPEND and writes 2 more. Final file has 4 lines in
        // monotonic paymaster_nonce order.
        let tmp = TempDir::new().unwrap();
        let nonce_file = tmp.path().join("paymaster_nonce");
        let audit_file = tmp.path().join("audit.jsonl");
        let cfg = || PaymasterConfig {
            require_user_sig: false,
            per_sender_rps: 0.0,
            per_sender_burst: 0,
            audit_log: Some(audit_file.clone()),
            allowed_inner_variants: None,
            idempotency_max_keys: 0,
            idempotency_ttl_secs: 0,
        };

        {
            let kp = HybridKeypair::generate();
            let pm =
                Paymaster::new_with_config(kp, "test", &nonce_file, cfg()).unwrap();
            for _ in 0..2 {
                pm.sponsor(&mut blank_user_op()).unwrap();
            }
        }
        {
            let kp = HybridKeypair::generate();
            let pm =
                Paymaster::new_with_config(kp, "test", &nonce_file, cfg()).unwrap();
            for _ in 0..2 {
                pm.sponsor(&mut blank_user_op()).unwrap();
            }
        }

        let contents = std::fs::read_to_string(&audit_file).unwrap();
        let lines: Vec<_> = contents
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 4);
        let nonces: Vec<u64> = lines
            .iter()
            .map(|v| v["paymaster_nonce"].as_u64().unwrap())
            .collect();
        assert_eq!(nonces, vec![0, 1, 2, 3]);
    }

    #[test]
    fn audit_log_records_call_data_hash() {
        // call_data_hash in the audit line matches blake3(call_data)
        // — operators reconcile against `paymaster_sponsorship_payload`
        // which binds the same hash.
        let tmp = TempDir::new().unwrap();
        let nonce_file = tmp.path().join("paymaster_nonce");
        let audit_file = tmp.path().join("audit.jsonl");
        let kp = HybridKeypair::generate();
        let pm = Paymaster::new_with_config(
            kp,
            "test",
            nonce_file,
            PaymasterConfig {
                require_user_sig: false,
                per_sender_rps: 0.0,
                per_sender_burst: 0,
                audit_log: Some(audit_file.clone()),
                allowed_inner_variants: None,
                idempotency_max_keys: 0,
                idempotency_ttl_secs: 0,
            },
        )
        .unwrap();

        let mut uo = blank_user_op();
        uo.call_data = b"hello-paymaster".to_vec();
        let expected = format!(
            "0x{}",
            hex::encode(blake3::hash(&uo.call_data).as_bytes())
        );
        pm.sponsor(&mut uo).unwrap();

        let contents = std::fs::read_to_string(&audit_file).unwrap();
        let entry: serde_json::Value =
            serde_json::from_str(contents.lines().next().unwrap()).unwrap();
        assert_eq!(entry["call_data_hash"], expected);
    }

    // ─── Day 10: per-paymaster inner-tx whitelist tests ──────────────

    fn permissive_with_inner_whitelist(
        tmp: &TempDir,
        chain_id: &str,
        allowed: Option<Vec<InnerVariant>>,
    ) -> Paymaster {
        let kp = HybridKeypair::generate();
        let nonce_file = tmp.path().join("paymaster_nonce");
        Paymaster::new_with_config(
            kp,
            chain_id,
            nonce_file,
            PaymasterConfig {
                require_user_sig: false,
                per_sender_rps: 0.0,
                per_sender_burst: 0,
                audit_log: None,
                allowed_inner_variants: allowed,
                idempotency_max_keys: 0,
                idempotency_ttl_secs: 0,
            },
        )
        .unwrap()
    }

    fn user_op_wrapping(
        inner: Transaction,
    ) -> UserOpTx {
        UserOpTx {
            sender: [1u8; 32],
            nonce: 0,
            call_data: serde_json::to_vec(&inner).unwrap(),
            call_gas_limit: 50_000,
            paymaster: None,
            paymaster_nonce: None,
            paymaster_data: None,
            paymaster_signature: None,
            paymaster_public_key: None,
            signature: None,
            public_key: None,
        }
    }

    #[test]
    fn inner_whitelist_default_none_accepts_any_inner_variant() {
        let tmp = TempDir::new().unwrap();
        let pm = permissive_with_inner_whitelist(&tmp, "test", None);
        for inner in [
            Transaction::Transfer(evaporchain_types::TransferTx {
                from: [1u8; 32],
                to: [2u8; 32],
                amount: 1,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            }),
            Transaction::CallScript(evaporchain_types::CallScriptTx {
                caller: [1u8; 32],
                contract_id: 1,
                method: "noop".into(),
                args: "[]".into(),
                epoch: 0,
                signature: None,
                public_key: None,
            }),
            Transaction::CallContract(evaporchain_types::CallContractTx {
                caller: [1u8; 32],
                contract_id: 1,
                method: "noop".into(),
                args: "{}".into(),
                epoch: 0,
                signature: None,
                public_key: None,
            }),
        ] {
            let mut uo = user_op_wrapping(inner);
            pm.sponsor(&mut uo).expect("None whitelist allows everything");
        }
    }

    #[test]
    fn inner_whitelist_transfer_only_rejects_call_script() {
        let tmp = TempDir::new().unwrap();
        let pm = permissive_with_inner_whitelist(
            &tmp,
            "test",
            Some(vec![InnerVariant::Transfer]),
        );
        // Transfer accepted.
        let mut transfer_uo = user_op_wrapping(Transaction::Transfer(
            evaporchain_types::TransferTx {
                from: [1u8; 32],
                to: [2u8; 32],
                amount: 1,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            },
        ));
        pm.sponsor(&mut transfer_uo).expect("Transfer in whitelist");

        // CallScript rejected even though it's in the chain's V1 set.
        let mut script_uo = user_op_wrapping(Transaction::CallScript(
            evaporchain_types::CallScriptTx {
                caller: [1u8; 32],
                contract_id: 1,
                method: "noop".into(),
                args: "[]".into(),
                epoch: 0,
                signature: None,
                public_key: None,
            },
        ));
        let r = pm.sponsor(&mut script_uo);
        assert!(
            matches!(
                r,
                Err(PaymasterError::InnerVariantNotAllowed { ref variant })
                    if variant == "call_script"
            ),
            "got: {r:?}"
        );
        // Counter NOT advanced for the rejected request.
        assert_eq!(pm.next_paymaster_nonce(), 1);
    }

    #[test]
    fn inner_whitelist_empty_call_data_always_allowed() {
        // Even an empty (i.e., maximally restrictive) whitelist
        // accepts gas-only sponsorships — no inner intent to classify.
        let tmp = TempDir::new().unwrap();
        let pm = permissive_with_inner_whitelist(&tmp, "test", Some(vec![]));
        let mut uo = blank_user_op();
        // call_data is `vec![]` from blank_user_op.
        pm.sponsor(&mut uo).expect("empty call_data bypasses whitelist");
    }

    #[test]
    fn inner_whitelist_rejects_non_chain_whitelisted_variant() {
        // Chain rejects MultiSig as inner; paymaster surfaces this
        // earlier via from_transaction returning None.
        let tmp = TempDir::new().unwrap();
        let pm = permissive_with_inner_whitelist(
            &tmp,
            "test",
            Some(vec![InnerVariant::Transfer]),
        );
        let inner = Transaction::MultiSig(evaporchain_types::MultiSigTx {
            multisig_address: [1u8; 32],
            threshold: 1,
            signers: vec![],
            inner_tx_bytes: vec![],
            signatures: vec![],
            public_keys: vec![],
            nonce: 0,
        });
        let mut uo = user_op_wrapping(inner);
        let r = pm.sponsor(&mut uo);
        assert!(matches!(
            r,
            Err(PaymasterError::InnerVariantNotAllowed { .. })
        ));
    }

    #[test]
    fn inner_whitelist_surfaces_decode_error() {
        // call_data that's not a JSON Transaction → CallDataDecode.
        // Without `allowed_inner_variants` set, the paymaster doesn't
        // decode and the chain handles it later. With the whitelist
        // active, we decode here and reject early.
        let tmp = TempDir::new().unwrap();
        let pm = permissive_with_inner_whitelist(
            &tmp,
            "test",
            Some(vec![InnerVariant::Transfer]),
        );
        let mut uo = blank_user_op();
        uo.call_data = b"not-json".to_vec();
        let r = pm.sponsor(&mut uo);
        assert!(matches!(r, Err(PaymasterError::CallDataDecode(_))));
    }

    // ─── Day 12: idempotency tests ───────────────────────────────────

    fn idempotent_paymaster(tmp: &TempDir) -> Paymaster {
        let kp = HybridKeypair::generate();
        let nonce_file = tmp.path().join("paymaster_nonce");
        Paymaster::new_with_config(
            kp,
            "test",
            nonce_file,
            PaymasterConfig {
                require_user_sig: false,
                per_sender_rps: 0.0,
                per_sender_burst: 0,
                audit_log: None,
                allowed_inner_variants: None,
                idempotency_max_keys: 16,
                idempotency_ttl_secs: 60,
            },
        )
        .unwrap()
    }

    #[test]
    fn idempotent_replay_returns_cached_response() {
        let tmp = TempDir::new().unwrap();
        let pm = idempotent_paymaster(&tmp);
        let mut a = blank_user_op();
        let key = "wallet-tx-uuid-123";
        let first = pm.sponsor_idempotent(Some(key), &mut a).unwrap();
        let first_nonce = match first {
            SponsorOutcome::Fresh { paymaster_nonce } => paymaster_nonce,
            SponsorOutcome::Replay { .. } => panic!("first call must be Fresh"),
        };
        let first_sig = a.paymaster_signature.clone().unwrap();

        // Retry under the same key — must replay.
        let mut b = blank_user_op();
        let second = pm.sponsor_idempotent(Some(key), &mut b).unwrap();
        match second {
            SponsorOutcome::Replay { paymaster_nonce } => {
                assert_eq!(paymaster_nonce, first_nonce);
            }
            SponsorOutcome::Fresh { .. } => panic!("retry must be Replay"),
        }
        assert_eq!(b.paymaster_signature.as_ref().unwrap(), &first_sig);
        assert_eq!(b.paymaster_nonce, Some(first_nonce));

        // Counter NOT advanced on replay.
        assert_eq!(pm.next_paymaster_nonce(), 1);
    }

    #[test]
    fn idempotent_distinct_keys_get_distinct_nonces() {
        let tmp = TempDir::new().unwrap();
        let pm = idempotent_paymaster(&tmp);
        let mut a = blank_user_op();
        let mut b = blank_user_op();
        let mut c = blank_user_op();
        let n1 = pm.sponsor_idempotent(Some("k1"), &mut a).unwrap();
        let n2 = pm.sponsor_idempotent(Some("k2"), &mut b).unwrap();
        let n3 = pm.sponsor_idempotent(Some("k3"), &mut c).unwrap();
        assert_eq!(n1.paymaster_nonce(), 0);
        assert_eq!(n2.paymaster_nonce(), 1);
        assert_eq!(n3.paymaster_nonce(), 2);
    }

    #[test]
    fn idempotent_no_key_always_fresh() {
        // None for key bypasses the cache entirely.
        let tmp = TempDir::new().unwrap();
        let pm = idempotent_paymaster(&tmp);
        let mut a = blank_user_op();
        let mut b = blank_user_op();
        let n1 = pm.sponsor_idempotent(None, &mut a).unwrap();
        let n2 = pm.sponsor_idempotent(None, &mut b).unwrap();
        assert!(matches!(n1, SponsorOutcome::Fresh { paymaster_nonce: 0 }));
        assert!(matches!(n2, SponsorOutcome::Fresh { paymaster_nonce: 1 }));
    }

    #[test]
    fn idempotent_replay_increments_replay_metric_not_ok() {
        let tmp = TempDir::new().unwrap();
        let pm = idempotent_paymaster(&tmp);
        let mut a = blank_user_op();
        let mut b = blank_user_op();
        pm.sponsor_idempotent(Some("k"), &mut a).unwrap(); // Fresh → ok
        pm.sponsor_idempotent(Some("k"), &mut b).unwrap(); // Replay → replay metric
        let m = pm.metrics();
        assert_eq!(m.sponsorships_ok.load(Ordering::Relaxed), 1);
        assert_eq!(
            m.sponsorships_idempotent_replay.load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn idempotent_failure_does_not_cache() {
        // A sponsor failure (e.g. AlreadySigned) MUST NOT cache —
        // otherwise a retry would also fail with the cached error,
        // and any future request with that key would be poisoned.
        let tmp = TempDir::new().unwrap();
        let pm = idempotent_paymaster(&tmp);
        let mut a = blank_user_op();
        a.paymaster_signature = Some(vec![0u8; 32]); // forces AlreadySigned
        let r = pm.sponsor_idempotent(Some("k"), &mut a);
        assert!(matches!(r, Err(PaymasterError::AlreadySigned)));

        // Retry with a clean UserOp under the same key — must be
        // Fresh (the prior failure was not cached).
        let mut b = blank_user_op();
        let outcome = pm.sponsor_idempotent(Some("k"), &mut b).unwrap();
        assert!(matches!(outcome, SponsorOutcome::Fresh { .. }));
    }

    #[test]
    fn idempotent_disabled_when_max_keys_zero() {
        // Paymaster with idempotency_max_keys = 0: keys are silently
        // ignored; every call is Fresh.
        let tmp = TempDir::new().unwrap();
        // Construct a paymaster with idempotency_max_keys = 0 to test
        // the disabled path (permissive() now leaves the cache enabled).
        let kp = HybridKeypair::generate();
        let nonce_file = tmp.path().join("alt_paymaster_nonce");
        let pm = Paymaster::new_with_config(
            kp,
            "test",
            nonce_file,
            PaymasterConfig {
                require_user_sig: false,
                per_sender_rps: 0.0,
                per_sender_burst: 0,
                audit_log: None,
                allowed_inner_variants: None,
                idempotency_max_keys: 0,
                idempotency_ttl_secs: 0,
            },
        )
        .unwrap();
        let mut a = blank_user_op();
        let mut b = blank_user_op();
        let n1 = pm.sponsor_idempotent(Some("k"), &mut a).unwrap();
        let n2 = pm.sponsor_idempotent(Some("k"), &mut b).unwrap();
        // Both Fresh; second got a different nonce (no replay).
        assert!(matches!(n1, SponsorOutcome::Fresh { paymaster_nonce: 0 }));
        assert!(matches!(n2, SponsorOutcome::Fresh { paymaster_nonce: 1 }));
    }

    #[test]
    fn idempotent_lru_evicts_oldest_when_full() {
        // max_keys = 2; after inserting 3 distinct keys, the oldest
        // (k1) should be evicted. We then verify k1 misses while k2
        // and k3 still hit. We DON'T retry k1 first because that
        // miss would itself insert k1 into the cache and evict
        // another key, contaminating the LRU semantics under test.
        let kp = HybridKeypair::generate();
        let tmp = TempDir::new().unwrap();
        let nonce_file = tmp.path().join("paymaster_nonce");
        let pm = Paymaster::new_with_config(
            kp,
            "test",
            nonce_file,
            PaymasterConfig {
                require_user_sig: false,
                per_sender_rps: 0.0,
                per_sender_burst: 0,
                audit_log: None,
                allowed_inner_variants: None,
                idempotency_max_keys: 2,
                idempotency_ttl_secs: 60,
            },
        )
        .unwrap();

        for k in ["k1", "k2", "k3"] {
            pm.sponsor_idempotent(Some(k), &mut blank_user_op()).unwrap();
        }
        // Cache state after 3 inserts: [k2, k3]; k1 evicted as oldest.
        // Retry k2 + k3 first — both must Replay.
        let outcome = pm
            .sponsor_idempotent(Some("k2"), &mut blank_user_op())
            .unwrap();
        assert!(
            matches!(outcome, SponsorOutcome::Replay { .. }),
            "k2 should still be cached"
        );
        let outcome = pm
            .sponsor_idempotent(Some("k3"), &mut blank_user_op())
            .unwrap();
        assert!(
            matches!(outcome, SponsorOutcome::Replay { .. }),
            "k3 should still be cached"
        );
        // Now confirm k1 was evicted (would Fresh, but inserting k1
        // back would evict k2 or k3 — that's fine, we already
        // verified above).
        let outcome = pm
            .sponsor_idempotent(Some("k1"), &mut blank_user_op())
            .unwrap();
        assert!(
            matches!(outcome, SponsorOutcome::Fresh { .. }),
            "k1 should have been evicted"
        );
    }

    #[test]
    fn info_exposes_idempotency_config() {
        // /info surfaces the cache size + TTL so wallets know whether
        // sending Idempotency-Key will actually do anything.
        let tmp = TempDir::new().unwrap();
        let pm = idempotent_paymaster(&tmp);
        let info = pm.info();
        assert_eq!(info.idempotency_max_keys, 16);
        assert_eq!(info.idempotency_ttl_secs, 60);
    }

    // ─── Day 11: /info policy surface tests ──────────────────────────

    #[test]
    fn info_exposes_default_strict_config() {
        let tmp = TempDir::new().unwrap();
        let nonce_file = tmp.path().join("paymaster_nonce");
        let kp = HybridKeypair::generate();
        let pm = Paymaster::new(kp, "evaporchain-mainnet", nonce_file).unwrap();
        let info = pm.info();
        assert!(info.require_user_sig, "default is strict");
        assert_eq!(info.per_sender_rps, 5.0);
        assert_eq!(info.per_sender_burst, 10);
        assert!(!info.audit_log_enabled);
        assert!(info.allowed_inner_variants.is_none());
    }

    #[test]
    fn info_exposes_permissive_config() {
        let tmp = TempDir::new().unwrap();
        let pm = fresh_paymaster(&tmp, "evaporchain-mainnet");
        let info = pm.info();
        assert!(!info.require_user_sig);
        assert_eq!(info.per_sender_rps, 0.0);
        assert_eq!(info.per_sender_burst, 0);
    }

    #[test]
    fn info_exposes_audit_log_enabled_when_set() {
        let tmp = TempDir::new().unwrap();
        let nonce_file = tmp.path().join("paymaster_nonce");
        let audit_file = tmp.path().join("audit.jsonl");
        let kp = HybridKeypair::generate();
        let pm = Paymaster::new_with_config(
            kp,
            "test",
            nonce_file,
            PaymasterConfig {
                require_user_sig: false,
                per_sender_rps: 0.0,
                per_sender_burst: 0,
                audit_log: Some(audit_file),
                allowed_inner_variants: None,
                idempotency_max_keys: 0,
                idempotency_ttl_secs: 0,
            },
        )
        .unwrap();
        let info = pm.info();
        assert!(info.audit_log_enabled);
    }

    #[test]
    fn info_exposes_inner_variants_whitelist_in_cli_form() {
        let tmp = TempDir::new().unwrap();
        let pm = permissive_with_inner_whitelist(
            &tmp,
            "test",
            Some(vec![InnerVariant::Transfer, InnerVariant::CallScript]),
        );
        let info = pm.info();
        assert_eq!(
            info.allowed_inner_variants,
            Some(vec!["transfer".to_string(), "call_script".to_string()])
        );
    }

    #[test]
    fn info_round_trips_through_serde() {
        // Wire-format check — a wallet built against the post-Day-11
        // shape can decode an info response that's been marshalled
        // through serde_json (the actual transport).
        let tmp = TempDir::new().unwrap();
        let pm = permissive_with_inner_whitelist(
            &tmp,
            "evaporchain-mainnet",
            Some(vec![InnerVariant::Transfer]),
        );
        let info = pm.info();
        let json = serde_json::to_string(&info).unwrap();
        let decoded: PaymasterInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.paymaster_address_hex, info.paymaster_address_hex);
        assert_eq!(decoded.chain_id, info.chain_id);
        assert_eq!(decoded.allowed_inner_variants, info.allowed_inner_variants);
        assert_eq!(decoded.audit_log_enabled, info.audit_log_enabled);
    }

    #[test]
    fn info_decodes_old_paymaster_response_with_missing_fields() {
        // Backwards compat — a wallet built post-Day-11 hitting a
        // paymaster from the Day-1..10 era sees a response without
        // the policy fields. serde defaults must fill in
        // permissive-baseline values.
        let old_response = serde_json::json!({
            "paymaster_address_hex": "deadbeef",
            "next_paymaster_nonce": 7,
            "chain_id": "test"
        })
        .to_string();
        let info: PaymasterInfo = serde_json::from_str(&old_response).unwrap();
        assert_eq!(info.next_paymaster_nonce, 7);
        // All policy fields default to permissive-baseline.
        assert!(!info.require_user_sig);
        assert_eq!(info.per_sender_rps, 0.0);
        assert_eq!(info.per_sender_burst, 0);
        assert!(!info.audit_log_enabled);
        assert!(info.allowed_inner_variants.is_none());
    }

    #[test]
    fn inner_variant_parse_cli_round_trips() {
        for v in [
            InnerVariant::Transfer,
            InnerVariant::CallScript,
            InnerVariant::CallContract,
        ] {
            assert_eq!(InnerVariant::parse_cli(v.as_str()), Some(v));
        }
        assert_eq!(InnerVariant::parse_cli("Transfer"), None); // case-sensitive
        assert_eq!(InnerVariant::parse_cli("create_object"), None);
    }

    // ─── Day 9: metrics tests ─────────────────────────────────────────

    #[test]
    fn metrics_increment_on_successful_sponsor() {
        let tmp = TempDir::new().unwrap();
        let pm = fresh_paymaster(&tmp, "test");
        assert_eq!(pm.metrics().sponsorships_ok.load(Ordering::Relaxed), 0);
        for _ in 0..3 {
            pm.sponsor(&mut blank_user_op()).unwrap();
        }
        assert_eq!(pm.metrics().sponsorships_ok.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn metrics_increment_per_error_variant() {
        // Build a strict paymaster + rate-limited bucket so we can
        // hit invalid_user_sig + rate_limited counters distinctly.
        let tmp = TempDir::new().unwrap();
        let nonce_file = tmp.path().join("paymaster_nonce");
        let kp = HybridKeypair::generate();
        let pm = Paymaster::new_with_config(
            kp,
            "test",
            nonce_file,
            PaymasterConfig {
                require_user_sig: true,
                per_sender_rps: 0.000_001,
                per_sender_burst: 1,
                audit_log: None,
                allowed_inner_variants: None,
                idempotency_max_keys: 0,
                idempotency_ttl_secs: 0,
            },
        )
        .unwrap();

        // 1 invalid_user_sig (rate limit grants 1 then user-sig
        // check fails because no signature).
        let r = pm.sponsor(&mut blank_user_op());
        assert!(matches!(r, Err(PaymasterError::InvalidUserSignature)));
        // 1 rate_limited (bucket exhausted).
        let r = pm.sponsor(&mut blank_user_op());
        assert!(matches!(r, Err(PaymasterError::RateLimited { .. })));
        // 1 already_signed (different sender so rate limit fresh).
        let mut uo = blank_user_op();
        uo.sender = [0xCC; 32];
        uo.paymaster_signature = Some(vec![0u8; 32]);
        let r = pm.sponsor(&mut uo);
        assert!(matches!(r, Err(PaymasterError::AlreadySigned)));

        let m = pm.metrics();
        assert_eq!(m.sponsorships_invalid_user_sig.load(Ordering::Relaxed), 1);
        assert_eq!(m.sponsorships_rate_limited.load(Ordering::Relaxed), 1);
        assert_eq!(m.sponsorships_already_signed.load(Ordering::Relaxed), 1);
        assert_eq!(m.sponsorships_ok.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn prometheus_metrics_format_is_well_formed() {
        let tmp = TempDir::new().unwrap();
        let pm = fresh_paymaster(&tmp, "test");
        pm.sponsor(&mut blank_user_op()).unwrap();
        pm.sponsor(&mut blank_user_op()).unwrap();
        let body = pm.prometheus_metrics();

        // Each metric MUST have HELP + TYPE + at least one sample line.
        for (metric, ty) in [
            ("evaporchain_paymaster_sponsorships_total", "counter"),
            ("evaporchain_paymaster_next_nonce", "gauge"),
            ("evaporchain_paymaster_active_senders", "gauge"),
            ("evaporchain_paymaster_uptime_seconds", "gauge"),
            ("evaporchain_paymaster_idempotent_replays_total", "counter"),
        ] {
            assert!(
                body.contains(&format!("# HELP {metric}")),
                "missing HELP for {metric}"
            );
            assert!(
                body.contains(&format!("# TYPE {metric} {ty}")),
                "missing TYPE for {metric}"
            );
        }
        // ok counter reflects the two successful sponsorships.
        assert!(
            body.contains("evaporchain_paymaster_sponsorships_total{status=\"ok\"} 2"),
            "ok counter not at 2; body:\n{body}"
        );
        // next_nonce gauge advanced to 2.
        assert!(
            body.contains("evaporchain_paymaster_next_nonce 2"),
            "next_nonce gauge not at 2; body:\n{body}"
        );
        // All 7 sponsorship outcome labels MUST be emitted (even at 0)
        // so Prometheus rate() / increase() over status="..." selectors
        // never returns a NaN for the unseen variants.
        for status in [
            "ok",
            "already_signed",
            "invalid_user_sig",
            "rate_limited",
            "nonce_io",
            "audit_io",
            "other",
        ] {
            assert!(
                body.contains(&format!(
                    "evaporchain_paymaster_sponsorships_total{{status=\"{status}\"}}"
                )),
                "missing status='{status}' line in metrics body"
            );
        }
    }

    #[test]
    fn audit_log_disabled_writes_nothing() {
        // No audit_log set → no file is created and sponsorships proceed.
        let tmp = TempDir::new().unwrap();
        let nonce_file = tmp.path().join("paymaster_nonce");
        let audit_file = tmp.path().join("audit.jsonl");
        let kp = HybridKeypair::generate();
        let pm = Paymaster::new_with_config(
            kp,
            "test",
            nonce_file,
            PaymasterConfig {
                require_user_sig: false,
                per_sender_rps: 0.0,
                per_sender_burst: 0,
                audit_log: None,
                allowed_inner_variants: None,
                idempotency_max_keys: 0,
                idempotency_ttl_secs: 0,
            },
        )
        .unwrap();
        pm.sponsor(&mut blank_user_op()).unwrap();
        assert!(!audit_file.exists());
    }

    #[test]
    fn rate_limit_disabled_when_rps_or_burst_is_zero() {
        // per_sender_rps = 0.0 → limiter disabled regardless of burst.
        let tmp = TempDir::new().unwrap();
        let kp = HybridKeypair::generate();
        let nonce_file = tmp.path().join("paymaster_nonce");
        let pm = Paymaster::new_with_config(
            kp,
            "test",
            nonce_file,
            PaymasterConfig {
                require_user_sig: false,
                per_sender_rps: 0.0,
                per_sender_burst: 1,
                audit_log: None,
                allowed_inner_variants: None,
                idempotency_max_keys: 0,
                idempotency_ttl_secs: 0,
            },
        )
        .unwrap();
        for _ in 0..50 {
            let mut uo = blank_user_op();
            pm.sponsor(&mut uo).expect("rate limit disabled");
        }
        assert_eq!(pm.next_paymaster_nonce(), 50);
    }
}
