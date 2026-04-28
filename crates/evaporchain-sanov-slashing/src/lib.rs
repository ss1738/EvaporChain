//! Sanov-Slashing.
//!
//! Per `research/INVENTION_STACK.md` §A1.3:
//!
//! > **Sanov-Slashing** | Cramér 1938; Sanov 1957 | slash magnitude =
//! > stake × KL-rate function `I(observed ‖ honest)`; replaces ad-hoc
//! > percentages with the *exact* large-deviation cost.
//!
//! Sanov's theorem: the probability that an empirical distribution
//! `Q_n` (over `n` observations) deviates from the truth `P` is
//! approximately `exp(−n · KL(Q_n ‖ P))`. The exponent is the
//! "rate function" `I(Q ‖ P) = KL(Q ‖ P)`. So *the natural cost a
//! validator should pay for misbehaving in a way that looks `Q`-like
//! when honest behaviour is `P`-like* is proportional to `KL(Q ‖ P)` —
//! the very quantity that bounds the probability of seeing such
//! misbehaviour by chance.
//!
//! ## Why this is novel at L1
//!
//! Existing chains hard-code percentages (Cosmos: 5% double-sign, 1%
//! downtime; Ethereum: variable but parameter-bounded; EvaporChain
//! pre-Sanov: 10% Equivocation / 1% Downtime per block). All chosen
//! by intuition. Sanov-Slashing replaces every per-incident percentage
//! with a *closed-form* cost driven by the same large-deviation
//! exponent that bounds the Bayesian-posterior probability that the
//! validator was honest. There is no parameter to tune.
//!
//! ## Module map
//!
//! - [`distribution`] — `Distribution` over a finite outcome alphabet
//!   (sums to a fixed-point representation of 1.0).
//! - [`kl`] — integer Kullback-Leibler divergence
//!   `KL(Q ‖ P) = Σ Q_i · log_2(Q_i / P_i)` in millibits.
//! - [`slash`] — `sanov_slash(stake, observed, honest) -> Energy`
//!   plus the `apply_slash` integration with the energy-kernel.

pub mod distribution;
pub mod kl;
pub mod slash;

pub use distribution::{Distribution, DistributionError, FIXED_POINT_SCALE};
pub use kl::{kl_millibits, KlError};
pub use slash::{apply_slash, sanov_slash, SlashError};
