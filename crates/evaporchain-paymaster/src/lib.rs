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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

/// JSON response body for `/info`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymasterInfo {
    pub paymaster_address_hex: String,
    pub next_paymaster_nonce: u64,
    pub chain_id: String,
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
}

impl Default for PaymasterConfig {
    fn default() -> Self {
        Self {
            require_user_sig: true,
            per_sender_rps: 5.0,
            per_sender_burst: 10,
        }
    }
}

impl PaymasterConfig {
    /// Permissive profile for testnet / dev — disables both the
    /// user-sig pre-check and the rate limiter. Do NOT use in prod.
    pub fn permissive() -> Self {
        Self {
            require_user_sig: false,
            per_sender_rps: 0.0,
            per_sender_burst: 0,
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

        Ok(assigned)
    }

    /// Build a `PaymasterInfo` for the `/info` endpoint.
    pub fn info(&self) -> PaymasterInfo {
        PaymasterInfo {
            paymaster_address_hex: hex::encode(self.address),
            next_paymaster_nonce: self.next_paymaster_nonce(),
            chain_id: self.chain_id.clone(),
        }
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
