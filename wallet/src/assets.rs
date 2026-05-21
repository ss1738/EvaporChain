//! Asset manager — unified portfolio view with real-time decay tracking.
//!
//! Aggregates native balance, state objects, NFTs, and tokens for an
//! address into a single `Portfolio`. Provides client-side decay
//! prediction using `energy_at_epoch` from evaporchain-types.

use evaporchain_types::energy_at_epoch;
use thiserror::Error;

use crate::rpc::{
    AddressDetailResponse, NftResponse, ObjectResponse, RpcClient, RpcError, TokenBalanceEntry,
};

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("rpc error: {0}")]
    Rpc(#[from] RpcError),
}

// ──────────────────────────── Portfolio ────────────────────────────────

/// Complete portfolio for an address.
#[derive(Debug, Clone)]
pub struct Portfolio {
    pub address: String,
    pub balance: u64,
    pub nonce: u64,
    pub objects: Vec<OwnedObject>,
    pub nfts: Vec<OwnedNft>,
    pub tokens: Vec<TokenBalance>,
    /// Sum of current energy across all decaying assets.
    pub total_energy_at_risk: u64,
}

/// A state object owned by the user, with computed decay fields.
#[derive(Debug, Clone)]
pub struct OwnedObject {
    pub id: String,
    pub name: String,
    pub energy: u64,
    pub max_energy: u64,
    pub current_energy: u64,
    pub half_life: u64,
    pub state: String,
    pub created_epoch: u64,
    pub last_refreshed: u64,
    pub decay_percentage: f64,
    /// Estimated epochs until energy reaches grace threshold (1).
    pub epochs_until_zero: Option<u64>,
}

/// An NFT owned by the user, with decay info.
#[derive(Debug, Clone)]
pub struct OwnedNft {
    pub id: u64,
    pub name: String,
    pub collection: String,
    pub current_energy: u64,
    pub max_energy: u64,
    pub half_life: u64,
    pub state: String,
    pub decay_percentage: f64,
    pub epochs_remaining: u64,
    pub grace_epoch: Option<u64>,
    pub evaporated_epoch: Option<u64>,
}

/// Token balance for the user.
#[derive(Debug, Clone)]
pub struct TokenBalance {
    pub token_id: u64,
    pub symbol: String,
    pub name: String,
    pub balance: u64,
}

// ──────────────────────────── Asset Manager ────────────────────────────

/// Manages portfolio queries and decay computations.
pub struct AssetManager {
    rpc: RpcClient,
}

impl AssetManager {
    /// Create a new asset manager.
    pub fn new(rpc: RpcClient) -> Self {
        Self { rpc }
    }

    /// Fetch the complete portfolio for an address.
    pub async fn fetch_portfolio(&self, address: &str) -> Result<Portfolio, AssetError> {
        let detail = self.rpc.get_address_detail(address).await?;
        Ok(build_portfolio(detail))
    }

    /// Get a reference to the RPC client.
    pub fn rpc(&self) -> &RpcClient {
        &self.rpc
    }
}

// ──────────────────────────── Decay Math ───────────────────────────────

/// Predict energy at a future epoch given current state.
/// Uses the same decay formula as the chain: E(t) = E₀ × 2^(-t/τ).
pub fn predict_energy(current_energy: u64, half_life: u64, epochs_forward: u64) -> u64 {
    if half_life == 0 || current_energy == 0 {
        return 0;
    }
    energy_at_epoch(current_energy, half_life, epochs_forward)
}

/// Calculate how many epochs until energy drops below a given threshold.
/// Returns `None` if current energy is already at or below threshold,
/// or if half_life is 0.
pub fn epochs_until_threshold(current_energy: u64, half_life: u64, threshold: u64) -> Option<u64> {
    if half_life == 0 || current_energy <= threshold {
        return None;
    }
    // Binary search for the epoch where energy drops below threshold
    let mut lo: u64 = 0;
    let mut hi: u64 = half_life.saturating_mul(64); // energy is 0 well before 64 halvings
    if energy_at_epoch(current_energy, half_life, hi) > threshold {
        return None; // shouldn't happen, but safeguard
    }
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if energy_at_epoch(current_energy, half_life, mid) > threshold {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(hi)
}

/// Calculate how many epochs until energy reaches zero (effectively).
pub fn epochs_until_zero(current_energy: u64, half_life: u64) -> Option<u64> {
    epochs_until_threshold(current_energy, half_life, 0)
}

// ──────────────────────────── Internal ─────────────────────────────────

fn build_portfolio(detail: AddressDetailResponse) -> Portfolio {
    let objects: Vec<OwnedObject> = detail.objects.iter().map(map_object).collect();
    let nfts: Vec<OwnedNft> = detail.nfts.iter().map(map_nft).collect();
    let tokens: Vec<TokenBalance> = detail.tokens.iter().map(map_token).collect();

    let total_energy_at_risk: u64 = objects.iter().map(|o| o.current_energy).sum::<u64>()
        + nfts.iter().map(|n| n.current_energy).sum::<u64>();

    Portfolio {
        address: detail.address,
        balance: detail.balance,
        nonce: detail.nonce,
        objects,
        nfts,
        tokens,
        total_energy_at_risk,
    }
}

fn map_object(o: &ObjectResponse) -> OwnedObject {
    OwnedObject {
        id: o.id.clone(),
        name: o.name.clone(),
        energy: o.energy,
        max_energy: o.max_energy,
        current_energy: o.current_energy,
        half_life: o.half_life,
        state: o.state.clone(),
        created_epoch: o.created_epoch,
        last_refreshed: o.last_refreshed,
        decay_percentage: o.decay_percentage,
        epochs_until_zero: epochs_until_zero(o.current_energy, o.half_life),
    }
}

fn map_nft(n: &NftResponse) -> OwnedNft {
    OwnedNft {
        id: n.id,
        name: n.name.clone(),
        collection: n.collection.clone(),
        current_energy: n.current_energy,
        max_energy: n.max_energy,
        half_life: n.half_life,
        state: n.state.clone(),
        decay_percentage: n.decay_percentage,
        epochs_remaining: n.epochs_remaining,
        grace_epoch: n.grace_epoch,
        evaporated_epoch: n.evaporated_epoch,
    }
}

fn map_token(t: &TokenBalanceEntry) -> TokenBalance {
    TokenBalance {
        token_id: t.token_id,
        symbol: t.symbol.clone(),
        name: t.name.clone(),
        balance: t.balance,
    }
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::ObjectResponse;

    #[test]
    fn test_predict_energy_basic() {
        // After 1 half-life, energy should be ~half
        let result = predict_energy(1000, 100, 100);
        assert_eq!(result, 500);
    }

    #[test]
    fn test_predict_energy_two_halvings() {
        let result = predict_energy(1000, 100, 200);
        assert_eq!(result, 250);
    }

    #[test]
    fn test_predict_energy_zero_half_life() {
        assert_eq!(predict_energy(1000, 0, 50), 0);
    }

    #[test]
    fn test_predict_energy_zero_current() {
        assert_eq!(predict_energy(0, 100, 50), 0);
    }

    #[test]
    fn test_predict_energy_no_elapsed() {
        assert_eq!(predict_energy(1000, 100, 0), 1000);
    }

    #[test]
    fn test_epochs_until_threshold() {
        // Energy 1000, half_life 100. When does it drop below 100?
        // After ~3.32 halvings = ~332 epochs
        let result = epochs_until_threshold(1000, 100, 100).unwrap();
        // Verify: energy at (result-1) should be > 100, at result should be <= 100
        assert!(energy_at_epoch(1000, 100, result - 1) > 100);
        assert!(energy_at_epoch(1000, 100, result) <= 100);
    }

    #[test]
    fn test_epochs_until_threshold_already_below() {
        assert!(epochs_until_threshold(50, 100, 100).is_none());
    }

    #[test]
    fn test_epochs_until_zero() {
        let result = epochs_until_zero(1000, 100).unwrap();
        assert!(energy_at_epoch(1000, 100, result) == 0);
        assert!(energy_at_epoch(1000, 100, result - 1) > 0);
    }

    #[test]
    fn test_epochs_until_zero_half_life_zero() {
        assert!(epochs_until_zero(1000, 0).is_none());
    }

    #[test]
    fn test_build_portfolio() {
        let detail = AddressDetailResponse {
            address: "0xaa".to_string(),
            balance: 50000,
            nonce: 3,
            objects: vec![ObjectResponse {
                id: "0xcc".to_string(),
                name: "test-obj".to_string(),
                owner: "0xaa".to_string(),
                owner_name: "Alice".to_string(),
                energy: 5000,
                max_energy: 5000,
                half_life: 100,
                state: "Active".to_string(),
                created_epoch: 0,
                last_refreshed: 0,
                grace_epoch: None,
                current_energy: 4500,
                decay_percentage: 10.0,
            }],
            nfts: vec![],
            tokens: vec![TokenBalanceEntry {
                token_id: 1,
                symbol: "EVAP".to_string(),
                name: "EvaporToken".to_string(),
                balance: 1000,
            }],
        };

        let portfolio = build_portfolio(detail);
        assert_eq!(portfolio.balance, 50000);
        assert_eq!(portfolio.objects.len(), 1);
        assert_eq!(portfolio.objects[0].current_energy, 4500);
        assert!(portfolio.objects[0].epochs_until_zero.is_some());
        assert_eq!(portfolio.tokens.len(), 1);
        assert_eq!(portfolio.tokens[0].symbol, "EVAP");
        assert_eq!(portfolio.total_energy_at_risk, 4500);
    }

    #[test]
    fn test_portfolio_total_energy_includes_nfts() {
        let detail = AddressDetailResponse {
            address: "0xaa".to_string(),
            balance: 1000,
            nonce: 0,
            objects: vec![ObjectResponse {
                id: "0xcc".to_string(),
                name: "obj".to_string(),
                owner: "0xaa".to_string(),
                owner_name: "A".to_string(),
                energy: 3000,
                max_energy: 3000,
                half_life: 50,
                state: "Active".to_string(),
                created_epoch: 0,
                last_refreshed: 0,
                grace_epoch: None,
                current_energy: 2000,
                decay_percentage: 33.3,
            }],
            nfts: vec![NftResponse {
                id: 1,
                name: "nft".to_string(),
                collection: "art".to_string(),
                owner: "0xaa".to_string(),
                metadata_hash: "0x".to_string(),
                energy: 5000,
                max_energy: 5000,
                current_energy: 3000,
                half_life: 100,
                minted_epoch: 0,
                last_refreshed: 0,
                state: "Active".to_string(),
                decay_percentage: 40.0,
                epochs_remaining: 80,
                grace_epoch: None,
                evaporated_epoch: None,
                ghost_proof: None,
            }],
            tokens: vec![],
        };

        let portfolio = build_portfolio(detail);
        assert_eq!(portfolio.total_energy_at_risk, 2000 + 3000);
    }

    // ─── Additional coverage tests ────────────────────────────────────────────

    #[test]
    fn test_asset_manager_new_and_rpc_covers_lines_88_101() {
        use crate::rpc::RpcClient;
        let rpc = RpcClient::new("http://localhost:9999").unwrap();
        let manager = AssetManager::new(rpc);
        // rpc() returns a reference — just verify it doesn't panic
        let _ = manager.rpc();
    }
}
