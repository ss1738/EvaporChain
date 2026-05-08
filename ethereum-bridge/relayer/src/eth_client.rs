//! Ethereum-side client. Wraps an alloy provider + signer and binds the
//! `EvaporHeaderInbox` ABI generated from the Foundry artifact at
//! `../contracts/out/EvaporHeaderInbox.sol/EvaporHeaderInbox.json`.
//!
//! Two operations:
//!   - `latest_height` — read storage to resume from the right height
//!   - `submit_header` — pack calldata, send tx, await receipt

use alloy::network::EthereumWallet;
use alloy::primitives::{Address, Bytes, FixedBytes};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use anyhow::{Context, Result};
use std::str::FromStr;

use crate::config::Config;

// Contract bindings auto-generated from the Foundry artifact.
// `Header` and `Validator` structs are defined alongside the contract
// at module scope (the sol! macro flattens contract-internal structs
// into the surrounding module).
alloy::sol!(
    #[sol(rpc)]
    #[allow(missing_docs)]
    EvaporHeaderInbox,
    "../contracts/out/EvaporHeaderInbox.sol/EvaporHeaderInbox.json"
);
use BridgeTypes::Validator;
use EvaporHeaderInbox::Header;

pub struct EthClient {
    inbox_address: Address,
    rpc_url: String,
    signer: PrivateKeySigner,
}

impl EthClient {
    pub async fn new(cfg: &Config) -> Result<Self> {
        let signer = PrivateKeySigner::from_str(&cfg.signer_private_key)
            .context("parsing relayer private key")?;
        let inbox_address = Address::from_str(&cfg.inbox_address)
            .context("parsing inbox address")?;
        Ok(Self {
            inbox_address,
            rpc_url: cfg.ethereum_rpc.clone(),
            signer,
        })
    }

    /// Read the current `latestHeight` storage.
    pub async fn latest_height(&self) -> Result<u64> {
        let wallet = EthereumWallet::from(self.signer.clone());
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .on_http(self.rpc_url.parse()?);
        let inbox = EvaporHeaderInbox::new(self.inbox_address, provider);
        let h = inbox.latestHeight().call().await?;
        Ok(h._0)
    }

    /// Submit a finalised header. Returns the tx hash on inclusion.
    pub async fn submit_header(
        &self,
        height: u64,
        block_hash: [u8; 32],
        state_root: [u8; 32],
        mmr_root: [u8; 32],
        epoch: u64,
        validators_compressed: Vec<(Vec<u8>, u128)>,
        prev_pubkeys_uncompressed: Vec<u8>,
        signed_bitmap: Vec<u8>,
        agg_signature: Vec<u8>,
    ) -> Result<String> {
        let wallet = EthereumWallet::from(self.signer.clone());
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .on_http(self.rpc_url.parse()?);
        let inbox = EvaporHeaderInbox::new(self.inbox_address, provider);

        let header = Header {
            height,
            blockHash: FixedBytes::from(block_hash),
            stateRoot: FixedBytes::from(state_root),
            mmrRoot: FixedBytes::from(mmr_root),
            evaporchainEpoch: epoch,
        };

        let validators: Vec<Validator> = validators_compressed
            .into_iter()
            .map(|(pk, stake)| Validator {
                pubkey: Bytes::from(pk),
                stake,
            })
            .collect();

        let pending = inbox
            .submitHeader(
                header,
                validators,
                Bytes::from(prev_pubkeys_uncompressed),
                Bytes::from(signed_bitmap),
                Bytes::from(agg_signature),
            )
            .send()
            .await?;
        let receipt = pending.get_receipt().await?;
        Ok(format!("0x{:x}", receipt.transaction_hash))
    }
}
