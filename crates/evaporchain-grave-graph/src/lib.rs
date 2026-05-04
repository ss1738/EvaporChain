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

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "GraveGraph is the chain's first social structure
    /// where DEATH ADDS CONNECTIVITY. Living edges between alive
    /// parties go inert when one dies; Legacy edges declared by the
    /// living invert to Dedications on certified death of the source.
    /// Dedications cannot be created directly. Self-loops, unknown
    /// nodes, and dead-source edge-creation are all rejected."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let alice = NodeId([0xAAu8; 32]);
        let bob = NodeId([0xBBu8; 32]);

        let mut g = GraveGraph::new();
        g.register_node(alice);
        g.register_node(bob);

        // Self-loop rejected.
        assert!(matches!(
            g.add_edge(alice, alice, EdgeKind::Living, 0),
            Err(GraveGraphError::SelfLoop)
        ));

        // Unknown node rejected.
        let stranger = NodeId([0x99u8; 32]);
        assert!(matches!(
            g.add_edge(alice, stranger, EdgeKind::Living, 0),
            Err(GraveGraphError::UnknownNode(_))
        ));

        // Direct Dedication creation forbidden.
        assert!(g
            .add_edge(alice, bob, EdgeKind::Dedication { died_at_epoch: 0 }, 0)
            .is_err());

        // Legacy edge alice→bob declared while both alive.
        g.add_edge(alice, bob, EdgeKind::Legacy, 0).unwrap();

        // Certify alice dead → legacy edge inverts to Dedication.
        g.certify_death(alice, 100).unwrap();
        assert!(matches!(
            g.node_state(&alice),
            Some(NodeState::Dead { died_at_epoch: 100 })
        ));
        let dedications: Vec<_> = g.dedications_for(bob).collect();
        assert_eq!(dedications.len(), 1, "alice's legacy edge → dedication for bob");
        assert!(matches!(
            dedications[0].kind,
            EdgeKind::Dedication { died_at_epoch: 100 }
        ));

        // Cannot add new edges from a dead source.
        let carol = NodeId([0xCCu8; 32]);
        g.register_node(carol);
        assert!(matches!(
            g.add_edge(alice, carol, EdgeKind::Living, 200),
            Err(GraveGraphError::DeadSource(_))
        ));
    }
}
