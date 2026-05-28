//! End-to-end integration tests for evaporchain-ib-validators-v2.
//!
//! Non-trivial fixture: six-validator BFT committee under jail pressure.
//!
//! A consensus committee of 6 validators operates across 3 rounds.
//! A CHSH-failure event at epoch 0 jails 3 validators (V4, V5, V6) for
//! 50 epochs (expires_at=50). An energy-floor catches V5 even after the
//! CHSH jail expires. The V1 conformist (KL=0) always abstains regardless
//! of jail state.
//!
//!   Validators:
//!     V1  [0x01;32]  KL=0 (conformist)  energy=2_000
//!     V2  [0x02;32]  KL high            energy=2_000
//!     V3  [0x03;32]  KL high            energy=1_500
//!     V4  [0x04;32]  KL high            energy=  800  jailed 0–49 (CHSH)
//!     V5  [0x05;32]  KL high            energy=   50  jailed 0–49 (CHSH)
//!     V6  [0x06;32]  KL=0 (conformist)  energy=1_000  jailed 0–49 (CHSH)
//!
//!   Energy floor = 100.
//!
//!   Round 1 (epoch=10): commits = {V2, V3}          (2 of 6)
//!   Round 2 (epoch=25): commits = {V2, V3}          (2 of 6)
//!   Round 3 (epoch=50): jail expires → commits = {V2, V3, V4}  (3 of 6)
//!     V4 released, high KL, energy>floor → COMMIT
//!     V5 released, energy=50 < floor=100 → EnergyBelowFloor
//!     V6 released, KL=0 → ABSTAIN
//!
//! Doctrine claim (INVENTION_STACK A4.3.1 / V2 upgrade): "IB Validators
//! V2 adds a structural jail layer over the V1 IB vote gate. CHSH-window
//! failures block all active participants deterministically for jail_epochs
//! epochs, preventing compromised validators from casting votes. Energy-floor
//! expiry models stake decay without changing IB semantics. After the jail
//! clears, the V1 gate resumes unchanged — jail is a pure structural overlay."
//!
//! Adversarial fixture: zero energy floor, jail-before-energy-check ordering,
//! slash overwrite extends expiry, exclusive boundary, cross-validator isolation.
//!
//! INVENTION_STACK A4.3.1: Information-Bottleneck Validators V2.

use evaporchain_ib_validators::{IbParams, StateSignature};
use evaporchain_ib_validators_v2::{
    ib_vote_v2,
    vote::{apply_chsh_failure_jail, apply_energy_jail},
    JailEntry, JailReason, JailState, VoteV2, VoteV2Error,
};

// ── Helpers ───────────────────────────────────────────────────────────────

fn vid(b: u8) -> [u8; 32] {
    [b; 32]
}

/// Uniform prior: 16-account energy spread evenly across the 1024-unit scale.
fn prior_sig() -> StateSignature {
    let energies: Vec<u64> = (0..16).map(|i| i as u64 * 64).collect();
    StateSignature::from_energies(&energies, 1024)
}

/// High-KL local: all accounts in bin 0 (full decay) vs the uniform prior.
fn high_kl_sig() -> StateSignature {
    StateSignature::from_energies(&[0u64; 16], 1024)
}

/// Conformist: identical to prior (KL=0).
fn conformist_sig() -> StateSignature {
    prior_sig()
}

fn params() -> IbParams {
    IbParams { lambda_mb: 100 }
}

const ENERGY_FLOOR: u64 = 100;
const JAIL_EPOCHS: u64 = 50;
const CHSH_WIN_START: u64 = 100;
const CHSH_WIN_END: u64 = 200;

/// Build initial jail: V4, V5, V6 jailed by CHSH failure at epoch 0.
/// expires_at = 0 + 50 = 50 (epoch 50 is the first free epoch).
fn chsh_jail_state() -> JailState {
    let mut js = JailState::new();
    apply_chsh_failure_jail(
        &mut js,
        &[vid(0x04), vid(0x05), vid(0x06)],
        CHSH_WIN_START,
        CHSH_WIN_END,
        0,           // current_epoch at the time of the CHSH failure
        JAIL_EPOCHS, // jail for 50 epochs
    );
    js
}

/// Returns the sig for a validator by byte id (0x01/0x06 = conformist, rest = high-KL).
fn sig_for(b: u8) -> StateSignature {
    if b == 0x01 || b == 0x06 {
        conformist_sig()
    } else {
        high_kl_sig()
    }
}

/// All 6 validators as (id_byte, energy).
const COMMITTEE: &[(u8, u64)] = &[
    (0x01, 2_000),
    (0x02, 2_000),
    (0x03, 1_500),
    (0x04, 800),
    (0x05, 50),
    (0x06, 1_000),
];

fn commit_count(js: &JailState, epoch: u64) -> usize {
    let prior = prior_sig();
    let p = params();
    COMMITTEE
        .iter()
        .filter(|(b, energy)| {
            ib_vote_v2(
                &sig_for(*b),
                &prior,
                &p,
                &vid(*b),
                *energy,
                ENERGY_FLOOR,
                js,
                epoch,
            )
            .unwrap()
                == VoteV2::Commit
        })
        .count()
}

// ── Non-trivial fixture: BFT committee under jail pressure ────────────────

#[test]
fn round1_only_unjailed_high_kl_validators_commit() {
    // Round 1 (epoch=10): CHSH jail active for V4/V5/V6.
    // V1 (KL=0) always abstains; V2 and V3 commit; V4/V5/V6 jailed.
    let js = chsh_jail_state();
    let prior = prior_sig();
    let p = params();

    assert_eq!(
        ib_vote_v2(
            &conformist_sig(),
            &prior,
            &p,
            &vid(0x01),
            2_000,
            ENERGY_FLOOR,
            &js,
            10
        )
        .unwrap(),
        VoteV2::Abstain,
        "V1 (KL=0) must abstain"
    );
    assert_eq!(
        ib_vote_v2(
            &high_kl_sig(),
            &prior,
            &p,
            &vid(0x02),
            2_000,
            ENERGY_FLOOR,
            &js,
            10
        )
        .unwrap(),
        VoteV2::Commit,
        "V2 (high KL, unjailed) must commit"
    );
    assert_eq!(
        ib_vote_v2(
            &high_kl_sig(),
            &prior,
            &p,
            &vid(0x03),
            1_500,
            ENERGY_FLOOR,
            &js,
            10
        )
        .unwrap(),
        VoteV2::Commit,
        "V3 (high KL, unjailed) must commit"
    );
    assert!(
        matches!(
            ib_vote_v2(
                &high_kl_sig(),
                &prior,
                &p,
                &vid(0x04),
                800,
                ENERGY_FLOOR,
                &js,
                10
            )
            .unwrap(),
            VoteV2::Jailed {
                reason: JailReason::ChshFailedWindow {
                    window_start: 100,
                    window_end: 200
                }
            }
        ),
        "V4 must be CHSH-jailed"
    );
    assert!(
        matches!(
            ib_vote_v2(
                &high_kl_sig(),
                &prior,
                &p,
                &vid(0x05),
                50,
                ENERGY_FLOOR,
                &js,
                10
            )
            .unwrap(),
            VoteV2::Jailed {
                reason: JailReason::ChshFailedWindow { .. }
            }
        ),
        "V5 must be CHSH-jailed (jail check precedes energy check)"
    );
    assert!(
        matches!(
            ib_vote_v2(
                &conformist_sig(),
                &prior,
                &p,
                &vid(0x06),
                1_000,
                ENERGY_FLOOR,
                &js,
                10
            )
            .unwrap(),
            VoteV2::Jailed {
                reason: JailReason::ChshFailedWindow { .. }
            }
        ),
        "V6 must be CHSH-jailed"
    );
}

#[test]
fn round1_commit_count_is_two() {
    // Jail pressure reduces committee commits to 2 of 6.
    let js = chsh_jail_state();
    assert_eq!(
        commit_count(&js, 10),
        2,
        "only V2 and V3 commit at epoch=10 (V1 abstains, V4/V5/V6 jailed)"
    );
}

#[test]
fn round2_commit_count_still_two() {
    // Jail still active at epoch=25.
    let js = chsh_jail_state();
    assert_eq!(
        commit_count(&js, 25),
        2,
        "only V2 and V3 commit at epoch=25 (jail still active)"
    );
}

#[test]
fn round3_jail_expired_v4_commits() {
    // epoch=50: jail expires (exclusive). V4 (high KL, energy>floor) → COMMIT.
    let js = chsh_jail_state();
    let result = ib_vote_v2(
        &high_kl_sig(),
        &prior_sig(),
        &params(),
        &vid(0x04),
        800,
        ENERGY_FLOOR,
        &js,
        50,
    )
    .unwrap();
    assert_eq!(
        result,
        VoteV2::Commit,
        "V4 must commit at epoch=50 (jail expired, high KL, energy above floor)"
    );
}

#[test]
fn round3_v5_energy_jailed_after_chsh_expiry() {
    // epoch=50: CHSH jail expires but V5's energy=50 < floor=100 → EnergyBelowFloor.
    let js = chsh_jail_state();
    let result = ib_vote_v2(
        &high_kl_sig(),
        &prior_sig(),
        &params(),
        &vid(0x05),
        50,
        ENERGY_FLOOR,
        &js,
        50,
    )
    .unwrap();
    assert!(
        matches!(
            result,
            VoteV2::Jailed {
                reason: JailReason::EnergyBelowFloor {
                    observed: 50,
                    floor: 100
                }
            }
        ),
        "V5 must be energy-jailed at epoch=50"
    );
}

#[test]
fn round3_v6_abstains_after_chsh_expiry() {
    // epoch=50: V6 released from jail but KL=0 → ABSTAIN (V1 IB gate).
    let js = chsh_jail_state();
    let result = ib_vote_v2(
        &conformist_sig(),
        &prior_sig(),
        &params(),
        &vid(0x06),
        1_000,
        ENERGY_FLOOR,
        &js,
        50,
    )
    .unwrap();
    assert_eq!(
        result,
        VoteV2::Abstain,
        "V6 must abstain at epoch=50 (jail expired, KL=0)"
    );
}

#[test]
fn round3_commit_count_recovers_to_three() {
    // After jail expires: {V2, V3, V4} commit = 3 of 6.
    let js = chsh_jail_state();
    assert_eq!(
        commit_count(&js, 50),
        3,
        "at epoch=50 (jail expired), V2/V3/V4 commit (V1 abstains, V5 energy-jailed, V6 abstains)"
    );
}

#[test]
fn doctrine_jail_reduces_then_recovers_commit_count() {
    // INVENTION_STACK A4.3.1 V2: CHSH jail reduces quorum participation,
    // then committee recovers exactly when jail expires.
    let js = chsh_jail_state();
    let jailed_count = commit_count(&js, 10);
    let free_count = commit_count(&js, 50);
    assert!(
        jailed_count < free_count,
        "commit count under jail ({jailed_count}) must recover after expiry ({free_count})"
    );
    assert_eq!(jailed_count, 2, "exactly 2 commits under jail");
    assert_eq!(free_count, 3, "exactly 3 commits after jail expires");
}

#[test]
fn v1_conformist_always_abstains_regardless_of_jail() {
    // KL=0 must abstain even when unjailed and energy is above floor,
    // proving that the IB gate — not jail — drives the abstention.
    let empty_js = JailState::new();
    let result = ib_vote_v2(
        &conformist_sig(),
        &prior_sig(),
        &params(),
        &vid(0x01),
        2_000,
        ENERGY_FLOOR,
        &empty_js,
        0,
    )
    .unwrap();
    assert_eq!(
        result,
        VoteV2::Abstain,
        "KL=0 must abstain even with no jail and energy well above floor"
    );
}

#[test]
fn apply_energy_jail_persists_with_deterministic_expiry() {
    // apply_energy_jail writes a persistent JailState entry;
    // expiry is exactly at expires_at_epoch (exclusive).
    let mut js = JailState::new();
    let wrote = apply_energy_jail(&mut js, vid(0x07), 50, ENERGY_FLOOR, 200);
    assert!(wrote, "energy below floor must write jail entry");
    assert!(js.is_jailed(&vid(0x07), 50), "jailed before expiry");
    assert!(
        js.is_jailed(&vid(0x07), 199),
        "jailed at last epoch before expiry"
    );
    assert!(
        !js.is_jailed(&vid(0x07), 200),
        "free at expiry epoch (exclusive)"
    );
}

#[test]
fn prune_expired_frees_whole_committee_in_bulk() {
    // prune_expired models the per-epoch jail-state cleanup the chain runs.
    // After pruning, V4 can commit from a clean JailState.
    let mut js = chsh_jail_state(); // V4/V5/V6, expires_at=50

    let pruned = js.prune_expired(50);
    assert_eq!(
        pruned, 3,
        "all 3 CHSH-jailed entries must be pruned at epoch=50"
    );
    assert_eq!(js.len(), 0, "jail state must be empty after bulk prune");

    assert_eq!(
        ib_vote_v2(
            &high_kl_sig(),
            &prior_sig(),
            &params(),
            &vid(0x04),
            800,
            ENERGY_FLOOR,
            &js,
            50
        )
        .unwrap(),
        VoteV2::Commit,
        "V4 must commit freely after pruned jail state"
    );
}

#[test]
fn vote_is_deterministic() {
    let js = chsh_jail_state();
    let r1 = ib_vote_v2(
        &high_kl_sig(),
        &prior_sig(),
        &params(),
        &vid(0x02),
        2_000,
        ENERGY_FLOOR,
        &js,
        10,
    )
    .unwrap();
    let r2 = ib_vote_v2(
        &high_kl_sig(),
        &prior_sig(),
        &params(),
        &vid(0x02),
        2_000,
        ENERGY_FLOOR,
        &js,
        10,
    )
    .unwrap();
    assert_eq!(r1, r2, "vote must be deterministic for same inputs");
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_zero_energy_floor_rejected() {
    let err = ib_vote_v2(
        &high_kl_sig(),
        &prior_sig(),
        &params(),
        &vid(0x01),
        1_000,
        0,
        &JailState::new(),
        0,
    )
    .unwrap_err();
    assert_eq!(err, VoteV2Error::ZeroFloor, "zero floor must be rejected");
}

#[test]
fn adversarial_jail_check_precedes_energy_check() {
    // V5 is simultaneously CHSH-jailed AND energy-below-floor.
    // Jail lookup fires first — reason must be ChshFailedWindow, not EnergyBelowFloor.
    let js = chsh_jail_state();
    let result = ib_vote_v2(
        &high_kl_sig(),
        &prior_sig(),
        &params(),
        &vid(0x05),
        50,
        ENERGY_FLOOR,
        &js,
        10,
    )
    .unwrap();
    assert!(
        matches!(
            result,
            VoteV2::Jailed {
                reason: JailReason::ChshFailedWindow { .. }
            }
        ),
        "CHSH jail must take precedence over energy-below-floor check"
    );
}

#[test]
fn adversarial_slash_overwrites_shorter_chsh_jail() {
    // An operator slash issued after a shorter CHSH jail extends the expiry.
    // The CHSH expires at 30; the slash at 80 takes precedence.
    let mut js = JailState::new();
    apply_chsh_failure_jail(&mut js, &[vid(0x04)], 100, 200, 0, 30); // expires_at=30

    js.insert(
        vid(0x04),
        JailEntry {
            reason: JailReason::Slashed { code: 1 },
            expires_at_epoch: 80,
        },
    );

    assert!(
        js.is_jailed(&vid(0x04), 29),
        "jailed before original CHSH expiry"
    );
    assert!(
        js.is_jailed(&vid(0x04), 50),
        "jailed after CHSH expiry but before slash expiry"
    );
    assert!(js.is_jailed(&vid(0x04), 79), "jailed at epoch 79");
    assert!(
        !js.is_jailed(&vid(0x04), 80),
        "free at slash expiry (exclusive)"
    );

    let result = ib_vote_v2(
        &high_kl_sig(),
        &prior_sig(),
        &params(),
        &vid(0x04),
        1_000,
        ENERGY_FLOOR,
        &js,
        50,
    )
    .unwrap();
    assert!(
        matches!(
            result,
            VoteV2::Jailed {
                reason: JailReason::Slashed { code: 1 }
            }
        ),
        "slash overwrite must show Slashed reason"
    );
}

#[test]
fn adversarial_jail_expiry_boundary_is_exclusive() {
    // expires_at_epoch=50: epoch 49 is jailed; epoch 50 is the first free epoch.
    let js = chsh_jail_state();

    assert!(
        matches!(
            ib_vote_v2(
                &high_kl_sig(),
                &prior_sig(),
                &params(),
                &vid(0x04),
                800,
                ENERGY_FLOOR,
                &js,
                49
            )
            .unwrap(),
            VoteV2::Jailed { .. }
        ),
        "epoch 49 must be the last jailed epoch"
    );
    assert_eq!(
        ib_vote_v2(
            &high_kl_sig(),
            &prior_sig(),
            &params(),
            &vid(0x04),
            800,
            ENERGY_FLOOR,
            &js,
            50
        )
        .unwrap(),
        VoteV2::Commit,
        "epoch 50 must be the first free epoch (exclusive boundary)"
    );
}

#[test]
fn adversarial_cross_validator_jail_isolation() {
    // Jailing V4/V5/V6 must not affect V2's vote.
    let js = chsh_jail_state();
    assert_eq!(
        ib_vote_v2(
            &high_kl_sig(),
            &prior_sig(),
            &params(),
            &vid(0x02),
            2_000,
            ENERGY_FLOOR,
            &js,
            10
        )
        .unwrap(),
        VoteV2::Commit,
        "V2 must not be affected by other validators' jail entries"
    );
}
