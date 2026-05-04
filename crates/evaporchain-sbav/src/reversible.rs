//! `Reversible` trait — the contract every classical opcode must
//! satisfy. The trait is what gives SBAV its asymmetry: classical ops
//! implement `Reversible`; the unique irreversible op (`Decay`) does
//! not.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReversibilityError {
    #[error("op is irreversible — no inverse exists (Decay is the only such op in V1)")]
    Irreversible,
}

/// A reversible operation: composing `forward` and `inverse` must
/// produce the identity function on the VM state.
///
/// The trait deliberately separates the two directions so the
/// type-system enforces both halves — a "reversible" op without a
/// declared inverse won't compile.
pub trait Reversible {
    /// The op that *undoes* `self` when applied to the
    /// `self`-post-state. By design the inverse must itself be a
    /// reversible op.
    fn inverse(&self) -> Self
    where
        Self: Sized;
}
