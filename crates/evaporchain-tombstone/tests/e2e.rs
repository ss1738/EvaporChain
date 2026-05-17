//! End-to-end Tombstone + Eulogy-Trie fixture: a multi-cause death
//! cohort + Merkle-style root agreement.
//!
//! Mints 10 tombstones across all five `CauseOfDeath` variants and
//! asserts:
//!
//! 1. **Domain-separated commitments** — every tombstone is distinct
//!    (no commitment collisions across causes).
//! 2. **Order-independent root** — two nodes inserting the same cohort
//!    in opposite orders compute the same trie root.
//! 3. **Append-only enforcement** — re-evaporating the same address is
//!    rejected; the chain refuses to forget.
//! 4. **Cause-discriminant binding** — same address with different
//!    cause yields a different tombstone (cause is part of the
//!    commitment).
//!
//! Doctrine: `INVENTION_STACK.md` Amendment 2 §A2.5 — the chain's
//! deliberate exception to the anti-immutability rule.

use evaporchain_tombstone::{cause::CauseOfDeath, tombstone::mint, EulogyTrie};

fn addr(b: u8) -> [u8; 32] {
    [b; 32]
}

fn cohort() -> Vec<([u8; 32], evaporchain_tombstone::tombstone::Tombstone)> {
    let causes = [
        CauseOfDeath::Evaporated,
        CauseOfDeath::ForgottenViaDecayProof,
        CauseOfDeath::SlashedToZero,
        CauseOfDeath::RentExhausted,
        CauseOfDeath::Other(42),
    ];
    let mut out = vec![];
    for (i, c) in causes.iter().enumerate() {
        let a = addr(i as u8);
        out.push((a, mint(a, 1_000 + i as u64 * 100, 500, *c)));
        // Second account per cause.
        let a2 = addr(50 + i as u8);
        out.push((a2, mint(a2, 5_000, 500, *c)));
    }
    out
}

#[test]
fn e2e_full_cohort_commitments_are_distinct() {
    let c = cohort();
    assert_eq!(c.len(), 10);
    // All commitments distinct.
    let mut seen = std::collections::HashSet::new();
    for (_, t) in &c {
        assert!(seen.insert(t.commitment), "commitment collision");
    }
}

#[test]
fn e2e_root_is_order_independent() {
    let c = cohort();
    let mut t1 = EulogyTrie::new();
    let mut t2 = EulogyTrie::new();
    for (a, t) in c.iter() {
        t1.insert(*a, *t).unwrap();
    }
    // Reverse-order insertion.
    for (a, t) in c.iter().rev() {
        t2.insert(*a, *t).unwrap();
    }
    assert_eq!(t1.root(), t2.root(), "root must be order-independent");
    assert_eq!(t1.len(), t2.len());
}

#[test]
fn e2e_append_only_rejects_re_evaporation() {
    let mut t = EulogyTrie::new();
    let a = addr(7);
    let tomb = mint(a, 1_234, 500, CauseOfDeath::Evaporated);
    t.insert(a, tomb).unwrap();
    let err = t.insert(a, tomb);
    assert!(err.is_err(), "re-evaporation must be rejected — tombstones are forever");
    assert_eq!(t.len(), 1);
}

#[test]
fn e2e_cause_part_of_commitment() {
    let a = addr(11);
    let evaporated = mint(a, 1_000, 500, CauseOfDeath::Evaporated);
    let slashed = mint(a, 1_000, 500, CauseOfDeath::SlashedToZero);
    assert_ne!(
        evaporated.commitment, slashed.commitment,
        "cause must be bound into commitment"
    );
}

#[test]
fn e2e_other_variant_discriminant_distinct() {
    let a = addr(13);
    let other_a = mint(a, 100, 1, CauseOfDeath::Other(1));
    let other_b = mint(a, 100, 1, CauseOfDeath::Other(2));
    assert_ne!(
        other_a.commitment, other_b.commitment,
        "Other(n) discriminant must depend on n"
    );
}
