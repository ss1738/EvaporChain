//! Ultrametric distance over Z_p — the geometry that makes the Merkle
//! tree automatic.
//!
//! Distance is reported as `v_p(|x − y|)` (a u32 — large = close,
//! `u32::MAX` = identical). This is the *negative log base p* of the
//! conventional p-adic distance `|x − y|_p = p^{-v_p(x − y)}`. Working
//! in valuation units keeps everything integer-arithmetic and lines up
//! directly with Merkle-tree depth.
//!
//! ## Strong triangle inequality
//!
//! For any `x, y, z`:
//!
//! ```text
//!   v_p(x − z) ≥ min(v_p(x − y), v_p(y − z))
//! ```
//!
//! (equivalent to `d_p(x, z) ≤ max(d_p(x, y), d_p(y, z))` in the usual
//! distance form). Property-tested in `proptests::strong_triangle_*`.

use crate::valuation::valuation;

/// Ultrametric "distance" between two p-adic integers, reported as
/// `v_p(|x − y|)`. **Higher = closer.** Equal inputs return `u32::MAX`.
pub fn ultrametric_distance<const P: usize>(x: u64, y: u64) -> u32 {
    let diff = x.abs_diff(y);
    valuation::<P>(diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_equal_is_infinity() {
        assert_eq!(ultrametric_distance::<2>(42, 42), u32::MAX);
    }

    #[test]
    fn distance_2_one_apart_is_zero() {
        assert_eq!(ultrametric_distance::<2>(0, 1), 0);
        assert_eq!(ultrametric_distance::<2>(7, 8), 0);
    }

    #[test]
    fn distance_2_two_apart_is_one() {
        assert_eq!(ultrametric_distance::<2>(0, 2), 1);
        assert_eq!(ultrametric_distance::<2>(5, 7), 1);
    }

    #[test]
    fn distance_3_examples() {
        assert_eq!(ultrametric_distance::<3>(0, 3), 1);
        assert_eq!(ultrametric_distance::<3>(0, 9), 2);
        assert_eq!(ultrametric_distance::<3>(0, 27), 3);
        assert_eq!(ultrametric_distance::<3>(0, 1), 0);
    }

    #[test]
    fn distance_is_symmetric() {
        for (x, y) in [(0u64, 5), (12, 4), (1024, 1)] {
            assert_eq!(
                ultrametric_distance::<2>(x, y),
                ultrametric_distance::<2>(y, x),
            );
        }
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Strong triangle inequality, base 2.
        #[test]
        fn strong_triangle_p2(
            x in 0u64..1_000_000,
            y in 0u64..1_000_000,
            z in 0u64..1_000_000,
        ) {
            let d_xy = ultrametric_distance::<2>(x, y);
            let d_yz = ultrametric_distance::<2>(y, z);
            let d_xz = ultrametric_distance::<2>(x, z);
            // v_p form: v_p(x - z) >= min(v_p(x - y), v_p(y - z))
            prop_assert!(d_xz >= d_xy.min(d_yz));
        }

        /// Strong triangle inequality, base 3.
        #[test]
        fn strong_triangle_p3(
            x in 0u64..1_000_000,
            y in 0u64..1_000_000,
            z in 0u64..1_000_000,
        ) {
            let d_xy = ultrametric_distance::<3>(x, y);
            let d_yz = ultrametric_distance::<3>(y, z);
            let d_xz = ultrametric_distance::<3>(x, z);
            prop_assert!(d_xz >= d_xy.min(d_yz));
        }

        /// Strong triangle inequality, base 5 (sanity check non-trivial p).
        #[test]
        fn strong_triangle_p5(
            x in 0u64..1_000_000,
            y in 0u64..1_000_000,
            z in 0u64..1_000_000,
        ) {
            let d_xy = ultrametric_distance::<5>(x, y);
            let d_yz = ultrametric_distance::<5>(y, z);
            let d_xz = ultrametric_distance::<5>(x, z);
            prop_assert!(d_xz >= d_xy.min(d_yz));
        }

        /// Isosceles property: in any ultrametric space, every triangle
        /// is isosceles with the two largest sides equal. Equivalently:
        /// the smallest two of {d_xy, d_yz, d_xz} (in valuation form,
        /// these are the *largest* in true distance) — at most one of
        /// them is strictly smaller than the others.
        ///
        /// We check the contrapositive: if d_xy != d_xz, then d_yz must
        /// equal min(d_xy, d_xz).
        #[test]
        fn isosceles_property(
            x in 0u64..1_000_000,
            y in 0u64..1_000_000,
            z in 0u64..1_000_000,
        ) {
            let d_xy = ultrametric_distance::<2>(x, y);
            let d_xz = ultrametric_distance::<2>(x, z);
            let d_yz = ultrametric_distance::<2>(y, z);
            if d_xy != d_xz {
                prop_assert_eq!(d_yz, d_xy.min(d_xz));
            }
        }
    }
}
