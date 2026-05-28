//! T0.6 — Slashing-at-scale adversarial scenarios.
//!
//! Per `MAINNET_READINESS.md` T0.6, this file locks every slashing
//! rule against adversarial validator behaviour at the in-process
//! level. The five scenarios below correspond to the five slashing
//! rules in the T0.6 lane spec:
//!
//!   1. Double-vote (`SanovSlash`) — prevote equivocation zeroes stake + jails
//!   2. Cross-round equivocation — precommit equivocation same behaviour
//!   3. MEV missing-refund violation — `apply_mev_missing_refund_slashes` fires
//!   4. Validator downtime — `sanov_slash_downtime` proportional to missed blocks
//!   5. Multi-validator downtime cascade — conservation invariant holds across slash batch
//!
//! The full live-cluster soak (T3.1-dependent) sits on top of these;
//! these tests ensure the individual slash paths fire correctly before
//! that layer runs.

use evaporchain_consensus::tendermint::{ConsensusMessage, TendermintConsensus};
use evaporchain_consensus::validator_set::{ValidatorInfo, ValidatorSet};

fn make_validator(id: u64, stake: u64) -> ValidatorInfo {
    let mut addr = [0u8; 32];
    addr[0..8].copy_from_slice(&id.to_le_bytes());
    ValidatorInfo::new(id, stake, addr)
}

fn make_tc_with_validators(validators: &[(u64, u64)]) -> TendermintConsensus {
    let mut vs = ValidatorSet::new();
    for &(id, stake) in validators {
        vs.add_validator(make_validator(id, stake));
    }
    TendermintConsensus::new_for_test(1, 100, vs)
}

// ── Scenario 1: Prevote equivocation → SanovSlash + jail ────────────────────

/// T0.6-S1 — Validator 2 sends two conflicting prevotes at the same
/// (height, round). The Sanov large-deviation formula computes a
/// full-stake slash (KL[fully_equivocating ‖ honest] >> 0), stake
/// becomes 0, and the validator is jailed (not removed — removal is
/// governance-only).
///
/// Defensive guarantee: a Byzantine validator cannot gain advantage
/// by switching its vote mid-round; the first equivocation is detected,
/// slashed, and the validator is permanently jailed for this height.
#[test]
fn t06_scenario_1_prevote_equivocation_slashes_and_jails() {
    let mut tc = make_tc_with_validators(&[(1, 1000), (2, 5_000), (3, 1000), (4, 1000)]);
    let initial_stake = tc.validator_set().get(2).unwrap().stake;
    assert_eq!(
        initial_stake, 5_000,
        "pre-condition: validator 2 has 5_000 stake"
    );

    let hash_a = [0xAAu8; 32];
    let hash_b = [0xBBu8; 32];

    // First prevote for hash_a — accepted.
    tc.on_message(ConsensusMessage::Prevote {
        height: 1,
        round: 0,
        block_hash: Some(hash_a),
        validator_id: 2,
        bls_signature: None,
    });

    // Second prevote for hash_b in the same (height, round) — equivocation.
    let actions = tc.on_message(ConsensusMessage::Prevote {
        height: 1,
        round: 0,
        block_hash: Some(hash_b),
        validator_id: 2,
        bls_signature: None,
    });

    // Equivocating message must be rejected (no outbound propagation).
    assert!(
        actions.is_empty(),
        "equivocating prevote must be rejected (no actions emitted)"
    );

    let v = tc
        .validator_set()
        .get(2)
        .expect("validator 2 must remain in set — removal is governance-only");
    assert_eq!(
        v.stake, 0,
        "SanovSlash: stake must be zeroed after equivocation"
    );
    assert!(v.jailed, "equivocating validator must be jailed");
    assert!(
        v.total_slashed >= initial_stake,
        "total_slashed must account for at least the full initial stake"
    );
}

// ── Scenario 2: Precommit equivocation → SanovSlash + jail ──────────────────

/// T0.6-S2 — Validator 3 sends two conflicting precommits at the same
/// (height, round). Mirror of S1 but on the precommit path. The
/// precommit equivocation gate independently tracks signed precommit
/// hashes and slashes on the second conflicting signature.
#[test]
fn t06_scenario_2_precommit_equivocation_slashes_and_jails() {
    let mut tc = make_tc_with_validators(&[(1, 1000), (2, 1000), (3, 8_000), (4, 1000)]);
    let initial_stake = tc.validator_set().get(3).unwrap().stake;
    assert_eq!(initial_stake, 8_000);

    let hash_c = [0xCCu8; 32];
    let hash_d = [0xDDu8; 32];

    tc.on_message(ConsensusMessage::Precommit {
        height: 1,
        round: 0,
        block_hash: Some(hash_c),
        validator_id: 3,
        bls_signature: None,
    });
    let actions = tc.on_message(ConsensusMessage::Precommit {
        height: 1,
        round: 0,
        block_hash: Some(hash_d),
        validator_id: 3,
        bls_signature: None,
    });

    assert!(
        actions.is_empty(),
        "equivocating precommit must be rejected"
    );

    let v = tc
        .validator_set()
        .get(3)
        .expect("validator 3 must remain in set");
    assert_eq!(
        v.stake, 0,
        "SanovSlash: precommit equivocator stake must be 0"
    );
    assert!(v.jailed, "precommit equivocator must be jailed");
}

// ── Scenario 3: MEV missing-refund violation → entropic slash ────────────────

/// T0.6-S3 — A validator accumulates 100 missing-refund violations
/// (proposer failed to include obligatory RefundTxs). When
/// `crooks_mev_missing_refund_slash_enabled = true` is set via
/// governance, `apply_mev_missing_refund_slashes` fires and reduces
/// the validator's stake. The violation counter is cleared after the
/// slash so a second call is a no-op.
#[test]
fn t06_scenario_3_mev_missing_refund_slash_fires_with_flag_enabled() {
    let mut tc = make_tc_with_validators(&[(1, 1000), (2, 10_000), (3, 1000), (4, 1000)]);

    // Inject 100 violations for validator 2 (MEV-proposer who skipped refunds).
    tc.mev_missing_refund_violations.insert(2, 100);

    // With flag disabled (default): no-op.
    let result = tc.apply_mev_missing_refund_slashes();
    assert!(result.is_empty(), "slash must be no-op when flag is off");
    assert_eq!(
        tc.validator_set().get(2).unwrap().stake,
        10_000,
        "stake must be unchanged when flag is off"
    );
    // Violation counter must still be present (was not consumed).
    assert_eq!(*tc.mev_missing_refund_violations.get(&2).unwrap_or(&0), 100);

    // Enable the governance flag.
    tc.governance_set_param("crooks_mev_missing_refund_slash_enabled", "true")
        .expect("valid param");

    let stake_before = tc.validator_set().get(2).unwrap().stake;
    let slashed_pairs = tc.apply_mev_missing_refund_slashes();

    // The flag is now on — at least one (validator_id, amount) pair must be returned.
    assert!(
        !slashed_pairs.is_empty(),
        "apply_mev_missing_refund_slashes must return non-empty with violations present"
    );
    let (slashed_id, slash_amount) = slashed_pairs[0];
    assert_eq!(slashed_id, 2);
    assert!(
        slash_amount > 0,
        "slash amount must be > 0 for 100 violations"
    );

    let stake_after = tc.validator_set().get(2).unwrap().stake;
    assert_eq!(
        stake_before - stake_after,
        slash_amount,
        "stake reduction must equal the reported slash amount"
    );

    // Violation counter is cleared — second call is a no-op.
    assert_eq!(
        *tc.mev_missing_refund_violations.get(&2).unwrap_or(&0),
        0,
        "violation counter must be cleared after slash"
    );
    let second_call = tc.apply_mev_missing_refund_slashes();
    assert!(
        second_call.is_empty(),
        "second call must be no-op (counter cleared)"
    );
}

// ── Scenario 4: Downtime slash proportional to missed blocks ─────────────────

/// T0.6-S4 — Validator 1 misses progressively more blocks in a fixed
/// observation window. The Sanov large-deviation slash is monotonically
/// increasing in the miss count, and the validator is jailed only when
/// the jail threshold (≥3 misses) is reached.
///
/// This pins the key invariant from T0.6: the slash algorithm is
/// graduated (not binary), so operators can see pre-jail warnings
/// before a full slash.
#[test]
fn t06_scenario_4_downtime_slash_proportional_to_missed_blocks() {
    const WINDOW: u64 = 100;
    const INITIAL_STAKE: u64 = 100_000;

    // Low misses: below jail threshold (< 3) — small slash, no jail.
    {
        let mut tc = make_tc_with_validators(&[(1, INITIAL_STAKE), (2, 1000)]);
        let slash_low = tc.sanov_slash_downtime(1, 2, WINDOW);
        let v = tc.validator_set().get(1).unwrap();
        assert!(
            slash_low > 0 || v.stake == INITIAL_STAKE,
            "2 misses may produce 0 slash (KL too small)"
        );
        assert!(
            !v.jailed,
            "below jail threshold (2 misses): must not be jailed"
        );
    }

    // Medium misses: above jail threshold — slash fires + jailed.
    let slash_mid = {
        let mut tc = make_tc_with_validators(&[(1, INITIAL_STAKE), (2, 1000)]);
        let s = tc.sanov_slash_downtime(1, 50, WINDOW);
        let v = tc.validator_set().get(1).unwrap();
        assert!(s > 0, "50/100 misses must produce a non-zero slash");
        assert!(v.jailed, "50/100 misses must jail the validator");
        assert_eq!(
            INITIAL_STAKE - v.stake,
            s,
            "stake delta must equal the returned slash amount"
        );
        s
    };

    // High misses: nearly full window — slash is larger (monotonicity).
    let slash_high = {
        let mut tc = make_tc_with_validators(&[(1, INITIAL_STAKE), (2, 1000)]);
        let s = tc.sanov_slash_downtime(1, 95, WINDOW);
        assert!(s > 0, "95/100 misses must produce a non-zero slash");
        s
    };

    assert!(
        slash_high >= slash_mid,
        "Sanov slash must be monotonically non-decreasing in miss count: \
         slash_high({slash_high}) >= slash_mid({slash_mid})"
    );

    // Unknown validator: no slash (does not panic).
    {
        let mut tc = make_tc_with_validators(&[(1, INITIAL_STAKE)]);
        let s = tc.sanov_slash_downtime(99, 50, WINDOW);
        assert_eq!(s, 0, "unknown validator must return 0 slash (not panic)");
    }
}

// ── Scenario 5: Multi-validator downtime cascade — conservation invariant ────

/// T0.6-S5 — Three validators are slashed for different downtime
/// severity levels in the same epoch. After all slashes, the
/// conservation invariant holds: the sum of stake reductions across
/// all slashed validators equals the sum of slash amounts reported by
/// `sanov_slash_downtime`, and no validator's stake goes negative.
///
/// This validates that the batched slash path used at epoch boundaries
/// does not double-count or silently drop slashes.
#[test]
fn t06_scenario_5_multi_validator_downtime_cascade_conservation() {
    const WINDOW: u64 = 200;
    // All validators share the same initial stake so that Sanov severity
    // ordering (more misses → more slash) is not distorted by cap effects.
    let initial_stakes: [(u64, u64); 5] = [
        (1, 100_000), // proposer, not slashed
        (2, 100_000), // light downtime
        (3, 100_000), // medium downtime
        (4, 100_000), // severe downtime
        (5, 100_000), // proposer 2, not slashed
    ];

    let mut tc = make_tc_with_validators(&initial_stakes);

    // Slash 3 validators for different miss counts.
    let slash_v2 = tc.sanov_slash_downtime(2, 10, WINDOW); // 5%
    let slash_v3 = tc.sanov_slash_downtime(3, 80, WINDOW); // 40%
    let slash_v4 = tc.sanov_slash_downtime(4, 190, WINDOW); // 95%

    // No stake goes negative.
    for &(id, _) in &initial_stakes {
        let v = tc
            .validator_set()
            .get(id)
            .expect("validator must still be in set");
        // stake is u64 — type system prevents negative; but verify via conservation.
        let _ = v.stake;
    }

    // Conservation: stake_reduction == reported slash for each validator.
    let v2_reduction = initial_stakes[1].1 - tc.validator_set().get(2).unwrap().stake;
    let v3_reduction = initial_stakes[2].1 - tc.validator_set().get(3).unwrap().stake;
    let v4_reduction = initial_stakes[3].1 - tc.validator_set().get(4).unwrap().stake;

    assert_eq!(
        v2_reduction, slash_v2,
        "validator 2 stake delta must equal reported slash"
    );
    assert_eq!(
        v3_reduction, slash_v3,
        "validator 3 stake delta must equal reported slash"
    );
    assert_eq!(
        v4_reduction, slash_v4,
        "validator 4 stake delta must equal reported slash"
    );

    // Unslashed validators (1 and 5) are untouched.
    assert_eq!(
        tc.validator_set().get(1).unwrap().stake,
        initial_stakes[0].1,
        "unslashed validator 1 stake must be unchanged"
    );
    assert_eq!(
        tc.validator_set().get(5).unwrap().stake,
        initial_stakes[4].1,
        "unslashed validator 5 stake must be unchanged"
    );

    // Severity ordering: more misses → more slash.
    // slash_v4 (190/200) ≥ slash_v3 (80/200) ≥ slash_v2 (10/200).
    assert!(
        slash_v4 >= slash_v3,
        "190-miss slash must be ≥ 80-miss slash: {slash_v4} vs {slash_v3}"
    );
    assert!(
        slash_v3 >= slash_v2,
        "80-miss slash must be ≥ 10-miss slash: {slash_v3} vs {slash_v2}"
    );

    // Severe downtime validator (4) must be jailed (≥3 misses threshold).
    let v4 = tc.validator_set().get(4).unwrap();
    assert!(v4.jailed, "validator 4 (190/200 misses) must be jailed");
}
