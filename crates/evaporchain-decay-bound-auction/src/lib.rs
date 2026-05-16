//! Decay-bound sealed-bid auction primitive.
//!
//! An auction whose end-condition is energy-decay rather than block-height.
//! Bids are commit-reveal (per the NX4 audit pattern in
//! `contracts/evaporscript/sealed_bid_auction.es`), with chain_id +
//! auction_id binding so a commitment cannot be replayed across auctions
//! or chains.
//!
//! Lifecycle:
//!
//! ```text
//!     Open            → commit phase (bids = H(price || nonce || DST))
//!     CommitClosed    → reveal phase (bidders publish price + nonce)
//!     RevealClosed    → settle: high-bid wins iff ≥ reserve_price
//!     Settled | Evaporated
//! ```
//!
//! An auction `Evaporates` (vs. Settled) when its energy decays below a
//! configurable floor BEFORE the reveal window closes — no winner, all
//! committed deposits become refundable.  This matches the doctrine
//! pattern of "things that aren't refreshed cease to exist".
//!
//! Reference contract: `contracts/evaporscript/sealed_bid_auction.es`.
//! This crate provides the substrate-level state machine that contract
//! and any other auction-shaped dApp can compose against.

use evaporchain_types::{AccountAddress, Energy, Epoch};

/// Opaque auction identifier — same shape as `evaporchain-sddc`'s
/// `AuctionId` so a downstream caller can pass the SDDC id through
/// directly without conversion.  Kept local here to avoid coupling
/// the substrate primitive to the SDDC marketplace crate.
pub type AuctionId = [u8; 32];
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

// ─────────────────────── DSTs ──────────────────────────────────────────

/// Domain-separation tag for commitment hashing.  Without this an
/// attacker could replay a commitment from one auction as a commitment
/// for a different auction (or chain) by reusing the (price, nonce)
/// pair under a different `auction_id` / `chain_id`.
const COMMITMENT_DST: &[u8] = b"EvaporChain_DecayBoundAuction_Commit_v1\0";

// ─────────────────────── Caps ──────────────────────────────────────────

/// Maximum number of bidders per auction.  Bounds memory + every
/// settle-side O(n) sort.  Pure substrate parameter; the contract
/// layer can tighten it further per-deployment.
pub const MAX_BIDDERS_PER_AUCTION: usize = 1_024;

// ─────────────────────── Types ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuctionPhase {
    /// Commit phase — bidders submit `H(price || nonce || DST)` commitments.
    Open,
    /// Commit window closed; reveal window open.
    CommitClosed,
    /// Reveal window closed; ready to settle.
    RevealClosed,
    /// Winner declared, funds settled.
    Settled,
    /// Auction's energy decayed to zero before settle; refundable.
    Evaporated,
}

/// Sealed-bid commitment: `BLAKE3(DST || chain_id || auction_id ||
/// bidder || price_le || nonce)`.  All fields are length-prefixed where
/// needed so distinct decompositions can't alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BidCommitment(pub [u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bid {
    pub bidder: AccountAddress,
    pub commitment: BidCommitment,
    pub revealed_price: Option<u64>,
    pub revealed_nonce: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecayBoundAuction {
    pub auction_id: [u8; 32],
    pub chain_id: String,
    pub sddc_id: AuctionId,
    pub reserve_price: u64,
    pub commit_deadline_epoch: Epoch,
    pub reveal_deadline_epoch: Epoch,
    pub initial_energy: Energy,
    pub half_life_epochs: u64,
    pub phase: AuctionPhase,
    pub bids: BTreeMap<AccountAddress, Bid>,
    pub winner: Option<AccountAddress>,
    pub clearing_price: Option<u64>,
}

// ─────────────────────── Errors ────────────────────────────────────────

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuctionError {
    #[error("auction not in Open phase (current: {0:?})")]
    NotOpen(AuctionPhase),
    #[error("auction not in CommitClosed phase (current: {0:?})")]
    NotCommitClosed(AuctionPhase),
    #[error("auction not in RevealClosed phase (current: {0:?})")]
    NotRevealClosed(AuctionPhase),
    #[error("max bidders ({MAX_BIDDERS_PER_AUCTION}) reached")]
    TooManyBidders,
    #[error("bidder already has a commitment (multiple commitments per bidder forbidden)")]
    DuplicateBidder,
    #[error("no commitment found for bidder")]
    NoCommitment,
    #[error("reveal does not match committed hash")]
    RevealMismatch,
    #[error("commit deadline not yet reached")]
    CommitWindowOpen,
    #[error("reveal deadline not yet reached")]
    RevealWindowOpen,
    #[error("auction energy decayed to zero; auction evaporated")]
    EvaporatedDuringReveal,
}

// ─────────────────────── Construction ──────────────────────────────────

impl DecayBoundAuction {
    pub fn new(
        auction_id: [u8; 32],
        chain_id: String,
        sddc_id: AuctionId,
        reserve_price: u64,
        commit_deadline_epoch: Epoch,
        reveal_deadline_epoch: Epoch,
        initial_energy: Energy,
        half_life_epochs: u64,
    ) -> Self {
        Self {
            auction_id,
            chain_id,
            sddc_id,
            reserve_price,
            commit_deadline_epoch,
            reveal_deadline_epoch,
            initial_energy,
            half_life_epochs,
            phase: AuctionPhase::Open,
            bids: BTreeMap::new(),
            winner: None,
            clearing_price: None,
        }
    }

    /// Compute the canonical commitment for a bid.  Off-chain bidders
    /// MUST use this exact byte layout — divergence is a commit-reveal
    /// failure mode that re-opens the front-run window.
    pub fn compute_commitment(
        chain_id: &str,
        auction_id: &[u8; 32],
        bidder: &AccountAddress,
        price: u64,
        nonce: &[u8; 32],
    ) -> BidCommitment {
        assert!(
            chain_id.len() < 256,
            "compute_commitment: chain_id is {} bytes, must be < 256 (u8-encoded length)",
            chain_id.len()
        );
        let chain_bytes = chain_id.as_bytes();
        let mut input = Vec::with_capacity(
            COMMITMENT_DST.len() + 1 + chain_bytes.len() + 32 + 32 + 8 + 32,
        );
        input.extend_from_slice(COMMITMENT_DST);
        input.push(chain_bytes.len() as u8);
        input.extend_from_slice(chain_bytes);
        input.extend_from_slice(auction_id);
        input.extend_from_slice(bidder);
        input.extend_from_slice(&price.to_le_bytes());
        input.extend_from_slice(nonce);
        BidCommitment(*blake3::hash(&input).as_bytes())
    }
}

// ─────────────────────── Commit phase ──────────────────────────────────

impl DecayBoundAuction {
    pub fn submit_commitment(
        &mut self,
        bidder: AccountAddress,
        commitment: BidCommitment,
    ) -> Result<(), AuctionError> {
        if self.phase != AuctionPhase::Open {
            return Err(AuctionError::NotOpen(self.phase));
        }
        if self.bids.len() >= MAX_BIDDERS_PER_AUCTION {
            return Err(AuctionError::TooManyBidders);
        }
        if self.bids.contains_key(&bidder) {
            return Err(AuctionError::DuplicateBidder);
        }
        self.bids.insert(
            bidder,
            Bid {
                bidder,
                commitment,
                revealed_price: None,
                revealed_nonce: None,
            },
        );
        Ok(())
    }

    /// Operator transition: commit window has elapsed.  The caller
    /// passes the current epoch; if it's past `commit_deadline_epoch`
    /// the phase flips to `CommitClosed`.
    pub fn close_commits(&mut self, current_epoch: Epoch) -> Result<(), AuctionError> {
        if self.phase != AuctionPhase::Open {
            return Err(AuctionError::NotOpen(self.phase));
        }
        if current_epoch < self.commit_deadline_epoch {
            return Err(AuctionError::CommitWindowOpen);
        }
        self.phase = AuctionPhase::CommitClosed;
        Ok(())
    }
}

// ─────────────────────── Reveal phase ──────────────────────────────────

impl DecayBoundAuction {
    pub fn reveal_bid(
        &mut self,
        bidder: AccountAddress,
        price: u64,
        nonce: [u8; 32],
    ) -> Result<(), AuctionError> {
        if self.phase != AuctionPhase::CommitClosed {
            return Err(AuctionError::NotCommitClosed(self.phase));
        }
        let chain_id = self.chain_id.clone();
        let auction_id = self.auction_id;
        let bid = self.bids.get_mut(&bidder).ok_or(AuctionError::NoCommitment)?;
        let expected = Self::compute_commitment(&chain_id, &auction_id, &bidder, price, &nonce);
        if expected != bid.commitment {
            return Err(AuctionError::RevealMismatch);
        }
        bid.revealed_price = Some(price);
        bid.revealed_nonce = Some(nonce);
        Ok(())
    }

    pub fn close_reveals(&mut self, current_epoch: Epoch) -> Result<(), AuctionError> {
        if self.phase != AuctionPhase::CommitClosed {
            return Err(AuctionError::NotCommitClosed(self.phase));
        }
        if current_epoch < self.reveal_deadline_epoch {
            return Err(AuctionError::RevealWindowOpen);
        }
        self.phase = AuctionPhase::RevealClosed;
        Ok(())
    }
}

// ─────────────────────── Settle / evaporate ────────────────────────────

impl DecayBoundAuction {
    /// Current energy level of the auction's covenant at `epoch`,
    /// routed through the canonical energy-decay formula in
    /// `evaporchain-types`.  Doctrine layer-0 invariant.
    pub fn energy_at(&self, epoch: Epoch) -> Energy {
        // Routes through the canonical energy-decay formula
        // `evaporchain_types::energy_at_epoch` (doctrine Layer-0
        // invariant — no `>>` on energy values outside types).
        evaporchain_types::energy_at_epoch(
            self.initial_energy,
            self.half_life_epochs,
            epoch.saturating_sub(self.commit_deadline_epoch.saturating_sub(self.half_life_epochs)),
        )
    }

    /// Settle the auction. Walks every revealed bid (≥ reserve_price)
    /// and picks the highest; ties broken by lexicographic bidder
    /// address (deterministic).
    ///
    /// If `energy_at(current_epoch) == 0`, auction evaporates instead
    /// of settling — winner is `None`, all commits are refundable.
    pub fn settle(&mut self, current_epoch: Epoch) -> Result<Option<AccountAddress>, AuctionError> {
        if self.phase != AuctionPhase::RevealClosed {
            return Err(AuctionError::NotRevealClosed(self.phase));
        }

        if self.energy_at(current_epoch) == 0 {
            self.phase = AuctionPhase::Evaporated;
            return Err(AuctionError::EvaporatedDuringReveal);
        }

        let mut best: Option<(AccountAddress, u64)> = None;
        for (addr, bid) in &self.bids {
            if let Some(price) = bid.revealed_price {
                if price < self.reserve_price {
                    continue;
                }
                match best {
                    None => best = Some((*addr, price)),
                    Some((best_addr, best_price)) => {
                        if price > best_price || (price == best_price && *addr < best_addr) {
                            best = Some((*addr, price));
                        }
                    }
                }
            }
        }

        self.phase = AuctionPhase::Settled;
        match best {
            Some((addr, price)) => {
                self.winner = Some(addr);
                self.clearing_price = Some(price);
                Ok(Some(addr))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(b: u8) -> AccountAddress {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    fn make_auction() -> DecayBoundAuction {
        DecayBoundAuction::new(
            [0xAA; 32],
            "evaporchain-test-1".into(),
            AuctionId::default(),
            1_000,
            100,
            200,
            1_000_000,
            50,
        )
    }

    // ─── Construction ──────────────────────────────────────────────

    #[test]
    fn new_auction_starts_open_with_no_bids() {
        let a = make_auction();
        assert_eq!(a.phase, AuctionPhase::Open);
        assert!(a.bids.is_empty());
        assert!(a.winner.is_none());
    }

    // ─── Commitment binding ────────────────────────────────────────

    #[test]
    fn commitment_binds_chain_id() {
        let id = [0u8; 32];
        let b = addr(1);
        let n = [0u8; 32];
        let c1 = DecayBoundAuction::compute_commitment("chain-A", &id, &b, 100, &n);
        let c2 = DecayBoundAuction::compute_commitment("chain-B", &id, &b, 100, &n);
        assert_ne!(c1, c2);
    }

    #[test]
    fn commitment_binds_auction_id() {
        let b = addr(1);
        let n = [0u8; 32];
        let c1 = DecayBoundAuction::compute_commitment("c", &[1u8; 32], &b, 100, &n);
        let c2 = DecayBoundAuction::compute_commitment("c", &[2u8; 32], &b, 100, &n);
        assert_ne!(c1, c2);
    }

    #[test]
    fn commitment_binds_bidder() {
        let id = [0u8; 32];
        let n = [0u8; 32];
        let c1 = DecayBoundAuction::compute_commitment("c", &id, &addr(1), 100, &n);
        let c2 = DecayBoundAuction::compute_commitment("c", &id, &addr(2), 100, &n);
        assert_ne!(c1, c2);
    }

    #[test]
    fn commitment_binds_price() {
        let id = [0u8; 32];
        let b = addr(1);
        let n = [0u8; 32];
        let c1 = DecayBoundAuction::compute_commitment("c", &id, &b, 100, &n);
        let c2 = DecayBoundAuction::compute_commitment("c", &id, &b, 101, &n);
        assert_ne!(c1, c2);
    }

    #[test]
    fn commitment_binds_nonce() {
        let id = [0u8; 32];
        let b = addr(1);
        let c1 = DecayBoundAuction::compute_commitment("c", &id, &b, 100, &[0u8; 32]);
        let c2 = DecayBoundAuction::compute_commitment("c", &id, &b, 100, &[1u8; 32]);
        assert_ne!(c1, c2);
    }

    #[test]
    #[should_panic(expected = "compute_commitment: chain_id is")]
    fn commitment_panics_on_oversized_chain_id() {
        let oversized = "x".repeat(256);
        DecayBoundAuction::compute_commitment(&oversized, &[0; 32], &addr(1), 100, &[0; 32]);
    }

    // ─── Commit phase ──────────────────────────────────────────────

    #[test]
    fn submit_commitment_records_bid() {
        let mut a = make_auction();
        let c = BidCommitment([1u8; 32]);
        a.submit_commitment(addr(1), c).unwrap();
        assert_eq!(a.bids.len(), 1);
        assert_eq!(a.bids[&addr(1)].commitment, c);
    }

    #[test]
    fn submit_commitment_rejects_duplicate_bidder() {
        let mut a = make_auction();
        a.submit_commitment(addr(1), BidCommitment([1u8; 32])).unwrap();
        let err = a.submit_commitment(addr(1), BidCommitment([2u8; 32])).unwrap_err();
        assert_eq!(err, AuctionError::DuplicateBidder);
    }

    #[test]
    fn submit_commitment_rejects_when_at_cap() {
        let mut a = make_auction();
        for i in 0..MAX_BIDDERS_PER_AUCTION {
            let mut b = [0u8; 32];
            b[..8].copy_from_slice(&(i as u64).to_le_bytes());
            a.submit_commitment(b, BidCommitment([0u8; 32])).unwrap();
        }
        let err = a
            .submit_commitment(addr(0xFF), BidCommitment([0u8; 32]))
            .unwrap_err();
        assert_eq!(err, AuctionError::TooManyBidders);
    }

    #[test]
    fn close_commits_rejects_before_deadline() {
        let mut a = make_auction();
        let err = a.close_commits(99).unwrap_err();
        assert_eq!(err, AuctionError::CommitWindowOpen);
    }

    #[test]
    fn close_commits_advances_phase_after_deadline() {
        let mut a = make_auction();
        a.close_commits(100).unwrap();
        assert_eq!(a.phase, AuctionPhase::CommitClosed);
    }

    // ─── Reveal phase ──────────────────────────────────────────────

    #[test]
    fn reveal_bid_matches_committed_hash() {
        let mut a = make_auction();
        let nonce = [0x42; 32];
        let price = 1_500u64;
        let bidder = addr(1);
        let commitment = DecayBoundAuction::compute_commitment(
            &a.chain_id,
            &a.auction_id,
            &bidder,
            price,
            &nonce,
        );
        a.submit_commitment(bidder, commitment).unwrap();
        a.close_commits(100).unwrap();
        a.reveal_bid(bidder, price, nonce).unwrap();
        assert_eq!(a.bids[&bidder].revealed_price, Some(price));
    }

    #[test]
    fn reveal_bid_rejects_mismatched_price() {
        let mut a = make_auction();
        let nonce = [0x42; 32];
        let bidder = addr(1);
        let commitment = DecayBoundAuction::compute_commitment(
            &a.chain_id,
            &a.auction_id,
            &bidder,
            1_500,
            &nonce,
        );
        a.submit_commitment(bidder, commitment).unwrap();
        a.close_commits(100).unwrap();
        let err = a.reveal_bid(bidder, 9_999, nonce).unwrap_err();
        assert_eq!(err, AuctionError::RevealMismatch);
    }

    // ─── Settle ────────────────────────────────────────────────────

    #[test]
    fn settle_picks_highest_bid_above_reserve() {
        let mut a = make_auction();
        let nonce1 = [0x10; 32];
        let nonce2 = [0x20; 32];
        let nonce3 = [0x30; 32];
        let prices = [1_200u64, 2_500u64, 900u64]; // 3rd is below reserve=1_000
        let bidders = [addr(1), addr(2), addr(3)];
        let nonces = [nonce1, nonce2, nonce3];
        for i in 0..3 {
            let c = DecayBoundAuction::compute_commitment(
                &a.chain_id,
                &a.auction_id,
                &bidders[i],
                prices[i],
                &nonces[i],
            );
            a.submit_commitment(bidders[i], c).unwrap();
        }
        a.close_commits(100).unwrap();
        for i in 0..3 {
            a.reveal_bid(bidders[i], prices[i], nonces[i]).unwrap();
        }
        a.close_reveals(200).unwrap();
        let winner = a.settle(201).unwrap();
        assert_eq!(winner, Some(bidders[1]));
        assert_eq!(a.clearing_price, Some(2_500));
        assert_eq!(a.phase, AuctionPhase::Settled);
    }

    #[test]
    fn settle_returns_none_when_no_bid_meets_reserve() {
        let mut a = make_auction();
        let bidder = addr(1);
        let nonce = [0u8; 32];
        let price = 500u64; // below reserve_price=1_000
        let c = DecayBoundAuction::compute_commitment(
            &a.chain_id,
            &a.auction_id,
            &bidder,
            price,
            &nonce,
        );
        a.submit_commitment(bidder, c).unwrap();
        a.close_commits(100).unwrap();
        a.reveal_bid(bidder, price, nonce).unwrap();
        a.close_reveals(200).unwrap();
        let winner = a.settle(201).unwrap();
        assert_eq!(winner, None);
        assert_eq!(a.phase, AuctionPhase::Settled);
    }

    #[test]
    fn settle_ties_broken_by_lex_smaller_address() {
        let mut a = make_auction();
        let nonce = [0u8; 32];
        let price = 5_000u64;
        let b_small = addr(1);
        let b_large = addr(2);
        for bid in [b_small, b_large] {
            let c = DecayBoundAuction::compute_commitment(
                &a.chain_id,
                &a.auction_id,
                &bid,
                price,
                &nonce,
            );
            a.submit_commitment(bid, c).unwrap();
        }
        a.close_commits(100).unwrap();
        a.reveal_bid(b_small, price, nonce).unwrap();
        a.reveal_bid(b_large, price, nonce).unwrap();
        a.close_reveals(200).unwrap();
        let winner = a.settle(201).unwrap();
        assert_eq!(winner, Some(b_small));
    }

    #[test]
    fn settle_evaporates_when_energy_decayed_to_zero() {
        // 100 half-lives past deadline → energy_at_epoch returns 0.
        let mut a = make_auction();
        let nonce = [0u8; 32];
        let price = 5_000u64;
        let bidder = addr(1);
        let c = DecayBoundAuction::compute_commitment(
            &a.chain_id,
            &a.auction_id,
            &bidder,
            price,
            &nonce,
        );
        a.submit_commitment(bidder, c).unwrap();
        a.close_commits(100).unwrap();
        a.reveal_bid(bidder, price, nonce).unwrap();
        a.close_reveals(200).unwrap();
        // Far-future epoch — energy decayed past zero.
        let result = a.settle(10_000);
        match result {
            Err(AuctionError::EvaporatedDuringReveal) => {}
            other => panic!("expected EvaporatedDuringReveal, got {:?}", other),
        }
        assert_eq!(a.phase, AuctionPhase::Evaporated);
    }
}
