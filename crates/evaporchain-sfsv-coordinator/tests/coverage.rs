//! Coverage tests for the SFSV off-chain coordinator (`INVENTION_STACK.md
//! §A5.2`). First reference dApp coordinator — polls the node, clears
//! Dutch auctions off-chain, submits `record_sale` on-chain.
//!
//! Scope: this file covers the **pure-logic** modules (`config` +
//! `auctioneer`). The network-heavy modules (`bid_server`, `poller`,
//! `node`, `main`) are I/O-bound and need a running EvaporChain node;
//! they belong to an end-to-end harness, not the unit-coverage suite.

use evaporchain_sddc::auction::Auction;
use evaporchain_sfsv::predicate::Predicate;
use evaporchain_sfsv::vault::Vault;
use evaporchain_sfsv_coordinator::auctioneer::{Auctioneer, AuctioneerError, BidRequest};
use evaporchain_sfsv_coordinator::config::Config;

// =================================================================
// Config — defaults + env overrides
// =================================================================

#[test]
fn config_default_field_values() {
    let c = Config::default();
    assert_eq!(c.node_url, "http://localhost:8099");
    assert_eq!(c.auth_token, "");
    assert_eq!(c.caller_byte, 0);
    assert_eq!(c.bid_server_port, 9100);
}

#[test]
fn config_from_env_falls_back_to_defaults_when_unset() {
    // Defensively clear any SFSV_* vars that might leak from the shell.
    for k in ["SFSV_NODE_URL", "SFSV_AUTH_TOKEN", "SFSV_CALLER_BYTE", "SFSV_BID_PORT"] {
        // SAFETY: tests run single-threaded by default for cargo test
        // unless --test-threads is set; for this crate the suite is
        // small enough that the default ordering is safe.
        unsafe { std::env::remove_var(k); }
    }
    let c = Config::from_env();
    assert_eq!(c.node_url, "http://localhost:8099");
    assert_eq!(c.auth_token, "");
    assert_eq!(c.caller_byte, 0);
    assert_eq!(c.bid_server_port, 9100);
}

#[test]
fn config_from_env_honours_overrides() {
    // SAFETY: see note in fallback test above.
    unsafe {
        std::env::set_var("SFSV_NODE_URL", "http://remote:1234");
        std::env::set_var("SFSV_AUTH_TOKEN", "tok-abc");
        std::env::set_var("SFSV_CALLER_BYTE", "42");
        std::env::set_var("SFSV_BID_PORT", "8765");
    }
    let c = Config::from_env();
    assert_eq!(c.node_url, "http://remote:1234");
    assert_eq!(c.auth_token, "tok-abc");
    assert_eq!(c.caller_byte, 42);
    assert_eq!(c.bid_server_port, 8765);
    // Cleanup.
    unsafe {
        std::env::remove_var("SFSV_NODE_URL");
        std::env::remove_var("SFSV_AUTH_TOKEN");
        std::env::remove_var("SFSV_CALLER_BYTE");
        std::env::remove_var("SFSV_BID_PORT");
    }
}

#[test]
fn config_from_env_ignores_unparseable_numeric_overrides() {
    unsafe {
        std::env::set_var("SFSV_CALLER_BYTE", "not-a-number");
        std::env::set_var("SFSV_BID_PORT", "🚀");
    }
    let c = Config::from_env();
    // Unparseable → fallback to defaults via `.and_then(|s| s.parse().ok())`.
    assert_eq!(c.caller_byte, 0);
    assert_eq!(c.bid_server_port, 9100);
    unsafe {
        std::env::remove_var("SFSV_CALLER_BYTE");
        std::env::remove_var("SFSV_BID_PORT");
    }
}

// =================================================================
// Auctioneer — registry + bid intake
// =================================================================

fn make_vault() -> Vault {
    Vault::create(
        [0xAB; 32],
        /* creator     */ [0x01; 32],
        /* future_self */ [0x02; 32],
        /* deposit     */ 1_000,
        Predicate::EpochReached { release_epoch: 10_000 },
        /* created_at  */ 0,
    )
    .expect("valid vault")
}

fn make_auction(id_byte: u8) -> Auction {
    Auction::open(
        [id_byte; 32],
        /* ceiling */ 1_000,
        /* floor   */ 100,
        /* λ       */ 0,
        /* opened  */ 0,
        /* dur     */ 1_000,
    )
    .expect("valid auction header")
}

#[test]
fn auctioneer_new_is_empty() {
    let a = Auctioneer::new();
    assert!(!a.is_tracked(7));
}

#[test]
fn register_listing_succeeds_then_double_register_is_rejected() {
    let mut a = Auctioneer::new();
    a.register_listing(7, make_vault(), make_auction(1)).expect("first registration");
    assert!(a.is_tracked(7));
    let err = a
        .register_listing(7, make_vault(), make_auction(2))
        .expect_err("second must fail");
    assert!(matches!(err, AuctioneerError::AlreadyTracked(7)));
}

#[test]
fn drop_listing_removes_tracking() {
    let mut a = Auctioneer::new();
    a.register_listing(7, make_vault(), make_auction(1)).unwrap();
    assert!(a.is_tracked(7));
    a.drop_listing(7);
    assert!(!a.is_tracked(7));
}

#[test]
fn drop_listing_on_unknown_is_no_op() {
    let mut a = Auctioneer::new();
    // Must not panic.
    a.drop_listing(999);
    assert!(!a.is_tracked(999));
}

#[test]
fn submit_bid_for_unknown_listing_errors() {
    let mut a = Auctioneer::new();
    let req = BidRequest {
        contract_id: 99,
        bidder_hex: "01".repeat(32),
        max_price: 500,
        lambda_tolerance: 0,
        submitted_at: 1,
    };
    let err = a.submit_bid(&req).expect_err("must reject unknown listing");
    assert!(matches!(err, AuctioneerError::NotListed(99)));
}

#[test]
fn submit_bid_happy_path_accepts() {
    let mut a = Auctioneer::new();
    a.register_listing(7, make_vault(), make_auction(1)).unwrap();
    let req = BidRequest {
        contract_id: 7,
        bidder_hex: "ab".repeat(32),
        max_price: 600,
        lambda_tolerance: 0,
        submitted_at: 5,
    };
    a.submit_bid(&req).expect("bid accepted");
}

#[test]
fn submit_bid_handles_0x_prefixed_hex() {
    let mut a = Auctioneer::new();
    a.register_listing(7, make_vault(), make_auction(1)).unwrap();
    let prefixed = format!("0x{}", "cd".repeat(32));
    let req = BidRequest {
        contract_id: 7,
        bidder_hex: prefixed,
        max_price: 600,
        lambda_tolerance: 0,
        submitted_at: 5,
    };
    a.submit_bid(&req).expect("0x-prefixed hex accepted");
}

#[test]
fn submit_bid_with_short_hex_uses_zero_padding() {
    // Coordinator's hex_to_addr is defensive: short hex → padded to 32
    // bytes with leading zeros; invalid hex → all zeros. Either way it
    // accepts the bid (the chain enforces ML-DSA later).
    let mut a = Auctioneer::new();
    a.register_listing(7, make_vault(), make_auction(1)).unwrap();
    let req = BidRequest {
        contract_id: 7,
        bidder_hex: "01".to_string(), // only 1 byte of hex
        max_price: 600,
        lambda_tolerance: 0,
        submitted_at: 5,
    };
    a.submit_bid(&req).expect("short hex accepted (padded)");
}

#[test]
fn try_clear_all_on_empty_registry_returns_empty() {
    let mut a = Auctioneer::new();
    let cleared = a.try_clear_all(100);
    assert!(cleared.is_empty());
}

#[test]
fn try_clear_all_with_no_bids_keeps_listing_open() {
    let mut a = Auctioneer::new();
    a.register_listing(7, make_vault(), make_auction(1)).unwrap();
    let cleared = a.try_clear_all(50);
    assert!(cleared.is_empty(), "no bid ⇒ no clear");
    assert!(a.is_tracked(7), "listing remains tracked");
}

#[test]
fn auctioneer_serde_bidrequest_round_trips() {
    let req = BidRequest {
        contract_id: 42,
        bidder_hex: "ab".repeat(32),
        max_price: 1_234,
        lambda_tolerance: 5,
        submitted_at: 99,
    };
    let json = serde_json::to_string(&req).expect("serialize");
    let back: BidRequest = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.contract_id, req.contract_id);
    assert_eq!(back.bidder_hex, req.bidder_hex);
    assert_eq!(back.max_price, req.max_price);
    assert_eq!(back.lambda_tolerance, req.lambda_tolerance);
    assert_eq!(back.submitted_at, req.submitted_at);
}

#[test]
fn register_then_drop_then_register_succeeds_again() {
    let mut a = Auctioneer::new();
    a.register_listing(7, make_vault(), make_auction(1)).unwrap();
    a.drop_listing(7);
    // After drop, re-registering must succeed.
    a.register_listing(7, make_vault(), make_auction(2))
        .expect("re-registration after drop");
    assert!(a.is_tracked(7));
}
