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

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use evaporchain_crypto::signatures::{HybridKeypair, Signer};
use evaporchain_types::{AccountAddress, UserOpTx};
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
}

struct NonceState {
    next: u64,
    file: PathBuf,
}

impl Paymaster {
    /// Build a paymaster from an in-memory keypair. The caller is
    /// responsible for keeping the keypair file-permissioned — a
    /// leaked keypair lets anyone drain the paymaster's balance up to
    /// its on-chain nonce-allocation horizon.
    pub fn new(
        keypair: HybridKeypair,
        chain_id: impl Into<String>,
        nonce_file: impl Into<PathBuf>,
    ) -> Result<Self, PaymasterError> {
        let pk = keypair.public_key_bytes();
        let address: AccountAddress = *blake3::hash(&pk).as_bytes();
        let nonce_file: PathBuf = nonce_file.into();
        let next = load_nonce(&nonce_file)?;
        Ok(Self {
            keypair,
            address,
            chain_id: chain_id.into(),
            nonce_state: Arc::new(Mutex::new(NonceState {
                next,
                file: nonce_file,
            })),
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
        let kp = HybridKeypair::generate();
        let nonce_file = tmp.path().join("paymaster_nonce");
        Paymaster::new(kp, chain_id, nonce_file).expect("paymaster")
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
            let pm = Paymaster::new(kp, "test", &nonce_file).unwrap();
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
        let pm2 = Paymaster::new(kp2, "test", &nonce_file).unwrap();
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
}
