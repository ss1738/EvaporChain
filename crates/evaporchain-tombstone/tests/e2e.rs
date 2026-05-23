//! End-to-end integration tests for evaporchain-tombstone.
//!
//! Non-trivial fixture: block's evaporation sweep.
//!
//! A block processes 5 evaporating accounts at epoch 1000. Each is
//! minted a Tombstone and inserted into an EulogyTrie. We verify:
//!
//!   a) Each cause of death produces a distinct tombstone commitment.
//!   b) EulogyTrie.root() changes with each insertion — ordering matters.
//!   c) Lookup by address returns the correct tombstone.
//!   d) Duplicate insertion returns EulogyError::AlreadyRecorded.
//!   e) Address never evaporates twice — structural uniqueness invariant.
//!
//! Doctrine claim (INVENTION_STACK §A2.5 Tombstone):
//!   "Small deaths — the Tombstone primitive records the exact cause
//!    and epoch of every account's end. The EulogyTrie root is a
//!    commitment over the block's entire death register."
//!
//! INVENTION_STACK.md §A2.5 Tombstone.

use evaporchain_tombstone::cause::CauseOfDeath;
use evaporchain_tombstone::eulogy_trie::{EulogyError as TombstoneError, EulogyTrie};
use evaporchain_tombstone::tombstone::mint;

fn addr(seed: u8) -> [u8; 32] {
    [seed; 32]
}

const EPOCH: u64 = 1_000;

// ── Fixture: 5-account evaporation sweep ─────────────────────────────────

#[test]
fn block_evaporation_sweep() {
    let mut trie = EulogyTrie::new();
    assert!(trie.is_empty());

    let causes = [
        (addr(1), 1_000u64, CauseOfDeath::Evaporated),
        (addr(2), 0,        CauseOfDeath::ForgottenViaDecayProof),
        (addr(3), 500,      CauseOfDeath::SlashedToZero),
        (addr(4), 200,      CauseOfDeath::RentExhausted),
        (addr(5), 50,       CauseOfDeath::Other(7)),
    ];

    let mut prev_root = trie.root();

    for (a, balance, cause) in &causes {
        let tombstone = mint(*a, *balance, EPOCH, *cause);
        trie.insert(*a, tombstone).expect("first insert must succeed");

        // Root changes with every insertion.
        let new_root = trie.root();
        assert_ne!(new_root, prev_root, "root unchanged after insert of addr {:?}", a);
        prev_root = new_root;
    }

    assert_eq!(trie.len(), 5, "all 5 accounts recorded");

    // Each address is recoverable.
    for (a, balance, cause) in &causes {
        let expected = mint(*a, *balance, EPOCH, *cause);
        let got = trie.get(a).expect("address must be in trie");
        assert_eq!(*got, expected);
        assert!(trie.contains(a));
    }
}

// ── Cause-of-death produces distinct commitments ──────────────────────────

#[test]
fn all_causes_produce_distinct_commitments() {
    let a = addr(0xAB);
    let balance = 10_000u64;

    let t1 = mint(a, balance, EPOCH, CauseOfDeath::Evaporated);
    let t2 = mint(a, balance, EPOCH, CauseOfDeath::ForgottenViaDecayProof);
    let t3 = mint(a, balance, EPOCH, CauseOfDeath::SlashedToZero);
    let t4 = mint(a, balance, EPOCH, CauseOfDeath::RentExhausted);
    let t5 = mint(a, balance, EPOCH, CauseOfDeath::Other(42));

    let all = [t1, t2, t3, t4, t5];
    for i in 0..all.len() {
        for j in (i + 1)..all.len() {
            assert_ne!(
                all[i].commitment, all[j].commitment,
                "cause {i} and cause {j} collide"
            );
        }
    }
}

// ── Duplicate insertion rejected ──────────────────────────────────────────

#[test]
fn double_evaporation_rejected() {
    let mut trie = EulogyTrie::new();
    let a = addr(0xFF);
    let stone = mint(a, 0, EPOCH, CauseOfDeath::Evaporated);

    trie.insert(a, stone).expect("first insert must succeed");

    let err = trie.insert(a, stone).expect_err("duplicate insert must fail");
    assert!(
        matches!(err, TombstoneError::AlreadyMemorialised(_)),
        "expected AlreadyMemorialised, got {err:?}"
    );
    // Trie still has exactly 1 entry.
    assert_eq!(trie.len(), 1);
}

// ── Determinism: same inputs → same commitment ────────────────────────────

#[test]
fn mint_is_deterministic() {
    let a = addr(0x11);
    let t1 = mint(a, 999, EPOCH, CauseOfDeath::SlashedToZero);
    let t2 = mint(a, 999, EPOCH, CauseOfDeath::SlashedToZero);
    assert_eq!(t1.commitment, t2.commitment);
}

// ── Different epochs → different commitments ──────────────────────────────

#[test]
fn different_epoch_different_commitment() {
    let a = addr(0x22);
    let t1 = mint(a, 0, 100, CauseOfDeath::Evaporated);
    let t2 = mint(a, 0, 200, CauseOfDeath::Evaporated);
    assert_ne!(t1.commitment, t2.commitment, "epoch must be bound into commitment");
}
