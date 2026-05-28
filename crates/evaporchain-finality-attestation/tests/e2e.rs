//! End-to-end integration tests for evaporchain-finality-attestation.
//!
//! Non-trivial fixture: a 5-block finality chain.
//!
//! The chain coordinator finalises blocks 0–4. Each block extends the
//! Light-Cone causal root, some blocks switch Bell-Beacon seeds (epoch
//! lottery), and the set of evaporated-fork witnesses grows as
//! competing forks decay below threshold.
//!
//! Block 0 — genesis, no forks yet.
//! Block 1 — fork A evaporates; first witness issued.
//! Block 2 — fork B evaporates; two witnesses now in the list.
//! Block 3 — new Bell-Beacon seed (lottery fires); fork C evaporates.
//! Block 4 — canonical tip finalised; fork D evaporates; four witnesses.
//!
//! Key invariants exercised:
//!   - All 5 attestation roots are strictly unique.
//!   - Adding each new fork changes the root (Merkle commitment).
//!   - A light client that holds only `(block_hash, attestation,
//!     attestation_root)` can verify finality in O(1).
//!   - Replacing a block's `causal_root` with an earlier block's root
//!     is rejected (cross-block tampering).
//!   - Rolling back to an older `bell_seed` is rejected.
//!   - Injecting an extra fork witness into the canonical list diverges
//!     the root and is rejected.
//!   - Presenting the fork list in unsorted order is rejected at build
//!     time — no root is emitted.
//!
//! Doctrine claim (INVENTION_STACK §4.1 row 1 + §4.1 row 10 +
//! §4.2 Bell-Certified Beacon):
//!   "A single 32-byte attestation root folds Light-Cone V2
//!   causal_root, Bell-Beacon V2 bell_seed, and Evap-Fork-Cert V2
//!   witness list.  Any single-bit mutation in any field is detected."

use evaporchain_finality_attestation::{
    build_attestation, verify_attestation, AttestationError, EvaporatedForkWitnessRef,
    FinalityAttestation,
};

// ── Fixture helpers ──────────────────────────────────────────────────────────

fn fwr(root_byte: u8, witness_byte: u8) -> EvaporatedForkWitnessRef {
    let mut fork_root = [0u8; 32];
    fork_root[0] = root_byte;
    let mut witness = [0u8; 32];
    witness[0] = witness_byte;
    EvaporatedForkWitnessRef { fork_root, witness }
}

struct BlockInputs {
    block_hash: [u8; 32],
    finalised_at_epoch: u64,
    causal_root: [u8; 32],
    bell_seed: [u8; 32],
    evaporated_forks: Vec<EvaporatedForkWitnessRef>,
}

impl BlockInputs {
    fn attestation(&self) -> FinalityAttestation {
        FinalityAttestation {
            block_hash: self.block_hash,
            finalised_at_epoch: self.finalised_at_epoch,
            causal_root: self.causal_root,
            bell_seed: self.bell_seed,
            evaporated_forks: self.evaporated_forks.clone(),
        }
    }
}

fn mk_hash(b: u8) -> [u8; 32] {
    let mut h = [0u8; 32];
    h[0] = b;
    h
}

fn five_block_chain() -> Vec<BlockInputs> {
    let seed_v1 = mk_hash(0xBE);
    let seed_v2 = mk_hash(0xC0);

    vec![
        BlockInputs {
            block_hash: mk_hash(0x01),
            finalised_at_epoch: 0,
            causal_root: mk_hash(0x11),
            bell_seed: seed_v1,
            evaporated_forks: vec![],
        },
        BlockInputs {
            block_hash: mk_hash(0x02),
            finalised_at_epoch: 1,
            causal_root: mk_hash(0x12),
            bell_seed: seed_v1,
            evaporated_forks: vec![fwr(0x10, 0xAA)],
        },
        BlockInputs {
            block_hash: mk_hash(0x03),
            finalised_at_epoch: 2,
            causal_root: mk_hash(0x13),
            bell_seed: seed_v1,
            evaporated_forks: vec![fwr(0x10, 0xAA), fwr(0x20, 0xBB)],
        },
        BlockInputs {
            block_hash: mk_hash(0x04),
            finalised_at_epoch: 3,
            causal_root: mk_hash(0x14),
            bell_seed: seed_v2,
            evaporated_forks: vec![fwr(0x10, 0xAA), fwr(0x20, 0xBB), fwr(0x30, 0xCC)],
        },
        BlockInputs {
            block_hash: mk_hash(0x05),
            finalised_at_epoch: 4,
            causal_root: mk_hash(0x15),
            bell_seed: seed_v2,
            evaporated_forks: vec![
                fwr(0x10, 0xAA),
                fwr(0x20, 0xBB),
                fwr(0x30, 0xCC),
                fwr(0x40, 0xDD),
            ],
        },
    ]
}

// ── Happy-path: full chain lifecycle ────────────────────────────────────────

#[test]
fn full_5_block_chain_all_blocks_verify() {
    let chain = five_block_chain();
    for block in &chain {
        let att = block.attestation();
        let root = build_attestation(&att).unwrap();
        verify_attestation(&att, &root).unwrap();
    }
}

// ── Uniqueness ───────────────────────────────────────────────────────────────

#[test]
fn attestation_roots_are_strictly_unique_across_blocks() {
    let chain = five_block_chain();
    let roots: Vec<[u8; 32]> = chain
        .iter()
        .map(|b| build_attestation(&b.attestation()).unwrap())
        .collect();

    for i in 0..roots.len() {
        for j in (i + 1)..roots.len() {
            assert_ne!(
                roots[i], roots[j],
                "blocks {i} and {j} produced identical attestation roots"
            );
        }
    }
}

// ── Fork-set growth changes root ─────────────────────────────────────────────

#[test]
fn adding_each_fork_changes_attestation_root() {
    let chain = five_block_chain();
    let mut base = chain[4].attestation();
    base.evaporated_forks.pop();
    let root_3 = build_attestation(&base).unwrap();
    let root_4 = build_attestation(&chain[4].attestation()).unwrap();
    assert_ne!(
        root_3, root_4,
        "adding fork_D must change the attestation root"
    );
}

// ── Light-client scenario ─────────────────────────────────────────────────────

#[test]
fn light_client_verifies_block_4_from_attestation_root_alone() {
    let block4 = &five_block_chain()[4];
    let att = block4.attestation();
    let root = build_attestation(&att).unwrap();
    verify_attestation(&att, &root).expect("light-client verification must succeed");
}

// ── Adversarial: cross-block field swaps ────────────────────────────────────

#[test]
fn adversarial_causal_root_from_earlier_block_rejected() {
    let chain = five_block_chain();
    let att = chain[4].attestation();
    let root = build_attestation(&att).unwrap();
    let mut forged = att.clone();
    forged.causal_root = chain[2].causal_root;
    assert!(
        verify_attestation(&forged, &root).is_err(),
        "substituting an older causal_root must invalidate the attestation"
    );
}

#[test]
fn adversarial_bell_seed_rollback_rejected() {
    let chain = five_block_chain();
    let att = chain[4].attestation();
    let root = build_attestation(&att).unwrap();
    let mut forged = att.clone();
    forged.bell_seed = chain[1].bell_seed;
    assert!(
        verify_attestation(&forged, &root).is_err(),
        "rolling back the bell_seed must invalidate the attestation"
    );
}

#[test]
fn adversarial_block_hash_swap_rejected() {
    let chain = five_block_chain();
    let att = chain[4].attestation();
    let root = build_attestation(&att).unwrap();
    let mut forged = att.clone();
    forged.block_hash = chain[3].block_hash;
    assert!(verify_attestation(&forged, &root).is_err());
}

// ── Adversarial: fork-list tampering ────────────────────────────────────────

#[test]
fn adversarial_extra_fork_witness_injection_diverges_root() {
    let chain = five_block_chain();
    let att = chain[4].attestation();
    let root = build_attestation(&att).unwrap();
    let mut forged = att.clone();
    forged.evaporated_forks.push(fwr(0x50, 0xEE));
    assert!(
        verify_attestation(&forged, &root).is_err(),
        "injecting a nonexistent fork must diverge the root"
    );
}

#[test]
fn adversarial_unsorted_fork_list_rejected_at_build() {
    let chain = five_block_chain();
    let mut att = chain[4].attestation();
    att.evaporated_forks.reverse();
    let err = build_attestation(&att).unwrap_err();
    assert_eq!(
        err,
        AttestationError::UnsortedForks,
        "reversed fork list must be caught at build time"
    );
}

#[test]
fn adversarial_single_witness_bit_flip_rejected() {
    let chain = five_block_chain();
    let att = chain[4].attestation();
    let root = build_attestation(&att).unwrap();
    let mut forged = att.clone();
    forged.evaporated_forks[2].witness[0] ^= 0x01;
    assert!(
        verify_attestation(&forged, &root).is_err(),
        "a 1-bit witness mutation must invalidate the attestation"
    );
}

// ── Idempotency ──────────────────────────────────────────────────────────────

#[test]
fn build_is_deterministic_across_multiple_calls() {
    let chain = five_block_chain();
    let att = chain[4].attestation();
    let r1 = build_attestation(&att).unwrap();
    let r2 = build_attestation(&att).unwrap();
    assert_eq!(r1, r2, "build_attestation must be pure / deterministic");
}
