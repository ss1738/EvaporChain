//! Causal cone — greedy bulk-ward expansion through majority-
//! covered cells.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::disk::{CellId, HaPPYDisk, HaPPYDiskError};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConeError {
    #[error(transparent)]
    Disk(#[from] HaPPYDiskError),
    #[error("boundary subset contains non-boundary cell {0:?}")]
    NotBoundary(CellId),
}

/// Compute the causal cone of `boundary_subset`. Greedy expansion:
///
/// 1. Start with `cone = boundary_subset`.
/// 2. Repeatedly add any cell whose **majority of neighbours** are
///    already in the cone (this models the perfect-tensor's
///    contraction property: a cell is "covered" once its
///    output legs are determined by inputs in the cone).
/// 3. Stop when no further cells can be added.
///
/// Returns the full cone set.
pub fn causal_cone(
    disk: &HaPPYDisk,
    boundary_subset: &BTreeSet<CellId>,
) -> Result<BTreeSet<CellId>, ConeError> {
    // Validate that every member of boundary_subset is actually a
    // boundary cell.
    for id in boundary_subset {
        let cell = disk
            .cell(id)
            .ok_or(ConeError::Disk(HaPPYDiskError::UnknownCell(*id)))?;
        if !cell.is_boundary {
            return Err(ConeError::NotBoundary(*id));
        }
    }

    let mut cone: BTreeSet<CellId> = boundary_subset.clone();

    loop {
        let mut added_any = false;
        // Iterate snapshot to avoid mutate-while-iterating.
        let candidates: Vec<CellId> = disk
            .cells()
            .filter(|c| !cone.contains(&c.id))
            .map(|c| c.id)
            .collect();
        for c in candidates {
            let neighs = match disk.neighbours(&c) {
                Some(n) => n,
                None => continue,
            };
            if neighs.is_empty() {
                continue;
            }
            let in_cone = neighs.iter().filter(|n| cone.contains(n)).count();
            // Majority: strict majority (> half).
            if 2 * in_cone > neighs.len() {
                cone.insert(c);
                added_any = true;
            }
        }
        if !added_any {
            break;
        }
    }

    Ok(cone)
}

/// True iff the bulk cell is in the cone of the boundary subset.
pub fn cone_covers_bulk(
    disk: &HaPPYDisk,
    boundary_subset: &BTreeSet<CellId>,
) -> Result<bool, ConeError> {
    let cone = causal_cone(disk, boundary_subset)?;
    let bulk = disk.bulk_cell().map_err(ConeError::Disk)?;
    Ok(cone.contains(&bulk.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::CellId;

    fn cid(b: u32) -> CellId {
        CellId(b)
    }

    /// Disk:
    ///   bulk(0) — interior(1) — boundary(2)
    ///                     |
    ///                  boundary(3)
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

    #[test]
    fn empty_subset_cone_is_empty() {
        let d = small_disk();
        let cone = causal_cone(&d, &BTreeSet::new()).unwrap();
        assert!(cone.is_empty());
    }

    #[test]
    fn single_boundary_does_not_cover_bulk() {
        // {2} alone: cell 1 has 3 neighbours {0, 2, 3}; only 1
        // is in the cone. Not majority. Cone stops at {2}.
        let d = small_disk();
        let mut subset = BTreeSet::new();
        subset.insert(cid(2));
        let cone = causal_cone(&d, &subset).unwrap();
        assert!(!cone.contains(&cid(0)));
        assert_eq!(cone.len(), 1);
    }

    #[test]
    fn both_boundaries_cover_bulk() {
        // {2, 3}: cell 1 has 3 neighbours {0, 2, 3}; 2 are in cone.
        // 2·2 = 4 > 3 → cell 1 added. Now cell 0's neighbours are
        // {1}; 1 in cone of 1 total; 2·1 > 1 → cell 0 added.
        // Cone covers bulk.
        let d = small_disk();
        let mut subset = BTreeSet::new();
        subset.insert(cid(2));
        subset.insert(cid(3));
        let cone = causal_cone(&d, &subset).unwrap();
        assert!(cone.contains(&cid(0)));
        assert!(cone.contains(&cid(1)));
        assert!(cone_covers_bulk(&d, &subset).unwrap());
    }

    #[test]
    fn non_boundary_in_subset_rejected() {
        let d = small_disk();
        let mut subset = BTreeSet::new();
        subset.insert(cid(1)); // interior, not boundary
        let err = causal_cone(&d, &subset).unwrap_err();
        assert_eq!(err, ConeError::NotBoundary(cid(1)));
    }

    #[test]
    fn unknown_cell_in_subset_rejected() {
        let d = small_disk();
        let mut subset = BTreeSet::new();
        subset.insert(cid(99));
        let err = causal_cone(&d, &subset).unwrap_err();
        // The 99 cell is unknown → NotBoundary or UnknownCell wrap.
        assert!(matches!(
            err,
            ConeError::Disk(HaPPYDiskError::UnknownCell(_))
        ));
    }

    // ── disconnected boundary subsets fail to cover ──────────────

    /// Larger disk: two arcs of 2 boundary cells each, with NO
    /// shared interior between them.
    ///
    ///    boundary(2) — interior(1) — bulk(0) — interior(4) — boundary(5)
    ///       |                                                   |
    ///    boundary(3)                                        boundary(6)
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
    fn one_arc_covers_bulk() {
        // {2, 3}: cell 1's neighbours = {0, 2, 3}; 2 of 3 in cone →
        // cell 1 in. Then cell 0's neighbours = {1, 4}; 1 of 2 in
        // cone → NOT majority (2·1 == 2 == 2, not strictly >).
        // Cone stalls before bulk. Single arc INSUFFICIENT.
        let d = split_disk();
        let mut subset = BTreeSet::new();
        subset.insert(cid(2));
        subset.insert(cid(3));
        assert!(!cone_covers_bulk(&d, &subset).unwrap());
    }

    #[test]
    fn both_arcs_cover_bulk() {
        // {2, 3, 5, 6}: both interiors get covered (cells 1 and 4
        // each have 2-of-3 majority), then bulk has 2-of-2 in
        // cone → 2·2 > 2 → covered.
        let d = split_disk();
        let mut subset = BTreeSet::new();
        for c in [2, 3, 5, 6] {
            subset.insert(cid(c));
        }
        assert!(cone_covers_bulk(&d, &subset).unwrap());
    }

    #[test]
    fn disconnected_partial_arcs_fail() {
        // {2, 5}: cell 1 has 1-of-3 in cone, NOT majority.
        // cell 4 has 1-of-3 in cone, NOT majority. Cone stalls
        // at the boundary subset itself.
        let d = split_disk();
        let mut subset = BTreeSet::new();
        subset.insert(cid(2));
        subset.insert(cid(5));
        assert!(!cone_covers_bulk(&d, &subset).unwrap());
    }
}
