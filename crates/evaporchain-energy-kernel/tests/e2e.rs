//! End-to-end integration tests for evaporchain-energy-kernel.
//!
//! Non-trivial fixture: a 4-block chain segment exercising every energy
//! flow path in the conservation pipeline.
//!
//! Doctrine claim (INVENTION_STACK.md §1.1 + §1.2):
//!   "EvaporChain has ONE fundamental constant — λ. Every layer reads
//!   this crate's single constant. The conservation invariant is:
//!   total energy across {Accounts + Stake + RefreshPool + SlashedPool}
//!   decreases monotonically ONLY via the global λ-decay. No transition
//!   other than a decay step may reduce the total."
//!
//! Block-by-block conservation chain:
//!
//!   Genesis: Accounts=10_000_000  Stake=5_000_000  RP=0  SP=0  total=15_000_000
//!
//!   Block 1 (no epoch advance) — MEV burn:
//!     MevBurn: 50_000 Accounts → RefreshPool
//!     total preserved: 15_000_000
//!
//!   Block 2 (no epoch advance) — Slash:
//!     Slash: 200_000 Stake → SlashedPool
//!     total preserved: 15_000_000
//!
//!   Block 3 (no epoch advance) — SlashSettle + RefreshPayout:
//!     SlashSettle: 200_000 SlashedPool → RefreshPool
//!     RefreshPayout: 30_000 RefreshPool → Accounts
//!     total preserved: 15_000_000
//!
//!   Block 4 (100 epochs elapsed, half_life=100) — decay + demurrage:
//!     energy_at_epoch(15_000_000, 100, 100) = 15_000_000 >> 1 = 7_500_000
//!     Post-decay state: Accounts=7_000_000  Stake=0  RP=500_000  SP=0
//!     total=7_500_000 ≥ λ-floor (exact minimum) → PASS
//!
//! The RefreshPool ledger tracks per-namespace credits independently of the
//! EnergyAccumulator bucket, with accrue/payout round-trips on two namespaces.

use evaporchain_energy_kernel::redirect::RedirectKind;
use evaporchain_energy_kernel::refresh_pool::RefreshPoolError;
use evaporchain_energy_kernel::{
    energy_at_epoch, ChainLambda, Compartment, ConservationCheck, ConservationViolation,
    EnergyAccumulator, EnergyRedirect, Lambda, RefreshPool,
};

// ── Constants ─────────────────────────────────────────────────────────────────

const HALF_LIFE: u64 = 100;

fn cl() -> ChainLambda {
    ChainLambda::new(Lambda::from_epochs(HALF_LIFE))
}

fn ns(tag: u8) -> Vec<u8> {
    vec![tag; 4]
}

// ── Fixture helpers ───────────────────────────────────────────────────────────

fn genesis() -> EnergyAccumulator {
    EnergyAccumulator::new(10_000_000, 5_000_000, 0, 0)
}

// ── Full 4-block conservation chain simulation ────────────────────────────────

#[test]
fn full_4_block_chain_conservation_simulation() {
    // ── Genesis ──────────────────────────────────────────────────────
    let mut acc = genesis();
    assert_eq!(acc.total(), 15_000_000);

    // ── Block 1: MEV burn (Accounts → RefreshPool) ────────────────────
    let b0 = acc;
    EnergyRedirect::new(RedirectKind::MevBurn, 50_000)
        .apply(&mut acc)
        .unwrap();
    ConservationCheck::redirect(&b0, &acc).unwrap();
    assert_eq!(acc.total(), 15_000_000, "B1: MEV burn must preserve total");
    assert_eq!(acc[Compartment::Accounts], 9_950_000);
    assert_eq!(acc[Compartment::RefreshPool], 50_000);

    // ── Block 2: Slash (Stake → SlashedPool) ─────────────────────────
    let b1 = acc;
    EnergyRedirect::new(RedirectKind::Slash, 200_000)
        .apply(&mut acc)
        .unwrap();
    ConservationCheck::redirect(&b1, &acc).unwrap();
    assert_eq!(acc.total(), 15_000_000, "B2: Slash must preserve total");
    assert_eq!(acc[Compartment::Stake], 4_800_000);
    assert_eq!(acc[Compartment::SlashedPool], 200_000);

    // ── Block 3: SlashSettle + RefreshPayout ──────────────────────────
    let b2 = acc;
    EnergyRedirect::new(RedirectKind::SlashSettle, 200_000)
        .apply(&mut acc)
        .unwrap();
    EnergyRedirect::new(RedirectKind::RefreshPayout, 30_000)
        .apply(&mut acc)
        .unwrap();
    ConservationCheck::redirect(&b2, &acc).unwrap();
    assert_eq!(
        acc.total(),
        15_000_000,
        "B3: settle+payout must preserve total"
    );
    assert_eq!(acc[Compartment::SlashedPool], 0);
    assert_eq!(acc[Compartment::RefreshPool], 220_000);
    assert_eq!(acc[Compartment::Accounts], 9_980_000);

    // ── Block 4: 100-epoch decay (one full halving) ───────────────────
    let b3 = acc;
    // λ-floor: energy_at_epoch(15_000_000, 100, 100) = 15_000_000 >> 1 = 7_500_000.
    let lambda_floor = energy_at_epoch(b3.total(), HALF_LIFE, 100);
    assert_eq!(
        lambda_floor, 7_500_000,
        "one halving of 15M must floor to 7.5M"
    );

    // Post-decay state at exactly the λ-floor (boundary case).
    let acc_b4 = EnergyAccumulator::new(7_000_000, 0, 500_000, 0);
    assert_eq!(acc_b4.total(), 7_500_000);
    ConservationCheck::decay_step(&b3, &acc_b4, 100, cl()).unwrap();
}

// ── Redirect conservation ─────────────────────────────────────────────────────

#[test]
fn mev_burn_redirect_preserves_total_and_routes_correctly() {
    let mut acc = EnergyAccumulator::new(1_000_000, 0, 0, 0);
    let before = acc;
    EnergyRedirect::new(RedirectKind::MevBurn, 75_000)
        .apply(&mut acc)
        .unwrap();
    ConservationCheck::redirect(&before, &acc).unwrap();
    assert_eq!(acc[Compartment::Accounts], 925_000);
    assert_eq!(acc[Compartment::RefreshPool], 75_000);
}

#[test]
fn slash_lifecycle_stake_to_slashed_to_refresh_pool() {
    let mut acc = EnergyAccumulator::new(0, 500_000, 0, 0);
    let step0 = acc;

    // Step 1: Slash → SlashedPool.
    EnergyRedirect::new(RedirectKind::Slash, 100_000)
        .apply(&mut acc)
        .unwrap();
    ConservationCheck::redirect(&step0, &acc).unwrap();
    assert_eq!(acc[Compartment::Stake], 400_000);
    assert_eq!(acc[Compartment::SlashedPool], 100_000);
    let step1 = acc;

    // Step 2: SlashSettle → RefreshPool.
    EnergyRedirect::new(RedirectKind::SlashSettle, 100_000)
        .apply(&mut acc)
        .unwrap();
    ConservationCheck::redirect(&step1, &acc).unwrap();
    assert_eq!(acc[Compartment::SlashedPool], 0);
    assert_eq!(acc[Compartment::RefreshPool], 100_000);
    assert_eq!(
        acc.total(),
        500_000,
        "total preserved across the full slash lifecycle"
    );
}

#[test]
fn demurrage_redirect_accounts_to_refresh_pool() {
    let mut acc = EnergyAccumulator::new(2_000_000, 0, 0, 0);
    let before = acc;
    EnergyRedirect::new(RedirectKind::Demurrage, 12_345)
        .apply(&mut acc)
        .unwrap();
    ConservationCheck::redirect(&before, &acc).unwrap();
    assert_eq!(acc[Compartment::Accounts], 1_987_655);
    assert_eq!(acc[Compartment::RefreshPool], 12_345);
}

// ── Decay-step conservation ───────────────────────────────────────────────────

#[test]
fn decay_step_one_halving_at_exact_lambda_floor_passes() {
    // 100 epochs at half_life=100 → floor = before >> 1.
    let before = EnergyAccumulator::new(1_000_000, 0, 0, 0);
    let after = EnergyAccumulator::new(500_000, 0, 0, 0); // exactly the floor
    ConservationCheck::decay_step(&before, &after, 100, cl()).unwrap();
}

#[test]
fn decay_step_partial_decay_above_floor_also_passes() {
    // Holding more than the minimum is legal — decay is a ceiling on loss,
    // not a mandate that all decay must be applied immediately.
    let before = EnergyAccumulator::new(1_000_000, 0, 0, 0);
    let after = EnergyAccumulator::new(750_000, 0, 0, 0); // between floor and before
    ConservationCheck::decay_step(&before, &after, 100, cl()).unwrap();
}

#[test]
fn block_step_redirects_plus_decay_validates_composite() {
    // A composite block: redirects within the block + 100-epoch advance.
    // Conservation check only cares about (before_total, after_total, epochs).
    // Post-redirect total = before total; post-decay total ≥ λ-floor.
    let before = EnergyAccumulator::new(2_000_000, 0, 0, 0);
    // Simulated block: some MEV burn (preserves total) + 100-epoch decay.
    // total_after = 1_000_000 ≥ energy_at_epoch(2M, 100, 100) = 1M → boundary.
    let after = EnergyAccumulator::new(900_000, 100_000, 0, 0);
    ConservationCheck::block_step(&before, &after, 100, cl()).unwrap();
}

// ── Adversarial: conservation violations are caught ───────────────────────────

#[test]
fn conservation_checker_catches_energy_destruction_in_redirect() {
    let before = EnergyAccumulator::new(1_000_000, 500_000, 0, 0);
    // Bug: debit Stake without crediting anywhere else.
    let mut after = before;
    after.debit(Compartment::Stake, 100_000).unwrap();
    let err = ConservationCheck::redirect(&before, &after).unwrap_err();
    assert!(
        matches!(err, ConservationViolation::RedirectChangedTotal { .. }),
        "silent energy destruction must be caught by redirect check"
    );
}

#[test]
fn conservation_checker_catches_energy_creation_in_decay_step() {
    let before = EnergyAccumulator::new(1_000_000, 0, 0, 0);
    // Bug: total increased — impossible under λ-decay.
    let after = EnergyAccumulator::new(1_000_001, 0, 0, 0);
    let err = ConservationCheck::decay_step(&before, &after, 0, cl()).unwrap_err();
    assert!(
        matches!(err, ConservationViolation::DecayIncreasedTotal { .. }),
        "energy creation must be caught by decay-step check"
    );
}

#[test]
fn conservation_checker_catches_excess_decay_below_lambda_floor() {
    // 100 epochs at half_life=100 → floor = 500_000.
    // A buggy implementation that dropped to 499_999 (one below floor) is caught.
    let before = EnergyAccumulator::new(1_000_000, 0, 0, 0);
    let after = EnergyAccumulator::new(499_999, 0, 0, 0);
    let err = ConservationCheck::decay_step(&before, &after, 100, cl()).unwrap_err();
    assert!(
        matches!(err, ConservationViolation::DecayExceededLambda { .. }),
        "decay below λ-floor must be caught as DecayExceededLambda"
    );
}

#[test]
fn redirect_from_empty_compartment_fails_closed_accumulator_unchanged() {
    let mut acc = EnergyAccumulator::new(0, 50, 0, 0);
    let snapshot = acc;
    // Attempt to slash 100 from Stake (only has 50).
    let err = EnergyRedirect::new(RedirectKind::Slash, 100)
        .apply(&mut acc)
        .unwrap_err();
    use evaporchain_energy_kernel::redirect::RedirectError;
    assert!(
        matches!(err, RedirectError::InsufficientSource(..)),
        "insufficient source must be rejected"
    );
    assert_eq!(
        acc, snapshot,
        "accumulator must be unchanged after a failed redirect"
    );
}

// ── RefreshPool credit ledger ─────────────────────────────────────────────────

#[test]
fn refresh_pool_namespace_accrue_and_payout_lifecycle() {
    let mut pool = RefreshPool::new();
    let chain_ns = ns(1); // chain history namespace

    // Accrue credits from MEV burns and slash settlements.
    pool.accrue(chain_ns.clone(), 50_000, 1);
    pool.accrue(chain_ns.clone(), 200_000, 3);
    assert_eq!(pool.accrued_for(&chain_ns), 250_000);
    assert_eq!(pool.total_accrued(), 250_000);

    // Payout for keep-alive.
    let paid = pool.payout(&chain_ns, 30_000, 5).unwrap();
    assert_eq!(paid, 30_000);
    assert_eq!(pool.accrued_for(&chain_ns), 220_000);

    // Full payout removes the entry (GC).
    pool.payout(&chain_ns, 220_000, 7).unwrap();
    assert!(
        pool.is_empty(),
        "exhausted namespace must be garbage-collected"
    );
}

#[test]
fn refresh_pool_multi_namespace_credits_are_fully_isolated() {
    let mut pool = RefreshPool::new();
    let beacon_ns = ns(10);
    let lightcone_ns = ns(20);

    pool.accrue(beacon_ns.clone(), 100_000, 1);
    pool.accrue(lightcone_ns.clone(), 400_000, 1);
    assert_eq!(pool.total_accrued(), 500_000);

    // Payout from beacon must not affect light-cone credits.
    pool.payout(&beacon_ns, 100_000, 2).unwrap();
    assert_eq!(
        pool.accrued_for(&lightcone_ns),
        400_000,
        "light-cone credits must be unaffected by beacon payout"
    );
    assert_eq!(
        pool.accrued_for(&beacon_ns),
        0,
        "beacon namespace should be at zero after full payout"
    );

    // Unknown namespace returns 0 (does not create an entry).
    assert_eq!(pool.accrued_for(&ns(99)), 0);
}

#[test]
fn refresh_pool_insufficient_credit_error_leaves_state_unchanged() {
    let mut pool = RefreshPool::new();
    pool.accrue(ns(1), 50, 1);
    let err = pool.payout(&ns(1), 100, 2).unwrap_err();
    assert!(matches!(err, RefreshPoolError::InsufficientCredit { .. }));
    // Accrued balance unchanged.
    assert_eq!(pool.accrued_for(&ns(1)), 50);
}

// ── Lambda doctrine ───────────────────────────────────────────────────────────

#[test]
fn lambda_is_the_single_universal_decay_constant() {
    // Every layer that needs decay constructs a ChainLambda and reads half_life().
    // All compartments use the same half_life — no per-compartment decay constants.
    let lambda = Lambda::from_epochs(HALF_LIFE);
    let chain = ChainLambda::new(lambda);

    // Compartment energies after one half-life — same λ for all.
    let accounts = 4_000_000u64;
    let stake = 1_000_000u64;
    assert_eq!(
        energy_at_epoch(accounts, chain.half_life(), 100),
        accounts >> 1
    );
    assert_eq!(energy_at_epoch(stake, chain.half_life(), 100), stake >> 1);

    // Double the epochs → halve again.
    assert_eq!(
        energy_at_epoch(accounts, chain.half_life(), 200),
        accounts >> 2
    );

    // λ is non-degenerate (production value has half_life > 0).
    assert!(!lambda.is_degenerate());
    assert_eq!(chain.half_life(), HALF_LIFE);
}

#[test]
fn energy_at_epoch_is_monotone_decreasing_in_epochs() {
    let e0 = 1_000_000u64;
    let snapshots: Vec<u64> = (0..=10)
        .map(|i| energy_at_epoch(e0, HALF_LIFE, HALF_LIFE * i))
        .collect();
    for pair in snapshots.windows(2) {
        assert!(
            pair[1] <= pair[0],
            "energy_at_epoch must be non-increasing in elapsed epochs: {} > {}",
            pair[0],
            pair[1]
        );
    }
    // After 10 halvings: 1_000_000 >> 10 = 976 (not yet floored to 0).
    assert!(
        snapshots[10] < 1_000,
        "10 halvings must reduce energy to <0.1% of initial"
    );
}
