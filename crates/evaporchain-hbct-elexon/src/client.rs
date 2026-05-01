//! `ElexonOracleFeed` — OracleFeed backed by the Elexon BMRS v1 REST API.
//!
//! Dataset B1790: Actual Generation Output per Generation Unit.
//! Endpoint: `GET {base_url}/datasets/B1790`
//! Params: `settlementDate`, `settlementPeriod`, `ngcBmUnit`
//!
//! Response JSON (relevant fields):
//! ```json
//! {
//!   "data": [
//!     {
//!       "ngcBmUnit": "T_RATS-1",
//!       "settlementDate": "2024-01-01",
//!       "settlementPeriod": 1,
//!       "quantity": 123.45
//!     }
//!   ]
//! }
//! ```
//! `quantity` is in MW over a half-hour SP; MWh = quantity × 0.5.
//! We round down to u64 to stay in integer token units.

use serde::Deserialize;

use evaporchain_hbct::oracle::{OracleAttestation, OracleFeed};
use evaporchain_hbct::token::{DeliveryLocation, HourSlot};
use evaporchain_types::AccountAddress;

use crate::mapping::epoch_to_elexon_slot;

/// Configuration for the Elexon BMRS adapter.
#[derive(Debug, Clone)]
pub struct ElexonConfig {
    /// Elexon BMRS v1 base URL (without trailing slash).
    pub base_url: String,
    /// Unix seconds at chain genesis (epoch 0).
    pub genesis_unix_ts: u64,
    /// Chain epoch duration in seconds (default 12).
    pub epoch_duration_s: u64,
}

impl Default for ElexonConfig {
    fn default() -> Self {
        Self {
            base_url: "https://data.elexon.co.uk/bmrs/api/v1".to_owned(),
            genesis_unix_ts: 0,
            epoch_duration_s: 12,
        }
    }
}

/// Oracle feed backed by the live Elexon BMRS API.
#[derive(Debug, Clone)]
pub struct ElexonOracleFeed {
    pub config: ElexonConfig,
}

impl ElexonOracleFeed {
    pub fn new(config: ElexonConfig) -> Self {
        Self { config }
    }

    /// Query Elexon B1790 for the BMU's actual output in the given SP.
    /// Returns the MWh quantity on success, or `None` on any error/miss.
    fn fetch_mwh(&self, bmu_id: &str, date: &str, period: u8) -> Option<u64> {
        let url = format!("{}/datasets/B1790", self.config.base_url);
        let resp = ureq::get(&url)
            .query("settlementDate", date)
            .query("settlementPeriod", &period.to_string())
            .query("ngcBmUnit", bmu_id)
            .call()
            .ok()?;

        let body: B1790Response = resp.into_json().ok()?;

        // Sum MW across all matching records (may be >1 if data has revisions).
        // Elexon reports MW; multiply by 0.5 for MWh over a 30-min SP.
        let total_mw: f64 = body
            .data
            .iter()
            .filter(|r| r.ngc_bm_unit.eq_ignore_ascii_case(bmu_id))
            .map(|r| r.quantity)
            .sum();

        Some((total_mw * 0.5).floor() as u64)
    }
}

impl OracleFeed for ElexonOracleFeed {
    fn attest(
        &self,
        location: &DeliveryLocation,
        slot: HourSlot,
        holder: AccountAddress,
    ) -> Option<OracleAttestation> {
        let bmu_id = std::str::from_utf8(location).ok()?;
        let elexon_slot = epoch_to_elexon_slot(
            self.config.genesis_unix_ts,
            self.config.epoch_duration_s,
            slot,
        );
        let mwh = self.fetch_mwh(bmu_id, &elexon_slot.date, elexon_slot.period)?;

        Some(OracleAttestation {
            delivery_location: location.clone(),
            hour_slot: slot,
            holder,
            mwh_delivered: mwh,
            attested_at_epoch: slot,
        })
    }
}

// ── Elexon B1790 response shape ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct B1790Response {
    data: Vec<B1790Record>,
}

#[derive(Debug, Deserialize)]
struct B1790Record {
    #[serde(rename = "ngcBmUnit")]
    ngc_bm_unit: String,
    quantity: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke-test: default config is sane (no network).
    #[test]
    fn default_config_has_expected_base_url() {
        let feed = ElexonOracleFeed::new(ElexonConfig::default());
        assert!(feed
            .config
            .base_url
            .starts_with("https://data.elexon.co.uk"));
    }

    /// Non-UTF-8 location bytes return None without panicking.
    #[test]
    fn non_utf8_location_returns_none() {
        let feed = ElexonOracleFeed::new(ElexonConfig::default());
        let bad_location = vec![0xFF, 0xFE];
        let result = feed.attest(&bad_location, 100, [0u8; 32]);
        assert!(result.is_none());
    }

    /// Verify mwh rounding: 100 MW × 0.5h = 50 MWh.
    #[test]
    fn mwh_calculation() {
        let mw = 100.0f64;
        let mwh = (mw * 0.5).floor() as u64;
        assert_eq!(mwh, 50);
    }

    /// Fractional MW rounds down.
    #[test]
    fn mwh_rounds_down() {
        let mw = 99.9f64;
        let mwh = (mw * 0.5).floor() as u64;
        assert_eq!(mwh, 49);
    }
}
