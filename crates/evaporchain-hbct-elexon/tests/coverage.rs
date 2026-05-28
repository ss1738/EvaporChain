//! Coverage tests for the Elexon BMRS oracle adapter.
//!
//! Scope: pure-logic surface (`epoch_to_elexon_slot`, `ElexonSlot`,
//! `ElexonConfig::default`). The HTTP-IO `fetch_mwh` /
//! `OracleFeed::attest` paths need a synthetic server — out of scope.

use evaporchain_hbct_elexon::mapping::{epoch_to_elexon_slot, ElexonSlot};
use evaporchain_hbct_elexon::ElexonConfig;

// =================================================================
// ElexonConfig defaults
// =================================================================

#[test]
fn elexon_config_default_matches_doctrine() {
    let c = ElexonConfig::default();
    assert_eq!(c.base_url, "https://data.elexon.co.uk/bmrs/api/v1");
    assert_eq!(c.genesis_unix_ts, 0);
    assert_eq!(c.epoch_duration_s, 12);
}

// =================================================================
// epoch_to_elexon_slot — date encoding
// =================================================================

#[test]
fn unix_epoch_zero_maps_to_1970_01_01_sp1() {
    let s = epoch_to_elexon_slot(0, 12, 0);
    assert_eq!(s.date, "1970-01-01");
    assert_eq!(s.period, 1);
}

#[test]
fn one_day_later_yields_next_date() {
    // Genesis at midnight UTC; one slot per second; 86400 slots = one day.
    let s = epoch_to_elexon_slot(0, 1, 86_400);
    assert_eq!(s.date, "1970-01-02");
    assert_eq!(s.period, 1);
}

#[test]
fn leap_year_boundary_2000_02_29() {
    // Year 2000 was a leap year (div by 400). Pin Feb 29 produces correct date.
    // 2000-02-29 00:00:00 UTC = 951782400 unix seconds.
    let s = epoch_to_elexon_slot(951_782_400, 1, 0);
    assert_eq!(s.date, "2000-02-29");
    assert_eq!(s.period, 1);
}

#[test]
fn standard_year_2024_01_01() {
    // 2024-01-01 00:00:00 UTC = 1704067200 unix seconds.
    let s = epoch_to_elexon_slot(1_704_067_200, 1, 0);
    assert_eq!(s.date, "2024-01-01");
}

#[test]
fn end_of_year_boundary_2023_12_31() {
    // 2023-12-31 23:59:00 UTC = 1704066540 unix seconds → period 48.
    let s = epoch_to_elexon_slot(1_704_066_540, 1, 0);
    assert_eq!(s.date, "2023-12-31");
    assert_eq!(s.period, 48);
}

// =================================================================
// Settlement period encoding
// =================================================================

#[test]
fn settlement_periods_step_at_30_minutes() {
    // 00:00 → SP 1, 00:30 → SP 2, 01:00 → SP 3, ...
    for half_hour in 0..48u64 {
        let s = epoch_to_elexon_slot(half_hour * 1_800, 1, 0);
        assert_eq!(
            s.period,
            (half_hour + 1) as u8,
            "SP at +{half_hour} half-hours"
        );
    }
}

#[test]
fn settlement_period_clamps_at_48() {
    // 23:30 = 23*3600 + 30*60 = 84_600 → SP 48 exactly.
    let s = epoch_to_elexon_slot(84_600, 1, 0);
    assert_eq!(s.period, 48);
    // 23:59:59 = 86_399 → would compute SP 48 (48 is the cap).
    let s2 = epoch_to_elexon_slot(86_399, 1, 0);
    assert_eq!(s2.period, 48);
}

#[test]
fn settlement_period_inside_half_hour_uses_floor() {
    // 00:15 should still be SP 1 (the first half-hour bucket).
    let s = epoch_to_elexon_slot(15 * 60, 1, 0);
    assert_eq!(s.period, 1);
    // 00:29:59 is the last second of SP 1.
    let s_end = epoch_to_elexon_slot(29 * 60 + 59, 1, 0);
    assert_eq!(s_end.period, 1);
    // 00:30 starts SP 2.
    let s_next = epoch_to_elexon_slot(30 * 60, 1, 0);
    assert_eq!(s_next.period, 2);
}

// =================================================================
// Saturating math
// =================================================================

#[test]
fn saturating_mul_does_not_panic_on_u64_max_hour_slot() {
    // saturating_mul(u64::MAX, 12) clamps; saturating_add then clamps.
    // Must not panic. The resulting date is far in the future but well-formed.
    let _ = epoch_to_elexon_slot(0, 12, u64::MAX);
}

#[test]
fn saturating_add_does_not_panic_on_huge_genesis() {
    let _ = epoch_to_elexon_slot(u64::MAX - 100, 1, 1_000);
}

// =================================================================
// epoch_duration_s scaling
// =================================================================

#[test]
fn epoch_duration_scales_slot_offset() {
    // Genesis at 1970-01-01 00:00:00; 30-second epochs; 60 epochs → 30 min.
    let s = epoch_to_elexon_slot(0, 30, 60);
    // 60 * 30 = 1800 sec = 00:30 → SP 2.
    assert_eq!(s.date, "1970-01-01");
    assert_eq!(s.period, 2);
}

#[test]
fn one_epoch_at_zero_duration_is_no_op() {
    // epoch_duration_s = 0 → hour_slot doesn't advance time.
    let s = epoch_to_elexon_slot(0, 0, 1_000);
    assert_eq!(s.date, "1970-01-01");
    assert_eq!(s.period, 1);
}

// =================================================================
// ElexonSlot
// =================================================================

#[test]
fn elexon_slot_eq_and_debug() {
    let a = ElexonSlot {
        date: "2024-01-01".into(),
        period: 5,
    };
    let b = ElexonSlot {
        date: "2024-01-01".into(),
        period: 5,
    };
    let c = ElexonSlot {
        date: "2024-01-01".into(),
        period: 6,
    };
    assert_eq!(a, b);
    assert_ne!(a, c);
    // Debug renders without panic.
    let _ = format!("{a:?}");
}
