//! Transaction signing with ML-DSA and Hybrid (ECDSA + ML-DSA) keys.
//!
//! The `WalletSigner` wraps a signing keypair (either ML-DSA-only for legacy
//! or Hybrid ECDSA+ML-DSA for post-quantum security) and provides methods to
//! sign any EvaporChain transaction type.

use evaporchain_crypto::signatures::{HybridKeypair, MlDsaKeypair, Signer};
use evaporchain_types::{AccountAddress, Transaction};
use thiserror::Error;

use crate::address::{derive_address, parse_address as parse_address_hex_32};
use crate::keystore::{KeyStore, KeyStoreError};

#[derive(Debug, Error)]
pub enum SignerError {
    #[error("keystore error: {0}")]
    KeyStore(#[from] KeyStoreError),
    #[error("address mismatch: signer address {signer} does not match tx sender {sender}")]
    AddressMismatch { signer: String, sender: String },
}

enum SignerInner {
    MlDsa(MlDsaKeypair),
    Hybrid(HybridKeypair),
}

impl SignerInner {
    fn sign(&self, msg: &[u8]) -> Vec<u8> {
        match self {
            SignerInner::MlDsa(kp) => kp.sign(msg),
            SignerInner::Hybrid(kp) => kp.sign(msg),
        }
    }

    fn public_key_bytes(&self) -> Vec<u8> {
        match self {
            SignerInner::MlDsa(kp) => kp.public_key_bytes(),
            SignerInner::Hybrid(kp) => kp.public_key_bytes(),
        }
    }
}

/// A wallet signer backed by an unlocked keypair (ML-DSA or Hybrid).
pub struct WalletSigner {
    inner: SignerInner,
    address: AccountAddress,
}

impl WalletSigner {
    /// Create a signer from a raw ML-DSA keypair. Address is derived
    /// as `blake3(public_key)` — see [`Self::from_keypair_with_address`]
    /// for cases where the keystore entry holds an override address
    /// (genesis-allocated accounts whose address ≠ hash(pubkey)).
    pub fn from_keypair(keypair: MlDsaKeypair) -> Self {
        let address = derive_address(&keypair.public_key_bytes());
        Self {
            inner: SignerInner::MlDsa(keypair),
            address,
        }
    }

    /// Create a signer with a caller-supplied address that decouples
    /// from `hash(public_key)`. Used when unlocking a keystore entry
    /// imported via `--address-override` (e.g. validator-N genesis
    /// allocations at `[N, 0, 0, ...]`).
    pub fn from_keypair_with_address(keypair: MlDsaKeypair, address: AccountAddress) -> Self {
        Self {
            inner: SignerInner::MlDsa(keypair),
            address,
        }
    }

    /// Create a signer from a Hybrid ECDSA+ML-DSA keypair.
    pub fn from_hybrid(keypair: HybridKeypair) -> Self {
        let address = derive_address(&keypair.public_key_bytes());
        Self {
            inner: SignerInner::Hybrid(keypair),
            address,
        }
    }

    /// Unlock a key from the keystore by name and create a signer.
    /// The signer's address is read from the keystore entry's
    /// `address` field — honoring any `--address-override` set at
    /// import time. Falls back to `blake3(public_key)` derivation if
    /// the entry's address can't be parsed (defensive default; old
    /// keystores all hold derived addresses).
    pub fn unlock(store: &KeyStore, name: &str, password: &str) -> Result<Self, SignerError> {
        let keypair = store.unlock_key(name, password)?;
        let stored_address = store
            .entries
            .iter()
            .find(|e| e.name == name)
            .and_then(|e| parse_address_hex_32(&e.address).ok());
        Ok(match stored_address {
            Some(addr) => Self::from_keypair_with_address(keypair, addr),
            None => Self::from_keypair(keypair),
        })
    }

    /// Unlock a key from the keystore by address and create a signer.
    /// Signer's address is the caller-supplied lookup address (which
    /// by construction matches the keystore entry).
    pub fn unlock_by_address(
        store: &KeyStore,
        address: &AccountAddress,
        password: &str,
    ) -> Result<Self, SignerError> {
        let keypair = store.unlock_by_address(address, password)?;
        Ok(Self::from_keypair_with_address(keypair, *address))
    }

    /// Get the signer's address.
    pub fn address(&self) -> &AccountAddress {
        &self.address
    }

    /// Get the signer's public key bytes.
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.inner.public_key_bytes()
    }

    /// Sign raw bytes and return the signature.
    pub fn sign_bytes(&self, msg: &[u8]) -> Vec<u8> {
        self.inner.sign(msg)
    }

    /// Sign a transaction in-place WITHOUT chain-id binding.
    ///
    /// **DEPRECATED — produces signatures the chain rejects under
    /// `verify_signatures: true`.** Use `sign_transaction_for_chain`
    /// instead. This signs over `tx.signable_bytes()`, but the chain's
    /// `verify_tx_signature` (`evaporchain-execution::SimpleExecutor`
    /// at lib.rs:1146) verifies against
    /// `tx.signing_message(chain_id)` which prefixes the body with
    /// `LE32(chain_id.len()) || chain_id_bytes` for cross-chain
    /// replay protection (EIP-155-style).
    ///
    /// Kept for backwards-compat with tests + tools that don't have
    /// a chain_id handy yet (e.g., offline-signing flows where
    /// chain_id is supplied later). Prefer the `_for_chain` variant
    /// for any path that actually submits to a verifying chain.
    #[deprecated(
        note = "produces signatures the chain rejects under verify_signatures=true; \
                use sign_transaction_for_chain (chain-id-bound)"
    )]
    pub fn sign_transaction(&self, tx: &mut Transaction) {
        let msg = tx.signable_bytes();
        let sig = self.inner.sign(&msg);
        let pk = self.inner.public_key_bytes();
        set_signature(tx, sig, pk);
    }

    /// Sign a transaction and return a new signed copy WITHOUT
    /// chain-id binding. **DEPRECATED — see `sign_transaction`.**
    #[deprecated(
        note = "produces signatures the chain rejects under verify_signatures=true; \
                use sign_for_chain (chain-id-bound)"
    )]
    pub fn sign(&self, tx: &Transaction) -> Transaction {
        let mut signed = tx.clone();
        // Allow self-call to deprecated method; the deprecation is
        // about the public API surface, not internal use.
        #[allow(deprecated)]
        self.sign_transaction(&mut signed);
        signed
    }

    /// Sign a transaction in-place, binding the signature to
    /// `chain_id`. The result satisfies the chain's
    /// `verify_tx_signature` under `verify_signatures: true`.
    ///
    /// `chain_id` should match the target chain's `chain_id`
    /// (queryable via `GET /api/status` → `chain_id`). Wallets
    /// signing offline can stamp the chain_id at compose-time.
    pub fn sign_transaction_for_chain(&self, tx: &mut Transaction, chain_id: &str) {
        let msg = tx.signing_message(chain_id);
        let sig = self.inner.sign(&msg);
        let pk = self.inner.public_key_bytes();
        set_signature(tx, sig, pk);
    }

    /// Sign a transaction and return a signed copy, binding the
    /// signature to `chain_id`. See `sign_transaction_for_chain`.
    pub fn sign_for_chain(&self, tx: &Transaction, chain_id: &str) -> Transaction {
        let mut signed = tx.clone();
        self.sign_transaction_for_chain(&mut signed, chain_id);
        signed
    }
}

/// Set signature and public key on a transaction.
fn set_signature(tx: &mut Transaction, sig: Vec<u8>, pk: Vec<u8>) {
    match tx {
        Transaction::Transfer(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::Refresh(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::CreateObject(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::DeployContract(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::CallContract(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::DeployScript(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::CallScript(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::ValidatorStake(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::ValidatorExit(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::ValidatorClaimStake(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::Shield(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        // Unshield and PrivateTransfer are ZK-authenticated — no signature needed.
        Transaction::Unshield(_) | Transaction::PrivateTransfer(_) => {}
        Transaction::Deferred(d) => {
            d.signature = Some(sig);
            d.public_key = Some(pk);
        }
        Transaction::Blob(b) => {
            b.signature = Some(sig);
            b.public_key = Some(pk);
        }
        Transaction::Governance(g) => {
            g.signature = Some(sig);
            g.public_key = Some(pk);
        }
        // MultiSig uses threshold signatures, not single-signer.
        Transaction::MultiSig(_) => {}
        Transaction::UserOp(u) => {
            u.signature = Some(sig);
            u.public_key = Some(pk);
        }
        Transaction::UpgradeContract(u) => {
            u.signature = Some(sig);
            u.public_key = Some(pk);
        }
        Transaction::Delegate(d) => {
            d.signature = Some(sig);
            d.public_key = Some(pk);
        }
        Transaction::Undelegate(u) => {
            u.signature = Some(sig);
            u.public_key = Some(pk);
        }
        Transaction::RotateValidatorKey(r) => {
            r.signature = Some(sig);
            r.public_key = Some(pk);
        }
        Transaction::ClaimDelegation(c) => {
            c.signature = Some(sig);
            c.public_key = Some(pk);
        }
        // Refund is protocol-issued — no caller signature to set.
        Transaction::Refund(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_crypto::signatures::{HybridVerifier, Verifier};
    use evaporchain_types::TransferTx;

    fn make_signer() -> WalletSigner {
        let kp = MlDsaKeypair::generate();
        WalletSigner::from_keypair(kp)
    }

    fn make_hybrid_signer() -> WalletSigner {
        let kp = HybridKeypair::generate();
        WalletSigner::from_hybrid(kp)
    }

    #[test]
    fn test_sign_bytes() {
        let signer = make_signer();
        let msg = b"hello evaporchain";
        let sig = signer.sign_bytes(msg);
        assert!(!sig.is_empty());

        let pk = signer.public_key_bytes();
        assert!(HybridVerifier::verify(msg, &sig, &pk));
    }

    #[test]
    fn test_sign_transfer_transaction() {
        let signer = make_signer();
        let mut tx = Transaction::Transfer(TransferTx {
            from: *signer.address(),
            to: [2u8; 32],
            amount: 1000,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });

        signer.sign_transaction_for_chain(&mut tx, "");

        assert!(tx.signature().is_some());
        assert!(tx.public_key().is_some());

        let msg = tx.signing_message("");
        let sig = tx.signature().unwrap();
        let pk = tx.public_key().unwrap();
        assert!(HybridVerifier::verify(&msg, sig, pk));
    }

    #[test]
    fn test_sign_returns_new_copy() {
        let signer = make_signer();
        let tx = Transaction::Transfer(TransferTx {
            from: *signer.address(),
            to: [2u8; 32],
            amount: 500,
            nonce: 1,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });

        let signed = signer.sign_for_chain(&tx, "");
        assert!(tx.signature().is_none());
        assert!(signed.signature().is_some());
    }

    #[test]
    fn test_signer_from_keystore() {
        let mut store = KeyStore::new();
        store.generate_key("signer_test", "pass").unwrap();

        let signer = WalletSigner::unlock(&store, "signer_test", "pass").unwrap();
        assert_ne!(*signer.address(), [0u8; 32]);

        let sig = signer.sign_bytes(b"test");
        assert!(HybridVerifier::verify(
            b"test",
            &sig,
            &signer.public_key_bytes()
        ));
    }

    #[test]
    fn test_address_derivation_matches() {
        let signer = make_signer();
        let expected = derive_address(&signer.public_key_bytes());
        assert_eq!(*signer.address(), expected);
    }

    /// Regression test (commit ed01c86): WalletSigner::unlock must honor
    /// the keystore entry's `address` field, not re-derive from the
    /// pubkey. This is the path that genesis-allocated accounts (e.g.
    /// validator-N operator at [N, 0, 0, ...]) take when imported via
    /// `account import --address-override`.
    #[test]
    fn test_unlock_honors_stored_address_override() {
        // Override address that does NOT match hash(pubkey).
        let override_addr: AccountAddress = [0x01; 32];

        let kp = MlDsaKeypair::generate();
        let pk_bytes = kp.public_key_bytes();
        let sk_bytes: Vec<u8> = kp.secret_key().to_vec();
        let derived = derive_address(&pk_bytes);
        assert_ne!(derived, override_addr,
            "test setup invariant: derived must differ from override");

        let mut store = KeyStore::new();
        let returned_addr = store.import_key_with_address(
            "validator-1",
            "pass",
            &pk_bytes,
            &sk_bytes,
            override_addr,
        ).expect("import_key_with_address");
        assert_eq!(returned_addr, override_addr);

        // Round-trip the keystore through serde so we exercise the
        // load path too (matches what `wallet send` does on a fresh
        // process).
        let json = serde_json::to_string(&store).unwrap();
        let reloaded: KeyStore = serde_json::from_str(&json).unwrap();

        // CRITICAL: unlock must produce a signer whose address equals
        // the OVERRIDE address, not the derived address. Pre-fix this
        // assertion failed (signer used derive_address(pubkey)).
        let signer = WalletSigner::unlock(&reloaded, "validator-1", "pass").unwrap();
        assert_eq!(*signer.address(), override_addr,
            "signer must use the keystore entry's address, not re-derive from pubkey");

        // Also verify a signature signed by this signer carries the
        // SAME pubkey as the original keypair (so the chain can
        // verify it).
        let pk_emitted = signer.public_key_bytes();
        assert_eq!(pk_emitted, pk_bytes);
    }

    /// Default path: unlock without override yields a signer whose
    /// address is derive_address(pubkey). Confirms backwards-compat —
    /// fresh keys generated via `account create` continue to work.
    #[test]
    fn test_unlock_default_path_uses_derived_address() {
        let mut store = KeyStore::new();
        let addr = store.generate_key("alice", "pass").unwrap();
        let signer = WalletSigner::unlock(&store, "alice", "pass").unwrap();
        assert_eq!(*signer.address(), addr,
            "default unlock must derive address from pubkey");
    }

    // ─── Hybrid Signer Tests ──────────────────────────────────────────

    #[test]
    fn test_hybrid_sign_bytes() {
        let signer = make_hybrid_signer();
        let msg = b"hybrid signing";
        let sig = signer.sign_bytes(msg);
        let pk = signer.public_key_bytes();
        assert!(HybridVerifier::verify(msg, &sig, &pk));
    }

    #[test]
    fn test_hybrid_sign_transfer_transaction() {
        let signer = make_hybrid_signer();
        let mut tx = Transaction::Transfer(TransferTx {
            from: *signer.address(),
            to: [2u8; 32],
            amount: 1000,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });

        signer.sign_transaction_for_chain(&mut tx, "");

        let msg = tx.signing_message("");
        let sig = tx.signature().unwrap();
        let pk = tx.public_key().unwrap();
        assert!(HybridVerifier::verify(&msg, sig, pk));
        assert!(HybridVerifier::is_hybrid_sig(sig));
        assert!(HybridVerifier::is_hybrid_pk(pk));
    }

    #[test]
    fn test_hybrid_address_differs_from_mldsa() {
        let mldsa_signer = make_signer();
        let hybrid_signer = make_hybrid_signer();
        assert_ne!(mldsa_signer.address(), hybrid_signer.address());
    }
}
