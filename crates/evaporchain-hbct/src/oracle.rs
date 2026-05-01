//! `OracleFeed` trait — settlement-system / smart-meter attestation.
//!
//! Production wires GB Elexon BMRS + ENTSO-E adapters via this
//! trait. Substrate exposes the interface and a deterministic
//! in-memory `MockOracleFeed` impl for tests.

use serde::{Deserialize, Serialize};

use evaporchain_types::AccountAddress;

use crate::token::{DeliveryLocation, HourSlot};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OracleAttestation {
    pub delivery_location: DeliveryLocation,
    pub hour_slot: HourSlot,
    pub holder: AccountAddress,
    /// Actual MWh delivered as confirmed by the settlement system.
    /// May be less than the issued token if delivery underperformed.
    pub mwh_delivered: u64,
    /// Wall-clock signal that the attestation was emitted at this
    /// chain epoch (chain-side check that the oracle isn't sending
    /// pre-slot-close attestations).
    pub attested_at_epoch: u64,
}

/// Trait the chain calls to confirm a slot's actual delivery.
/// Real impls wrap GB Elexon BMRS / ENTSO-E HTTP feeds; substrate
/// supplies a deterministic mock for unit testing.
pub trait OracleFeed {
    fn attest(
        &self,
        location: &DeliveryLocation,
        slot: HourSlot,
        holder: AccountAddress,
    ) -> Option<OracleAttestation>;
}

/// In-memory mock that returns whatever attestations the test
/// pre-populated. Convenient for unit testing the burn/settle path
/// without needing a live grid feed.
#[derive(Debug, Clone, Default)]
pub struct MockOracleFeed {
    pub attestations: Vec<OracleAttestation>,
}

impl OracleFeed for MockOracleFeed {
    fn attest(
        &self,
        location: &DeliveryLocation,
        slot: HourSlot,
        holder: AccountAddress,
    ) -> Option<OracleAttestation> {
        self.attestations
            .iter()
            .find(|a| &a.delivery_location == location && a.hour_slot == slot && a.holder == holder)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::AccountAddress;

    fn addr(b: u8) -> AccountAddress {
        [b; 32]
    }

    #[test]
    fn mock_returns_pre_populated() {
        let mock = MockOracleFeed {
            attestations: vec![OracleAttestation {
                delivery_location: b"BMU-1".to_vec(),
                hour_slot: 100,
                holder: addr(1),
                mwh_delivered: 50,
                attested_at_epoch: 100,
            }],
        };
        let a = mock.attest(&b"BMU-1".to_vec(), 100, addr(1)).unwrap();
        assert_eq!(a.mwh_delivered, 50);
    }

    #[test]
    fn mock_returns_none_for_unknown() {
        let mock = MockOracleFeed::default();
        assert!(mock.attest(&b"BMU-1".to_vec(), 100, addr(1)).is_none());
    }

    #[test]
    fn mock_distinguishes_holders() {
        let mock = MockOracleFeed {
            attestations: vec![OracleAttestation {
                delivery_location: b"BMU-1".to_vec(),
                hour_slot: 100,
                holder: addr(1),
                mwh_delivered: 50,
                attested_at_epoch: 100,
            }],
        };
        assert!(mock.attest(&b"BMU-1".to_vec(), 100, addr(1)).is_some());
        assert!(mock.attest(&b"BMU-1".to_vec(), 100, addr(2)).is_none());
    }
}
