//! Plücker commitment for a `TropicalMatrix`.
//!
//! `plucker_commitment(m)` returns a 32-byte blake3 hash over the
//! canonical serialization of `m`: domain tag `"tropical-plucker"`,
//! followed by `dim` (u32, LE), followed by row-major entries
//! (1-byte variant tag + 8-byte i64 LE for `Finite`, 1-byte
//! tag for `Infinity`).
//!
//! Two matrices have the same commitment iff they have the same `dim`
//! and the same entries in the same positions. (Symmetry is *not*
//! required by the commitment, but is generally required by the
//! four-point condition for the matrix to be a valid tree-metric.)

use blake3::Hasher;

use crate::matrix::TropicalMatrix;
use crate::scalar::TropicalScalar;

const DOMAIN_TAG: &[u8] = b"tropical-plucker";
const TAG_FINITE: u8 = 0x01;
const TAG_INFINITY: u8 = 0x02;

pub fn plucker_commitment(m: &TropicalMatrix) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(DOMAIN_TAG);
    h.update(&(m.dim() as u32).to_le_bytes());
    for s in m.raw() {
        match s {
            TropicalScalar::Finite(v) => {
                h.update(&[TAG_FINITE]);
                h.update(&v.to_le_bytes());
            }
            TropicalScalar::Infinity => {
                h.update(&[TAG_INFINITY]);
            }
        }
    }
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::star::star_tree_distances;

    #[test]
    fn commitment_is_deterministic() {
        let m = star_tree_distances(&[1, 2, 4, 8]);
        let c1 = plucker_commitment(&m);
        let c2 = plucker_commitment(&m);
        assert_eq!(c1, c2);
    }

    #[test]
    fn permuting_leaves_changes_commitment() {
        // Same multiset of energies, different leaf order → matrix
        // entries land in different (i, j) positions → commitment differs.
        let m_a = star_tree_distances(&[1, 2, 4, 8]);
        let m_b = star_tree_distances(&[8, 4, 2, 1]);
        assert_ne!(plucker_commitment(&m_a), plucker_commitment(&m_b));
    }

    #[test]
    fn changing_one_entry_changes_commitment() {
        let mut m = star_tree_distances(&[1, 2, 4, 8]);
        let c0 = plucker_commitment(&m);
        m.set(0, 1, TropicalScalar::finite(999));
        let c1 = plucker_commitment(&m);
        assert_ne!(c0, c1);
    }

    #[test]
    fn different_dims_yield_different_commitments() {
        let m_3 = star_tree_distances(&[1, 2, 4]);
        let m_4 = star_tree_distances(&[1, 2, 4, 8]);
        assert_ne!(plucker_commitment(&m_3), plucker_commitment(&m_4));
    }

    #[test]
    fn empty_matrix_has_a_well_defined_commitment() {
        let m = TropicalMatrix::new(0);
        let _ = plucker_commitment(&m); // just must not panic
    }
}
