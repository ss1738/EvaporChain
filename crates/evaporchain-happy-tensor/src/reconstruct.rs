//! V2 reconstruction gate: cone-cover check + delegate to V1
//! Shamir for the bit-recovery.

use std::collections::BTreeSet;

use thiserror::Error;

use evaporchain_happy_code::{reconstruct_bulk, ReconstructError, Share};

use crate::cone::{cone_covers_bulk, ConeError};
use crate::disk::{CellId, HaPPYDisk};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReconstructV2Error {
    #[error("boundary subset is insufficient — its causal cone does not cover the bulk")]
    CauseConeMissesBulk,
    #[error(transparent)]
    Cone(#[from] ConeError),
    #[error(transparent)]
    InnerShamir(#[from] ReconstructError),
}

/// V2 reconstruction:
/// 1. Build the causal cone of `boundary_subset`.
/// 2. If bulk not in cone, fail closed with `CauseConeMissesBulk`.
///    (Even if shares are individually fresh.)
/// 3. Otherwise, delegate to V1 Shamir threshold reconstruction
///    on the shares whose cell ids are in `boundary_subset`.
///    The energy floor still gates structurally.
pub fn reconstruct_v2(
    disk: &HaPPYDisk,
    boundary_subset: &BTreeSet<CellId>,
    shares: &[Share],
    k_threshold: usize,
    reconstruction_floor: u64,
) -> Result<u64, ReconstructV2Error> {
    if !cone_covers_bulk(disk, boundary_subset)? {
        return Err(ReconstructV2Error::CauseConeMissesBulk);
    }
    Ok(reconstruct_bulk(shares, k_threshold, reconstruction_floor)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_happy_code::encode_bulk;

    fn cid(b: u32) -> CellId {
        CellId(b)
    }

    fn small_disk() -> HaPPYDisk {
        let mut d = HaPPYDisk::new();
        d.add_cell(cid(0), false, true);
        d.add_cell(cid(1), false, false);
        d.add_cell(cid(2), true, false);
        d.add_cell(cid(3), true, false);
        d.add_edge(cid(0), cid(1)).unwrap();
        d.add_edge(cid(1), cid(2)).unwrap();
        d.add_edge(cid(1), cid(3)).unwrap();
        d
    }

    fn fresh_shares(bulk: u64) -> Vec<Share> {
        encode_bulk(bulk, 2, 2, [0xAA; 32], 1000).unwrap()
    }

    // ── cone-coverage gate ───────────────────────────────────────

    #[test]
    fn full_boundary_with_fresh_shares_recovers_bulk() {
        let disk = small_disk();
        let mut subset = BTreeSet::new();
        subset.insert(cid(2));
        subset.insert(cid(3));
        let shares = fresh_shares(42);
        let recovered = reconstruct_v2(&disk, &subset, &shares, 2, 100).unwrap();
        assert_eq!(recovered, 42);
    }

    #[test]
    fn single_boundary_blocks_reconstruction_even_with_fresh_shares() {
        // V2 differs from V1: a single boundary share's cone does
        // NOT cover the bulk in this 4-cell topology, regardless
        // of share count or freshness.
        let disk = small_disk();
        let mut subset = BTreeSet::new();
        subset.insert(cid(2));
        let shares = fresh_shares(42);
        let err = reconstruct_v2(&disk, &subset, &shares, 2, 100).unwrap_err();
        assert!(matches!(err, ReconstructV2Error::CauseConeMissesBulk));
    }

    #[test]
    fn empty_boundary_subset_blocks() {
        let disk = small_disk();
        let subset = BTreeSet::new();
        let shares = fresh_shares(42);
        let err = reconstruct_v2(&disk, &subset, &shares, 2, 100).unwrap_err();
        assert!(matches!(err, ReconstructV2Error::CauseConeMissesBulk));
    }

    // ── disconnected-arcs case (the V2 win over V1) ──────────────

    fn split_disk() -> HaPPYDisk {
        let mut d = HaPPYDisk::new();
        d.add_cell(cid(0), false, true);
        d.add_cell(cid(1), false, false);
        d.add_cell(cid(2), true, false);
        d.add_cell(cid(3), true, false);
        d.add_cell(cid(4), false, false);
        d.add_cell(cid(5), true, false);
        d.add_cell(cid(6), true, false);
        d.add_edge(cid(0), cid(1)).unwrap();
        d.add_edge(cid(0), cid(4)).unwrap();
        d.add_edge(cid(1), cid(2)).unwrap();
        d.add_edge(cid(1), cid(3)).unwrap();
        d.add_edge(cid(4), cid(5)).unwrap();
        d.add_edge(cid(4), cid(6)).unwrap();
        d
    }

    #[test]
    fn disconnected_arcs_blocked_even_at_high_share_count() {
        // V1 would accept 2 fresh shares with k=2. V2 demands
        // CONE coverage: shares from disconnected arcs (one share
        // each side of bulk) cannot cone-cover.
        let disk = split_disk();
        let mut subset = BTreeSet::new();
        subset.insert(cid(2));
        subset.insert(cid(5)); // one boundary on each side
        let shares = encode_bulk(99, 4, 2, [0xCD; 32], 1000).unwrap();
        let err = reconstruct_v2(&disk, &subset, &shares, 2, 100).unwrap_err();
        assert!(matches!(err, ReconstructV2Error::CauseConeMissesBulk));
    }

    #[test]
    fn full_arc_on_one_side_does_not_cover_for_split_disk() {
        // {2, 3} all on one side; cell 1 has 2-of-3 majority but
        // bulk's neighbours {1, 4} only have 1-of-2 in cone →
        // not majority. Single arc INSUFFICIENT.
        let disk = split_disk();
        let mut subset = BTreeSet::new();
        subset.insert(cid(2));
        subset.insert(cid(3));
        let shares = encode_bulk(99, 4, 2, [0xCD; 32], 1000).unwrap();
        let err = reconstruct_v2(&disk, &subset, &shares, 2, 100).unwrap_err();
        assert!(matches!(err, ReconstructV2Error::CauseConeMissesBulk));
    }

    #[test]
    fn both_arcs_cover_bulk_in_split_disk() {
        let disk = split_disk();
        let mut subset = BTreeSet::new();
        for c in [2, 3, 5, 6] {
            subset.insert(cid(c));
        }
        // 4 shares; k=4; encode covers all of them with high
        // initial energy.
        let shares = encode_bulk(99, 4, 4, [0xCD; 32], 1000).unwrap();
        let recovered = reconstruct_v2(&disk, &subset, &shares, 4, 100).unwrap();
        assert_eq!(recovered, 99);
    }

    // ── energy floor still bites ──────────────────────────────────

    #[test]
    fn cone_covers_but_decayed_shares_blocks() {
        // V2 requires cone coverage AND fresh shares. Cone covers,
        // but shares are decayed below floor → V1 inner check
        // fires, returns InsufficientFreshShares (wrapped as
        // InnerShamir).
        let disk = small_disk();
        let mut subset = BTreeSet::new();
        subset.insert(cid(2));
        subset.insert(cid(3));
        let mut shares = fresh_shares(42);
        for s in &mut shares {
            s.energy = 50; // below floor 100
        }
        let err = reconstruct_v2(&disk, &subset, &shares, 2, 100).unwrap_err();
        assert!(matches!(err, ReconstructV2Error::InnerShamir(_)));
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "HaPPY V2 ships the Pastawski-Yoshida-Harlow-
        // Preskill 2015 perfect-tensor topology. Reconstruction
        // requires a CONNECTED, CONE-COVERING boundary subset —
        // strictly stronger than V1's flat any-k threshold. An
        // adversary holding boundary shares scattered around the
        // disk's edge cannot reconstruct, even with high share
        // count and full freshness — until their subset's
        // causal cone covers the bulk."

        let disk = split_disk();
        let shares = encode_bulk(0xCAFE, 4, 4, [0x77; 32], 1000).unwrap();

        // Disconnected attacker: 2 shares from opposite arcs.
        // V1 would accept (k=2); V2 rejects (cone misses bulk).
        let mut split_subset = BTreeSet::new();
        split_subset.insert(cid(2));
        split_subset.insert(cid(5));
        let err = reconstruct_v2(&disk, &split_subset, &shares, 2, 100).unwrap_err();
        assert!(matches!(err, ReconstructV2Error::CauseConeMissesBulk));

        // Honest reconstruction: full boundary set.
        let mut full = BTreeSet::new();
        for c in [2, 3, 5, 6] {
            full.insert(cid(c));
        }
        let recovered = reconstruct_v2(&disk, &full, &shares, 4, 100).unwrap();
        assert_eq!(recovered, 0xCAFE);
    }
}
