//! Transaction pipeline: build, sign, submit, and track transactions.
//!
//! Provides high-level async methods that handle the full lifecycle:
//! nonce management, transaction construction, ML-DSA signing,
//! hex-encoding for the API, and submission to the node.

use evaporchain_types::{AccountAddress, Energy, Epoch, HalfLife, ObjectId, Transaction};
use thiserror::Error;

use crate::address::format_address;
use crate::rpc::{
    self, CallContractRequest, CreateObjectRequest, DeployContractRequest, DeployTokenRequest,
    MintNftRequest, RefreshNftRequest, RefreshRequest, RpcClient, RpcError, TokenTransferRequest,
    TransferNftRequest, TransferRequest, TxResultResponse,
};
use crate::signer::WalletSigner;
use crate::tx_builder::TxBuilder;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("rpc error: {0}")]
    Rpc(#[from] RpcError),
    #[error("transaction rejected: {0}")]
    Rejected(String),
}

// ──────────────────────────── Pending Tx ───────────────────────────────

/// Status of a submitted transaction.
#[derive(Debug, Clone)]
pub enum TxStatus {
    Pending,
    Confirmed(String),
    Failed(String),
}

/// Tracks a submitted transaction.
#[derive(Debug, Clone)]
pub struct PendingTx {
    pub tx_hash: String,
    pub tx_type: String,
    pub status: TxStatus,
}

// ──────────────────────────── Pipeline ─────────────────────────────────

/// Transaction pipeline that handles build → sign → submit.
pub struct TxPipeline {
    rpc: RpcClient,
    history: Vec<PendingTx>,
    /// Cached chain_id from `/api/status` — populated lazily on
    /// first call to `chain_id_cached()`. Pipeline-scoped: a wallet
    /// switching chains should construct a fresh pipeline.
    chain_id: tokio::sync::OnceCell<String>,
}

impl TxPipeline {
    /// Create a new pipeline with the given RPC client.
    pub fn new(rpc: RpcClient) -> Self {
        Self {
            rpc,
            history: Vec::new(),
            chain_id: tokio::sync::OnceCell::new(),
        }
    }

    /// Resolve the chain's `chain_id`, caching after first fetch.
    /// Required for `verify_signatures: true` chains — the wallet
    /// signs over `tx.signing_message(chain_id)` which the chain
    /// then verifies under the same binding.
    ///
    /// Source: `GET /api/chain` returns `ChainInfoResponse.chain_id`.
    /// (`/api/status`'s `StatusResponse` exposes `chain_name`, which
    /// is human-readable and NOT the chain_id used in signature
    /// binding — picking the wrong one was the original audit
    /// gotcha.)
    async fn chain_id_cached(&self) -> Result<&str, PipelineError> {
        self.chain_id
            .get_or_try_init(|| async {
                let info = self.rpc.get_chain_info().await?;
                Ok::<String, PipelineError>(info.chain_id)
            })
            .await
            .map(|s| s.as_str())
    }

    /// Get a reference to the RPC client.
    pub fn rpc(&self) -> &RpcClient {
        &self.rpc
    }

    /// Get submitted transaction history.
    pub fn history(&self) -> &[PendingTx] {
        &self.history
    }

    /// Clear transaction history.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    // ── Core pipeline ──────────────────────────────────────────────────

    /// Sign a transaction and submit it to the node.
    /// Returns the API response and records the submission in history.
    async fn sign_and_submit_transfer(
        &mut self,
        signer: &WalletSigner,
        to: &AccountAddress,
        amount: u64,
        nonce: u64,
    ) -> Result<TxResultResponse, PipelineError> {
        let builder = TxBuilder::new(*signer.address());
        let tx = builder.transfer(*to, amount, nonce);
        let chain_id = self.chain_id_cached().await?;
        let signed = signer.sign_for_chain(&tx, chain_id);

        let sig = hex::encode(signed.signature().unwrap());
        let pk = hex::encode(signed.public_key().unwrap());

        let req = TransferRequest {
            from: format_address(signer.address()),
            to: format_address(to),
            amount,
            nonce,
            signature: Some(sig),
            public_key: Some(pk),
        };

        let resp = self.rpc.submit_transfer(&req).await?;
        self.record("transfer", &resp);
        if !resp.success {
            return Err(PipelineError::Rejected(resp.message.clone()));
        }
        Ok(resp)
    }

    /// Submit a transfer transaction.
    pub async fn transfer(
        &mut self,
        signer: &WalletSigner,
        to: &AccountAddress,
        amount: u64,
        nonce: u64,
    ) -> Result<TxResultResponse, PipelineError> {
        self.sign_and_submit_transfer(signer, to, amount, nonce)
            .await
    }

    /// Submit a create-object transaction.
    pub async fn create_object(
        &mut self,
        signer: &WalletSigner,
        object_id: &ObjectId,
        energy: Energy,
        half_life: HalfLife,
        data: Vec<u8>,
    ) -> Result<TxResultResponse, PipelineError> {
        let builder = TxBuilder::new(*signer.address());
        let tx = builder.create_object(*object_id, energy, half_life, data);
        let chain_id = self.chain_id_cached().await?;
        let signed = signer.sign_for_chain(&tx, chain_id);

        let sig = hex::encode(signed.signature().unwrap());
        let pk = hex::encode(signed.public_key().unwrap());

        let req = CreateObjectRequest {
            creator: format_address(signer.address()),
            object_id: format!("0x{}", hex::encode(object_id)),
            energy,
            half_life,
            signature: Some(sig),
            public_key: Some(pk),
        };

        let resp = self.rpc.submit_create_object(&req).await?;
        self.record("create_object", &resp);
        if !resp.success {
            return Err(PipelineError::Rejected(resp.message.clone()));
        }
        Ok(resp)
    }

    /// Submit a refresh transaction.
    pub async fn refresh_object(
        &mut self,
        signer: &WalletSigner,
        object_id: &ObjectId,
        energy_deposit: Energy,
    ) -> Result<TxResultResponse, PipelineError> {
        let builder = TxBuilder::new(*signer.address());
        let tx = builder.refresh(*object_id, energy_deposit);
        let chain_id = self.chain_id_cached().await?;
        let signed = signer.sign_for_chain(&tx, chain_id);

        let sig = hex::encode(signed.signature().unwrap());
        let pk = hex::encode(signed.public_key().unwrap());

        let req = RefreshRequest {
            object_id: format!("0x{}", hex::encode(object_id)),
            energy_deposit,
            signature: Some(sig),
            public_key: Some(pk),
        };

        let resp = self.rpc.submit_refresh(&req).await?;
        self.record("refresh", &resp);
        if !resp.success {
            return Err(PipelineError::Rejected(resp.message.clone()));
        }
        Ok(resp)
    }

    /// Submit a resurrect transaction (same as refresh but to /resurrect endpoint).
    pub async fn resurrect_object(
        &mut self,
        signer: &WalletSigner,
        object_id: &ObjectId,
        energy_deposit: Energy,
    ) -> Result<TxResultResponse, PipelineError> {
        let builder = TxBuilder::new(*signer.address());
        let tx = builder.refresh(*object_id, energy_deposit);
        let chain_id = self.chain_id_cached().await?;
        let signed = signer.sign_for_chain(&tx, chain_id);

        let sig = hex::encode(signed.signature().unwrap());
        let pk = hex::encode(signed.public_key().unwrap());

        let req = RefreshRequest {
            object_id: format!("0x{}", hex::encode(object_id)),
            energy_deposit,
            signature: Some(sig),
            public_key: Some(pk),
        };

        let resp = self.rpc.submit_resurrect(&req).await?;
        self.record("resurrect", &resp);
        if !resp.success {
            return Err(PipelineError::Rejected(resp.message.clone()));
        }
        Ok(resp)
    }

    /// Submit a deploy-contract transaction.
    pub async fn deploy_contract(
        &mut self,
        signer: &WalletSigner,
        template: &str,
        init_args: serde_json::Value,
        energy: Energy,
        half_life: HalfLife,
    ) -> Result<TxResultResponse, PipelineError> {
        let req = DeployContractRequest {
            deployer: format_address(signer.address()),
            template: template.to_string(),
            init_args,
            energy,
            half_life,
            rules: None,
        };

        let resp = self.rpc.submit_deploy_contract(&req).await?;
        self.record("deploy_contract", &resp);
        if !resp.success {
            return Err(PipelineError::Rejected(resp.message.clone()));
        }
        Ok(resp)
    }

    /// Submit a call-contract transaction.
    pub async fn call_contract(
        &mut self,
        signer: &WalletSigner,
        contract_id: u64,
        method: &str,
        args: serde_json::Value,
        epoch: Epoch,
    ) -> Result<TxResultResponse, PipelineError> {
        let req = CallContractRequest {
            caller: format_address(signer.address()),
            contract_id,
            method: method.to_string(),
            args,
            epoch,
        };

        let resp = self.rpc.submit_call_contract(&req).await?;
        self.record("call_contract", &resp);
        if !resp.success {
            return Err(PipelineError::Rejected(resp.message.clone()));
        }
        Ok(resp)
    }

    // ── NFT operations (auth-token based) ──────────────────────────────

    /// Mint a new NFT.
    pub async fn mint_nft(
        &mut self,
        name: &str,
        collection: Option<&str>,
        metadata: &str,
        energy: Energy,
        half_life: HalfLife,
        owner: Option<&str>,
    ) -> Result<TxResultResponse, PipelineError> {
        let req = MintNftRequest {
            name: name.to_string(),
            collection: collection.map(|s| s.to_string()),
            metadata: metadata.to_string(),
            energy,
            half_life,
            owner: owner.map(|s| s.to_string()),
        };

        let resp = self.rpc.mint_nft(&req).await?;
        self.record("mint_nft", &resp);
        if !resp.success {
            return Err(PipelineError::Rejected(resp.message.clone()));
        }
        Ok(resp)
    }

    /// Transfer an NFT.
    pub async fn transfer_nft(
        &mut self,
        nft_id: u64,
        to: &str,
    ) -> Result<TxResultResponse, PipelineError> {
        let req = TransferNftRequest {
            nft_id,
            to: to.to_string(),
        };

        let resp = self.rpc.transfer_nft(&req).await?;
        self.record("transfer_nft", &resp);
        if !resp.success {
            return Err(PipelineError::Rejected(resp.message.clone()));
        }
        Ok(resp)
    }

    /// Refresh an NFT's energy.
    pub async fn refresh_nft(
        &mut self,
        nft_id: u64,
        energy: Energy,
    ) -> Result<TxResultResponse, PipelineError> {
        let req = RefreshNftRequest { nft_id, energy };

        let resp = self.rpc.refresh_nft(&req).await?;
        self.record("refresh_nft", &resp);
        if !resp.success {
            return Err(PipelineError::Rejected(resp.message.clone()));
        }
        Ok(resp)
    }

    // ── Token operations (auth-token based) ────────────────────────────

    /// Deploy a new token.
    pub async fn deploy_token(
        &mut self,
        name: &str,
        symbol: &str,
        total_supply: u64,
        decay_half_life: u64,
        deployer: Option<&str>,
    ) -> Result<TxResultResponse, PipelineError> {
        let req = DeployTokenRequest {
            name: name.to_string(),
            symbol: symbol.to_string(),
            total_supply,
            decay_half_life,
            deployer: deployer.map(|s| s.to_string()),
        };

        let resp = self.rpc.deploy_token(&req).await?;
        self.record("deploy_token", &resp);
        if !resp.success {
            return Err(PipelineError::Rejected(resp.message.clone()));
        }
        Ok(resp)
    }

    /// Transfer tokens.
    pub async fn transfer_token(
        &mut self,
        token_id: u64,
        from: &str,
        to: &str,
        amount: u64,
    ) -> Result<TxResultResponse, PipelineError> {
        let req = TokenTransferRequest {
            token_id,
            from: from.to_string(),
            to: to.to_string(),
            amount,
        };

        let resp = self.rpc.transfer_token(&req).await?;
        self.record("transfer_token", &resp);
        if !resp.success {
            return Err(PipelineError::Rejected(resp.message.clone()));
        }
        Ok(resp)
    }

    // ── Faucet ─────────────────────────────────────────────────────────

    /// Request testnet tokens from the faucet.
    pub async fn faucet(&mut self, address: &str) -> Result<rpc::FaucetResponse, PipelineError> {
        let resp = self.rpc.faucet(address).await?;
        Ok(resp)
    }

    // ── Internal ───────────────────────────────────────────────────────

    fn record(&mut self, tx_type: &str, resp: &TxResultResponse) {
        let status = if resp.success {
            TxStatus::Confirmed(resp.message.clone())
        } else {
            TxStatus::Failed(resp.message.clone())
        };
        self.history.push(PendingTx {
            tx_hash: resp.tx_hash.clone().unwrap_or_default(),
            tx_type: tx_type.to_string(),
            status,
        });
    }

    /// Poll the node for a transaction's lifecycle status.
    ///
    /// Returns the [`rpc::TxStatus`] once the tx has reached `included`,
    /// `finalised`, or `rejected` — i.e. it has landed in a block (or been
    /// rejected at execution). Returns `None` after `max_attempts` if the
    /// tx is still `pending` or sitting in `mempool`.
    ///
    /// TODO: callers that need the originating tx body (sender, recipient,
    /// amount, type) must call `/api/transactions` (the historical list)
    /// to find the matching hash — the new `/api/tx/:hash` endpoint
    /// returns lifecycle state only.
    pub async fn confirm_tx(
        &self,
        tx_hash: &str,
        max_attempts: u32,
        delay_ms: u64,
    ) -> Result<Option<rpc::TxStatus>, PipelineError> {
        for _ in 0..max_attempts {
            match self.rpc.get_tx(tx_hash).await {
                Ok(tx) => match tx.state {
                    rpc::TxState::Included | rpc::TxState::Finalised | rpc::TxState::Rejected => {
                        return Ok(Some(tx));
                    }
                    rpc::TxState::Pending | rpc::TxState::Mempool => {
                        // Not yet landed — wait and retry.
                    }
                },
                Err(_) => {
                    // Transient lookup error — wait and retry.
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        Ok(None)
    }
}

// ──────────────────────────── Helper ───────────────────────────────────

/// Sign a transaction and extract hex-encoded signature + public key.
///
/// **DEPRECATED — produces a signature without chain-id binding.**
/// Use `sign_and_encode_for_chain` instead. Kept for backwards-compat
/// with offline-signing tooling that doesn't have a chain_id handy
/// at signing time.
#[deprecated(
    note = "produces signatures the chain rejects under verify_signatures=true; \
            use sign_and_encode_for_chain"
)]
pub fn sign_and_encode(signer: &WalletSigner, tx: &Transaction) -> (String, String) {
    #[allow(deprecated)]
    let signed = signer.sign(tx);
    let sig = hex::encode(signed.signature().unwrap());
    let pk = hex::encode(signed.public_key().unwrap());
    (sig, pk)
}

/// Sign a transaction and extract hex-encoded signature + public key,
/// binding the signature to `chain_id`. The chain's
/// `verify_tx_signature` accepts the result under
/// `verify_signatures: true`.
pub fn sign_and_encode_for_chain(
    signer: &WalletSigner,
    tx: &Transaction,
    chain_id: &str,
) -> (String, String) {
    let signed = signer.sign_for_chain(tx, chain_id);
    let sig = hex::encode(signed.signature().unwrap());
    let pk = hex::encode(signed.public_key().unwrap());
    (sig, pk)
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signer::WalletSigner;
    use evaporchain_crypto::signatures::MlDsaKeypair;

    fn make_signer() -> WalletSigner {
        WalletSigner::from_keypair(MlDsaKeypair::generate())
    }

    fn make_pipeline() -> TxPipeline {
        let rpc = RpcClient::new("http://localhost:3000").unwrap();
        TxPipeline::new(rpc)
    }

    #[test]
    fn test_pipeline_creation() {
        let pipeline = make_pipeline();
        assert!(pipeline.history().is_empty());
    }

    #[test]
    fn test_sign_and_encode() {
        let signer = make_signer();
        let builder = TxBuilder::new(*signer.address());
        let tx = builder.transfer([0xBBu8; 32], 1000, 0);

        // Pipeline tests intentionally exercise the chain-id-bound
        // signing helper (Day 12C audit fix). Using "" simulates the
        // legacy verify_signatures=false test profile.
        let (sig_hex, pk_hex) = sign_and_encode_for_chain(&signer, &tx, "");
        assert!(!sig_hex.is_empty());
        assert!(!pk_hex.is_empty());

        // Verify the hex decodes back to valid bytes
        let sig_bytes = hex::decode(&sig_hex).unwrap();
        let pk_bytes = hex::decode(&pk_hex).unwrap();
        assert_eq!(pk_bytes.len(), 1952); // ML-DSA public key size
        assert!(!sig_bytes.is_empty());
    }

    #[test]
    fn test_sign_and_encode_signature_valid() {
        use evaporchain_crypto::signatures::{MlDsaVerifier, Verifier};

        let signer = make_signer();
        let builder = TxBuilder::new(*signer.address());
        let tx = builder.transfer([0xBBu8; 32], 500, 1);

        // Pipeline tests intentionally exercise the chain-id-bound
        // signing helper (Day 12C audit fix). Using "" simulates the
        // legacy verify_signatures=false test profile.
        let (sig_hex, pk_hex) = sign_and_encode_for_chain(&signer, &tx, "");
        let sig = hex::decode(&sig_hex).unwrap();
        let pk = hex::decode(&pk_hex).unwrap();

        // The signature is now bound to the chain's canonical
        // signing message (chain-id-prefixed signable_bytes), NOT
        // signable_bytes alone. This matches what the chain's
        // `verify_tx_signature` checks.
        let msg = tx.signing_message("");
        assert!(MlDsaVerifier::verify(&msg, &sig, &pk));
    }

    #[test]
    fn test_record_success() {
        let mut pipeline = make_pipeline();
        let resp = TxResultResponse {
            success: true,
            message: "Transfer queued".to_string(),
            tx_hash: Some("0xabc123".to_string()),
        };

        pipeline.record("transfer", &resp);
        assert_eq!(pipeline.history().len(), 1);
        assert_eq!(pipeline.history()[0].tx_type, "transfer");
        assert_eq!(pipeline.history()[0].tx_hash, "0xabc123");
        assert!(matches!(
            pipeline.history()[0].status,
            TxStatus::Confirmed(_)
        ));
    }

    #[test]
    fn test_record_failure() {
        let mut pipeline = make_pipeline();
        let resp = TxResultResponse {
            success: false,
            message: "Insufficient balance".to_string(),
            tx_hash: None,
        };

        pipeline.record("transfer", &resp);
        assert!(matches!(pipeline.history()[0].status, TxStatus::Failed(_)));
    }

    #[test]
    fn test_clear_history() {
        let mut pipeline = make_pipeline();
        let resp = TxResultResponse {
            success: true,
            message: "ok".to_string(),
            tx_hash: Some("0x1".to_string()),
        };
        pipeline.record("transfer", &resp);
        pipeline.record("refresh", &resp);

        assert_eq!(pipeline.history().len(), 2);
        pipeline.clear_history();
        assert!(pipeline.history().is_empty());
    }

    #[test]
    fn test_transfer_request_construction() {
        let signer = make_signer();
        let to = [0xBBu8; 32];
        let builder = TxBuilder::new(*signer.address());
        let tx = builder.transfer(to, 1000, 5);
        let signed = signer.sign(&tx);

        let req = TransferRequest {
            from: format_address(signer.address()),
            to: format_address(&to),
            amount: 1000,
            nonce: 5,
            signature: Some(hex::encode(signed.signature().unwrap())),
            public_key: Some(hex::encode(signed.public_key().unwrap())),
        };

        assert!(req.from.starts_with("0x"));
        assert!(req.to.starts_with("0x"));
        assert_eq!(req.amount, 1000);
        assert_eq!(req.nonce, 5);
        assert!(req.signature.is_some());
        assert!(req.public_key.is_some());
    }

    #[test]
    fn test_create_object_request_construction() {
        let signer = make_signer();
        let obj_id = [0xCCu8; 32];
        let builder = TxBuilder::new(*signer.address());
        let tx = builder.create_object(obj_id, 5000, 100, vec![0xAB; 8]);
        let signed = signer.sign(&tx);

        let req = CreateObjectRequest {
            creator: format_address(signer.address()),
            object_id: format!("0x{}", hex::encode(obj_id)),
            energy: 5000,
            half_life: 100,
            signature: Some(hex::encode(signed.signature().unwrap())),
            public_key: Some(hex::encode(signed.public_key().unwrap())),
        };

        assert!(req.creator.starts_with("0x"));
        assert!(req.object_id.starts_with("0x"));
        assert_eq!(req.energy, 5000);
    }

    #[test]
    fn test_refresh_request_construction() {
        let signer = make_signer();
        let obj_id = [0xCCu8; 32];
        let builder = TxBuilder::new(*signer.address());
        let tx = builder.refresh(obj_id, 500);
        let signed = signer.sign(&tx);

        let req = RefreshRequest {
            object_id: format!("0x{}", hex::encode(obj_id)),
            energy_deposit: 500,
            signature: Some(hex::encode(signed.signature().unwrap())),
            public_key: Some(hex::encode(signed.public_key().unwrap())),
        };

        assert_eq!(req.energy_deposit, 500);
    }

    #[test]
    fn test_multiple_operations_history() {
        let mut pipeline = make_pipeline();
        for (i, tx_type) in ["transfer", "create_object", "refresh", "mint_nft"]
            .iter()
            .enumerate()
        {
            pipeline.record(
                tx_type,
                &TxResultResponse {
                    success: true,
                    message: format!("op {}", i),
                    tx_hash: Some(format!("0x{}", i)),
                },
            );
        }
        assert_eq!(pipeline.history().len(), 4);
        assert_eq!(pipeline.history()[0].tx_type, "transfer");
        assert_eq!(pipeline.history()[3].tx_type, "mint_nft");
    }
}
