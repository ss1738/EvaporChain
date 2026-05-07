//! Multi-account manager with balance/nonce caching.
//!
//! Bridges the keystore (local encrypted keys) with the on-chain state
//! (balance, nonce) fetched via the RPC client. Supports multiple named
//! accounts with an "active account" concept.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use evaporchain_types::AccountAddress;
use thiserror::Error;

use crate::address::format_address;
use crate::keystore::{KeyStore, KeyStoreError};
use crate::rpc::{RpcClient, RpcError};
use crate::signer::{SignerError, WalletSigner};

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum AccountError {
    #[error("keystore error: {0}")]
    KeyStore(#[from] KeyStoreError),
    #[error("signer error: {0}")]
    Signer(#[from] SignerError),
    #[error("rpc error: {0}")]
    Rpc(#[from] RpcError),
    #[error("no active account set")]
    NoActiveAccount,
    #[error("account not found: {0}")]
    NotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

// ──────────────────────────── Cached State ─────────────────────────────

/// Cached on-chain state for an account.
#[derive(Debug, Clone)]
pub struct CachedAccountState {
    pub balance: u64,
    pub nonce: u64,
    pub last_fetched: Instant,
}

impl CachedAccountState {
    /// How many seconds since the cache was last refreshed.
    pub fn age_secs(&self) -> u64 {
        self.last_fetched.elapsed().as_secs()
    }
}

// ──────────────────────────── Account Info ─────────────────────────────

/// Summary of a named account (for listing).
#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub name: String,
    pub address: String,
    pub balance: Option<u64>,
    pub nonce: Option<u64>,
    pub is_active: bool,
}

// ──────────────────────────── Account Manager ──────────────────────────

/// Manages multiple named accounts backed by a keystore.
pub struct AccountManager {
    keystore: KeyStore,
    rpc: RpcClient,
    active_account: Option<String>,
    cache: HashMap<String, CachedAccountState>,
}

impl AccountManager {
    /// Create a new account manager.
    pub fn new(keystore: KeyStore, rpc: RpcClient) -> Self {
        Self {
            keystore,
            rpc,
            active_account: None,
            cache: HashMap::new(),
        }
    }

    /// Load a keystore from file and create an account manager.
    pub fn load<P: AsRef<Path>>(path: P, rpc: RpcClient) -> Result<Self, AccountError> {
        let keystore = KeyStore::load(path)?;
        Ok(Self::new(keystore, rpc))
    }

    /// Save the keystore to a file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), AccountError> {
        self.keystore.save(path)?;
        Ok(())
    }

    /// Get a reference to the underlying keystore.
    pub fn keystore(&self) -> &KeyStore {
        &self.keystore
    }

    /// Get a mutable reference to the underlying keystore.
    pub fn keystore_mut(&mut self) -> &mut KeyStore {
        &mut self.keystore
    }

    /// Get a reference to the RPC client.
    pub fn rpc(&self) -> &RpcClient {
        &self.rpc
    }

    // ── Account CRUD ───────────────────────────────────────────────────

    /// Generate a new account and add it to the keystore.
    pub fn create_account(
        &mut self,
        name: &str,
        password: &str,
    ) -> Result<AccountAddress, AccountError> {
        let addr = self.keystore.generate_key(name, password)?;
        // Auto-set as active if it's the first account
        if self.active_account.is_none() {
            self.active_account = Some(name.to_string());
        }
        Ok(addr)
    }

    /// Import an existing key pair into the keystore. Account address
    /// is derived as `blake3(public_key)` — see
    /// [`Self::import_account_with_address`] for cases where the
    /// on-chain address was hand-picked at genesis and decouples
    /// from `hash(public_key)`.
    pub fn import_account(
        &mut self,
        name: &str,
        password: &str,
        public_key: &[u8],
        secret_key: &[u8],
    ) -> Result<AccountAddress, AccountError> {
        let addr = self
            .keystore
            .import_key(name, password, public_key, secret_key)?;
        if self.active_account.is_none() {
            self.active_account = Some(name.to_string());
        }
        Ok(addr)
    }

    /// Import a keypair WITH an explicit address override.
    ///
    /// The chain accepts signed transactions where `from` decouples
    /// from `hash(public_key)` — used by genesis allocations whose
    /// addresses were hand-picked (e.g. validator-N operator
    /// accounts at `[N, 0, 0, ...]` in `genesis-mainnet.json`). This
    /// method binds the keystore entry to a caller-supplied address
    /// instead of deriving from the pubkey.
    ///
    /// Safety: callers MUST verify the keypair actually controls the
    /// override address on-chain (e.g. the chain accepted a prior
    /// signed tx from this address with this public_key). Mis-pairing
    /// will produce signed txs the chain rejects on signature-verify.
    pub fn import_account_with_address(
        &mut self,
        name: &str,
        password: &str,
        public_key: &[u8],
        secret_key: &[u8],
        address: AccountAddress,
    ) -> Result<AccountAddress, AccountError> {
        let addr = self.keystore.import_key_with_address(
            name,
            password,
            public_key,
            secret_key,
            address,
        )?;
        if self.active_account.is_none() {
            self.active_account = Some(name.to_string());
        }
        Ok(addr)
    }

    /// Remove an account from the keystore and clear its cache.
    pub fn remove_account(&mut self, name: &str) -> bool {
        let removed = self.keystore.remove(name);
        if removed {
            let addr_hex = self
                .keystore
                .list()
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, a)| a.to_string());
            if let Some(addr) = addr_hex {
                self.cache.remove(&addr);
            }
            if self.active_account.as_deref() == Some(name) {
                self.active_account = self.keystore.list().first().map(|(n, _)| n.to_string());
            }
        }
        removed
    }

    /// List all accounts with cached balance/nonce info.
    pub fn list_accounts(&self) -> Vec<AccountInfo> {
        self.keystore
            .list()
            .iter()
            .map(|(name, addr)| {
                let cached = self.cache.get(*addr);
                AccountInfo {
                    name: name.to_string(),
                    address: addr.to_string(),
                    balance: cached.map(|c| c.balance),
                    nonce: cached.map(|c| c.nonce),
                    is_active: self.active_account.as_deref() == Some(*name),
                }
            })
            .collect()
    }

    /// Number of accounts in the keystore.
    pub fn account_count(&self) -> usize {
        self.keystore.len()
    }

    // ── Active account ─────────────────────────────────────────────────

    /// Set the active account by name.
    pub fn set_active(&mut self, name: &str) -> Result<(), AccountError> {
        if !self.keystore.list().iter().any(|(n, _)| *n == name) {
            return Err(AccountError::NotFound(name.to_string()));
        }
        self.active_account = Some(name.to_string());
        Ok(())
    }

    /// Get the name of the active account.
    pub fn active_name(&self) -> Option<&str> {
        self.active_account.as_deref()
    }

    /// Get the address of the active account.
    pub fn active_address(&self) -> Option<AccountAddress> {
        self.active_account
            .as_ref()
            .and_then(|name| self.keystore.get_address(name))
    }

    /// Get the formatted hex address of the active account.
    pub fn active_address_hex(&self) -> Option<String> {
        self.active_address().map(|a| format_address(&a))
    }

    // ── Balance & nonce ────────────────────────────────────────────────

    /// Fetch the latest balance and nonce for an account from the node.
    pub async fn refresh_balance(&mut self, name: &str) -> Result<(u64, u64), AccountError> {
        let addr = self
            .keystore
            .get_address(name)
            .ok_or_else(|| AccountError::NotFound(name.to_string()))?;
        let addr_hex = format_address(&addr);

        let detail = self.rpc.get_address_detail(&addr_hex).await?;

        let state = CachedAccountState {
            balance: detail.balance,
            nonce: detail.nonce,
            last_fetched: Instant::now(),
        };
        self.cache.insert(addr_hex, state);

        Ok((detail.balance, detail.nonce))
    }

    /// Refresh balances for all accounts.
    pub async fn refresh_all(&mut self) -> Result<(), AccountError> {
        let names: Vec<String> = self
            .keystore
            .list()
            .iter()
            .map(|(n, _)| n.to_string())
            .collect();
        for name in names {
            self.refresh_balance(&name).await?;
        }
        Ok(())
    }

    /// Get cached balance and nonce (no network call).
    pub fn cached_balance(&self, name: &str) -> Option<(u64, u64)> {
        let addr = self.keystore.get_address(name)?;
        let addr_hex = format_address(&addr);
        self.cache.get(&addr_hex).map(|c| (c.balance, c.nonce))
    }

    /// Get the cached nonce for an account.
    pub fn cached_nonce(&self, name: &str) -> Option<u64> {
        self.cached_balance(name).map(|(_, n)| n)
    }

    /// Increment the cached nonce (call after successful tx submission).
    pub fn increment_nonce(&mut self, name: &str) {
        if let Some(addr) = self.keystore.get_address(name) {
            let addr_hex = format_address(&addr);
            if let Some(state) = self.cache.get_mut(&addr_hex) {
                state.nonce += 1;
            }
        }
    }

    // ── Signing ────────────────────────────────────────────────────────

    /// Unlock a signer for the named account.
    pub fn get_signer(&self, name: &str, password: &str) -> Result<WalletSigner, AccountError> {
        let signer = WalletSigner::unlock(&self.keystore, name, password)?;
        Ok(signer)
    }

    /// Unlock a signer for the active account.
    pub fn get_active_signer(&self, password: &str) -> Result<WalletSigner, AccountError> {
        let name = self
            .active_account
            .as_deref()
            .ok_or(AccountError::NoActiveAccount)?;
        self.get_signer(name, password)
    }
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manager() -> AccountManager {
        let keystore = KeyStore::new();
        let rpc = RpcClient::new("http://localhost:3000").unwrap();
        AccountManager::new(keystore, rpc)
    }

    #[test]
    fn test_create_account() {
        let mut mgr = make_manager();
        let addr = mgr.create_account("alice", "pass123").unwrap();
        assert_ne!(addr, [0u8; 32]);
        assert_eq!(mgr.account_count(), 1);
    }

    #[test]
    fn test_first_account_becomes_active() {
        let mut mgr = make_manager();
        mgr.create_account("alice", "pass").unwrap();
        assert_eq!(mgr.active_name(), Some("alice"));
    }

    #[test]
    fn test_list_accounts() {
        let mut mgr = make_manager();
        mgr.create_account("alice", "pass1").unwrap();
        mgr.create_account("bob", "pass2").unwrap();

        let list = mgr.list_accounts();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "alice");
        assert!(list[0].is_active);
        assert!(!list[1].is_active);
    }

    #[test]
    fn test_set_active() {
        let mut mgr = make_manager();
        mgr.create_account("alice", "pass1").unwrap();
        mgr.create_account("bob", "pass2").unwrap();

        mgr.set_active("bob").unwrap();
        assert_eq!(mgr.active_name(), Some("bob"));
    }

    #[test]
    fn test_set_active_nonexistent() {
        let mut mgr = make_manager();
        let result = mgr.set_active("nobody");
        assert!(result.is_err());
    }

    #[test]
    fn test_active_address() {
        let mut mgr = make_manager();
        let addr = mgr.create_account("alice", "pass").unwrap();
        assert_eq!(mgr.active_address(), Some(addr));
        assert!(mgr.active_address_hex().unwrap().starts_with("0x"));
    }

    #[test]
    fn test_get_signer() {
        let mut mgr = make_manager();
        mgr.create_account("alice", "pass").unwrap();

        let signer = mgr.get_signer("alice", "pass").unwrap();
        assert_ne!(*signer.address(), [0u8; 32]);
    }

    #[test]
    fn test_get_active_signer() {
        let mut mgr = make_manager();
        mgr.create_account("alice", "pass").unwrap();

        let signer = mgr.get_active_signer("pass").unwrap();
        assert_eq!(*signer.address(), mgr.active_address().unwrap());
    }

    #[test]
    fn test_no_active_account_error() {
        let mgr = make_manager();
        let result = mgr.get_active_signer("pass");
        assert!(matches!(result, Err(AccountError::NoActiveAccount)));
    }

    #[test]
    fn test_cached_balance_none_before_refresh() {
        let mut mgr = make_manager();
        mgr.create_account("alice", "pass").unwrap();
        assert!(mgr.cached_balance("alice").is_none());
    }

    #[test]
    fn test_remove_account() {
        let mut mgr = make_manager();
        mgr.create_account("alice", "pass1").unwrap();
        mgr.create_account("bob", "pass2").unwrap();

        assert!(mgr.remove_account("alice"));
        assert_eq!(mgr.account_count(), 1);
        // Active should switch to bob
        assert_eq!(mgr.active_name(), Some("bob"));
    }

    #[test]
    fn test_increment_nonce() {
        let mut mgr = make_manager();
        mgr.create_account("alice", "pass").unwrap();
        let addr = mgr.active_address().unwrap();
        let addr_hex = format_address(&addr);

        // Manually seed the cache
        mgr.cache.insert(
            addr_hex,
            CachedAccountState {
                balance: 10000,
                nonce: 5,
                last_fetched: Instant::now(),
            },
        );

        assert_eq!(mgr.cached_nonce("alice"), Some(5));
        mgr.increment_nonce("alice");
        assert_eq!(mgr.cached_nonce("alice"), Some(6));
    }
}
