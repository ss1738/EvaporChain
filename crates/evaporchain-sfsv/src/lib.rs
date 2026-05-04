//! Singh Future-Self Vault (SFSV).
//!
//! Per `research/INVENTION_STACK.md` §A5.2:
//!
//! > User posts an energy-denominated commitment that pays out to their
//! > *own future address* when a *decay-state predicate* clears.
//! > Secondary market clears third parties bidding for that future
//! > claim at a discount. Energy-decay is the unforgeable clock no
//! > validator collusion can manipulate.
//!
//! > **Pitch:** *"You can now sell your future self's money — and your
//! > future self can't sue."*
//!
//! > **Build:** 6 weeks. Cheapest survivor; mainstream-press friendly.
//! > **Build first.**
//!
//! ## What's the actual mechanism?
//!
//! 1. **Lock**: a creator deposits `Energy` and declares a decay-state
//!    predicate that, when satisfied, releases the funds. Two predicates
//!    are supported in V1:
//!       - `EpochReached(t)` — release at consensus epoch ≥ t.
//!       - `EnergyDecaysBelow(threshold)` — release when the vault's
//!         attached lot energy decays below `threshold` (using the
//!         standard exponential half-life rule from `evaporchain-types`).
//! 2. **Bequeath**: the eventual recipient is the creator's *future*
//!    address (typically the same account, but the field is explicit so
//!    cold-storage / heir flows can target a different one).
//! 3. **Sell** *(optional)*: at any point before maturity, the creator
//!    lists the claim on a secondary market. Bidders submit
//!    `(discount_price, max_decay_until_payout)` via SDDC. The
//!    foundational mechanism is reused — there is no SFSV-specific
//!    clearing rule, only an SFSV-specific *interpretation* of the two
//!    SDDC axes.
//! 4. **Payout**: at maturity, the vault unlocks to whoever currently
//!    holds the claim. If untraded, that's the creator's future-self.
//!    If sold, it's the latest secondary-market clearer.
//!
//! ## Why this is "build first" per A5.2
//!
//! - Cheapest doctrine survivor — only two axes of complexity, both
//!   reused from substrate (Energy, Epoch, SDDC).
//! - Mainstream press friendly — "sell your future self's money" is a
//!   one-sentence pitch that lands without crypto vocabulary.
//! - Demonstrates structural decay: the predicate `EnergyDecaysBelow`
//!   is *only possible* on a chain with a single λ. No bridges, no
//!   oracles, no external clock.
//!
//! ## Module map
//!
//! - [`predicate`] — [`Predicate`] + [`evaluate`].
//! - [`vault`] — [`Vault`], [`VaultId`], [`VaultStatus`], lifecycle.
//! - [`market`] — wraps SDDC for the secondary-market path.
//! - [`payout`] — [`payout`] resolver: who receives, how much.

pub mod market;
pub mod payout;
pub mod predicate;
pub mod vault;

pub use market::{list_for_sale, settle_secondary, MarketError};
pub use payout::{payout, PayoutError, PayoutResolution};
pub use predicate::{evaluate, Predicate, PredicateContext};
pub use vault::{Vault, VaultError, VaultId, VaultStatus};
