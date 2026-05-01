//! Nonce management with auto-retry on stale nonce.
//!
//! Tracks the expected next nonce per address. On submission failure
//! due to stale nonce, re-fetches from the node and retries.

use std::collections::HashMap;

use crate::rpc::{RpcClient, RpcError};

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum NonceError {
    #[error("rpc error: {0}")]
    Rpc(#[from] RpcError),
    #[error("address not tracked: {0}")]
    NotTracked(String),
}

// ──────────────────────────── NonceManager ─────────────────────────────

/// Tracks per-address nonces with refresh-on-conflict.
pub struct NonceManager {
    /// address_hex → next expected nonce
    cache: HashMap<String, u64>,
}

impl NonceManager {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Fetch the current nonce from the node and cache it.
    pub async fn sync(&mut self, rpc: &RpcClient, address: &str) -> Result<u64, NonceError> {
        let detail = rpc.get_address_detail(address).await?;
        let nonce = detail.nonce;
        self.cache.insert(address.to_string(), nonce);
        Ok(nonce)
    }

    /// Get the cached nonce for an address, or fetch it if not cached.
    pub async fn get(&mut self, rpc: &RpcClient, address: &str) -> Result<u64, NonceError> {
        if let Some(&nonce) = self.cache.get(address) {
            Ok(nonce)
        } else {
            self.sync(rpc, address).await
        }
    }

    /// Increment the cached nonce after a successful submission.
    pub fn increment(&mut self, address: &str) {
        if let Some(nonce) = self.cache.get_mut(address) {
            *nonce += 1;
        }
    }

    /// Force-set the nonce (e.g., after a retry).
    pub fn set(&mut self, address: &str, nonce: u64) {
        self.cache.insert(address.to_string(), nonce);
    }

    /// Check if a submission error looks like a stale nonce.
    pub fn is_nonce_error(message: &str) -> bool {
        let lower = message.to_lowercase();
        lower.contains("nonce")
            || lower.contains("already used")
            || lower.contains("too low")
            || lower.contains("stale")
    }

    /// Re-sync from node and return the fresh nonce.
    /// Call this after a nonce-related submission failure.
    pub async fn resync(&mut self, rpc: &RpcClient, address: &str) -> Result<u64, NonceError> {
        self.sync(rpc, address).await
    }

    /// Execute a closure with automatic nonce retry.
    ///
    /// 1. Gets current nonce
    /// 2. Calls `f(nonce)` to submit the transaction
    /// 3. On nonce error, re-syncs and retries once
    /// 4. On success, increments cached nonce
    pub async fn with_retry<F, Fut, T, E>(
        &mut self,
        rpc: &RpcClient,
        address: &str,
        mut f: F,
    ) -> Result<T, NonceError>
    where
        F: FnMut(u64) -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        let nonce = self.get(rpc, address).await?;

        match f(nonce).await {
            Ok(result) => {
                self.increment(address);
                Ok(result)
            }
            Err(e) => {
                let msg = e.to_string();
                if Self::is_nonce_error(&msg) {
                    // Retry with fresh nonce
                    let fresh_nonce = self.resync(rpc, address).await?;
                    match f(fresh_nonce).await {
                        Ok(result) => {
                            self.increment(address);
                            Ok(result)
                        }
                        Err(e2) => Err(NonceError::Rpc(RpcError::Api {
                            status: 400,
                            message: e2.to_string(),
                        })),
                    }
                } else {
                    Err(NonceError::Rpc(RpcError::Api {
                        status: 400,
                        message: msg,
                    }))
                }
            }
        }
    }
}

impl Default for NonceManager {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────── Tests ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_empty() {
        let mgr = NonceManager::new();
        assert!(mgr.cache.is_empty());
    }

    #[test]
    fn test_set_and_increment() {
        let mut mgr = NonceManager::new();
        mgr.set("0xabc", 5);
        assert_eq!(mgr.cache.get("0xabc"), Some(&5));
        mgr.increment("0xabc");
        assert_eq!(mgr.cache.get("0xabc"), Some(&6));
    }

    #[test]
    fn test_increment_missing_is_noop() {
        let mut mgr = NonceManager::new();
        mgr.increment("0xmissing"); // should not panic
    }

    #[test]
    fn test_is_nonce_error() {
        assert!(NonceManager::is_nonce_error("nonce too low"));
        assert!(NonceManager::is_nonce_error(
            "Transaction nonce already used"
        ));
        assert!(NonceManager::is_nonce_error(
            "Stale nonce: expected 5, got 3"
        ));
        assert!(!NonceManager::is_nonce_error("insufficient balance"));
        assert!(!NonceManager::is_nonce_error("object not found"));
    }

    #[test]
    fn test_set_overwrites() {
        let mut mgr = NonceManager::new();
        mgr.set("0xabc", 10);
        mgr.set("0xabc", 20);
        assert_eq!(mgr.cache.get("0xabc"), Some(&20));
    }

    #[test]
    fn test_multiple_addresses() {
        let mut mgr = NonceManager::new();
        mgr.set("0xaaa", 1);
        mgr.set("0xbbb", 5);
        mgr.increment("0xaaa");
        assert_eq!(mgr.cache.get("0xaaa"), Some(&2));
        assert_eq!(mgr.cache.get("0xbbb"), Some(&5));
    }
}
