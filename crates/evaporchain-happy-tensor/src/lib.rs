//! HaPPY V2 — perfect-tensor cell tiling of a hyperbolic disk.
//!
//! ## What V1 vs V2
//!
//! V1 (`evaporchain-happy-code`) ships the **classical analog**:
//! Shamir-style threshold sharing where any k-of-N fresh shares
//! reconstruct the bulk. Flat topology — every share is
//! interchangeable.
//!
//! V2 (this crate) ships the **HaPPY tensor-network topology**
//! (Pastawski-Yoshida-Harlow-Preskill 2015):
//!
//! - The disk is tiled by hyperbolic cells; each cell is a
//!   **`(k_in, k_out)` perfect tensor** with `k_in` "input legs"
//!   wired to neighbouring cells (or the bulk centre) and
//!   `k_out` "output legs" wired toward the boundary.
//! - **Bulk qubits** sit at central cells; **boundary qubits**
//!   sit at the disk's edge.
//! - Reconstruction requires a **connected boundary subset**
//!   `R` whose **causal cone** (greedy bulk-ward expansion
//!   through cells with majority-coverage) covers the target
//!   bulk node.
//!
//! ## Why this is strictly stronger than V1
//!
//! In V1, an attacker holding any k boundary shares (in any
//! order, anywhere on the disk) recovers the bulk. In V2, the
//! attacker must hold a *connected, contiguous arc* of boundary
//! shares of sufficient size to reach the bulk through cone
//! coverage. Disconnected boundary shards do not reconstruct,
//! even if their total count is high.
//!
//! Combined with V1's per-share energy decay, V2 gives **two
//! orthogonal anti-Sybil mechanisms**:
//! - Each share's energy must be above floor (V1 inheritance).
//! - The fresh-shares set must form a connected boundary arc
//!   that cone-covers the bulk (V2 addition).
//!
//! ## What this crate does NOT do
//!
//! - Does NOT implement the actual perfect-tensor algebra
//!   (linear algebra over a finite field). V1 of V2 ships the
//!   **topology + cone-coverage logic**; the tensor contraction
//!   is a stub that delegates to V1 Shamir for the actual bit-
//!   reconstruction.
//! - Does NOT verify a hyperbolic embedding. The disk is
//!   represented as a discrete graph: cells + edges, with
//!   "boundary" cells flagged.
//! - Does NOT model multi-bulk encoding (multiple bulk nodes
//!   per disk). V1 of V2 ships single-bulk; multi-bulk is V2.2.
//!
//! ## Module map
//!
//! - [`disk`] — [`HaPPYDisk`] cell-tiling + boundary flagging.
//! - [`cone`] — [`causal_cone`] greedy bulk-ward expansion +
//!   coverage check.
//! - [`reconstruct`] — [`reconstruct_v2`] gate: cone-cover check
//!   + delegate to V1 Shamir.

pub mod cone;
pub mod disk;
pub mod reconstruct;

pub use cone::{causal_cone, ConeError};
pub use disk::{CellId, HaPPYDisk, HaPPYDiskError};
pub use reconstruct::{reconstruct_v2, ReconstructV2Error};
