//! GraveGraph — mortality-aware social network primitive.
//!
//! ## What this is
//!
//! A directed-graph data structure where users declare two kinds
//! of edges:
//!
//! - **Living edge**: `from → to` while both are alive.
//!   Bidirectional discoverability — both endpoints can list it.
//! - **Legacy edge**: `from → to` where `from` declares this
//!   edge survives their death. On certified death of `from`,
//!   the edge becomes a **dedication**: a Dead→Living
//!   relationship the surviving `to` party can curate (accept,
//!   reject, hide).
//!
//! The chain's first social structure where **death adds
//! connectivity**: a person's posthumous footprint is the set
//! of dedications they made + the dedications others made
//! about them.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Edge inversion is one-way.** Living→Dead edges become
//!    Dead→Living dedications on certified death. Once
//!    inverted, the edge cannot be uninverted (the dead can't
//!    revoke).
//!
//! 2. **Survivor curation is bounded.** A surviving recipient
//!    of a dedication can mark it `Accepted`, `Rejected`, or
//!    `Hidden`. The chain stores all three; rendering layers
//!    decide which to show. Rejection is structural — the
//!    survivor cannot rewrite the dead's intent, only refuse
//!    to display it on their own profile.
//!
//! 3. **No edges from non-existing nodes.** Adding an edge
//!    requires both nodes registered first. Prevents
//!    impersonation by edge-spam.
//!
//! ## What this crate does NOT do
//!
//! - Does NOT verify death certificates. Caller passes a
//!    flag; chain's higher layer validates the m-of-n
//!    threshold attestation.
//! - Does NOT model edge weights / temporal decay. V1 edges
//!    are present-or-absent; weighted-by-engagement is V2.
//! - Does NOT model multi-graph (multiple edges between same
//!    pair). V1 single-edge per (from, to, kind).
//!
//! ## Module map
//!
//! - [`graph`] — [`GraveGraph`] state machine.

pub mod graph;

pub use graph::{
    Curation, EdgeKind, GraveGraph, GraveGraphError, NodeId, NodeState,
};
