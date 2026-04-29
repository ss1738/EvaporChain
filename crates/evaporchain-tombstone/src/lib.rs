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
