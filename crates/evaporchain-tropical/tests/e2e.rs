//! §A1.4 — tropical Plücker commitment: validator-energy accountability archive e2e
//!
//! Scenario: An EvaporChain epoch snapshot tracks 5 validators' remaining energy.
//! The chain commits to the energy distribution via a tropical star-tree Plücker
//! commitment: each validator is a leaf, edge weight = −log_2(energy). A single
//! u8 hash distinguishes every distinct energy configuration.
//!
//! Doctrine: Speyer-Sturmfels 2004 — the tropical Grassmannian Gr_{2,n} =
//! space of n-leaf phylogenetic trees. The four-point condition is the
//! cut-out equation; star trees trivially satisfy it. Plücker commitment
//! = BLAKE3 over canonical row-major serialization (domain tag
//! "tropical-plucker").

use evaporchain_tropical::{
    plucker_commitment, satisfies_four_point, star_tree_distances, tropical_weight,
    TropicalMatrix, TropicalScalar,
};

// ── Validator energies (all exact powers of 2 for integer-exact weights) ──
// Weight formula: tropical_weight(2^k) = -(k)
const ALICE: u64 = 4096; // w = -12  (full stake validator)
const BOB: u64 = 1024;   // w = -10
const CAROL: u64 = 64;   // w =  -6
const DAVE: u64 = 4;     // w =  -2
const EVE: u64 = 1;      // w =   0  (barely alive)

fn validator_energies() -> Vec<u64> {
    vec![ALICE, BOB, CAROL, DAVE, EVE]
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn validator_energy_archive_full_lifecycle() {
    // Build star tree, confirm four-point, commit, mutate one validator
    // (energy decay), confirm commitment changes.
    let m = star_tree_distances(&validator_energies());

    // §A1.4 — star tree must be a valid tree-metric (Buneman four-point)
    assert!(satisfies_four_point(&m), "validator star tree must satisfy four-point condition");

    // Commitment is deterministic over repeated calls
    let c1 = plucker_commitment(&m);
    let c2 = plucker_commitment(&m);
    assert_eq!(c1, c2, "Plücker commitment must be deterministic");

    // After energy decay (BOB halves: 1024 → 512), commitment must change
    let decayed = vec![ALICE, 512, CAROL, DAVE, EVE];
    let m_decayed = star_tree_distances(&decayed);
    let c_decayed = plucker_commitment(&m_decayed);
    assert_ne!(c1, c_decayed, "commitment must change after validator energy decay");
}

#[test]
fn tropical_semiring_axioms_with_validator_energies() {
    // §A1.4 — (min, +) semiring: ZERO_T=∞ is additive identity;
    //          ONE_T=0 is multiplicative identity; mul is +.
    let w_alice = tropical_weight(ALICE); // -12
    let w_bob   = tropical_weight(BOB);   // -10

    // Additive identity: w ⊕ ∞ = w
    assert_eq!(w_alice.add(TropicalScalar::ZERO_T), w_alice);
    assert_eq!(TropicalScalar::ZERO_T.add(w_bob), w_bob);

    // Multiplicative identity: w ⊗ 0 = w
    assert_eq!(w_alice.mul(TropicalScalar::ONE_T), w_alice);
    assert_eq!(TropicalScalar::ONE_T.mul(w_bob), w_bob);

    // Tropical add = min: min(-12, -10) = -12
    assert_eq!(w_alice.add(w_bob), w_alice, "min(-12,-10) must be -12 (Alice's weight)");

    // Tropical mul = ordinary +: -12 + (-10) = -22
    assert_eq!(
        w_alice.mul(w_bob),
        TropicalScalar::finite(-22),
        "(-12) + (-10) must be -22"
    );
}

#[test]
fn weight_monotone_higher_energy_shorter_edge() {
    // §A1.4 — higher remaining energy → more-negative weight → "shorter" tropical edge.
    // In (min, +) shorter means closer to the multiplicative identity (0).
    let w_alice = tropical_weight(ALICE); // -12
    let w_bob   = tropical_weight(BOB);   // -10
    let w_carol = tropical_weight(CAROL); // -6
    let w_dave  = tropical_weight(DAVE);  // -2
    let w_eve   = tropical_weight(EVE);   //  0

    // Strictly decreasing with energy (more negative = less in ℤ = shorter edge)
    assert!(w_alice < w_bob,  "Alice (4096 energy) must have shorter edge than Bob");
    assert!(w_bob   < w_carol,"Bob (1024 energy) must have shorter edge than Carol");
    assert!(w_carol < w_dave, "Carol (64 energy) must have shorter edge than Dave");
    assert!(w_dave  < w_eve,  "Dave (4 energy) must have shorter edge than Eve");

    // Eve at energy=1 has weight exactly 0 (the multiplicative identity)
    assert_eq!(w_eve, TropicalScalar::ONE_T, "Eve's weight must equal tropical ONE_T");
}

#[test]
fn star_tree_pairwise_distances_exact() {
    // §A1.4 — d_ij = w_i + w_j (tropical mul = ordinary +) for star tree.
    let m = star_tree_distances(&validator_energies());

    // d(ALICE=0, BOB=1) = -12 + -10 = -22
    assert_eq!(m.get(0, 1), TropicalScalar::finite(-22), "d(Alice,Bob) must be -22");
    // d(ALICE=0, CAROL=2) = -12 + -6 = -18
    assert_eq!(m.get(0, 2), TropicalScalar::finite(-18), "d(Alice,Carol) must be -18");
    // d(ALICE=0, DAVE=3) = -12 + -2 = -14
    assert_eq!(m.get(0, 3), TropicalScalar::finite(-14), "d(Alice,Dave) must be -14");
    // d(ALICE=0, EVE=4) = -12 + 0 = -12
    assert_eq!(m.get(0, 4), TropicalScalar::finite(-12), "d(Alice,Eve) must be -12");
    // d(BOB=1, EVE=4) = -10 + 0 = -10
    assert_eq!(m.get(1, 4), TropicalScalar::finite(-10), "d(Bob,Eve) must be -10");
    // d(CAROL=2, DAVE=3) = -6 + -2 = -8
    assert_eq!(m.get(2, 3), TropicalScalar::finite(-8), "d(Carol,Dave) must be -8");
    // d(DAVE=3, EVE=4) = -2 + 0 = -2
    assert_eq!(m.get(3, 4), TropicalScalar::finite(-2), "d(Dave,Eve) must be -2");
}

#[test]
fn four_point_all_sums_equal_for_star_tree() {
    // §A1.4 — for a star tree, all three pairwise-sum triples are EQUAL.
    // (ALICE, BOB, CAROL, DAVE) quadruple:
    //   s1 = d_AB + d_CD = (-22) + (-8) = -30
    //   s2 = d_AC + d_BD = (-18) + (-12) = -30
    //   s3 = d_AD + d_BC = (-14) + (-16) = -30
    let m = star_tree_distances(&validator_energies());
    let s1 = m.get(0, 1).mul(m.get(2, 3)); // d_AB ⊗ d_CD = + in ℤ
    let s2 = m.get(0, 2).mul(m.get(1, 3)); // d_AC ⊗ d_BD
    let s3 = m.get(0, 3).mul(m.get(1, 2)); // d_AD ⊗ d_BC
    assert_eq!(s1, TropicalScalar::finite(-30), "s1 must be -30");
    assert_eq!(s2, TropicalScalar::finite(-30), "s2 must be -30");
    assert_eq!(s3, TropicalScalar::finite(-30), "s3 must be -30");
    assert_eq!(s1, s2, "all three pairwise sums must be equal for star tree");
    assert_eq!(s2, s3, "all three pairwise sums must be equal for star tree");
}

#[test]
fn commitment_distinguishes_energy_orderings() {
    // §A1.4 — Plücker commitment is order-sensitive (canonical row-major
    // serialization). Same multiset, different leaf order → different commitment.
    let fwd = star_tree_distances(&[ALICE, BOB, CAROL, DAVE, EVE]);
    let rev = star_tree_distances(&[EVE, DAVE, CAROL, BOB, ALICE]);
    assert_ne!(
        plucker_commitment(&fwd),
        plucker_commitment(&rev),
        "reversed energy order must produce a different commitment"
    );
}

#[test]
fn dead_validator_pulls_all_distances_to_infinity() {
    // §A1.4 — energy=0 leaf has weight +∞ (tropical zero). Tropical mul
    // with ∞ absorbs: every distance from/to that leaf is +∞.
    // The surviving pairs retain their exact distances.
    let energies = vec![ALICE, 0u64, CAROL, DAVE, EVE]; // BOB fully decayed
    let m = star_tree_distances(&energies);

    // All distances involving leaf 1 (dead BOB) must be Infinity
    for j in [0, 2, 3, 4] {
        assert_eq!(m.get(1, j), TropicalScalar::Infinity, "dead leaf must have ∞ distance");
        assert_eq!(m.get(j, 1), TropicalScalar::Infinity, "dead leaf must have ∞ distance (sym)");
    }

    // Distances among living validators are unaffected
    assert_eq!(m.get(0, 2), TropicalScalar::finite(-18), "Alice↔Carol unaffected");
    assert_eq!(m.get(3, 4), TropicalScalar::finite(-2),  "Dave↔Eve unaffected");
}

#[test]
fn energy_decay_trace_commits_distinctly() {
    // §A1.4 — simulating ALICE decaying epoch by epoch: each state produces
    // a distinct Plücker commitment, creating a verifiable decay trail.
    let epochs: &[u64] = &[4096, 2048, 1024, 512, 256, 128, 64, 32, 16, 8, 4, 2, 1, 0];
    let mut commitments = Vec::new();
    for &alice_e in epochs {
        let m = star_tree_distances(&[alice_e, BOB, CAROL, DAVE, EVE]);
        commitments.push(plucker_commitment(&m));
    }
    // All commitments before the dead epoch must be distinct
    let living: Vec<_> = commitments[..commitments.len() - 1].iter().collect();
    let mut dedup = living.clone();
    dedup.dedup();
    assert_eq!(dedup.len(), living.len(), "each decay epoch must produce a distinct commitment");
    // The dead epoch (energy=0) differs from all living epochs
    let dead_c = *commitments.last().unwrap();
    for c in &commitments[..commitments.len() - 1] {
        assert_ne!(*c, dead_c, "dead-epoch commitment must differ from all living");
    }
}

#[test]
fn adversarial_non_tree_metric_fails_four_point() {
    // §A1.4 adversarial — a metric that is NOT a tree-metric violates
    // the four-point condition.  Construct a 4×4 symmetric matrix with
    // three DISTINCT pairwise sums (so max is achieved exactly once → fails).
    //
    //  d_01=1, d_02=2, d_03=3, d_12=4, d_13=5, d_23=99
    //  Sums for (0,1,2,3):
    //    s1 = d_01 + d_23 = 1 + 99 = 100
    //    s2 = d_02 + d_13 = 2 + 5 = 7
    //    s3 = d_03 + d_12 = 3 + 4 = 7
    //  Max=100 achieved once → four-point fails.
    let mut m = TropicalMatrix::new(4);
    let entries = [
        (0, 1, 1i64), (0, 2, 2), (0, 3, 3),
        (1, 2, 4),    (1, 3, 5), (2, 3, 99),
    ];
    for (i, j, v) in entries {
        m.set(i, j, TropicalScalar::finite(v));
        m.set(j, i, TropicalScalar::finite(v));
    }
    assert!(
        !satisfies_four_point(&m),
        "adversarial non-tree metric must violate four-point condition"
    );
}

#[test]
fn commitment_changes_on_single_entry_mutation() {
    // §A1.4 — altering a single matrix entry changes the Plücker commitment.
    let mut m = star_tree_distances(&validator_energies());
    let c0 = plucker_commitment(&m);
    m.set(0, 1, TropicalScalar::finite(-999));
    let c1 = plucker_commitment(&m);
    assert_ne!(c0, c1, "single entry mutation must change commitment");
}

#[test]
fn weight_extremes_exact() {
    // §A1.4 — boundary weights: energy=1 → w=0; energy=u64::MAX → w=-63.
    assert_eq!(tropical_weight(1), TropicalScalar::finite(0),   "w(1) = 0");
    assert_eq!(tropical_weight(2), TropicalScalar::finite(-1),  "w(2) = -1");
    assert_eq!(tropical_weight(4), TropicalScalar::finite(-2),  "w(4) = -2");
    assert_eq!(tropical_weight(1024), TropicalScalar::finite(-10), "w(1024) = -10");
    assert_eq!(tropical_weight(u64::MAX), TropicalScalar::finite(-63), "w(u64::MAX) = -63");
    assert_eq!(tropical_weight(0), TropicalScalar::Infinity, "w(0) = ∞");
}

#[test]
fn star_tree_symmetric_and_infinity_diagonal() {
    // §A1.4 — well-formedness: star tree is symmetric; diagonal is +∞
    //          (no self-distance defined in the star-tree topology).
    let m = star_tree_distances(&validator_energies());
    assert!(m.is_symmetric(), "star tree must be symmetric");
    for i in 0..5 {
        assert_eq!(m.get(i, i), TropicalScalar::Infinity, "diagonal must be Infinity");
    }
}
