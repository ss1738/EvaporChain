//! End-to-end integration tests for evaporchain-grave-graph.
//!
//! Non-trivial fixture: a literary estate social-graph lifecycle modelling
//! an author's posthumous footprint and survivor curation.
//!
//! Doctrine claim (INVENTION_STACK §A5.5 — GraveGraph / Singh Mortis):
//!   "GraveGraph is the chain's first social structure where DEATH ADDS
//!   CONNECTIVITY. Living edges between alive parties go inert when one
//!   dies; Legacy edges declared by the living invert to Dedications on
//!   certified death of the source. Dedications cannot be created directly.
//!   Self-loops, unknown nodes, and dead-source edge-creation are all
//!   rejected."
//!
//! Literary-estate scenario:
//!
//!   Nodes:
//!     ALICE  (0xAA) — author; declares edges before death
//!     BOB    (0xBB) — close friend; receives Legacy dedication; accepts it
//!     CAROL  (0xCC) — literary colleague; receives Legacy dedication; rejects
//!     DAVE   (0xDD) — estranged nephew; receives Living edge only (no legacy)
//!     EVE    (0xEE) — fan; no edges from Alice
//!     FRANK  (0xFF) — fellow author; also dies independently
//!
//!   Pre-death graph (epoch 0):
//!     Alice → Bob:   Legacy       (explicitly posthumous)
//!     Alice → Carol: Legacy       (explicitly posthumous)
//!     Alice → Dave:  Living       (alive relationship, no legacy)
//!     Bob   → Alice: Living       (friendship is mutual; Bob stays alive)
//!     Frank → Carol: Legacy       (Frank's own posthumous declaration)
//!
//!   Alice's certified death (epoch 100):
//!     Alice → Dave:  Living edge CLEARED
//!     Alice → Bob:   Legacy → Dedication{died_at_epoch:100}
//!     Alice → Carol: Legacy → Dedication{died_at_epoch:100}
//!
//!   Post-death:
//!     Alice's footprint: 2 dedications (Bob, Carol).
//!     Bob curates: Accepted (dedication displays on Bob's profile).
//!     Carol curates: Rejected (dedication hidden on Carol's profile).
//!     Alice cannot add new edges (DeadSource).
//!     Bob→Alice Living edge unaffected (Bob is still alive).
//!
//!   Frank's certified death (epoch 200):
//!     Carol → Frank: Legacy → Dedication{died_at_epoch:200}
//!     Footprints are fully independent — Alice's 2 dedications unchanged.

use evaporchain_grave_graph::{Curation, EdgeKind, GraveGraph, GraveGraphError, NodeId, NodeState};

// ── Node IDs ──────────────────────────────────────────────────────────────────

const ALICE: u8 = 0xAA;
const BOB: u8 = 0xBB;
const CAROL: u8 = 0xCC;
const DAVE: u8 = 0xDD;
const EVE: u8 = 0xEE;
const FRANK: u8 = 0xFF;

fn n(tag: u8) -> NodeId {
    NodeId([tag; 32])
}

// ── Fixture builder ───────────────────────────────────────────────────────────

/// Pre-death state with all 6 nodes registered and Alice's + Carol's edges.
fn pre_death_graph() -> GraveGraph {
    let mut g = GraveGraph::new();
    for tag in [ALICE, BOB, CAROL, DAVE, EVE, FRANK] {
        g.register_node(n(tag));
    }
    // Alice's outgoing edges.
    g.add_edge(n(ALICE), n(BOB), EdgeKind::Legacy, 0).unwrap();
    g.add_edge(n(ALICE), n(CAROL), EdgeKind::Legacy, 0).unwrap();
    g.add_edge(n(ALICE), n(DAVE), EdgeKind::Living, 0).unwrap();
    // Bob's reciprocal friendship.
    g.add_edge(n(BOB), n(ALICE), EdgeKind::Living, 0).unwrap();
    // Frank's own posthumous declaration to Carol.
    g.add_edge(n(FRANK), n(CAROL), EdgeKind::Legacy, 0).unwrap();
    g
}

// ── Main fixture: full literary-estate lifecycle ──────────────────────────────

#[test]
fn author_social_network_full_lifecycle() {
    let mut g = pre_death_graph();

    // Pre-death: Alice has 3 outgoing edges; Bob + Carol + Dave visible.
    assert_eq!(g.outgoing(n(ALICE)).count(), 3);
    assert_eq!(
        g.footprint_of(n(ALICE)).count(),
        0,
        "pre-death: no dedications in Alice's footprint"
    );

    // ── Alice's certified death at epoch 100 ─────────────────────────
    let inverted = g.certify_death(n(ALICE), 100).unwrap();
    assert_eq!(
        inverted, 2,
        "exactly 2 Legacy edges inverted: Alice→Bob and Alice→Carol"
    );

    // Alice is now Dead.
    assert!(matches!(
        g.node_state(&n(ALICE)),
        Some(NodeState::Dead { died_at_epoch: 100 })
    ));

    // Alice→Dave (Living) was cleared. Alice now has exactly 2 outgoing (dedications).
    assert_eq!(g.outgoing(n(ALICE)).count(), 2);

    // Posthumous footprint = 2.
    let footprint: Vec<_> = g.footprint_of(n(ALICE)).collect();
    assert_eq!(footprint.len(), 2, "footprint has exactly 2 dedications");
    for e in &footprint {
        assert!(
            matches!(e.kind, EdgeKind::Dedication { died_at_epoch: 100 }),
            "all footprint entries must be Dedication{{died_at_epoch:100}}"
        );
        assert_eq!(
            e.curation,
            Curation::Pending,
            "uncurated dedications start Pending"
        );
    }

    // Bob→Alice Living edge is UNAFFECTED (Bob is still alive).
    assert_eq!(
        g.outgoing(n(BOB)).count(),
        1,
        "Bob's Living edge to Alice persists after Alice's death"
    );
    assert!(matches!(
        g.outgoing(n(BOB)).next().unwrap().kind,
        EdgeKind::Living
    ));

    // ── Survivor curation: Bob accepts, Carol rejects ─────────────────
    g.curate_dedication(n(BOB), n(ALICE), Curation::Accepted)
        .unwrap();
    g.curate_dedication(n(CAROL), n(ALICE), Curation::Rejected)
        .unwrap();

    // Both dedications still exist on chain (structural intent preserved).
    assert_eq!(
        g.footprint_of(n(ALICE)).count(),
        2,
        "curation does not delete the dedication — only decorates it"
    );

    // Bob's copy: Accepted.
    let bob_dedn = g.dedications_for(n(BOB)).next().unwrap();
    assert_eq!(bob_dedn.curation, Curation::Accepted);

    // Carol's copy: Rejected.
    let carol_dedn = g.dedications_for(n(CAROL)).next().unwrap();
    assert_eq!(carol_dedn.curation, Curation::Rejected);

    // ── Alice cannot declare new edges ────────────────────────────────
    let err = g
        .add_edge(n(ALICE), n(EVE), EdgeKind::Living, 150)
        .unwrap_err();
    assert!(
        matches!(err, GraveGraphError::DeadSource(_)),
        "dead source must be rejected with DeadSource error"
    );

    // ── Frank's independent death at epoch 200 ────────────────────────
    let f_inverted = g.certify_death(n(FRANK), 200).unwrap();
    assert_eq!(
        f_inverted, 1,
        "Frank's Legacy→Carol inverts to Dedication on Frank's death"
    );

    // Alice's footprint unchanged.
    assert_eq!(
        g.footprint_of(n(ALICE)).count(),
        2,
        "Alice's footprint is independent of Frank's death"
    );

    // Frank's footprint = 1 (his own dedication to Carol).
    assert_eq!(g.footprint_of(n(FRANK)).count(), 1);
}

// ── Doctrine claim: death adds connectivity ───────────────────────────────────

#[test]
fn death_adds_connectivity_pre_and_post_footprint() {
    let mut g = pre_death_graph();

    let pre_footprint = g.footprint_of(n(ALICE)).count();
    assert_eq!(
        pre_footprint, 0,
        "pre-death: no posthumous footprint exists"
    );

    g.certify_death(n(ALICE), 100).unwrap();

    let post_footprint = g.footprint_of(n(ALICE)).count();
    assert_eq!(
        post_footprint, 2,
        "post-death: footprint = 2 dedications (Bob + Carol)"
    );

    assert!(
        post_footprint > pre_footprint,
        "DOCTRINE: death added connectivity — post({post_footprint}) > pre({pre_footprint})"
    );
}

// ── Living edge cleared on source death ──────────────────────────────────────

#[test]
fn living_edge_cleared_on_death_dave_loses_alice_connection() {
    let mut g = pre_death_graph();

    // Dave receives a Living edge from Alice.
    assert_eq!(
        g.incoming(n(DAVE)).count(),
        1,
        "Dave has one incoming (Alice Living)"
    );

    g.certify_death(n(ALICE), 100).unwrap();

    // After Alice's death the Living edge Alice→Dave is gone.
    assert_eq!(
        g.incoming(n(DAVE)).count(),
        0,
        "Living edge Alice→Dave must be cleared on Alice's certified death"
    );
    assert_eq!(
        g.dedications_for(n(DAVE)).count(),
        0,
        "Dave received no Legacy edge — no dedication for him"
    );
}

// ── Legacy → Dedication inversion is exact ───────────────────────────────────

#[test]
fn legacy_inverts_to_dedication_with_correct_epoch() {
    let mut g = pre_death_graph();
    g.certify_death(n(ALICE), 100).unwrap();

    for edge in g.footprint_of(n(ALICE)) {
        assert!(
            matches!(edge.kind, EdgeKind::Dedication { died_at_epoch: 100 }),
            "inverted edge must carry died_at_epoch=100, got {:?}",
            edge.kind
        );
    }
}

// ── Inversion is irreversible: the dead cannot revoke ────────────────────────

#[test]
fn dead_source_cannot_add_or_revoke_edges() {
    let mut g = pre_death_graph();
    g.certify_death(n(ALICE), 100).unwrap();

    // Attempt: Alice adds a new Living edge.
    assert!(matches!(
        g.add_edge(n(ALICE), n(EVE), EdgeKind::Living, 101),
        Err(GraveGraphError::DeadSource(_))
    ));

    // Attempt: Alice adds a Legacy edge.
    assert!(matches!(
        g.add_edge(n(ALICE), n(EVE), EdgeKind::Legacy, 101),
        Err(GraveGraphError::DeadSource(_))
    ));

    // Alice's footprint is still intact — the 2 dedications she declared in life persist.
    assert_eq!(
        g.footprint_of(n(ALICE)).count(),
        2,
        "existing dedications survive regardless of blocked new-edge attempts"
    );
}

// ── Survivor curation is independent and non-destructive ─────────────────────

#[test]
fn survivor_curation_independent_does_not_delete_dedication() {
    let mut g = pre_death_graph();
    g.certify_death(n(ALICE), 100).unwrap();

    // Bob hides; Carol accepts.
    g.curate_dedication(n(BOB), n(ALICE), Curation::Hidden)
        .unwrap();
    g.curate_dedication(n(CAROL), n(ALICE), Curation::Accepted)
        .unwrap();

    // On-chain count is still 2 (rejection/hiding is decoration, not deletion).
    assert_eq!(g.footprint_of(n(ALICE)).count(), 2);

    // Bob's curation doesn't affect Carol's.
    let carol_d = g.dedications_for(n(CAROL)).next().unwrap();
    assert_eq!(
        carol_d.curation,
        Curation::Accepted,
        "Bob's Hidden curation must not affect Carol's Accepted curation"
    );
}

// ── Dead recipient cannot curate ─────────────────────────────────────────────

#[test]
fn dead_recipient_cannot_curate_dedication() {
    let mut g = pre_death_graph();
    g.certify_death(n(ALICE), 100).unwrap();

    // Bob also dies before curating.
    g.certify_death(n(BOB), 150).unwrap();

    let err = g
        .curate_dedication(n(BOB), n(ALICE), Curation::Accepted)
        .unwrap_err();
    assert!(
        matches!(err, GraveGraphError::NotRecipient),
        "dead recipient must not be allowed to curate"
    );
}

// ── Multiple legacy edges: all invert on single certify_death ─────────────────

#[test]
fn multiple_legacy_edges_all_invert_simultaneously() {
    let mut g = GraveGraph::new();
    for tag in [ALICE, BOB, CAROL, DAVE, EVE] {
        g.register_node(n(tag));
    }
    // Alice leaves legacy to Bob, Carol, Dave, Eve — 4 legacy edges.
    for target in [BOB, CAROL, DAVE, EVE] {
        g.add_edge(n(ALICE), n(target), EdgeKind::Legacy, 0)
            .unwrap();
    }

    let inverted = g.certify_death(n(ALICE), 50).unwrap();
    assert_eq!(
        inverted, 4,
        "all 4 legacy edges must invert on one certify_death"
    );
    assert_eq!(
        g.footprint_of(n(ALICE)).count(),
        4,
        "footprint must include all 4 dedications"
    );

    // Each surviving node receives exactly 1 dedication.
    for target in [BOB, CAROL, DAVE, EVE] {
        assert_eq!(
            g.dedications_for(n(target)).count(),
            1,
            "each recipient must have exactly 1 dedication from Alice"
        );
    }
}

// ── Posthumous footprint = only Legacy (not Living) edges ────────────────────

#[test]
fn footprint_bounded_to_legacy_not_living_edges() {
    let mut g = GraveGraph::new();
    for tag in [ALICE, BOB, CAROL, DAVE] {
        g.register_node(n(tag));
    }
    // Alice declares 2 legacy and 2 living edges.
    g.add_edge(n(ALICE), n(BOB), EdgeKind::Legacy, 0).unwrap();
    g.add_edge(n(ALICE), n(CAROL), EdgeKind::Legacy, 0).unwrap();
    g.add_edge(n(ALICE), n(DAVE), EdgeKind::Living, 0).unwrap();

    g.certify_death(n(ALICE), 10).unwrap();

    // Footprint = only the 2 legacies; the 1 living edge was cleared.
    assert_eq!(
        g.footprint_of(n(ALICE)).count(),
        2,
        "footprint must not include cleared Living edges"
    );
    assert_eq!(
        g.dedications_for(n(DAVE)).count(),
        0,
        "Dave received no legacy → no dedication for him"
    );
}

// ── Two independent deaths: footprints are fully isolated ────────────────────

#[test]
fn two_independent_deaths_have_separate_footprints() {
    let mut g = pre_death_graph();

    // Alice dies first.
    g.certify_death(n(ALICE), 100).unwrap();
    assert_eq!(g.footprint_of(n(ALICE)).count(), 2);
    assert_eq!(g.footprint_of(n(FRANK)).count(), 0);

    // Frank dies later; Frank's Legacy→Carol inverts.
    g.certify_death(n(FRANK), 200).unwrap();
    assert_eq!(
        g.footprint_of(n(FRANK)).count(),
        1,
        "Frank's footprint = his own dedication to Carol"
    );

    // Alice's footprint is unchanged.
    assert_eq!(
        g.footprint_of(n(ALICE)).count(),
        2,
        "Alice's footprint must be unaffected by Frank's death"
    );
}

// ── Adversarial: direct Dedication creation is forbidden ─────────────────────

#[test]
fn direct_dedication_creation_rejected_at_all_epochs() {
    let mut g = GraveGraph::new();
    g.register_node(n(ALICE));
    g.register_node(n(BOB));

    // Even before any death, directly creating a Dedication must be rejected.
    let err = g
        .add_edge(
            n(ALICE),
            n(BOB),
            EdgeKind::Dedication { died_at_epoch: 0 },
            0,
        )
        .unwrap_err();
    assert!(
        matches!(err, GraveGraphError::DeadSource(_)),
        "direct Dedication creation must always be rejected (only via inversion)"
    );
}

// ── Adversarial: certify death of unknown node rejected ──────────────────────

#[test]
fn certify_death_unknown_node_rejected() {
    let mut g = GraveGraph::new();
    let err = g.certify_death(n(ALICE), 50).unwrap_err();
    assert!(
        matches!(err, GraveGraphError::UnknownNode(_)),
        "certify_death on an unregistered node must be rejected"
    );
}

// ── Adversarial: Eve tries to curate a dedication she never received ──────────

#[test]
fn non_recipient_cannot_curate_dedication() {
    let mut g = pre_death_graph();
    g.certify_death(n(ALICE), 100).unwrap();

    // Eve has no edge from Alice — there is no dedication to curate.
    let err = g
        .curate_dedication(n(EVE), n(ALICE), Curation::Accepted)
        .unwrap_err();
    assert!(
        matches!(err, GraveGraphError::EdgeNotFound { .. }),
        "non-recipient curate attempt must yield EdgeNotFound"
    );
}

// ── Graph-level edge count invariant under death ──────────────────────────────

#[test]
fn edge_count_invariant_living_cleared_dedications_added() {
    let mut g = GraveGraph::new();
    for tag in [ALICE, BOB, CAROL, DAVE] {
        g.register_node(n(tag));
    }
    // 2 legacy + 1 living from Alice; 1 living from Bob (reciprocal).
    g.add_edge(n(ALICE), n(BOB), EdgeKind::Legacy, 0).unwrap();
    g.add_edge(n(ALICE), n(CAROL), EdgeKind::Legacy, 0).unwrap();
    g.add_edge(n(ALICE), n(DAVE), EdgeKind::Living, 0).unwrap();
    g.add_edge(n(BOB), n(ALICE), EdgeKind::Living, 0).unwrap();
    assert_eq!(g.edge_count(), 4);

    // Alice dies: 1 Living (Alice→Dave) cleared; 2 Legacy inverted (no net change in count).
    g.certify_death(n(ALICE), 100).unwrap();
    // Edge count = 2 (Alice→Bob dedication) + 1 (Alice→Carol dedication) + 1 (Bob→Alice Living) = 3.
    // Wait: Alice→Dave Living is cleared (−1), Alice→Bob stays (Legacy→Dedication), Alice→Carol stays.
    // Bob→Alice Living is unaffected. Total: 4 − 1 = 3.
    assert_eq!(
        g.edge_count(),
        3,
        "death removes Living edges from dead source; Legacy edges become Dedications (same count)"
    );
}
