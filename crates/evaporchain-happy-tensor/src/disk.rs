//! `HaPPYDisk` — discrete cell tiling.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct CellId(pub u32);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HaPPYDiskError {
    #[error("cell {0:?} not in disk")]
    UnknownCell(CellId),
    #[error("self-edge not allowed")]
    SelfEdge,
    #[error("disk has no bulk cell registered")]
    NoBulk,
    #[error("disk has no boundary cells registered")]
    NoBoundary,
}

/// One cell in the hyperbolic tiling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub id: CellId,
    /// `is_boundary == true` means this cell sits at the disk's
    /// edge and carries an "output leg" / boundary qubit.
    pub is_boundary: bool,
    /// `is_bulk == true` means this cell carries the protected
    /// bulk qubit. Exactly one cell should be marked bulk in a
    /// single-bulk encoding (this V1 of V2 enforces).
    pub is_bulk: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HaPPYDisk {
    cells: BTreeMap<CellId, Cell>,
    /// Adjacency list: neighbours of each cell. Symmetric.
    adj: BTreeMap<CellId, BTreeSet<CellId>>,
}

impl HaPPYDisk {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_cell(&mut self, id: CellId, is_boundary: bool, is_bulk: bool) {
        self.cells.insert(
            id,
            Cell {
                id,
                is_boundary,
                is_bulk,
            },
        );
        self.adj.entry(id).or_insert_with(BTreeSet::new);
    }

    pub fn add_edge(&mut self, u: CellId, v: CellId) -> Result<(), HaPPYDiskError> {
        if u == v {
            return Err(HaPPYDiskError::SelfEdge);
        }
        if !self.cells.contains_key(&u) {
            return Err(HaPPYDiskError::UnknownCell(u));
        }
        if !self.cells.contains_key(&v) {
            return Err(HaPPYDiskError::UnknownCell(v));
        }
        self.adj.entry(u).or_default().insert(v);
        self.adj.entry(v).or_default().insert(u);
        Ok(())
    }

    pub fn cell(&self, id: &CellId) -> Option<&Cell> {
        self.cells.get(id)
    }

    pub fn neighbours(&self, id: &CellId) -> Option<&BTreeSet<CellId>> {
        self.adj.get(id)
    }

    pub fn cells(&self) -> impl Iterator<Item = &Cell> {
        self.cells.values()
    }

    pub fn boundary_cells(&self) -> impl Iterator<Item = &Cell> {
        self.cells.values().filter(|c| c.is_boundary)
    }

    pub fn bulk_cell(&self) -> Result<&Cell, HaPPYDiskError> {
        self.cells
            .values()
            .find(|c| c.is_bulk)
            .ok_or(HaPPYDiskError::NoBulk)
    }

    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(b: u32) -> CellId {
        CellId(b)
    }

    /// Build a tiny test disk:
    ///   bulk(0) — interior(1) — boundary(2)
    ///                     |
    ///                  boundary(3)
    fn small_disk() -> HaPPYDisk {
        let mut d = HaPPYDisk::new();
        d.add_cell(cid(0), false, true); // bulk
        d.add_cell(cid(1), false, false);
        d.add_cell(cid(2), true, false);
        d.add_cell(cid(3), true, false);
        d.add_edge(cid(0), cid(1)).unwrap();
        d.add_edge(cid(1), cid(2)).unwrap();
        d.add_edge(cid(1), cid(3)).unwrap();
        d
    }

    #[test]
    fn add_cell_then_edge_succeeds() {
        let d = small_disk();
        assert_eq!(d.cell_count(), 4);
        assert!(d.cell(&cid(0)).unwrap().is_bulk);
        assert!(d.cell(&cid(2)).unwrap().is_boundary);
    }

    #[test]
    fn self_edge_rejected() {
        let mut d = HaPPYDisk::new();
        d.add_cell(cid(0), false, true);
        let err = d.add_edge(cid(0), cid(0)).unwrap_err();
        assert_eq!(err, HaPPYDiskError::SelfEdge);
    }

    #[test]
    fn unknown_cell_in_edge_rejected() {
        let mut d = HaPPYDisk::new();
        d.add_cell(cid(0), false, true);
        let err = d.add_edge(cid(0), cid(99)).unwrap_err();
        assert_eq!(err, HaPPYDiskError::UnknownCell(cid(99)));
    }

    #[test]
    fn boundary_cells_iterates_correctly() {
        let d = small_disk();
        let boundaries: Vec<u32> = d.boundary_cells().map(|c| c.id.0).collect();
        assert_eq!(boundaries.len(), 2);
        assert!(boundaries.contains(&2));
        assert!(boundaries.contains(&3));
    }

    #[test]
    fn bulk_cell_lookup() {
        let d = small_disk();
        assert_eq!(d.bulk_cell().unwrap().id, cid(0));
    }

    #[test]
    fn no_bulk_errors() {
        let mut d = HaPPYDisk::new();
        d.add_cell(cid(0), true, false);
        let err = d.bulk_cell().unwrap_err();
        assert_eq!(err, HaPPYDiskError::NoBulk);
    }

    #[test]
    fn neighbours_are_symmetric() {
        let d = small_disk();
        assert!(d.neighbours(&cid(0)).unwrap().contains(&cid(1)));
        assert!(d.neighbours(&cid(1)).unwrap().contains(&cid(0)));
    }

    #[test]
    fn round_trip_serde() {
        let d = small_disk();
        let s = serde_json::to_string(&d).unwrap();
        let back: HaPPYDisk = serde_json::from_str(&s).unwrap();
        assert_eq!(back.cell_count(), 4);
        assert_eq!(back.bulk_cell().unwrap().id, cid(0));
    }
}
