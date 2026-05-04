//! App-glue — wires the application-layer primitives into the
//! chain's four-act narrative spine.
//!
//! Per `research/INVENTION_STACK.md` §A2.5 the chain ships four acts:
//!
//! | Act | Crate | This glue's role |
//! |-----|-------|------------------|
//! | Birth | `evaporchain-llsa` (genesis invariant proof) | not in scope here |
//! | Life | `evaporchain-sentinel` (homeostatic governance) | expose patina floor as a `BoundedParameter` |
//! | Small deaths | `evaporchain-tombstone` (eulogy trie + 32-byte memorials) | adapt Mayfly + Posthuma deaths to Tombstones |
//! | Final death | `evaporchain-mortis` (chain death certificate) | not directly coupled here |
//!
//! ## What this crate ships
//!
//! Six adapter modules — three "deaths" (small-death tombstones for
//! the application primitives that can permanently retire) and three
//! "life" parameters (Sentinel-governed knobs the chain can vote on
//! within hard bounds). Each adapter is thin — a pure function — but
//! expressing them in one place makes the four-act / homeostasis
//! coupling real.
//!
//! ## Small-death adapters (→ Tombstone)
//!
//! 1. **Mayfly → Tombstone** ([`mayfly_tombstone`]) — when a
//!    Provably-Mortal NFT reaches its certified death epoch, mint a
//!    Tombstone for it.
//! 2. **Posthuma → Tombstone** ([`posthuma_tombstone`]) — when a
//!    Singh-Posthuma `Testament` transitions to `Memorial`, mint a
//!    Tombstone keyed on the testament's id.
//! 3. **WitnessFit → Tombstone** ([`witnessfit_tombstone`]) — when a
//!    Singh-Streak's strength evaporates to Broken AND the holder
//!    voluntarily retires it, mint a Tombstone preserving the
//!    lifetime audit trail.
//! 4. **MnemoChain → Tombstone** ([`mnemochain_tombstone`]) — when a
//!    learning Card decays past the unrecoverable threshold and the
//!    holder accepts the loss, mint a Tombstone preserving the
//!    portable cognitive credential as a final attestation.
//!
//! ## Life-knob adapters (→ Sentinel)
//!
//! 5. **PatinaFloor → Sentinel** ([`patina_governance`]) — the
//!    Singh-Sabi 15% asymptote, hard-bounded `[5%, 50%]`.
//! 6. **ResonanceCoupling → Sentinel** ([`resonance_governance`]) —
//!    the engagement-coupled λ saturation knob, hard-bounded
//!    `[100, 100_000]` so it cannot collapse to zero (every-vote-
//!    saturates) or explode to MAX (no-vote-ever-saturates). The
//!    anti-Black-Mirror max-scale knob also exposed.
//!
//! ## Final-death observation surface
//!
//! 7. **Mortis → MortalityState** ([`mortis_observation`]) — projects
//!    the consensus-internal `MortisMonitor` to a coarse 3-state
//!    enum (`Alive` / `Sickening { remaining_epochs }` / `Dead`)
//!    that wallets, dApps, light clients and indexers can render.
//!    Sickening is reversible (homeostasis); Dead is terminal.

pub mod mayfly_tombstone;
pub mod mnemochain_tombstone;
pub mod mortis_observation;
pub mod patina_governance;
pub mod posthuma_tombstone;
pub mod resonance_governance;
pub mod witnessfit_tombstone;

pub use mayfly_tombstone::{tombstone_for_mayfly, MayflyTombstoneError};
pub use mnemochain_tombstone::{tombstone_for_card, MnemoTombstoneError};
pub use mortis_observation::{observe, MortalityState};
pub use patina_governance::{patina_floor_parameter, PatinaGovernanceError};
pub use posthuma_tombstone::{tombstone_for_memorial, PosthumaTombstoneError};
pub use resonance_governance::{
    resonance_max_scale_parameter, resonance_saturation_parameter, ResonanceGovernanceError,
};
pub use witnessfit_tombstone::{tombstone_for_streak, WitnessFitTombstoneError};
