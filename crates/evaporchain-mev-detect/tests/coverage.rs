//! Coverage tests for Crooks-MEV sandwich detector.

use evaporchain_mev_detect::{
    compute_observation_refund, mev_state_digest, scan_block, AttackerStat, MevObservation,
    CROOKS_MEV_DEFAULT_BETA_MB, CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
    CROOKS_MEV_DEFAULT_GRACE_PERIOD_BLOCKS, CROOKS_MEV_DEFAULT_REFUND_WINDOW_BLOCKS,
    CROOKS_MEV_DEFAULT_WINDOW_BLOCKS,
};
use evaporchain_types::{AccountAddress, Transaction, TransferTx};
use std::collections::{HashMap, VecDeque};

fn addr(seed: u8) -> AccountAddress {
    let mut a = [0u8; 32];
    a[0] = seed;
    a
}

fn transfer(from: u8, to: u8, amount: u64, nonce: u64) -> Transaction {
    Transaction::Transfer(TransferTx {
        from: addr(from),
        to: addr(to),
        amount,
        nonce,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    })
}

fn obs(block_height: u64) -> MevObservation {
    MevObservation {
        block_height,
        attacker_pre_idx: 0,
        victim_idx: 1,
        attacker_post_idx: 2,
        attacker: addr(3),
        victim: addr(4),
        target: addr(5),
        work_estimate: 200,
        confidence_score_ppm: 1_000_000,
        refund_amount: None,
    }
}

// =================================================================
// Doctrine constants
// =================================================================

#[test]
fn doctrine_constants_pin_default_values() {
    assert_eq!(CROOKS_MEV_DEFAULT_BETA_MB, 1000);
    assert_eq!(CROOKS_MEV_DEFAULT_WINDOW_BLOCKS, 256);
    assert_eq!(CROOKS_MEV_DEFAULT_GRACE_PERIOD_BLOCKS, 5);
    assert_eq!(CROOKS_MEV_DEFAULT_REFUND_WINDOW_BLOCKS, 256);
    assert_eq!(CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM, 500_000);
}

// =================================================================
// scan_block
// =================================================================

#[test]
fn scan_empty_block_yields_no_observations() {
    let out = scan_block(&[], 0);
    assert!(out.is_empty());
}

#[test]
fn scan_block_with_two_txs_no_sandwich() {
    let txs = vec![transfer(1, 2, 100, 0), transfer(2, 1, 100, 0)];
    assert!(scan_block(&txs, 1).is_empty());
}

#[test]
fn scan_block_detects_simple_sandwich() {
    let txs = vec![
        transfer(3, 5, 100, 0),
        transfer(4, 5, 200, 0),
        transfer(3, 5, 100, 1),
    ];
    let out = scan_block(&txs, 10);
    assert_eq!(out.len(), 1);
    let o = &out[0];
    assert_eq!(o.attacker_pre_idx, 0);
    assert_eq!(o.victim_idx, 1);
    assert_eq!(o.attacker_post_idx, 2);
    assert_eq!(o.attacker, addr(3));
    assert_eq!(o.victim, addr(4));
    assert_eq!(o.target, addr(5));
    assert_eq!(o.block_height, 10);
    assert_eq!(o.confidence_score_ppm, 1_000_000);
    assert!(o.refund_amount.is_none());
}

#[test]
fn scan_block_no_sandwich_when_victim_targets_different_account() {
    let txs = vec![
        transfer(3, 5, 100, 0),
        transfer(4, 6, 200, 0),
        transfer(3, 5, 100, 1),
    ];
    assert!(scan_block(&txs, 1).is_empty());
}

#[test]
fn scan_block_no_sandwich_when_attacker_addresses_differ() {
    let txs = vec![
        transfer(3, 5, 100, 0),
        transfer(4, 5, 200, 0),
        transfer(7, 5, 100, 1),
    ];
    assert!(scan_block(&txs, 1).is_empty());
}

#[test]
fn scan_block_stamps_block_height() {
    let txs = vec![
        transfer(3, 5, 100, 0),
        transfer(4, 5, 200, 0),
        transfer(3, 5, 100, 1),
    ];
    let out = scan_block(&txs, 9_999);
    assert_eq!(out[0].block_height, 9_999);
}

// =================================================================
// compute_observation_refund
// =================================================================

#[test]
fn compute_observation_refund_zero_beta_returns_none() {
    let o = obs(1);
    let stat = AttackerStat {
        sandwich_count: 10,
        first_seen_height: 0,
        last_seen_height: 10,
    };
    assert!(compute_observation_refund(&o, &stat, 0, 256).is_none());
}

#[test]
fn compute_observation_refund_zero_window_returns_none() {
    let o = obs(1);
    let stat = AttackerStat {
        sandwich_count: 10,
        first_seen_height: 0,
        last_seen_height: 10,
    };
    assert!(compute_observation_refund(&o, &stat, 1000, 0).is_none());
}

#[test]
fn compute_observation_refund_returns_some_on_valid_inputs() {
    let o = obs(1);
    let stat = AttackerStat {
        sandwich_count: 10,
        first_seen_height: 0,
        last_seen_height: 10,
    };
    let r = compute_observation_refund(
        &o,
        &stat,
        CROOKS_MEV_DEFAULT_BETA_MB,
        CROOKS_MEV_DEFAULT_WINDOW_BLOCKS,
    );
    // Result may be Some(0) (negative refund clamped) or Some(>0); both are valid.
    assert!(r.is_some());
}

// =================================================================
// AttackerStat
// =================================================================

#[test]
fn attacker_stat_struct_constructs_and_eq() {
    let a = AttackerStat {
        sandwich_count: 0,
        first_seen_height: 0,
        last_seen_height: 0,
    };
    let b = AttackerStat {
        sandwich_count: 0,
        first_seen_height: 0,
        last_seen_height: 0,
    };
    let c = AttackerStat {
        sandwich_count: 1,
        first_seen_height: 0,
        last_seen_height: 0,
    };
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// =================================================================
// mev_state_digest
// =================================================================

#[test]
fn mev_state_digest_deterministic_on_empty() {
    let obs_empty: VecDeque<MevObservation> = VecDeque::new();
    let stats_empty: HashMap<AccountAddress, AttackerStat> = HashMap::new();
    let d1 = mev_state_digest(&obs_empty, &stats_empty);
    let d2 = mev_state_digest(&obs_empty, &stats_empty);
    assert_eq!(d1, d2);
}

#[test]
fn mev_state_digest_differs_when_observation_height_changes() {
    let mut q1: VecDeque<MevObservation> = VecDeque::new();
    q1.push_back(obs(1));
    let mut q2: VecDeque<MevObservation> = VecDeque::new();
    q2.push_back(obs(2));
    let stats: HashMap<AccountAddress, AttackerStat> = HashMap::new();
    assert_ne!(mev_state_digest(&q1, &stats), mev_state_digest(&q2, &stats));
}

#[test]
fn mev_state_digest_differs_when_attacker_stat_changes() {
    let q: VecDeque<MevObservation> = VecDeque::new();
    let mut s1 = HashMap::new();
    s1.insert(
        addr(3),
        AttackerStat {
            sandwich_count: 1,
            first_seen_height: 0,
            last_seen_height: 1,
        },
    );
    let mut s2 = HashMap::new();
    s2.insert(
        addr(3),
        AttackerStat {
            sandwich_count: 2,
            first_seen_height: 0,
            last_seen_height: 1,
        },
    );
    assert_ne!(mev_state_digest(&q, &s1), mev_state_digest(&q, &s2));
}
