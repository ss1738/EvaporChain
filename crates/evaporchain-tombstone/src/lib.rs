//! Tombstone — the "small deaths" act of EvaporChain's four-act
//! narrative spine.
//!
//! Per `research/INVENTION_STACK.md` Amendment 2 §A2.5:
//!
//! > **Tombstone** — 32-byte commitment for every fully-evaporated
//! > account, written to non-decaying eulogy trie. The Maya Lin
//! > parallel writes itself.
//!
//! ## Why this exists structurally
//!
//! The chain's anti-feature manifesto (§2.2) forbids immutable data
//! structures. Tombstone is the **deliberate exception** — it is the
//! one place the chain refuses to forget. Every account that fully
//! decays is memorialised in 32 bytes; the eulogy trie itself never
//! evaporates.
//!
//! That tension is the structural meaning: a chain that promises
//! mortality must also commit to its dead. Bitcoin promises
//! immortality and quietly fails; EvaporChain admits its small
//! deaths and engraves them.
//!
//! ## Substrate
//!
//! - [`cause`] — `CauseOfDeath` enum: `Evaporated`, `ForgottenViaDecayProof`,
//!   `SlashedToZero`, `RentExhausted`, `Other(u32)`.
//! - [`tombstone`] — `Tombstone { commitment: [u8; 32] }` and
//!   `mint(addr, final_balance, final_epoch, cause)` — the 32-byte
//!   memorial for one evaporated account.
//! - [`eulogy_trie`] — append-only `EulogyTrie` keyed by the original
//!   account address. Domain-separated blake3 root commitment;
//!   order-independent (BTreeMap-backed). Inserting twice for the
//!   same address is rejected (tombstones are *forever*).

pub mod cause;
pub mod eulogy_trie;
pub mod tombstone;

pub use cause::CauseOfDeath;
pub use eulogy_trie::{EulogyError, EulogyTrie};
pub use tombstone::{mint, Tombstone};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Tombstone is the deliberate exception to
    /// EvaporChain's anti-immortality rule. Every evaporated account
    /// is memorialised with a 32-byte BLAKE3 commitment in a
    /// non-decaying eulogy trie. Mint is deterministic and
    /// domain-separated; double-insertion for the same address is
    /// rejected (tombstones are forever)."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let addr = [7u8; 32];
        let t1 = mint(addr, 100, 1_000, CauseOfDeath::Evaporated);
        let t2 = mint(addr, 100, 1_000, CauseOfDeath::Evaporated);
        // Determinism: same inputs → byte-identical commitment.
        assert_eq!(t1, t2);
        // Domain separation by cause: different cause → different
        // commitment (CauseOfDeath::SlashedToZero has a distinct
        // discriminant).
        let t3 = mint(addr, 100, 1_000, CauseOfDeath::SlashedToZero);
        assert_ne!(t1, t3);

        // Eulogy trie: tombstones are forever — double insert rejects.
        let mut trie = EulogyTrie::new();
        trie.insert(addr, t1).unwrap();
        let err = trie.insert(addr, t1).unwrap_err();
        let _ = err;
        assert!(trie.contains(&addr));
        assert_eq!(trie.len(), 1);
    }
}
