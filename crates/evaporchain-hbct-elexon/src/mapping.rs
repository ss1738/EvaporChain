//! Chain epoch → Elexon settlement date + period.

/// Decoded settlement slot ready for an Elexon API query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElexonSlot {
    /// Calendar date string "YYYY-MM-DD" in UTC.
    pub date: String,
    /// Settlement period 1..=48 (each 30 minutes; clamped at 48).
    pub period: u8,
}

/// Map a chain epoch to an Elexon settlement slot.
///
/// - `genesis_unix_ts`: Unix seconds at chain epoch 0.
/// - `epoch_duration_s`: chain seconds per epoch.
/// - `hour_slot`: the epoch at which the capacity slot *closes*.
pub fn epoch_to_elexon_slot(
    genesis_unix_ts: u64,
    epoch_duration_s: u64,
    hour_slot: u64,
) -> ElexonSlot {
    let slot_unix_ts = genesis_unix_ts.saturating_add(hour_slot.saturating_mul(epoch_duration_s));

    let days_since_epoch = slot_unix_ts / 86400;
    let secs_in_day = slot_unix_ts % 86400;

    // Settlement period = floor(secs_in_day / 1800) + 1, clamped 1..=48.
    let period = ((secs_in_day / 1800) + 1).min(48) as u8;

    // Gregorian calendar from days-since-Unix-epoch (1970-01-01).
    let date = unix_days_to_date(days_since_epoch);

    ElexonSlot { date, period }
}

/// Convert days since Unix epoch (1970-01-01) to "YYYY-MM-DD" string.
/// Implements the Gregorian algorithm without external date libraries.
fn unix_days_to_date(days: u64) -> String {
    // Algorithm: civil date from days offset.
    // Reference: Howard Hinnant's civil_from_days (public domain).
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_epoch_start_is_1970_01_01() {
        assert_eq!(unix_days_to_date(0), "1970-01-01");
    }

    #[test]
    fn known_date() {
        // 2024-01-01 = 19723 days since Unix epoch.
        assert_eq!(unix_days_to_date(19723), "2024-01-01");
    }

    #[test]
    fn midnight_is_period_1() {
        // genesis at 2024-01-01T00:00:00Z, 12s epochs, slot 0 = midnight.
        let s = epoch_to_elexon_slot(1_704_067_200, 12, 0);
        assert_eq!(s.date, "2024-01-01");
        assert_eq!(s.period, 1);
    }

    #[test]
    fn first_half_hour_boundary_is_period_2() {
        // 1800 seconds into day → second SP.
        let genesis = 1_704_067_200u64; // 2024-01-01T00:00:00Z
                                        // epoch 150 = 1800 s → start of SP 2
        let s = epoch_to_elexon_slot(genesis, 12, 150);
        assert_eq!(s.period, 2);
    }

    #[test]
    fn period_clamped_at_48() {
        // SP 49/50 edge: 23:59:00 → floor(86340/1800)+1 = 48+1 = 49, clamped to 48.
        let genesis = 0u64;
        let slot_unix = 86340u64; // 23:59:00
        let slot = slot_unix / 12;
        let s = epoch_to_elexon_slot(genesis, 12, slot);
        assert_eq!(s.period, 48);
    }

    #[test]
    fn correct_date_after_many_epochs() {
        // genesis 2024-01-01T00:00:00Z, 12s epochs, 3600/12=300 epochs per hour.
        // slot = 300*24 = one day later → 2024-01-02.
        let genesis = 1_704_067_200u64;
        let s = epoch_to_elexon_slot(genesis, 12, 300 * 24);
        assert_eq!(s.date, "2024-01-02");
        assert_eq!(s.period, 1);
    }
}
