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

/// Synthetic block-level inputs — what the consensus layer would feed into
/// the attestation builder.
struct BlockInputs {
    block_hash:         [u8; 32],
    finalised_at_epoch: u64,
    /// Light-Cone V2 output for this block.
    causal_root:        [u8; 32],
    /// Bell-Beacon V2 seed valid at this epoch.
    bell_seed:          [u8; 32],
    /// Sorted, deduplicated Evap-Fork-Cert V2 witnesses accumulated so far.
    evaporated_forks:   Vec<EvaporatedForkWitnessRef>,
}

impl BlockInputs {
    fn attestation(&self) -> FinalityAttestation {
        FinalityAttestation {
            block_hash:         self.block_hash,
            finalised_at_epoch: self.finalised_at_epoch,
            causal_root:        self.causal_root,
            bell_seed:          self.bell_seed,
            evaporated_forks:   self.evaporated_forks.clone(),
        }
    }
}

fn mk_hash(b: u8) -> [u8; 32] {
    let mut h = [0u8; 32];
    h[0] = b;
    h
}

/// Build the canonical 5-block fixture. Fork_root bytes are chosen so that
/// fork_A (0x10) < fork_B (0x20) < fork_C (0x30) < fork_D (0x40) — the
/// required sorted order.
fn five_block_chain() -> Vec<BlockInputs> {
    // Beacon seed rotates at epoch 3 (Bell lottery).
    let seed_v1 = mk_hash(0xBE); // epochs 0–2
    let seed_v2 = mk_hash(0xC0); // epochs 3–4

    vec![
        // Block 0 — genesis
        BlockInputs {
            block_hash:         mk_hash(0x01),
            finalised_at_epoch: 0,
            causal_root:        mk_hash(0x11),
            bell_seed:          seed_v1,
            evaporated_forks:   vec![],
        },
        // Block 1 — fork A decayed
        BlockInputs {
            block_hash:         mk_hash(0x02),
            finalised_at_epoch: 1,
            causal_root:        mk_hash(0x12),
            bell_seed:          seed_v1,
            evaporated_forks:   vec![fwr(0x10, 0xAA)],
        },
        // Block 2 — fork B decayed
        BlockInputs {
            block_hash:         mk_hash(0x03),
            finalised_at_epoch: 2,
            causal_root:        mk_hash(0x13),
            bell_seed:          seed_v1,
            evaporated_forks:   vec![fwr(0x10, 0xAA), fwr(0x20, 0xBB)],
        },
        // Block 3 — new beacon seed; fork C decayed
        BlockInputs {
            block_hash:         mk_hash(0x04),
            finalised_at_epoch: 3,
            causal_root:        mk_hash(0x14),
            bell_seed:          seed_v2,
            evaporated_forks:   vec![fwr(0x10, 0xAA), fwr(0x20, 0xBB), fwr(0x30, 0xCC)],
        },
        // Block 4 — canonical tip; fork D decayed
        BlockInputs {
            block_hash:         mk_hash(0x05),
            finalised_at_epoch: 4,
            causal_root:        mk_hash(0x15),
            bell_seed:          seed_v2,
            evaporated_forks:   vec![
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

// ── Uniqueness: all 5 roots must differ ─────────────────────────────────────

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
                "blocks {i} and {j} produced identical attestation roots",
            );
        }
    }
}

// ── Fork-set growth changes root ─────────────────────────────────────────────

#[test]
fn adding_each_fork_changes_attestation_root() {
    let chain = five_block_chain();
    // Blocks 1..=4 each add a fork relative to the prior block; since other
    // fields also differ (block_hash, epoch, causal_root) the root must change.
    // This test isolates the fork contribution by holding all other fields fixed.
    let mut base = chain[4].attestation();
    base.evaporated_forks.pop(); // 3 forks
    let root_3 = build_attestation(&base).unwrap();

    let full = chain[4].attestation(); // 4 forks
    let root_4 = build_attestation(&full).unwrap();

    assert_ne!(root_3, root_4, "adding fork_D must change the attestation root");
}

// ── Light-client scenario ─────────────────────────────────────────────────────

#[test]
fn light_client_verifies_block_4_from_attestation_root_alone() {
    // The light client receives (block_hash, FinalityAttestation, root) from a
    // full node. It does NOT hold the DAG, the beacon archive, or fork blocks.
    // Verification must succeed from these three inputs alone.
    let block4 = &five_block_chain()[4];
    let att = block4.attestation();
    let root = build_attestation(&att).unwrap(); // full node pre-computes this

    // Light client: feed the root to verify_attestation.
    verify_attestation(&att, &root).expect("light-client verification must succeed");
}

// ── Adversarial: cross-block field swaps ────────────────────────────────────

#[test]
fn adversarial_causal_root_from_earlier_block_rejected() {
    // An attacker presents block 4's block_hash + epoch but substitutes
    // block 2's causal_root.  Verification must reject.
    let chain = five_block_chain();
    let block4_att = chain[4].attestation();
    let root_4 = build_attestation(&block4_att).unwrap();

    let mut forged = block4_att.clone();
    forged.causal_root = chain[2].causal_root; // older causal root
    assert!(
        verify_attestation(&forged, &root_4).is_err(),
        "substituting an older causal_root must invalidate the attestation"
    );
}

#[test]
fn adversarial_bell_seed_rollback_rejected() {
    // An attacker replaces block 4's beacon seed (seed_v2) with the
    // older seed_v1 (blocks 0–2) while keeping the original root.
    let chain = five_block_chain();
    let att = chain[4].attestation();
    let root = build_attestation(&att).unwrap();

    let mut forged = att.clone();
    forged.bell_seed = chain[1].bell_seed; // rolled-back seed
    assert!(
        verify_attestation(&forged, &root).is_err(),
        "rolling back the bell_seed must invalidate the attestation"
    );
}

#[test]
fn adversarial_block_hash_swap_rejected() {
    // Fork an attestation by swapping in a different block hash.
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
    // Attacker appends a nonexistent fork to block 4's witness list and
    // presents block 4's original root.  The root diverges → rejected.
    let chain = five_block_chain();
    let att = chain[4].attestation();
    let root = build_attestation(&att).unwrap();

    let mut forged = att.clone();
    forged.evaporated_forks.push(fwr(0x50, 0xEE)); // non-existent fork E
    assert!(
        verify_attestation(&forged, &root).is_err(),
        "injecting a nonexistent fork must diverge the root"
    );
}

#[test]
fn adversarial_unsorted_fork_list_rejected_at_build() {
    // Present block 4's witness list in reverse order.  build_attestation
    // must fail before producing any root — no root to check.
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
    // Mutate one byte in one fork's witness field.  Even a 1-bit change
    // must propagate through the Merkle commitment and invalidate the root.
    let chain = five_block_chain();
    let att = chain[4].attestation();
    let root = build_attestation(&att).unwrap();

    let mut forged = att.clone();
    forged.evaporated_forks[2].witness[0] ^= 0x01; // flip LSB of fork_C witness
    assert!(
        verify_attestation(&forged, &root).is_err(),
        "a 1-bit witness mutation must invalidate the attestation"
    );
}

// ── Idempotency: root is deterministic ──────────────────────────────────────

#[test]
fn build_is_deterministic_across_multiple_calls() {
    let chain = five_block_chain();
    let att = chain[4].attestation();
    let r1 = build_attestation(&att).unwrap();
    let r2 = build_attestation(&att).unwrap();
    assert_eq!(r1, r2, "build_attestation must be pure / deterministic");
}
