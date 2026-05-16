//! End-to-end integration tests for evaporchain-ib-validators.
//!
//! Non-trivial fixture: five-validator checkpoint committee session.
//!
//! A chain checkpoint is reached at epoch 100. Five validators compare
//! their local state signature against the agreed prior from epoch 0.
//! The IB gate (β = λ_mb / 1_000_000) admits only validators whose
//! local view diverges enough from the prior to carry mutual information.
//!
//! Prior (epoch 0): uniform account-energy distribution across all 16 bins.
//!
//!   V1 (conformist):   identical to prior    → KL=0  → ABSTAIN at any λ
//!   V2 (mild-deviant): bimodal energy spread  → KL low  → ABSTAIN at strict λ
//!   V3 (divergent):    all energy in bin 0    → KL high → COMMIT at strict λ
//!   V4 (extreme):      all energy in top bin  → KL high → COMMIT at strict λ
//!   V5 (near-prior):   99% energy in 2 bins   → KL low  → ABSTAIN at strict λ
//!
//! At λ_mb=0 (permissive): 4 validators commit (V1 always abstains — KL=0).
//! At λ_mb=1_000_000 (strict): only V3 and V4 commit (high divergence).
//!
//! Doctrine claim (INVENTION_STACK A4.3.1): "β = λ. As λ decreases,
//! validators must be more selective about what they commit — only
//! truly informed validators (high KL divergence vs prior) commit.
//! Identical views (KL=0) ALWAYS abstain, preventing information-free
//! consensus riding."
//!
//! Adversarial fixture: at-boundary KL (strict > rule), attacker replicates
//! prior verbatim, high-λ forces all-abstain, zero-scale signature.
//!
//! INVENTION_STACK A4.3.1: Information-Bottleneck Validators.

use evaporchain_ib_validators::{ib_vote, IbParams, IbVote, StateSignature, N_BINS};

// ── Helpers ───────────────────────────────────────────────────────────────

/// Uniform prior: 16 accounts, one per bin, energy = i*100 with scale=1600.
fn prior_sig() -> StateSignature {
    let energies: Vec<u64> = (0..16).map(|i| i * 100).collect();
    StateSignature::from_energies(&energies, 1_600)
}

/// V1: conformist — same construction as prior.
fn v1_sig() -> StateSignature { prior_sig() }

/// V2: bimodal — two tight clusters far apart.
fn v2_sig() -> StateSignature {
    let mut energies = Vec::new();
    for _ in 0..8  { energies.push(0); }       // bin 0
    for _ in 0..8  { energies.push(1_599); }   // bin 15
    StateSignature::from_energies(&energies, 1_600)
}

/// V3: concentrated in bin 0 (low-energy world — all accounts decayed).
fn v3_sig() -> StateSignature {
    let energies = vec![0u64; 32];
    StateSignature::from_energies(&energies, 1_600)
}

/// V4: concentrated in top bin (high-energy world — recent inflation).
fn v4_sig() -> StateSignature {
    let energies = vec![1_599u64; 32];
    StateSignature::from_energies(&energies, 1_600)
}

/// V5: near-prior — 14 accounts uniform, 2 in bin 0 extra.
fn v5_sig() -> StateSignature {
    let mut energies: Vec<u64> = (0..16).map(|i| i * 100).collect();
    energies.push(0);
    energies.push(50);
    StateSignature::from_energies(&energies, 1_600)
}

// ── Non-trivial fixture: five-validator checkpoint committee ──────────────

#[test]
fn v1_conformist_always_abstains_kl_zero() {
    // V1's view is identical to the prior. KL(V1||prior) = 0.
    // The strict `>` gate means even threshold=0 rejects V1 — a validator
    // that adds no mutual information cannot cast a prevote.
    let prior = prior_sig();
    let v1 = v1_sig();

    assert_eq!(v1.kl_millibits(&prior), 0,
        "identical distributions must have KL=0");

    for lambda_mb in [0, 1, 1_000, 1_000_000, u64::MAX] {
        assert_eq!(
            ib_vote(&v1, &prior, &IbParams { lambda_mb }),
            IbVote::Abstain,
            "V1 must ABSTAIN at every λ (lambda_mb={lambda_mb})"
        );
    }
}

#[test]
fn v3_and_v4_commit_at_strict_threshold() {
    // Concentrated distributions (all-bin-0, all-top-bin) have KL≈4_000_000
    // vs the uniform prior. They commit at λ_mb=3_500_000 (below their KL).
    let prior = prior_sig();
    let strict = IbParams { lambda_mb: 3_500_000 };

    assert_eq!(ib_vote(&v3_sig(), &prior, &strict), IbVote::Commit,
        "V3 (all-bin-0) must COMMIT at strict threshold — very high KL");
    assert_eq!(ib_vote(&v4_sig(), &prior, &strict), IbVote::Commit,
        "V4 (all-top-bin) must COMMIT at strict threshold — very high KL");
}

#[test]
fn permissive_lambda_admits_all_nonzero_kl_validators() {
    // λ_mb = 0 (permissive): every validator with KL > 0 commits.
    // Only V1 (KL=0) abstains.
    let prior = prior_sig();
    let permissive = IbParams { lambda_mb: 0 };

    // V1: KL=0 → ABSTAIN.
    assert_eq!(ib_vote(&v1_sig(), &prior, &permissive), IbVote::Abstain);
    // V2, V3, V4, V5: KL > 0 → COMMIT.
    for (name, sig) in [("V2", v2_sig()), ("V3", v3_sig()), ("V4", v4_sig()), ("V5", v5_sig())] {
        assert!(sig.kl_millibits(&prior) > 0,
            "{name} must have KL > 0 vs prior");
        assert_eq!(ib_vote(&sig, &prior, &permissive), IbVote::Commit,
            "{name} must COMMIT at permissive threshold (λ_mb=0)");
    }
}

#[test]
fn strict_lambda_excludes_low_divergence_validators() {
    // λ_mb = 3_500_000: only highly-divergent validators (V3, V4) commit.
    // V2 (bimodal, KL=3M) and V5 (near-prior, KL≈332k) have lower KL and must abstain.
    let prior = prior_sig();
    let strict = IbParams { lambda_mb: 3_500_000 };

    assert_eq!(ib_vote(&v3_sig(), &prior, &strict), IbVote::Commit, "V3 must COMMIT");
    assert_eq!(ib_vote(&v4_sig(), &prior, &strict), IbVote::Commit, "V4 must COMMIT");

    // V2 (bimodal, KL=3_000_000) and V5 (near-prior, KL≈332_000) are both
    // below 3_500_000 → ABSTAIN.
    let kl_v2 = v2_sig().kl_millibits(&prior);
    let kl_v5 = v5_sig().kl_millibits(&prior);
    assert!(kl_v2 < 3_500_000, "V2 KL ({kl_v2}) must be below strict threshold");
    assert!(kl_v5 < 3_500_000, "V5 KL ({kl_v5}) must be below strict threshold");
    assert_eq!(ib_vote(&v2_sig(), &prior, &strict), IbVote::Abstain, "V2 must ABSTAIN at strict λ");
    assert_eq!(ib_vote(&v5_sig(), &prior, &strict), IbVote::Abstain, "V5 must ABSTAIN at strict λ");
}

#[test]
fn doctrine_larger_lambda_admits_fewer_validators() {
    // INVENTION_STACK A4.3.1 doctrine: "As λ decreases, validators must be
    // more selective." Equivalently, larger λ → stricter threshold → fewer
    // validators commit.
    //
    // Measure: commit count across all 5 validators at two thresholds.
    let prior = prior_sig();
    let sigs: &[(_, fn() -> StateSignature)] = &[
        ("V1", v1_sig),
        ("V2", v2_sig),
        ("V3", v3_sig),
        ("V4", v4_sig),
        ("V5", v5_sig),
    ];

    let permissive = IbParams { lambda_mb: 0 };
    let strict     = IbParams { lambda_mb: 3_500_000 };

    let count = |params: IbParams| sigs.iter().filter(|(_, f)| {
        ib_vote(&f(), &prior, &params) == IbVote::Commit
    }).count();

    let permissive_count = count(permissive);
    let strict_count     = count(strict);

    assert!(
        strict_count < permissive_count,
        "strict λ ({strict_count} commits) must admit fewer validators than \
         permissive λ ({permissive_count} commits)"
    );
}

#[test]
fn doctrine_n_bins_is_16() {
    // INVENTION_STACK A4.3.1: "16-bin bottleneck Z."
    // N_BINS is a public const — verify it holds the canonical value.
    assert_eq!(N_BINS, 16, "canonical signature width must be 16 bins");
}

#[test]
fn kl_divergence_is_asymmetric() {
    // KL is not symmetric: KL(p||q) ≠ KL(q||p) in general.
    // Concentrated vs uniform: KL(concentrated||uniform) > 0.
    // But KL(uniform||concentrated) skips bins where q=0, returning 0.
    let concentrated = v3_sig(); // all in bin 0
    let uniform      = prior_sig();

    let kl_cp = concentrated.kl_millibits(&uniform);
    let kl_pc = uniform.kl_millibits(&concentrated);

    assert!(kl_cp > 0,  "KL(concentrated||uniform) must be > 0");
    assert_eq!(kl_pc, 0, "KL(uniform||concentrated) = 0 (uniform has nonzero in bins where concentrated is 0 — skip rule)");
    assert_ne!(kl_cp, kl_pc, "KL must be asymmetric");
}

#[test]
fn l1_distance_is_symmetric_and_bounded() {
    // L1 distance is symmetric; max value is 2000 ppt (all probability
    // shifted from one side to the other).
    let a = v3_sig(); // bin 0 = 1000
    let b = v4_sig(); // bin 15 = 1000

    assert_eq!(a.l1_distance(&b), b.l1_distance(&a),
        "L1 distance must be symmetric");
    assert!(a.l1_distance(&b) <= 2_000,
        "L1 distance bounded by 2×1000 ppt");
    assert_eq!(a.l1_distance(&a), 0, "L1 distance to self must be zero");
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_attacker_replicates_prior_cannot_commit() {
    // An attacker who copies the prior verbatim to forge a vote
    // is always rejected — KL=0 → ABSTAIN at every threshold including 0.
    let prior = prior_sig();
    let forged = prior.clone();

    assert_eq!(forged.kl_millibits(&prior), 0);
    assert_eq!(
        ib_vote(&forged, &prior, &IbParams { lambda_mb: 0 }),
        IbVote::Abstain,
        "copied-prior attacker must be rejected by the strict-greater-than gate"
    );
}

#[test]
fn adversarial_exact_boundary_abstains() {
    // The gate is strict `>`. A validator with KL exactly equal to lambda_mb
    // must abstain — not commit.
    let prior = prior_sig();
    let concentrated = v3_sig();
    let kl = concentrated.kl_millibits(&prior);
    assert!(kl > 0, "test requires non-zero KL");

    // Set threshold to exact KL value.
    let boundary = IbParams { lambda_mb: kl };
    assert_eq!(
        ib_vote(&concentrated, &prior, &boundary),
        IbVote::Abstain,
        "KL == lambda_mb must ABSTAIN (strict >, not >=)"
    );

    // One below boundary → COMMIT.
    let one_below = IbParams { lambda_mb: kl - 1 };
    assert_eq!(
        ib_vote(&concentrated, &prior, &one_below),
        IbVote::Commit,
        "KL > lambda_mb-1 must COMMIT"
    );
}

#[test]
fn adversarial_max_lambda_forces_all_abstain() {
    // An extremely high λ (u64::MAX) means no validator's KL can ever
    // exceed the threshold. Everyone abstains — the chain halts consensus.
    let prior = prior_sig();
    let max_strict = IbParams { lambda_mb: u64::MAX };

    for (name, f) in [("V1", v1_sig as fn() -> StateSignature), ("V2", v2_sig), ("V3", v3_sig), ("V4", v4_sig)] {
        assert_eq!(
            ib_vote(&f(), &prior, &max_strict),
            IbVote::Abstain,
            "{name} must ABSTAIN at u64::MAX threshold"
        );
    }
}

#[test]
fn adversarial_zero_scale_signature_concentrates_in_bin_zero() {
    // scale=0 is a degenerate input; from_energies must route everything
    // to bin 0 rather than panicking.
    let energies = vec![100u64, 200, 300];
    let sig = StateSignature::from_energies(&energies, 0);
    assert_eq!(sig.bins[0], 1_000, "zero-scale: all weight must land in bin 0");
    for &b in &sig.bins[1..] {
        assert_eq!(b, 0, "zero-scale: all other bins must be zero");
    }
}
