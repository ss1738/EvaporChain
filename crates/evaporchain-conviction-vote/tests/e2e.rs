//! End-to-end integration tests for evaporchain-conviction-vote.
//!
//! Non-trivial fixture: a 3-proposal community-governance DAO.
//!
//! Doctrine claim (INVENTION_STACK §4.3 — Evaporating Conviction Vote):
//!   "Sustained governance requires sustained engagement.
//!   Conviction has TWO time scales — integration (α) and decay (λ).
//!   A flash-mob voter who dumps stake and disappears sees conviction
//!   peak then decline before reaching threshold. Only voters who
//!   continuously re-anchor their decaying stake sustain the conviction
//!   needed to cross the threshold."
//!
//!   ALPHA = 0.9 = 900_000 micros.  Asymptote at constant stake S: 10·S.
//!   Thresholds are placed to require coalition or sustained patience:
//!     above a single-voter's asymptote requires coalition;
//!     well above any flash-mob's 10-tick peak requires sustained engagement.
//!
//!   Conviction arithmetic (integer, no f64):
//!     c' = (alpha_micros * c) / MICROS + stake
//!   Key values (Alice+Bob = 2_000_000/tick):
//!     tick 1: c = 2_000_000
//!     tick 2: c = 1_800_000 + 2_000_000 = 3_800_000
//!     tick 3: c = 3_420_000 + 2_000_000 = 5_420_000  → passes THRESHOLD_P1 = 5_000_000
//!
//! Three proposals:
//!   P1 "Parameter tweak"   threshold =  5_000_000  — Alice+Bob pass at tick 3
//!   P2 "Protocol upgrade"  threshold = 12_000_000  — Alice+Bob pass at tick 9
//!   P3 "Treasury release"  threshold = 30_000_000  — requires 5M/tick coalition (Alice+Bob+Carol)
//!                                                     Flash-mob (Carol alone 10 ticks) peaks ≈19.5M → FAILS
//!
//! Four voters:
//!   Alice  1_000_000 stake/tick  — steady long-term voter
//!   Bob    1_000_000 stake/tick  — steady long-term voter
//!   Carol  3_000_000 stake/tick  — flash-mob attacker in adversarial tests; coalition member elsewhere
//!   Dave   2_500_000 stake/tick  — late joiner (enters at tick 101)

use evaporchain_conviction_vote::{
    ConvictionRegistry, Proposal, ProposalError, ProposalId, RegistryError, VoterId,
    ALPHA_MICROS_DEFAULT, MICROS,
};

// ── Fixture helpers ───────────────────────────────────────────────────────────

const THRESHOLD_P1: u128 = 5_000_000;
const THRESHOLD_P2: u128 = 12_000_000;
const THRESHOLD_P3: u128 = 30_000_000;

const STAKE_ALICE: u128 = 1_000_000;
const STAKE_BOB: u128 = 1_000_000;
const STAKE_CAROL: u128 = 3_000_000;
const STAKE_DAVE: u128 = 2_500_000;

fn pid(b: u8) -> ProposalId {
    ProposalId([b; 32])
}
fn vid(b: u8) -> VoterId {
    VoterId([b; 32])
}

fn p1() -> ProposalId {
    pid(1)
}
fn p2() -> ProposalId {
    pid(2)
}
fn p3() -> ProposalId {
    pid(3)
}

fn alice() -> VoterId {
    vid(10)
}
fn bob() -> VoterId {
    vid(20)
}
fn carol() -> VoterId {
    vid(30)
}
fn dave() -> VoterId {
    vid(40)
}

fn new_proposal(id: ProposalId, threshold: u128) -> Proposal {
    Proposal::new(id, ALPHA_MICROS_DEFAULT, threshold, 0).unwrap()
}

// ── Arithmetic calibration ────────────────────────────────────────────────────

#[test]
fn conviction_arithmetic_matches_integer_recurrence_at_known_ticks() {
    // Alice + Bob = 2_000_000/tick.  Verifies the integer update rule exactly.
    //   tick 1: c' = (900_000 * 0) / 1_000_000 + 2_000_000 = 2_000_000
    //   tick 2: c' = (900_000 * 2_000_000) / 1_000_000 + 2_000_000 = 3_800_000
    //   tick 3: c' = (900_000 * 3_800_000) / 1_000_000 + 2_000_000 = 5_420_000
    let mut reg = ConvictionRegistry::new();
    reg.register(new_proposal(pid(99), u128::MAX / 2)).unwrap();
    reg.allocate(pid(99), alice(), STAKE_ALICE).unwrap();
    reg.allocate(pid(99), bob(), STAKE_BOB).unwrap();

    reg.tick(pid(99), 1).unwrap();
    assert_eq!(
        reg.get(&pid(99)).unwrap().conviction_micros,
        2_000_000,
        "tick 1 conviction must be exactly 2_000_000"
    );

    reg.tick(pid(99), 2).unwrap();
    assert_eq!(
        reg.get(&pid(99)).unwrap().conviction_micros,
        3_800_000,
        "tick 2 conviction must be exactly 3_800_000"
    );

    reg.tick(pid(99), 3).unwrap();
    assert_eq!(
        reg.get(&pid(99)).unwrap().conviction_micros,
        5_420_000,
        "tick 3 conviction must be exactly 5_420_000"
    );
}

// ── Full 3-proposal DAO governance session ────────────────────────────────────

#[test]
fn full_dao_three_proposal_governance_session() {
    let mut reg = ConvictionRegistry::new();
    reg.register(new_proposal(p1(), THRESHOLD_P1)).unwrap();
    reg.register(new_proposal(p2(), THRESHOLD_P2)).unwrap();
    reg.register(new_proposal(p3(), THRESHOLD_P3)).unwrap();

    // Alice + Bob vote on P1 and P2 (2M/tick each).
    reg.allocate(p1(), alice(), STAKE_ALICE).unwrap();
    reg.allocate(p1(), bob(), STAKE_BOB).unwrap();
    reg.allocate(p2(), alice(), STAKE_ALICE).unwrap();
    reg.allocate(p2(), bob(), STAKE_BOB).unwrap();

    // Alice + Bob + Carol vote on P3 (5M/tick combined).
    reg.allocate(p3(), alice(), STAKE_ALICE).unwrap();
    reg.allocate(p3(), bob(), STAKE_BOB).unwrap();
    reg.allocate(p3(), carol(), STAKE_CAROL).unwrap();

    let (mut p1_pass_tick, mut p2_pass_tick, mut p3_pass_tick) = (None, None, None);

    for t in 1u64..=20 {
        reg.tick(p1(), t).unwrap();
        reg.tick(p2(), t).unwrap();
        reg.tick(p3(), t).unwrap();

        if p1_pass_tick.is_none() && reg.get(&p1()).unwrap().is_passed() {
            p1_pass_tick = Some(t);
        }
        if p2_pass_tick.is_none() && reg.get(&p2()).unwrap().is_passed() {
            p2_pass_tick = Some(t);
        }
        if p3_pass_tick.is_none() && reg.get(&p3()).unwrap().is_passed() {
            p3_pass_tick = Some(t);
        }
    }

    let p1_t = p1_pass_tick.expect("P1 must pass within 20 ticks");
    let p2_t = p2_pass_tick.expect("P2 must pass within 20 ticks");
    let p3_t = p3_pass_tick.expect("P3 must pass within 20 ticks");

    // Higher threshold takes longer under the same coalition.
    assert!(
        p1_t < p2_t,
        "P1 (5M threshold) must pass before P2 (12M threshold)"
    );

    // P3 has a larger coalition (5M/tick) so passes ≤ P2 (2M/tick).
    assert!(
        p3_t <= p2_t,
        "P3 coalition (5M/tick) passes no later than P2 coalition (2M/tick)"
    );
}

// ── Flash-mob adversarial ─────────────────────────────────────────────────────

#[test]
fn flash_mob_fails_on_high_threshold_proposal() {
    // Carol alone puts 3M/tick for 10 ticks then withdraws.
    // Peak conviction ≈ 19_539_645 < THRESHOLD_P3 = 30_000_000.
    let mut reg = ConvictionRegistry::new();
    reg.register(new_proposal(p3(), THRESHOLD_P3)).unwrap();
    reg.allocate(p3(), carol(), STAKE_CAROL).unwrap();

    for t in 1u64..=10 {
        reg.tick(p3(), t).unwrap();
    }
    let peak = reg.get(&p3()).unwrap().conviction_micros;
    assert!(
        peak < THRESHOLD_P3,
        "Carol's 10-tick flash-mob peak {peak} must not reach threshold {THRESHOLD_P3}"
    );
    assert!(
        !reg.get(&p3()).unwrap().is_passed(),
        "P3 must not be passed after Carol's flash mob"
    );

    // Carol withdraws completely.
    reg.allocate(p3(), carol(), 0).unwrap();

    // 200 more ticks of zero stake: conviction decays toward 0.
    for t in 11u64..=210 {
        reg.tick(p3(), t).unwrap();
    }
    assert!(
        !reg.get(&p3()).unwrap().is_passed(),
        "P3 must remain unpassed after flash-mob withdrawal and 200-tick decay"
    );
    assert!(
        reg.get(&p3()).unwrap().conviction_micros < 1_000,
        "conviction must decay to near-zero after flash mob withdraws"
    );
}

// ── Two-timescale doctrine ────────────────────────────────────────────────────

#[test]
fn two_timescale_doctrine_engaged_passes_depositor_fails() {
    // The core doctrine: conviction has TWO time scales (α = integration,
    // λ = stake decay). A voter who deposits once and leaves sees conviction
    // peak at ≤ stake (tick 1) then decay toward 0 — never reaches threshold.
    // A voter who re-anchors every tick builds conviction toward the asymptote.
    //
    // Both start with STAKE_ALICE = 1_000_000.  Asymptote = 10M > THRESHOLD_P1 = 5M.
    // Depositor: c(1) = 1M, then 0. Max ever = 1M << 5M threshold. → FAILS.
    // Engaged: conviction grows to 10M asymptote, crosses 5M. → PASSES.
    let mut engaged = Proposal::new(pid(10), ALPHA_MICROS_DEFAULT, THRESHOLD_P1, 0).unwrap();
    let mut depositor = Proposal::new(pid(11), ALPHA_MICROS_DEFAULT, THRESHOLD_P1, 0).unwrap();

    engaged.tick(1, STAKE_ALICE).unwrap();
    depositor.tick(1, STAKE_ALICE).unwrap(); // one-time deposit

    for t in 2u64..=200 {
        engaged.tick(t, STAKE_ALICE).unwrap(); // re-anchors every tick
        depositor.tick(t, 0).unwrap(); // gone: stake decays to 0
    }

    assert!(
        engaged.is_passed(),
        "engaged voter with sustained stake must pass threshold"
    );
    assert!(!depositor.is_passed(),
        "deposit-and-leave voter must never pass threshold — conviction decays faster than it built");
    assert!(
        depositor.conviction_micros < 100,
        "depositor conviction must decay to near-zero after 200 ticks (got {})",
        depositor.conviction_micros
    );
}

// ── Pass-state stickiness ─────────────────────────────────────────────────────

#[test]
fn passed_state_sticky_after_full_stake_withdrawal() {
    // Once passed, a proposal stays passed even if all stake is later
    // withdrawn and conviction decays below the threshold (no flapping).
    let mut reg = ConvictionRegistry::new();
    reg.register(new_proposal(p1(), THRESHOLD_P1)).unwrap();
    reg.allocate(p1(), alice(), STAKE_ALICE).unwrap();
    reg.allocate(p1(), bob(), STAKE_BOB).unwrap();

    // Run until passed.
    let mut pass_tick = None;
    for t in 1u64..=20 {
        reg.tick(p1(), t).unwrap();
        if pass_tick.is_none() && reg.get(&p1()).unwrap().is_passed() {
            pass_tick = Some(t);
        }
    }
    let pass_tick = pass_tick.expect("P1 must pass within 20 ticks");
    let conviction_at_pass = reg.get(&p1()).unwrap().conviction_micros;

    // Withdraw all stake.
    reg.allocate(p1(), alice(), 0).unwrap();
    reg.allocate(p1(), bob(), 0).unwrap();

    // Continue from tick 21 (the loop above ran to 20); run 200 more ticks with zero stake.
    for t in 21u64..=221 {
        reg.tick(p1(), t).unwrap();
    }

    assert!(
        reg.get(&p1()).unwrap().is_passed(),
        "passed proposal must remain passed after stake withdrawal (no flapping)"
    );
    assert!(
        reg.get(&p1()).unwrap().conviction_micros < conviction_at_pass,
        "conviction must have decayed after 200 ticks of zero stake"
    );
}

// ── Asymptote ceiling ─────────────────────────────────────────────────────────

#[test]
fn alice_alone_below_asymptote_threshold_never_passes() {
    // Alice (1M/tick) has asymptote = 1M / (1-0.9) = 10M.
    // A threshold above the asymptote (15M) can never be reached no matter
    // how many ticks she votes.
    let mut p = Proposal::new(pid(50), ALPHA_MICROS_DEFAULT, 15_000_000, 0).unwrap();
    for t in 1u64..=1_000 {
        p.tick(t, STAKE_ALICE).unwrap();
    }
    assert!(
        !p.is_passed(),
        "Alice alone cannot exceed her asymptote (10M); threshold 15M is unreachable"
    );
    // After 1000 ticks, conviction approaches but stays below asymptote.
    assert!(
        p.conviction_micros < 11_000_000,
        "conviction must stay below asymptote"
    );
    assert!(
        p.conviction_micros > 9_900_000,
        "after 1000 ticks, conviction must be near asymptote"
    );
}

// ── Multi-voter coalition via registry ────────────────────────────────────────

#[test]
fn multi_voter_coalition_stake_sums_and_drives_conviction_via_registry() {
    // Verify that registry.tick() sums all voter allocations correctly.
    // Alice (1M) + Bob (1M) + Carol (3M) = 5M/tick.
    // tick 1: c = 5M; tick 2: c = 4.5M + 5M = 9.5M.
    // Threshold = 8M → passes at tick 2.
    let mut reg = ConvictionRegistry::new();
    reg.register(new_proposal(pid(7), 8_000_000)).unwrap();
    reg.allocate(pid(7), alice(), STAKE_ALICE).unwrap();
    reg.allocate(pid(7), bob(), STAKE_BOB).unwrap();
    reg.allocate(pid(7), carol(), STAKE_CAROL).unwrap();

    assert_eq!(
        reg.total_stake_on(pid(7)),
        5_000_000,
        "total stake must equal Alice+Bob+Carol = 5_000_000"
    );

    reg.tick(pid(7), 1).unwrap();
    assert_eq!(
        reg.get(&pid(7)).unwrap().conviction_micros,
        5_000_000,
        "tick 1 conviction = 5_000_000 (five voters' combined stake)"
    );
    assert!(
        !reg.get(&pid(7)).unwrap().is_passed(),
        "threshold not yet crossed at tick 1"
    );

    reg.tick(pid(7), 2).unwrap();
    assert_eq!(
        reg.get(&pid(7)).unwrap().conviction_micros,
        9_500_000,
        "tick 2 conviction = 9_500_000"
    );
    assert!(
        reg.get(&pid(7)).unwrap().is_passed(),
        "coalition of 5M/tick must cross 8M threshold by tick 2"
    );
}

// ── Late-joiner tips stalled proposal ─────────────────────────────────────────

#[test]
fn late_joiner_tips_stalled_proposal_over_threshold() {
    // Alice (1M/tick) alone: asymptote = 10M < THRESHOLD_P3 (30M).
    // She can never pass alone. Dave (2.5M) joins at tick 101:
    // combined 3.5M/tick, asymptote = 35M > 30M → eventually passes.
    let mut reg = ConvictionRegistry::new();
    reg.register(new_proposal(p3(), THRESHOLD_P3)).unwrap();
    reg.allocate(p3(), alice(), STAKE_ALICE).unwrap();

    for t in 1u64..=100 {
        reg.tick(p3(), t).unwrap();
    }
    let conv_before_dave = reg.get(&p3()).unwrap().conviction_micros;
    assert!(
        conv_before_dave < THRESHOLD_P3,
        "Alice alone (asymptote 10M) cannot reach threshold 30M: got {conv_before_dave}"
    );
    assert!(!reg.get(&p3()).unwrap().is_passed());

    // Dave joins.
    reg.allocate(p3(), dave(), STAKE_DAVE).unwrap();

    for t in 101u64..=250 {
        reg.tick(p3(), t).unwrap();
    }
    assert!(
        reg.get(&p3()).unwrap().is_passed(),
        "Alice (1M) + Dave (2.5M) = 3.5M/tick, asymptote 35M, must eventually pass 30M threshold"
    );
}

// ── Proposals fully independent ───────────────────────────────────────────────

#[test]
fn separate_proposals_are_fully_independent_in_registry() {
    let mut reg = ConvictionRegistry::new();
    reg.register(new_proposal(p1(), THRESHOLD_P1)).unwrap();
    reg.register(new_proposal(p2(), THRESHOLD_P2)).unwrap();

    // Alice+Bob vote on P1 only; P2 gets no votes.
    reg.allocate(p1(), alice(), STAKE_ALICE).unwrap();
    reg.allocate(p1(), bob(), STAKE_BOB).unwrap();

    for t in 1u64..=20 {
        reg.tick(p1(), t).unwrap();
        reg.tick(p2(), t).unwrap();
    }

    assert!(
        reg.get(&p1()).unwrap().is_passed(),
        "P1 with Alice+Bob must pass"
    );
    assert!(
        !reg.get(&p2()).unwrap().is_passed(),
        "P2 with zero stake must not pass"
    );
    assert_eq!(
        reg.get(&p2()).unwrap().conviction_micros,
        0,
        "P2 conviction must be 0 — no votes allocated"
    );
}

// ── Adversarial: registration / allocation errors ─────────────────────────────

#[test]
fn adversarial_duplicate_proposal_registration_rejected() {
    let mut reg = ConvictionRegistry::new();
    reg.register(new_proposal(p1(), THRESHOLD_P1)).unwrap();
    let err = reg.register(new_proposal(p1(), 999)).unwrap_err();
    assert!(
        matches!(err, RegistryError::AlreadyRegistered(_)),
        "second registration of same ProposalId must be rejected"
    );
}

#[test]
fn adversarial_unknown_proposal_tick_rejected() {
    let mut reg = ConvictionRegistry::new();
    let err = reg.tick(pid(99), 1).unwrap_err();
    assert!(
        matches!(err, RegistryError::UnknownProposal(_)),
        "tick on unregistered proposal must return UnknownProposal"
    );
}

#[test]
fn adversarial_non_monotone_tick_propagates_via_registry() {
    let mut reg = ConvictionRegistry::new();
    reg.register(new_proposal(p1(), THRESHOLD_P1)).unwrap();
    reg.allocate(p1(), alice(), STAKE_ALICE).unwrap();
    reg.tick(p1(), 10).unwrap();

    // Attempt to re-tick at a lower tick number.
    let err = reg.tick(p1(), 5).unwrap_err();
    assert!(
        matches!(
            err,
            RegistryError::Proposal(ProposalError::NonMonotoneTick {
                incoming: 5,
                last: 10
            })
        ),
        "non-monotone tick must propagate as RegistryError::Proposal(NonMonotoneTick)"
    );
}

// ── Allocation override (re-staking) ─────────────────────────────────────────

#[test]
fn voter_can_reduce_stake_mid_session_and_conviction_slows() {
    // Alice at 1M/tick for 10 ticks, then reduces to 200K/tick.
    // Conviction growth slows after the reduction — demonstrates re-anchoring flexibility.
    let mut reg = ConvictionRegistry::new();
    reg.register(new_proposal(p1(), 1_000_000_000)).unwrap(); // very high threshold — never passes

    reg.allocate(p1(), alice(), STAKE_ALICE).unwrap();
    for t in 1u64..=10 {
        reg.tick(p1(), t).unwrap();
    }
    let conv_high_stake = reg.get(&p1()).unwrap().conviction_micros;

    // Alice cuts stake to 200K.
    reg.allocate(p1(), alice(), 200_000).unwrap();
    for t in 11u64..=20 {
        reg.tick(p1(), t).unwrap();
    }
    let conv_low_stake = reg.get(&p1()).unwrap().conviction_micros;

    // After 10 ticks at high stake, conviction grew quickly. After the
    // reduction, the new asymptote (2M) is below the old (10M), so
    // conviction starts declining toward the new lower asymptote.
    assert!(
        conv_low_stake < conv_high_stake,
        "conviction must decline after stake reduction below current conviction level"
    );
}
