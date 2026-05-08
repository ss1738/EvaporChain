//! Offline transaction signing and broadcast.
//!
//! Sign transactions without a network connection, export as JSON,
//! and broadcast later when online. Enables air-gapped signing workflows.
//!
//! # Flow
//!
//! ```text
//! 1. Build unsigned tx (needs: recipient, amount, nonce)
//! 2. Sign locally with ML-DSA keypair
//! 3. Export signed tx as JSON file
//! 4. Transfer file to online machine
//! 5. Broadcast via `wallet offline broadcast <file>`
//! ```

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

use crate::address::format_address;
use crate::pipeline::sign_and_encode_for_chain;
use crate::rpc::{RpcClient, RpcError, TransferRequest, TxResultResponse};
use crate::signer::WalletSigner;
use crate::tx_builder::TxBuilder;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum OfflineError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("RPC error: {0}")]
    Rpc(#[from] RpcError),
    #[error("unsupported transaction type for offline broadcast: {0}")]
    UnsupportedType(String),
}

// ──────────────────────────── Signed Transaction ──────────────────────

/// A signed transaction ready for broadcast, serializable to JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedTransaction {
    /// Transaction type tag.
    pub tx_type: String,
    /// Sender address (hex).
    pub from: String,
    /// Recipient address (hex), if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Amount, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<u64>,
    /// Nonce used.
    pub nonce: u64,
    /// Hex-encoded ML-DSA signature.
    pub signature: String,
    /// Hex-encoded ML-DSA public key.
    pub public_key: String,
    /// Additional fields for non-transfer tx types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
    /// Timestamp when signed (ISO 8601).
    pub signed_at: String,
}

impl SignedTransaction {
    /// Save to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), OfflineError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, OfflineError> {
        let data = std::fs::read_to_string(path)?;
        let tx: SignedTransaction = serde_json::from_str(&data)?;
        Ok(tx)
    }
}

// ──────────────────────────── Offline Signer ──────────────────────────

/// Sign transactions offline without network access.
pub struct OfflineSigner;

impl OfflineSigner {
    /// Sign a transfer transaction offline. `chain_id` MUST match
    /// the target chain's `chain_id` — sigs are chain-id-bound and
    /// the chain rejects mismatched ones under `verify_signatures:
    /// true`. Pass `""` only when targeting a chain that runs with
    /// `verify_signatures: false` (legacy testnet).
    pub fn sign_transfer(
        signer: &WalletSigner,
        to: &evaporchain_types::AccountAddress,
        amount: u64,
        nonce: u64,
        chain_id: &str,
    ) -> SignedTransaction {
        let builder = TxBuilder::new(*signer.address());
        let tx = builder.transfer(*to, amount, nonce);
        let (sig, pk) = sign_and_encode_for_chain(signer, &tx, chain_id);

        SignedTransaction {
            tx_type: "Transfer".to_string(),
            from: format_address(signer.address()),
            to: Some(format_address(to)),
            amount: Some(amount),
            nonce,
            signature: sig,
            public_key: pk,
            extra: None,
            signed_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Sign a refresh transaction offline. See `sign_transfer` for
    /// the chain_id contract.
    pub fn sign_refresh(
        signer: &WalletSigner,
        object_id: &evaporchain_types::ObjectId,
        energy: u64,
        chain_id: &str,
    ) -> SignedTransaction {
        let builder = TxBuilder::new(*signer.address());
        let tx = builder.refresh(*object_id, energy);
        let (sig, pk) = sign_and_encode_for_chain(signer, &tx, chain_id);

        SignedTransaction {
            tx_type: "Refresh".to_string(),
            from: format_address(signer.address()),
            to: None,
            amount: None,
            nonce: 0,
            signature: sig,
            public_key: pk,
            extra: Some(serde_json::json!({
                "object_id": format!("0x{}", hex::encode(object_id)),
                "energy": energy,
            })),
            signed_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Sign a create-object transaction offline. See `sign_transfer`
    /// for the chain_id contract.
    pub fn sign_create_object(
        signer: &WalletSigner,
        object_id: &evaporchain_types::ObjectId,
        energy: u64,
        half_life: u64,
        data: Vec<u8>,
        chain_id: &str,
    ) -> SignedTransaction {
        let builder = TxBuilder::new(*signer.address());
        let tx = builder.create_object(*object_id, energy, half_life, data);
        let (sig, pk) = sign_and_encode_for_chain(signer, &tx, chain_id);

        SignedTransaction {
            tx_type: "CreateObject".to_string(),
            from: format_address(signer.address()),
            to: None,
            amount: None,
            nonce: 0,
            signature: sig,
            public_key: pk,
            extra: Some(serde_json::json!({
                "object_id": format!("0x{}", hex::encode(object_id)),
                "energy": energy,
                "half_life": half_life,
            })),
            signed_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

// ──────────────────────────── Broadcaster ─────────────────────────────

/// Broadcast pre-signed transactions to the node.
pub struct Broadcaster;

impl Broadcaster {
    /// Broadcast a signed transaction to the node.
    pub async fn broadcast(
        rpc: &RpcClient,
        signed: &SignedTransaction,
    ) -> Result<TxResultResponse, OfflineError> {
        match signed.tx_type.as_str() {
            "Transfer" => {
                let to = signed.to.as_deref().ok_or_else(|| {
                    OfflineError::UnsupportedType("Transfer missing 'to' field".into())
                })?;
                let amount = signed.amount.ok_or_else(|| {
                    OfflineError::UnsupportedType("Transfer missing 'amount' field".into())
                })?;

                let req = TransferRequest {
                    from: signed.from.clone(),
                    to: to.to_string(),
                    amount,
                    nonce: signed.nonce,
                    signature: Some(signed.signature.clone()),
                    public_key: Some(signed.public_key.clone()),
                };
                Ok(rpc.submit_transfer(&req).await?)
            }
            other => Err(OfflineError::UnsupportedType(format!(
                "Broadcast for '{}' not yet implemented — use online mode",
                other
            ))),
        }
    }
}

// ──────────────────────────── Tests ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signer::WalletSigner;
    use evaporchain_crypto::signatures::MlDsaKeypair;

    fn make_signer() -> WalletSigner {
        WalletSigner::from_keypair(MlDsaKeypair::generate())
    }

    #[test]
    fn test_sign_transfer_offline() {
        let signer = make_signer();
        let to = [1u8; 32];
        let signed = OfflineSigner::sign_transfer(&signer, &to, 1000, 5, "");

        assert_eq!(signed.tx_type, "Transfer");
        assert_eq!(signed.amount, Some(1000));
        assert_eq!(signed.nonce, 5);
        assert!(!signed.signature.is_empty());
        assert!(!signed.public_key.is_empty());
    }

    #[test]
    fn test_sign_refresh_offline() {
        let signer = make_signer();
        let obj_id = [2u8; 32];
        let signed = OfflineSigner::sign_refresh(&signer, &obj_id, 500, "");

        assert_eq!(signed.tx_type, "Refresh");
        assert!(signed.extra.is_some());
    }

    #[test]
    fn test_sign_create_object_offline() {
        let signer = make_signer();
        let obj_id = [3u8; 32];
        let signed = OfflineSigner::sign_create_object(&signer, &obj_id, 1000, 100, vec![0u8; 10], "");

        assert_eq!(signed.tx_type, "CreateObject");
        assert!(signed.extra.is_some());
    }

    #[test]
    fn test_json_roundtrip() {
        let signer = make_signer();
        let to = [1u8; 32];
        let signed = OfflineSigner::sign_transfer(&signer, &to, 1000, 5, "");

        let json = serde_json::to_string_pretty(&signed).unwrap();
        let loaded: SignedTransaction = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.tx_type, "Transfer");
        assert_eq!(loaded.signature, signed.signature);
        assert_eq!(loaded.nonce, 5);
    }

    #[test]
    fn test_file_save_and_load() {
        let signer = make_signer();
        let to = [1u8; 32];
        let signed = OfflineSigner::sign_transfer(&signer, &to, 1000, 5, "");

        let dir = std::env::temp_dir().join("evaporchain_offline_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("signed_tx.json");

        signed.save(&path).unwrap();
        let loaded = SignedTransaction::load(&path).unwrap();

        assert_eq!(loaded.signature, signed.signature);
        assert_eq!(loaded.amount, Some(1000));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_different_signers_different_signatures() {
        let s1 = make_signer();
        let s2 = make_signer();
        let to = [1u8; 32];

        let sig1 = OfflineSigner::sign_transfer(&s1, &to, 1000, 0, "");
        let sig2 = OfflineSigner::sign_transfer(&s2, &to, 1000, 0, "");

        assert_ne!(sig1.signature, sig2.signature);
        assert_ne!(sig1.public_key, sig2.public_key);
    }
}
