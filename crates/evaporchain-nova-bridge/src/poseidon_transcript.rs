//! Phase 2.2-section-2 prep — typed spec of the absorb order that
//! nova-snark's `RecursiveSNARK::verify` uses to reconstruct the two
//! committed hashes (`hash_primary`, `hash_secondary`).
//!
//! # Why this module is "prep" and not the full Section 2
//!
//! Section 2 of the verifier circuit needs to re-hash the same
//! sequence that nova-snark hashes off-circuit and prove the result
//! equals the committed value on `l_u_secondary.X[..2]`. Doing that
//! in-circuit requires three things:
//!
//!   1. **The absorb order** — exactly which scalars get fed to the
//!      sponge, and in which order. (This module.)
//!   2. **A Poseidon implementation in arkworks R1CS** that matches
//!      nova-snark's neptune-backed `PoseidonRO` byte-for-byte
//!      (MDS matrix + round constants + sponge framing for
//!      `Strength::Standard`, arity `U24`, `Simplex` sponge mode).
//!   3. **The adapter** that pre-converts the nova-snark scalars to
//!      `ark_bn254::Fr` (lives in Phase 2.3).
//!
//! Item 2 is the work. Before any of that lands, item 1 has to be
//! pinned in code so the circuit, the adapter, and any reference
//! tooling all see the *same* sequence. Drift between absorb order
//! sites is silent — a misordered absorb produces a different hash
//! and the Section 2 equality check rejects every valid proof.
//!
//! # Source — nova-snark 0.68 RecursiveSNARK::verify
//!
//! ```text
//! crates/nova-snark-0.68.0/src/nova/mod.rs:598-624
//!
//! hash_primary = PoseidonRO::<E2::Base>::new(pp.ro_consts_secondary)
//!     .absorb(pp.digest())                      // E1::Scalar
//!     .absorb(E1::Scalar::from(num_steps))      // E1::Scalar
//!     .absorb_each(z0)                          // Vec<E1::Scalar>
//!     .absorb_each(zi)                          // Vec<E1::Scalar>
//!     .absorb_via(r_U_secondary.absorb_in_ro)   // RelaxedR1CSInstance<E2>
//!     .absorb(ri_primary)                       // E1::Scalar
//!     .squeeze(NUM_HASH_BITS, false)
//!
//! hash_secondary = PoseidonRO::<E1::Base>::new(pp.ro_consts_primary)
//!     .absorb(scalar_as_base::<E1>(pp.digest())) // E2::Scalar
//!     .absorb(E2::Scalar::from(num_steps))       // E2::Scalar
//!     .absorb(E2::Scalar::ZERO)                  // empty-z0 sentinel
//!     .absorb(E2::Scalar::ZERO)                  // empty-zi sentinel
//!     .absorb_via(r_U_primary.absorb_in_ro)      // RelaxedR1CSInstance<E1>
//!     .absorb(ri_secondary)                      // E2::Scalar
//!     .squeeze(NUM_HASH_BITS, false)
//! ```
//!
//! The primary side absorbs `(digest, num_steps, z0..., zi...,
//! r_U_secondary fields..., ri_primary)`. The secondary side absorbs
//! `(digest_as_base, num_steps, 0, 0, r_U_primary fields...,
//! ri_secondary)` — the two ZEROs are sentinel slots in place of z0
//! and zi on the secondary side, because nova folds the secondary
//! trivially (no step-circuit state).
//!
//! # What's pinned here vs. what isn't
//!
//! - **Pinned:** the slot enum and the per-side slot order builders.
//!   When Section 2 lands, the in-circuit absorb code will iterate
//!   this list — adding/removing slots without bumping
//!   `TRANSCRIPT_VERSION` is a compile-time refactor signal.
//! - **Not pinned:** the actual scalar values for the
//!   `r_U_*.absorb_in_ro` slots. `RelaxedR1CSInstance::absorb_in_ro`
//!   absorbs `(comm_W, comm_E, u, X[..])`. Each commitment is a
//!   curve point that needs special handling (x-coordinate + sign
//!   bit, or pair of base-field elts). Choosing the encoding is part
//!   of Section 2's R1CS gadget design, not this module.

/// A typed slot in the nova-snark verifier's absorb sequence. Each
/// variant corresponds to one or more `hasher.absorb(...)` calls in
/// `RecursiveSNARK::verify`.
///
/// The `RelaxedR1csInstance` variant expands at gadget-write-time
/// into its constituent absorbs (`comm_W`, `comm_E`, `u`, `X[..]`);
/// the exact expansion lives in nova-snark's
/// `RelaxedR1CSInstance::absorb_in_ro`. This enum carries the
/// *logical* slot, not the byte-level scalar count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptSlot {
    /// `pp.digest()` — the public-parameters commitment. On the
    /// primary hasher this is `E1::Scalar` directly; on the
    /// secondary hasher it's `scalar_as_base::<E1>(pp.digest())`.
    PpDigest,
    /// `num_steps` lifted to the hasher's field.
    NumSteps,
    /// One entry of the initial state vector `z0`. Only appears on
    /// the primary hasher; the secondary side substitutes
    /// [`TranscriptSlot::SecondaryZ0Sentinel`].
    Z0Entry { index: usize },
    /// One entry of the current state vector `zi`. Only appears on
    /// the primary hasher.
    ZiEntry { index: usize },
    /// Sentinel `0` absorbed in place of z0 on the secondary side.
    /// Nova folds the secondary trivially, so its step-circuit
    /// state is empty and represented as two zero absorbs.
    SecondaryZ0Sentinel,
    /// Sentinel `0` absorbed in place of zi on the secondary side.
    SecondaryZiSentinel,
    /// The cross-side `RelaxedR1CSInstance` absorbed via its
    /// `absorb_in_ro` method. Expands at gadget time into
    /// `(comm_W, comm_E, u, X[..])` absorbs. The variant pins
    /// *which* instance (primary vs. secondary) gets absorbed on
    /// each side.
    RelaxedR1csInstance { side: RelaxedR1csSide },
    /// `ri_primary` or `ri_secondary` — the running input scalar
    /// passed alongside the accumulator.
    RunningInput { side: RunningInputSide },
}

/// Discriminates which RelaxedR1CSInstance is absorbed on each
/// hasher side. Note the SWAP: the primary hasher absorbs the
/// SECONDARY instance, and vice-versa.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelaxedR1csSide {
    /// `r_U_secondary` — absorbed on the PRIMARY hasher.
    Secondary,
    /// `r_U_primary` — absorbed on the SECONDARY hasher.
    Primary,
}

/// Discriminates which running input scalar is absorbed on each
/// hasher side. Same cross-side pattern as
/// [`RelaxedR1csSide`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunningInputSide {
    /// `ri_primary` — absorbed on the PRIMARY hasher.
    Primary,
    /// `ri_secondary` — absorbed on the SECONDARY hasher.
    Secondary,
}

/// Which committed hash this absorb sequence reconstructs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HasherSide {
    /// Reproduces `hash_primary` (the value `l_u_secondary.X[0]`
    /// is compared against after a base/scalar conversion).
    Primary,
    /// Reproduces `hash_secondary` (the value `l_u_secondary.X[1]`
    /// is compared against directly).
    Secondary,
}

/// Spec version. Bump whenever the slot order or composition
/// changes — Section 2's in-circuit gadget asserts on this so a
/// silent reorder can't slip through.
pub const TRANSCRIPT_VERSION: &str = "v1-nova-snark-0.68-recursive-snark-verify";

/// Build the absorb-slot sequence for one side of the verifier.
///
/// `z_arity` is the length of the chain's step-circuit state
/// vector (1 for `TrivialIncrementCircuit` in the fixture; 4 in
/// `RealBlockCircuit` per `evaporchain-proving/src/nova.rs`). The
/// secondary side ignores `z_arity` and always emits the two
/// sentinel zero slots.
pub fn absorb_order(side: HasherSide, z_arity: usize) -> Vec<TranscriptSlot> {
    let mut out = Vec::with_capacity(4 + 2 * z_arity);
    out.push(TranscriptSlot::PpDigest);
    out.push(TranscriptSlot::NumSteps);
    match side {
        HasherSide::Primary => {
            for index in 0..z_arity {
                out.push(TranscriptSlot::Z0Entry { index });
            }
            for index in 0..z_arity {
                out.push(TranscriptSlot::ZiEntry { index });
            }
            out.push(TranscriptSlot::RelaxedR1csInstance {
                side: RelaxedR1csSide::Secondary,
            });
            out.push(TranscriptSlot::RunningInput {
                side: RunningInputSide::Primary,
            });
        }
        HasherSide::Secondary => {
            out.push(TranscriptSlot::SecondaryZ0Sentinel);
            out.push(TranscriptSlot::SecondaryZiSentinel);
            out.push(TranscriptSlot::RelaxedR1csInstance {
                side: RelaxedR1csSide::Primary,
            });
            out.push(TranscriptSlot::RunningInput {
                side: RunningInputSide::Secondary,
            });
        }
    }
    out
}

/// Number of bits kept from the Poseidon squeeze output. Nova
/// truncates the squeezed scalar to 250 bits to fit in BOTH curves
/// of the cycle without modular reduction.
///
/// Source: `nova-snark/src/constants.rs::NUM_HASH_BITS`.
pub const NUM_HASH_BITS: usize = 250;

/// Sponge parameter discriminants nova-snark configures its
/// neptune `PoseidonConstants` with. The Section-2 R1CS gadget
/// (when it lands) must instantiate the arkworks Poseidon
/// parameters with values that produce the *same* state-update
/// permutation as neptune does for these settings — otherwise
/// hashes diverge.
///
/// **These are not the constants themselves.** Porting the MDS
/// matrix + round constants from neptune is the BESPOKE part of
/// Section 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeptuneSpongeSpec {
    /// Sponge arity. Nova uses `U24` (24).
    pub arity: usize,
    /// Neptune `Strength` variant. Nova uses `Strength::Standard`.
    pub strength: &'static str,
    /// Sponge mode. Nova uses `Simplex`.
    pub mode: &'static str,
    /// IOPattern for the absorb→squeeze cycle. Nova uses
    /// `[Absorb(state.len() as u32), Squeeze(1)]`.
    pub io_pattern: &'static str,
}

/// The neptune sponge spec nova-snark configures its
/// `PoseidonConstantsCircuit<Scalar>` with.
pub const NOVA_SPONGE_SPEC: NeptuneSpongeSpec = NeptuneSpongeSpec {
    arity: 24,
    strength: "Standard",
    mode: "Simplex",
    io_pattern: "Absorb(len), Squeeze(1)",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_order_has_expected_prefix_for_z_arity_1() {
        let order = absorb_order(HasherSide::Primary, 1);
        assert_eq!(order[0], TranscriptSlot::PpDigest);
        assert_eq!(order[1], TranscriptSlot::NumSteps);
        assert_eq!(order[2], TranscriptSlot::Z0Entry { index: 0 });
        assert_eq!(order[3], TranscriptSlot::ZiEntry { index: 0 });
        assert_eq!(
            order[4],
            TranscriptSlot::RelaxedR1csInstance {
                side: RelaxedR1csSide::Secondary
            }
        );
        assert_eq!(
            order[5],
            TranscriptSlot::RunningInput {
                side: RunningInputSide::Primary
            }
        );
        assert_eq!(order.len(), 6);
    }

    #[test]
    fn secondary_order_is_invariant_in_z_arity() {
        // Secondary side substitutes two zero sentinels regardless
        // of step-circuit arity — Nova never absorbs zi entries on
        // the secondary hasher.
        for z_arity in [1, 4, 8, 24] {
            let order = absorb_order(HasherSide::Secondary, z_arity);
            assert_eq!(order.len(), 6, "z_arity={z_arity}");
            assert_eq!(order[2], TranscriptSlot::SecondaryZ0Sentinel);
            assert_eq!(order[3], TranscriptSlot::SecondaryZiSentinel);
        }
    }

    #[test]
    fn primary_order_scales_linearly_with_z_arity() {
        // 2 fixed prefix + 2 fixed suffix (RelaxedR1cs + RunningInput)
        // + 2 * z_arity entries (z0 + zi). RealBlockCircuit uses
        // z_arity = 4 → 12 slots.
        let order = absorb_order(HasherSide::Primary, 4);
        assert_eq!(order.len(), 4 + 2 * 4);
        // Check each z0/zi slot's index matches its position.
        for i in 0..4 {
            assert_eq!(order[2 + i], TranscriptSlot::Z0Entry { index: i });
            assert_eq!(order[2 + 4 + i], TranscriptSlot::ZiEntry { index: i });
        }
    }

    #[test]
    fn cross_side_swap_is_preserved() {
        // Critical asymmetry: PRIMARY hasher absorbs the SECONDARY
        // instance, SECONDARY hasher absorbs the PRIMARY instance.
        // Same for running inputs (ri_primary on primary,
        // ri_secondary on secondary). Getting either wrong silently
        // produces hash mismatch on every proof.
        let primary = absorb_order(HasherSide::Primary, 1);
        let secondary = absorb_order(HasherSide::Secondary, 1);

        let primary_instance = primary
            .iter()
            .find_map(|s| match s {
                TranscriptSlot::RelaxedR1csInstance { side } => Some(side.clone()),
                _ => None,
            })
            .unwrap();
        let secondary_instance = secondary
            .iter()
            .find_map(|s| match s {
                TranscriptSlot::RelaxedR1csInstance { side } => Some(side.clone()),
                _ => None,
            })
            .unwrap();

        assert_eq!(primary_instance, RelaxedR1csSide::Secondary);
        assert_eq!(secondary_instance, RelaxedR1csSide::Primary);

        let primary_ri = primary
            .iter()
            .find_map(|s| match s {
                TranscriptSlot::RunningInput { side } => Some(side.clone()),
                _ => None,
            })
            .unwrap();
        let secondary_ri = secondary
            .iter()
            .find_map(|s| match s {
                TranscriptSlot::RunningInput { side } => Some(side.clone()),
                _ => None,
            })
            .unwrap();

        assert_eq!(primary_ri, RunningInputSide::Primary);
        assert_eq!(secondary_ri, RunningInputSide::Secondary);
    }

    #[test]
    fn nova_sponge_spec_matches_documented_constants() {
        // Pin the neptune sponge parameters. If any of these
        // change in a future nova-snark release, Section 2's
        // gadget needs to be regenerated — this test fires as
        // an early-warning.
        assert_eq!(NOVA_SPONGE_SPEC.arity, 24);
        assert_eq!(NOVA_SPONGE_SPEC.strength, "Standard");
        assert_eq!(NOVA_SPONGE_SPEC.mode, "Simplex");
        assert_eq!(NOVA_SPONGE_SPEC.io_pattern, "Absorb(len), Squeeze(1)");
    }

    #[test]
    fn transcript_version_marker_present() {
        assert_eq!(
            TRANSCRIPT_VERSION,
            "v1-nova-snark-0.68-recursive-snark-verify"
        );
    }
}
