//! Off-chain auction state machine.
//!
//! The auctioneer maintains a registry of known vault listings.  For each
//! listing it holds an off-chain `Auction` (from `evaporchain-sddc`) and the
//! bid book.  On every poll tick it:
//!
//! 1. Runs `evaporchain_sfsv::market::settle_secondary` against accumulated
//!    bids to see if any bid clears at the current epoch's price.
//! 2. If a bid clears it hands the winner address back to the coordinator
//!    loop which submits a `record_sale` TX on-chain.
//!
//! The on-chain `record_sale` enforces the vault state machine guards
//! (listed / not-expired / not-released) — the off-chain SDDC clear does NOT
//! bypass them.  This is the key invariant of the SFSV coordinator design.
//!
//! INVENTION_STACK §A5.2: SFSV off-chain coordinator.

use std::collections::HashMap;

use evaporchain_sddc::{auction::Auction, bid::Bid};
use evaporchain_sfsv::{
    market::{self, MarketError},
    vault::Vault,
};
use evaporchain_types::{AccountAddress, DecayRate, Energy, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

/// Stable key identifying a listing (contract id on-chain).
pub type ContractId = u64;

#[derive(Debug, Error)]
pub enum AuctioneerError {
    #[error("no active listing for contract {0}")]
    NotListed(ContractId),
    #[error("listing already tracked for contract {0}")]
    AlreadyTracked(ContractId),
    #[error("market error: {0}")]
    Market(#[from] MarketError),
}

/// Summary of a just-cleared listing returned to the coordinator loop.
#[derive(Debug, Clone)]
pub struct ClearResult {
    pub contract_id: ContractId,
    pub winner: AccountAddress,
    pub price_paid: Energy,
    pub epoch: Epoch,
}

/// Per-listing off-chain state.
struct ListingState {
    vault: Vault,
    auction: Auction,
    bids: Vec<Bid>,
}

/// A `BidRequest` is what arrives on the HTTP bid-intake endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BidRequest {
    pub contract_id: ContractId,
    /// Bidder's on-chain address (32-byte hex, no `0x` prefix).
    pub bidder_hex: String,
    /// Maximum energy the bidder will pay.
    pub max_price: Energy,
    /// Maximum λ (decay rate) the bidder is willing to hold.
    pub lambda_tolerance: DecayRate,
    /// Epoch the bid was submitted at.
    pub submitted_at: Epoch,
}

pub struct Auctioneer {
    listings: HashMap<ContractId, ListingState>,
}

impl Auctioneer {
    pub fn new() -> Self {
        Self {
            listings: HashMap::new(),
        }
    }

    /// Register a new listing discovered from the node. Idempotent — if
    /// the listing is already tracked nothing changes.
    pub fn register_listing(
        &mut self,
        contract_id: ContractId,
        vault: Vault,
        auction: Auction,
    ) -> Result<(), AuctioneerError> {
        if self.listings.contains_key(&contract_id) {
            return Err(AuctioneerError::AlreadyTracked(contract_id));
        }
        info!(contract_id, "registered new SFSV listing");
        self.listings.insert(
            contract_id,
            ListingState {
                vault,
                auction,
                bids: Vec::new(),
            },
        );
        Ok(())
    }

    /// Remove a listing (after sale or expiry).
    pub fn drop_listing(&mut self, contract_id: ContractId) {
        self.listings.remove(&contract_id);
    }

    pub fn is_tracked(&self, contract_id: ContractId) -> bool {
        self.listings.contains_key(&contract_id)
    }

    /// Append a bid to the book for `contract_id`.
    pub fn submit_bid(&mut self, req: &BidRequest) -> Result<(), AuctioneerError> {
        let state = self
            .listings
            .get_mut(&req.contract_id)
            .ok_or(AuctioneerError::NotListed(req.contract_id))?;

        let bidder = hex_to_addr(&req.bidder_hex);
        match Bid::new(bidder, req.max_price, req.lambda_tolerance, req.submitted_at) {
            Ok(bid) => {
                info!(
                    contract_id = req.contract_id,
                    bidder = %req.bidder_hex,
                    max_price = req.max_price,
                    "bid accepted"
                );
                state.bids.push(bid);
                Ok(())
            }
            Err(e) => Err(AuctioneerError::Market(MarketError::Clearing(
                evaporchain_sddc::clearing::ClearError::Lifecycle(
                    evaporchain_sddc::auction::LifecycleError::NotOpen,
                ),
            ))),
        }
    }

    /// Try to clear all open listings at `epoch_now`. Returns all clears
    /// (one per listing that finds a winning bid). Cleared listings are
    /// removed from the registry.
    pub fn try_clear_all(&mut self, epoch_now: Epoch) -> Vec<ClearResult> {
        let mut cleared = Vec::new();
        let ids: Vec<ContractId> = self.listings.keys().copied().collect();
        for id in ids {
            let state = self.listings.get_mut(&id).unwrap();
            match market::settle_secondary(
                &mut state.vault,
                &mut state.auction,
                &state.bids,
                epoch_now,
            ) {
                Ok(Some(c)) => {
                    info!(
                        contract_id = id,
                        winner = ?c.winner,
                        price_paid = c.price_paid,
                        epoch = epoch_now,
                        "SDDC clear ✓ — will submit record_sale"
                    );
                    cleared.push(ClearResult {
                        contract_id: id,
                        winner: c.winner,
                        price_paid: c.price_paid,
                        epoch: epoch_now,
                    });
                    self.listings.remove(&id);
                }
                Ok(None) => {
                    // No winning bid yet; auction still open.
                }
                Err(e) => {
                    warn!(contract_id = id, error = %e, "clearing error — dropping listing");
                    self.listings.remove(&id);
                }
            }
        }
        cleared
    }
}

fn hex_to_addr(hex: &str) -> AccountAddress {
    let mut addr = [0u8; 32];
    if let Ok(bytes) = hex::decode(hex.trim_start_matches("0x")) {
        let len = bytes.len().min(32);
        addr[..len].copy_from_slice(&bytes[..len]);
    }
    addr
}
