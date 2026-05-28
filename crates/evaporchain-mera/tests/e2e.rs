//! §MERA — research artefact e2e (§A1.8 / §A4.4 gate FAILED 2026-05-03)
//!
//! Scenario: "MERA gate post-mortem" — MERA's tensor-network state
//! commitment was gated on an empirical entropy measurement of real
//! Ethereum data (R² ≥ 0.85 threshold). The gate FAILED across three
//! independent angles (all R² 0.66-0.71). EvaporChain ships
//! Energy-Verkle Trie instead. This crate is retained as a research
//! artefact.
//!
//! The suite proves: the mathematics are correct (per-account proofs
//! round-trip against the commitment); tampering with energy or swapping
//! account indices causes verification to fail; the commitment is
//! deterministic; the gate-failure result is pinned as a named constant
//! to prevent re-litigation.

use evaporchain_mera::{commit, verify_account};

const KNOWN_MAX_R2_ON_REAL_ETHEREUM: f64 = 0.7112;
const DOCTRINE_THRESHOLD_R2: f64 = 0.85;

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn gate_failed_below_threshold() {
    // Pins the empirical result so no future session silently re-enables MERA.
    assert!(
        KNOWN_MAX_R2_ON_REAL_ETHEREUM < DOCTRINE_THRESHOLD_R2,
        "MERA gate must remain FAILED: best R²={KNOWN_MAX_R2_ON_REAL_ETHEREUM} < threshold={DOCTRINE_THRESHOLD_R2}"
    );
}

#[test]
fn honest_proof_verifies_for_all_accounts() {
    let energies: Vec<u64> = (0..16u64).map(|i| 1_000 + i * 100).collect();
    let (commitment, tree) = commit(&energies, 100, 50);

    for (i, &e) in energies.iter().enumerate() {
        let proof = evaporchain_mera::proof::MeraProof::generate(&tree, i);
        verify_account(i, e, &proof, &commitment)
            .unwrap_or_else(|err| panic!("account {i} must verify: {:?}", err));
    }
}

#[test]
fn tampered_energy_rejected() {
    let energies: Vec<u64> = (0..8u64).map(|i| 2_000 + i * 50).collect();
    let (commitment, tree) = commit(&energies, 100, 50);

    let i = 3;
    let proof = evaporchain_mera::proof::MeraProof::generate(&tree, i);
    let result = verify_account(i, energies[i] + 1, &proof, &commitment);
    assert!(result.is_err(), "tampered energy must be rejected");
}

#[test]
fn wrong_account_index_with_other_proof_rejected() {
    let energies: Vec<u64> = (0..8u64).map(|i| 3_000 + i * 200).collect();
    let (commitment, tree) = commit(&energies, 100, 50);

    let proof_for_5 = evaporchain_mera::proof::MeraProof::generate(&tree, 5);
    // Verify account 3 using account 5's proof → must fail.
    let result = verify_account(3, energies[3], &proof_for_5, &commitment);
    assert!(result.is_err(), "swapped proof must be rejected");
}

#[test]
fn commitment_is_deterministic() {
    let energies: Vec<u64> = (0..16u64).map(|i| 500 + i * 100).collect();
    let (c1, _) = commit(&energies, 100, 50);
    let (c2, _) = commit(&energies, 100, 50);
    assert_eq!(
        c1.root_hash, c2.root_hash,
        "commitment must be deterministic"
    );
}

#[test]
fn different_energies_produce_different_commitments() {
    let e1: Vec<u64> = vec![1_000; 8];
    let e2: Vec<u64> = vec![2_000; 8];
    let (c1, _) = commit(&e1, 100, 50);
    let (c2, _) = commit(&e2, 100, 50);
    assert_ne!(
        c1.root_hash, c2.root_hash,
        "different energies must produce different roots"
    );
}

#[test]
fn different_lambda_produces_different_commitment() {
    let energies: Vec<u64> = vec![1_000; 8];
    let (c_fast, _) = commit(&energies, 50, 25);
    let (c_slow, _) = commit(&energies, 200, 100);
    assert_ne!(
        c_fast.root_hash, c_slow.root_hash,
        "different λ must produce different commitments"
    );
}

#[test]
fn single_account_proof_round_trips() {
    let energies = vec![42_000u64];
    let (commitment, tree) = commit(&energies, 100, 50);
    let proof = evaporchain_mera::proof::MeraProof::generate(&tree, 0);
    verify_account(0, 42_000, &proof, &commitment).expect("single-account proof must verify");
}

#[test]
fn proof_for_first_and_last_account_verify() {
    let energies: Vec<u64> = (0..16u64).map(|i| 100 * (i + 1)).collect();
    let (commitment, tree) = commit(&energies, 100, 50);

    let first_proof = evaporchain_mera::proof::MeraProof::generate(&tree, 0);
    verify_account(0, energies[0], &first_proof, &commitment)
        .expect("first account proof must verify");

    let last_proof = evaporchain_mera::proof::MeraProof::generate(&tree, 15);
    verify_account(15, energies[15], &last_proof, &commitment)
        .expect("last account proof must verify");
}

#[test]
fn mera_gate_postmortem_full_arc() {
    // Full arc: build a MERA tree, run per-account verifications, then
    // confirm the tamper detection works. This is what a future
    // chain would do if it ever ran this crate — it's correct math,
    // just not the right fit for Ethereum-like workloads.
    let energies: Vec<u64> = (1u64..=32).map(|i| i * 1_000).collect();
    let (commitment, tree) = commit(&energies, 100, 50);

    // All honest proofs verify.
    let mut verified = 0;
    for (i, &e) in energies.iter().enumerate() {
        let proof = evaporchain_mera::proof::MeraProof::generate(&tree, i);
        if verify_account(i, e, &proof, &commitment).is_ok() {
            verified += 1;
        }
    }
    assert_eq!(verified, energies.len(), "all account proofs must verify");

    // Gate-failure is documented and stable.
    assert!(
        KNOWN_MAX_R2_ON_REAL_ETHEREUM < DOCTRINE_THRESHOLD_R2,
        "gate result must remain FAILED"
    );
}
