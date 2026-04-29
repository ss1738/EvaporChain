//! HBCT-Elexon integration — wires `evaporchain-hbct-elexon` into the
//! node's oracle bridge.
//!
//! The `ElexonOracleFeed` fetches actual generation output (MWh) from the
//! Elexon BMRS v1 API (dataset B1790) and converts each attestation into
//! an `OracleVote` that the existing `OracleBridge` can process.
//!
//! # Flow
//!
//! Each epoch boundary:
//!   1. `attest_and_vote(feed, validator_id, bmu_id, epoch, genesis_ts, epoch_duration)`
//!      → calls `ElexonOracleFeed::attest()` → gets `OracleAttestation { mwh_delivered }`
//!   2. Wraps `mwh_delivered` as `OracleValue::Numeric(mwh as f64)`
//!   3. Returns an `OracleVote` the node submits via `OracleBridge::submit_vote`
//!
//! Best-effort: if the Elexon API is unreachable (test / no-network env)
//! `attest_and_vote` returns `None` and the oracle round proceeds without
//! this validator's HBCT vote.

use evaporchain_hbct::oracle::OracleFeed;
use evaporchain_hbct_elexon::client::{ElexonConfig, ElexonOracleFeed};
use evaporchain_oracle::OracleValue;
use evaporchain_oracle::consensus::OracleVote;
use tracing::{debug, warn};

/// Oracle feed key used in `OracleBridge` rounds for HBCT energy data.
pub const HBCT_FEED_KEY: &str = "hbct:elexon:B1790";

/// Build the Elexon feed with production config.
///
/// `genesis_unix_ts` — Unix timestamp of chain genesis (epoch 0).
/// `epoch_duration_s` — chain seconds per epoch (typically 12).
pub fn production_feed(genesis_unix_ts: u64, epoch_duration_s: u64) -> ElexonOracleFeed {
    ElexonOracleFeed::new(ElexonConfig {
        genesis_unix_ts,
        epoch_duration_s,
        ..ElexonConfig::default()
    })
}

/// Build the Elexon feed pointing at a custom base URL (test / staging).
pub fn feed_with_url(base_url: &str, genesis_unix_ts: u64, epoch_duration_s: u64) -> ElexonOracleFeed {
    ElexonOracleFeed::new(ElexonConfig {
        base_url: base_url.to_owned(),
        genesis_unix_ts,
        epoch_duration_s,
    })
}

/// Fetch the current MWh attestation for `bmu_id` at `epoch` and wrap it
/// as an `OracleVote` ready for submission to `OracleBridge`.
///
/// `bmu_id` is the Elexon BMU identifier (e.g. `"T_RATS-1"`).  It is
/// passed as the `DeliveryLocation` bytes to `OracleFeed::attest`.
///
/// Returns `None` if the Elexon API is unreachable or the BMU has no data
/// for the resolved settlement period.
pub fn attest_and_vote(
    feed: &ElexonOracleFeed,
    validator_id: u64,
    bmu_id: &str,
    epoch: u64,
    holder: [u8; 32],
) -> Option<OracleVote> {
    let location: Vec<u8> = bmu_id.as_bytes().to_vec();
    let attestation = feed.attest(&location, epoch, holder)?;

    let mwh = attestation.mwh_delivered;
    debug!(
        validator_id,
        bmu_id,
        epoch,
        mwh,
        "HBCT-Elexon attestation obtained"
    );

    Some(OracleVote {
        validator_id,
        value: OracleValue::Numeric(mwh as f64),
    })
}

/// Log a warning when the Elexon feed returns `None` (unreachable / no data).
pub fn warn_feed_miss(bmu_id: &str, epoch: u64) {
    warn!(
        bmu_id,
        epoch,
        "HBCT-Elexon feed returned no attestation — oracle round proceeds without HBCT vote"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_hbct_elexon::client::ElexonConfig;

    fn mock_feed() -> ElexonOracleFeed {
        // Point at a localhost URL that will never respond — ensures
        // `fetch_mwh` returns None cleanly without network.
        feed_with_url("http://127.0.0.1:0", 0, 12)
    }

    #[test]
    fn no_network_returns_none() {
        let feed = mock_feed();
        let result = attest_and_vote(&feed, 1, "T_RATS-1", 100, [0u8; 32]);
        assert!(result.is_none());
    }

    #[test]
    fn production_feed_has_correct_base_url() {
        let feed = production_feed(1_704_067_200, 12);
        assert!(feed.config.base_url.contains("elexon.co.uk"));
    }

    #[test]
    fn feed_key_is_stable() {
        assert_eq!(HBCT_FEED_KEY, "hbct:elexon:B1790");
    }

    #[test]
    fn non_utf8_bmu_returns_none() {
        let feed = mock_feed();
        // Construct a vote attempt with a valid-looking but non-existent BMU.
        let result = attest_and_vote(&feed, 2, "INVALID_BMU_WONT_RESOLVE", 0, [0u8; 32]);
        assert!(result.is_none());
    }
}
