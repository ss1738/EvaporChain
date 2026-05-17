//! End-to-end integration tests for evaporchain-bell-beacon-v2.
//!
//! Non-trivial fixture: three-window chain certification session.
//!
//! A proposer issues BellCertificates over three consecutive block
//! windows on "evaporchain-mainnet". Each window's cert uses the
//! previous cert's seed as `prev_block_hash`, creating a cryptographic
//! chain: genesis_prev → cert1.seed → cert2.seed → cert3.seed.
//!
//!   Window 1: heights [100, 200)  prev = [0x11;32]  (genesis anchor)
//!   Window 2: heights [200, 300)  prev = cert1.seed
//!   Window 3: heights [300, 400)  prev = cert2.seed
//!
//!   All three windows pass the doctrine CHSH gate (balanced 16-pair
//!   samples, 4 pairs per setting-pair bucket).
//!
//! Doctrine claim (INVENTION_STACK §4.1 #3 / V2 upgrade): "Bell-Certified
//! Beacon V2 binds CHSH-violation gating to the chain's block history.
//! The certificate seed = BLAKE3(domain || chain_id || gate_stats ||
//! sorted_pair_tags) is anchored to prev_block_hash, so a proposer
//! cannot choose the seed by reordering pairs or by pre-computing the
//! gate values before the window closes. Two validators who observe the
//! same window derive byte-identical certificates — cross-validator BFT
//! agreement on the beacon seed is guaranteed by construction."
//!
//! Adversarial fixture: cross-chain replay (testnet cert on mainnet),
//! tampered prev_block_hash, tampered seed, insufficient-coverage window,
//! empty window, pair-reordering invariance (anti-grinding).
//!
//! INVENTION_STACK §4.1 #3: Bell-Certified Beacon V2.

use evaporchain_bell_beacon_v2::{
    issue_certificate, verify_certificate,
    BellCertificate, BeaconError, ConcurrentPair, PairStats, VerifyError,
};
use evaporchain_causal_chsh::gate::GateThresholds;

// ── Helpers ───────────────────────────────────────────────────────────────

/// Build 16 concurrent pairs balanced across all 4 CHSH buckets.
/// Each pair's tag[31] & 0b11 routes it to one of 4 buckets (4 pairs each).
/// tag[0] = i makes all 16 tags distinct (important for seed derivation).
fn balanced_pairs() -> Vec<ConcurrentPair> {
    (0u8..16).map(|i| {
        let mut tag = [0u8; 32];
        tag[31] = i & 0b11;  // bucket assignment
        tag[0]  = i;         // uniqueness
        ConcurrentPair {
            first: PairStats {
                energy:   if i & 1 == 1 { 1_000 } else { 100 },
                tx_count: if (i >> 1) & 1 == 1 { 1_000 } else { 100 },
            },
            second: PairStats {
                energy:   if (i >> 2) & 1 == 1 { 1_000 } else { 100 },
                tx_count: if (i >> 3) & 1 == 1 { 1_000 } else { 100 },
            },
            tag,
        }
    }).collect()
}

const CHAIN_MAINNET: &str = "evaporchain-mainnet";
const CHAIN_TESTNET: &str = "evaporchain-testnet";
const GENESIS_PREV:  [u8; 32] = [0x11u8; 32];

fn thresholds() -> GateThresholds { GateThresholds::doctrine() }

/// Issue a cert for `chain_id` over the balanced window, returning it or panicking.
fn issue(chain_id: &str, w_start: u64, w_end: u64, prev: [u8; 32]) -> BellCertificate {
    issue_certificate(chain_id, w_start, w_end, &balanced_pairs(), thresholds(), prev).unwrap()
}

// ── Non-trivial fixture: three-window chain certification session ──────────

#[test]
fn window1_cert_passes_gate_and_verifies() {
    // Window 1 is anchored to the genesis prev block hash.
    let cert = issue(CHAIN_MAINNET, 100, 200, GENESIS_PREV);

    assert_eq!(cert.window_start,   100);
    assert_eq!(cert.window_end,     200);
    assert_eq!(cert.n_pairs,         16);
    assert_eq!(cert.bucket_counts.iter().sum::<u64>(), 16);
    assert_ne!(cert.seed, [0u8; 32], "seed must be non-trivial");
    assert_eq!(cert.prev_block_hash, GENESIS_PREV);

    // Cartel S > honest S — CHSH discrimination holds.
    assert!(cert.s_cartel_milli > cert.s_honest_milli,
        "cartel S must exceed honest S for the gate to pass");
    assert!(cert.gap_milli > 0, "gap must be positive");

    verify_certificate(CHAIN_MAINNET, &balanced_pairs(), GENESIS_PREV, thresholds(), &cert).unwrap();
}

#[test]
fn three_window_chain_seeds_are_distinct() {
    // Each cert uses the previous cert's seed as its prev_block_hash.
    // The chain of anchors means all three seeds must be distinct.
    let cert1 = issue(CHAIN_MAINNET, 100, 200, GENESIS_PREV);
    let cert2 = issue(CHAIN_MAINNET, 200, 300, cert1.seed);
    let cert3 = issue(CHAIN_MAINNET, 300, 400, cert2.seed);

    assert_ne!(cert1.seed, cert2.seed, "cert1 and cert2 seeds must differ (different prevs)");
    assert_ne!(cert2.seed, cert3.seed, "cert2 and cert3 seeds must differ");
    assert_ne!(cert1.seed, cert3.seed, "cert1 and cert3 seeds must differ");
}

#[test]
fn three_window_chain_all_verify() {
    // All three certs in the chain pass independent verification.
    let cert1 = issue(CHAIN_MAINNET, 100, 200, GENESIS_PREV);
    let cert2 = issue(CHAIN_MAINNET, 200, 300, cert1.seed);
    let cert3 = issue(CHAIN_MAINNET, 300, 400, cert2.seed);
    let pairs = balanced_pairs();

    verify_certificate(CHAIN_MAINNET, &pairs, GENESIS_PREV,  thresholds(), &cert1).unwrap();
    verify_certificate(CHAIN_MAINNET, &pairs, cert1.seed,    thresholds(), &cert2).unwrap();
    verify_certificate(CHAIN_MAINNET, &pairs, cert2.seed,    thresholds(), &cert3).unwrap();
}

#[test]
fn doctrine_different_prev_hashes_produce_different_seeds() {
    // INVENTION_STACK §4.1 doctrine: "The seed is anchored to
    // prev_block_hash — a proposer cannot pre-compute the beacon
    // seed before the previous block is finalized."
    let cert_a = issue(CHAIN_MAINNET, 100, 200, [0x01u8; 32]);
    let cert_b = issue(CHAIN_MAINNET, 100, 200, [0x02u8; 32]);

    // Same chain, same window, same pairs — ONLY prev differs.
    assert_ne!(cert_a.seed, cert_b.seed,
        "different prev_block_hash must yield different seeds");
    // Gate stats (S values) must be identical — they depend only on pairs.
    assert_eq!(cert_a.s_honest_milli, cert_b.s_honest_milli,
        "honest S must be identical for same pairs");
    assert_eq!(cert_a.bucket_counts, cert_b.bucket_counts,
        "bucket distribution must be identical for same pairs");
}

#[test]
fn doctrine_pair_reordering_does_not_change_seed() {
    // INVENTION_STACK §4.1 doctrine: "Anti-grinding — pair reordering
    // cannot change the seed because tags are sorted before hashing."
    let pairs = balanced_pairs();
    let cert_fwd = issue_certificate(CHAIN_MAINNET, 100, 200, &pairs, thresholds(), GENESIS_PREV).unwrap();

    let mut pairs_rev = pairs.clone();
    pairs_rev.reverse();
    let cert_rev = issue_certificate(CHAIN_MAINNET, 100, 200, &pairs_rev, thresholds(), GENESIS_PREV).unwrap();

    assert_eq!(cert_fwd.seed, cert_rev.seed,
        "seed must be invariant under pair reordering (sorted tags)");
}

#[test]
fn doctrine_bft_determinism_same_inputs_yield_identical_certs() {
    // Two BFT validators observing the same window must produce
    // byte-identical certs — required for cross-validator agreement.
    let cert_v1 = issue(CHAIN_MAINNET, 100, 200, GENESIS_PREV);
    let cert_v2 = issue(CHAIN_MAINNET, 100, 200, GENESIS_PREV);
    assert_eq!(cert_v1, cert_v2, "same inputs must produce byte-identical certificates");
}

#[test]
fn chain_id_isolation_mainnet_vs_testnet() {
    // INVENTION_STACK §4.1 doctrine H2 (chain-id binding): two chains
    // sharing the same window and prev_block_hash produce different seeds.
    let cert_main = issue(CHAIN_MAINNET, 100, 200, GENESIS_PREV);
    let cert_test = issue(CHAIN_TESTNET, 100, 200, GENESIS_PREV);

    assert_ne!(cert_main.seed, cert_test.seed,
        "mainnet and testnet seeds must differ (chain_id is bound into seed)");
    // Gate values are chain-independent for the same pair set.
    assert_eq!(cert_main.s_honest_milli, cert_test.s_honest_milli,
        "honest S is chain-independent");
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_cross_chain_replay_rejected() {
    // A cert issued for CHAIN_TESTNET cannot verify against CHAIN_MAINNET.
    // The chain_id is bound into the seed — seed mismatch catches the replay.
    let pairs = balanced_pairs();
    let testnet_cert = issue_certificate(
        CHAIN_TESTNET, 100, 200, &pairs, thresholds(), GENESIS_PREV,
    ).unwrap();

    let err = verify_certificate(
        CHAIN_MAINNET, &pairs, GENESIS_PREV, thresholds(), &testnet_cert,
    ).unwrap_err();
    assert_eq!(err, VerifyError::SeedMismatch,
        "testnet cert must not verify against mainnet chain_id");
}

#[test]
fn adversarial_tampered_seed_rejected() {
    let pairs = balanced_pairs();
    let mut cert = issue(CHAIN_MAINNET, 100, 200, GENESIS_PREV);
    cert.seed[0] ^= 0xff;

    let err = verify_certificate(CHAIN_MAINNET, &pairs, GENESIS_PREV, thresholds(), &cert).unwrap_err();
    assert_eq!(err, VerifyError::SeedMismatch, "tampered seed must be rejected");
}

#[test]
fn adversarial_tampered_prev_hash_rejected() {
    let pairs = balanced_pairs();
    let cert = issue(CHAIN_MAINNET, 100, 200, GENESIS_PREV);

    let err = verify_certificate(
        CHAIN_MAINNET, &pairs, [0xFFu8; 32], thresholds(), &cert,
    ).unwrap_err();
    assert_eq!(err, VerifyError::PrevHashMismatch,
        "mismatched prev_block_hash must be rejected");
}

#[test]
fn adversarial_tampered_s_honest_rejected() {
    let pairs = balanced_pairs();
    let mut cert = issue(CHAIN_MAINNET, 100, 200, GENESIS_PREV);
    cert.s_honest_milli += 100;

    let err = verify_certificate(CHAIN_MAINNET, &pairs, GENESIS_PREV, thresholds(), &cert).unwrap_err();
    assert!(matches!(err, VerifyError::SHonestMismatch { .. }),
        "tampered s_honest_milli must be rejected");
}

#[test]
fn adversarial_empty_window_rejected_at_issuance() {
    let err = issue_certificate(
        CHAIN_MAINNET, 100, 200, &[], thresholds(), GENESIS_PREV,
    ).unwrap_err();
    assert_eq!(err, BeaconError::EmptyWindow, "empty window must fail at issuance");
}

#[test]
fn adversarial_single_bucket_window_rejected() {
    // All pairs with tag[31] & 0b11 == 0 → only bucket Ab is populated.
    // build_samples returns None → InsufficientCoverage.
    let single_bucket: Vec<ConcurrentPair> = (0u8..8).map(|i| {
        let mut tag = [0u8; 32];
        tag[31] = 0; // all to bucket Ab
        tag[0]  = i;
        ConcurrentPair {
            first:  PairStats { energy: 500, tx_count: 500 },
            second: PairStats { energy: 500, tx_count: 500 },
            tag,
        }
    }).collect();

    let err = issue_certificate(
        CHAIN_MAINNET, 100, 200, &single_bucket, thresholds(), GENESIS_PREV,
    ).unwrap_err();
    assert_eq!(err, BeaconError::InsufficientCoverage,
        "window without all 4 buckets must be rejected");
}

#[test]
fn adversarial_invalid_window_range_rejected() {
    let err = issue_certificate(
        CHAIN_MAINNET, 200, 100, &balanced_pairs(), thresholds(), GENESIS_PREV,
    ).unwrap_err();
    assert!(matches!(err, BeaconError::InvalidWindowRange { start: 200, end: 100 }),
        "window_end <= window_start must be rejected");
}

#[test]
fn adversarial_prev_hash_in_chain_linkage_rejected() {
    // cert2 was issued using cert1.seed as prev. Verifying cert2 against
    // a wrong prev (e.g., GENESIS_PREV instead of cert1.seed) must reject.
    let cert1 = issue(CHAIN_MAINNET, 100, 200, GENESIS_PREV);
    let cert2 = issue(CHAIN_MAINNET, 200, 300, cert1.seed);
    let pairs = balanced_pairs();

    // Correct: cert1.seed as prev.
    verify_certificate(CHAIN_MAINNET, &pairs, cert1.seed, thresholds(), &cert2).unwrap();

    // Wrong prev (genesis instead of cert1.seed) → rejected.
    let err = verify_certificate(CHAIN_MAINNET, &pairs, GENESIS_PREV, thresholds(), &cert2).unwrap_err();
    assert_eq!(err, VerifyError::PrevHashMismatch,
        "wrong prev in chain linkage must reject cert2");
}
