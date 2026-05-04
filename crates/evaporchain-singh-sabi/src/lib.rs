//! Singh-Sabi (Patina Tokens) — NFTs that age toward "ruined-beautiful".
//!
//! Per `research/INVENTION_STACK.md` §A5.3:
//!
//! > Mint deposits fixed energy budget. As λ-decay drains, *visual
//! > entropy* increases on a deterministic curve: edges fray, palette
//! > desaturates, surface accrues procedural cracks/foxing/staining
//! > seeded by tokenId. Decay is *aesthetically tuned* — never reaches
//! > zero, reaches a "ruined-beautiful" asymptote (~15% energy floor).
//! > Owner cannot pause; only witness.
//!
//! > **Pitch:** *"A blockchain that lets art age like paper."*
//!
//! ## What's structurally different from other NFTs
//!
//! Two structural decisions distinguish Singh-Sabi from every other NFT
//! primitive that has tried "decaying art" cosmetically:
//!
//! 1. **The decay floor is non-zero.** Standard `energy_at_epoch`
//!    decay tends to zero. Patina decay tends to a configurable
//!    `floor_pct` (default 15%). Mathematically: `patina(t) = floor +
//!    (initial - floor) * 2^(-t/half_life)`. The token *never*
//!    evaporates — it asymptotes to "ruined-beautiful." Owner cannot
//!    pause it; only witness it.
//!
//! 2. **Visual entropy is deterministically derivable** from
//!    `(token_id, current_epoch)` alone. Validators agree on the
//!    rendered patina state without rendering pixels — they agree on
//!    a `PatinaState { score, cracks, desaturation, foxing, edge_fray }`
//!    tuple. The shader (off-chain) consumes this tuple plus the
//!    `token_id` seed to draw cracks/foxing/staining at canonical
//!    positions. *On-chain agreement is on the entropy state, not on
//!    pixels.*
//!
//! ## Why this is "lock-grade" per A5.3
//!
//! - Cultural lineage spans 4 centuries (wabi-sabi 16th c., kintsugi,
//!   Tarkovsky 1979, Basinski 2002, Banksy 2018) — not "blockchain
//!   art."
//! - Decay synergy is *structural* — the floor + the seeded entropy
//!   curve both require the chain's single-λ to be deterministic.
//!   Cosmetic decay disqualifies (per doctrine guardrail).
//! - 6-week build (no death oracle, no kin graph, no engagement
//!   coupling) — cheapest of the five A5.3 NFT primitives.
//!
//! ## Module map
//!
//! - [`patina`] — [`patina_score`] non-zero-floor decay function;
//!   [`PatinaParams`] for floor and half-life.
//! - [`entropy`] — [`PatinaState`] deterministic entropy tuple;
//!   [`derive_state`] from `(token_id, score, params)`.
//! - [`token`] — [`PatinaToken`], [`TokenId`]; mint, transfer, witness.

pub mod entropy;
pub mod patina;
pub mod token;

pub use entropy::{derive_state, PatinaState};
pub use patina::{patina_score, PatinaError, PatinaParams};
pub use token::{PatinaToken, TokenError, TokenId};
