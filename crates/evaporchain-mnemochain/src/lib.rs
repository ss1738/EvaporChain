//! MnemoChain — Anki on-chain with FSRS forgetting curves.
//!
//! Per `research/INVENTION_STACK.md` §A5.5:
//!
//! > Every flashcard is an on-chain object with half-life equal to
//! > its scientifically-modeled forgetting curve (Ebbinghaus → SM-2
//! > → FSRS, Wozniak/Ye 2023). When card's "memory energy" decays
//! > past threshold, surfaces for review. Correct recall
//! > *re-energizes* with longer half-life; wrong recall collapses
//! > it. The chain literally **is** the spaced-repetition scheduler.
//!
//! > **Decay synergy:** structural — this is the *only* candidate
//! > where decay isn't metaphor, it's the literal cognitive-science
//! > primitive. Without decay, you have Quizlet.
//!
//! > **The moat:** cards become **portable cognitive credentials.**
//! > "I have provably reviewed Spanish vocab card #4471 across 312
//! > sessions over 4 years" is a real attestation a university or
//! > employer can verify. Anki decks aren't portable; MnemoChain
//! > decks are.
//!
//! ## What this crate ships
//!
//! Three modules:
//!
//! - **Singh Curve** ([`curve`]) — the FSRS-style stability update
//!   model. Stability `S` is in epochs; after a successful review at
//!   `S`-stability, the new stability is `S * grade_multiplier(g)`
//!   (the "Singh curve" — a piecewise growth law per grade tier).
//!   Lapses (wrong recall) reset stability hard.
//! - **Card** ([`card`]) — the on-chain flashcard: `(id, owner,
//!   stability, last_reviewed_at, attempts, lapses, correct)`.
//!   Half-life = current `stability`. `is_due` predicate compares
//!   epochs-since-review to stability.
//! - **Trie pointer** ([`trie`]) — opaque commitment to the off-chain
//!   card content (the question-answer text + media live off-chain;
//!   the chain holds only the BLAKE3 hash of the canonical encoding).
//!
//! ## Module map
//!
//! - [`curve`] — [`Grade`], [`update_stability`], [`SinghCurveError`].
//! - [`card`] — [`Card`], [`CardError`], `review`, `is_due`,
//!   [`CredentialAttestation`].
//! - [`trie`] — [`MnemoTriePtr`] off-chain blob commitment.

pub mod card;
pub mod curve;
pub mod trie;

pub use card::{Card, CardError, CardId, CredentialAttestation, ReviewOutcome};
pub use curve::{update_stability, Grade, SinghCurveError};
pub use trie::{MnemoTriePtr, TrieError};
