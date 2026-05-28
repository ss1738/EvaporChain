//! Coverage tests for the single-λ energy kernel — cross-module
//! integrations of conservation + redirects + refresh pool that
//! the per-module #[cfg(test)] tests don't exercise together.

use evaporchain_energy_kernel::{
    energy_at_epoch, ChainLambda, Compartment, ConservationCheck, ConservationViolation,
    EnergyAccumulator, EnergyRedirect, Lambda, RedirectKind, RefreshCredit, RefreshPool,
    DEFAULT_LAMBDA,
};

// =================================================================
// Lambda + ChainLambda
// =================================================================

#[test]
fn lambda_degenerate_at_zero_only() {
    assert!(Lambda::from_epochs(0).is_degenerate());
    assert!(!Lambda::from_epochs(1).is_degenerate());
    assert!(!Lambda::from_epochs(u64::MAX).is_degenerate());
}

#[test]
fn default_lambda_pins_4096_epochs() {
    assert_eq!(DEFAULT_LAMBDA.epochs(), 4096);
    assert_eq!(ChainLambda::default_genesis().half_life(), 4096);
    assert_eq!(ChainLambda::default(), ChainLambda::default_genesis());
}

#[test]
fn chain_lambda_const_constructor_preserves_lambda() {
    let l = Lambda::from_epochs(123);
    let cl = ChainLambda::new(l);
    assert_eq!(cl.lambda(), l);
    assert_eq!(cl.half_life(), 123);
}

#[test]
fn lambda_serde_round_trips() {
    let l = Lambda::from_epochs(777);
    let json = serde_json::to_string(&l).unwrap();
    let back: Lambda = serde_json::from_str(&json).unwrap();
    assert_eq!(back, l);
    let cl = ChainLambda::new(l);
    let json = serde_json::to_string(&cl).unwrap();
    let back: ChainLambda = serde_json::from_str(&json).unwrap();
    assert_eq!(back, cl);
}

// =================================================================
// EnergyAccumulator
// =================================================================

#[test]
fn accumulator_total_saturates_on_overflow() {
    // Two buckets at u64::MAX → total saturates rather than overflows.
    let acc = EnergyAccumulator::new(u64::MAX, u64::MAX, 0, 0);
    assert_eq!(acc.total(), u64::MAX);
}

#[test]
fn credit_then_debit_round_trip_via_indexing() {
    let mut acc = EnergyAccumulator::default();
    acc.credit(Compartment::Stake, 1_000);
    assert_eq!(acc[Compartment::Stake], 1_000);
    acc.debit(Compartment::Stake, 250).unwrap();
    assert_eq!(acc[Compartment::Stake], 750);
}

#[test]
fn debit_underflow_leaves_compartment_intact() {
    let mut acc = EnergyAccumulator::new(0, 0, 50, 0);
    assert!(acc.debit(Compartment::RefreshPool, 51).is_err());
    assert_eq!(acc[Compartment::RefreshPool], 50);
}

#[test]
fn accumulator_serde_round_trips() {
    let acc = EnergyAccumulator::new(11, 22, 33, 44);
    let json = serde_json::to_string(&acc).unwrap();
    let back: EnergyAccumulator = serde_json::from_str(&json).unwrap();
    assert_eq!(back, acc);
    assert_eq!(back.total(), 110);
}

// =================================================================
// Redirects — every kind round-trips a known flow
// =================================================================

#[test]
fn every_redirect_kind_has_distinct_flow_or_known_overlap() {
    use std::collections::HashMap;
    let mut flows: HashMap<(Compartment, Compartment), Vec<RedirectKind>> = HashMap::new();
    for k in [
        RedirectKind::Slash,
        RedirectKind::SlashSettle,
        RedirectKind::MevBurn,
        RedirectKind::Demurrage,
        RedirectKind::RefreshPayout,
    ] {
        flows.entry(k.flow()).or_default().push(k);
    }
    // MevBurn and Demurrage share the same flow (Accounts → RefreshPool).
    // Document that explicitly: any change that splits or merges the
    // flow set must update this assertion.
    let mev_flow = (Compartment::Accounts, Compartment::RefreshPool);
    assert_eq!(flows[&mev_flow].len(), 2);
    // All four other flows are unique.
    for ((from, to), kinds) in &flows {
        if (*from, *to) == mev_flow {
            continue;
        }
        assert_eq!(
            kinds.len(),
            1,
            "flow {from:?}→{to:?} has overlapping kinds: {kinds:?}"
        );
    }
}

#[test]
fn redirect_preserves_total_for_every_kind() {
    for kind in [
        RedirectKind::Slash,
        RedirectKind::SlashSettle,
        RedirectKind::MevBurn,
        RedirectKind::Demurrage,
        RedirectKind::RefreshPayout,
    ] {
        let (from, _to) = kind.flow();
        // Build an accumulator with enough energy in the source bucket.
        let mut acc = EnergyAccumulator::default();
        acc.credit(from, 1_000);
        let before = acc.total();
        EnergyRedirect::new(kind, 250).apply(&mut acc).unwrap();
        assert_eq!(acc.total(), before, "kind {kind:?} must preserve total");
    }
}

#[test]
fn redirect_serde_round_trips() {
    let r = EnergyRedirect::new(RedirectKind::Slash, 1234);
    let json = serde_json::to_string(&r).unwrap();
    let back: EnergyRedirect = serde_json::from_str(&json).unwrap();
    assert_eq!(back, r);
}

// =================================================================
// ConservationCheck — sequenced redirects + decay
// =================================================================

#[test]
fn sequenced_redirects_remain_conservation_clean() {
    let mut acc = EnergyAccumulator::new(1_000, 1_000, 0, 0);
    let pre = acc.total();
    EnergyRedirect::new(RedirectKind::Slash, 200)
        .apply(&mut acc)
        .unwrap();
    EnergyRedirect::new(RedirectKind::SlashSettle, 200)
        .apply(&mut acc)
        .unwrap();
    EnergyRedirect::new(RedirectKind::MevBurn, 50)
        .apply(&mut acc)
        .unwrap();
    EnergyRedirect::new(RedirectKind::Demurrage, 25)
        .apply(&mut acc)
        .unwrap();
    assert_eq!(acc.total(), pre);
}

#[test]
fn decay_at_default_lambda_far_below_half_life_keeps_full_total() {
    let before = EnergyAccumulator::new(1_000_000, 0, 0, 0);
    let after = before;
    // 1 epoch elapsed at default λ (4096 epochs) — retained_min ≈ before.
    ConservationCheck::decay_step(&before, &after, 1, ChainLambda::default()).unwrap();
}

#[test]
fn decay_violation_carries_diagnostic_fields() {
    let before = EnergyAccumulator::new(1_000, 0, 0, 0);
    let after = EnergyAccumulator::new(100, 0, 0, 0);
    let lambda = ChainLambda::new(Lambda::from_epochs(100));
    let err = ConservationCheck::decay_step(&before, &after, 100, lambda).unwrap_err();
    match err {
        ConservationViolation::DecayExceededLambda {
            before,
            after,
            epochs,
            half_life,
            ..
        } => {
            assert_eq!(before, 1_000);
            assert_eq!(after, 100);
            assert_eq!(epochs, 100);
            assert_eq!(half_life, 100);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn redirect_then_decay_audited_by_block_step() {
    let before = EnergyAccumulator::new(1_000, 0, 0, 0);
    let lambda = ChainLambda::new(Lambda::from_epochs(100));
    // Compute the legal decay floor and craft an "after" at exactly that floor.
    let floor = energy_at_epoch(1_000, 100, 100);
    let after = EnergyAccumulator::new(floor, 0, 0, 0);
    ConservationCheck::block_step(&before, &after, 100, lambda).unwrap();
}

#[test]
fn conservation_violation_displays() {
    let e = ConservationViolation::RedirectChangedTotal {
        before: 100,
        after: 50,
    };
    let s = e.to_string();
    assert!(s.contains("100"));
    assert!(s.contains("50"));
}

// =================================================================
// RefreshPool
// =================================================================

#[test]
fn refresh_pool_payout_partial_keeps_entry() {
    let mut p = RefreshPool::new();
    p.accrue(b"ns1".to_vec(), 100, 1);
    p.payout(&b"ns1".to_vec(), 30, 2).unwrap();
    assert_eq!(p.accrued_for(&b"ns1".to_vec()), 70);
    assert!(!p.is_empty());
}

#[test]
fn refresh_credit_serde_round_trips() {
    let c = RefreshCredit {
        namespace: b"ns".to_vec(),
        accrued: 1234,
        last_touched_epoch: 9,
    };
    let json = serde_json::to_string(&c).unwrap();
    let back: RefreshCredit = serde_json::from_str(&json).unwrap();
    assert_eq!(back, c);
}

#[test]
fn refresh_pool_credits_iterator_is_namespace_ordered() {
    let mut p = RefreshPool::new();
    // Insert out-of-order.
    p.accrue(b"zzz".to_vec(), 1, 0);
    p.accrue(b"aaa".to_vec(), 2, 0);
    p.accrue(b"mmm".to_vec(), 3, 0);
    let ns: Vec<&[u8]> = p.credits().map(|c| c.namespace.as_slice()).collect();
    assert_eq!(ns, vec![b"aaa".as_ref(), b"mmm".as_ref(), b"zzz".as_ref()]);
}

#[test]
fn refresh_pool_total_matches_per_namespace_sum() {
    let mut p = RefreshPool::new();
    for i in 0..5u8 {
        p.accrue(vec![i], 100 * (i as u64 + 1), 0);
    }
    let sum: u64 = (0..5u8).map(|i| p.accrued_for(&vec![i])).sum();
    assert_eq!(p.total_accrued(), sum);
}
