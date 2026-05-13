//! `DeployReceipt` — the structured event the chain emits after a
//! successful deploy. The output analogue of `DeployRequest`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_app_templates::TemplateClass;

/// Domain-separation tag for the receipt commitment hash.
pub const RECEIPT_DOMAIN_TAG: &[u8] = b"evaporchain:receipt:v1\0";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReceiptError {
    #[error("class id {0:#010x} is outside the app-template range")]
    OutOfRange(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployReceipt {
    /// Back-pointer to the originating `DeployRequest`'s commitment.
    /// dApps poll on this to find their receipt without scanning.
    pub commitment: [u8; 32],
    /// The deterministic on-chain handle for the new instance —
    /// derived from `(class, deployer, nonce)` at materialise time.
    pub instance_id: [u8; 32],
    pub template_class: TemplateClass,
    pub deployer: [u8; 32],
    /// Gas fee actually charged. Equals `fees::fee_for(typed)`
    /// for a successful deploy at base network multiplier; the
    /// transaction layer may apply a per-block multiplier on top.
    pub fee_paid: u64,
    /// Epoch at which the deploy actually landed (chain-determined,
    /// may differ from the request's `submitted_at_epoch`).
    pub deployed_at_epoch: u64,
    /// The block height the deploy was finalised at — gives
    /// strong-consistency: "final at H" is a stronger claim than
    /// "happened somewhere."
    pub block_height: u64,
}

impl DeployReceipt {
    pub fn new(
        commitment: [u8; 32],
        instance_id: [u8; 32],
        template_class: TemplateClass,
        deployer: [u8; 32],
        fee_paid: u64,
        deployed_at_epoch: u64,
        block_height: u64,
    ) -> Result<Self, ReceiptError> {
        if !template_class.is_in_app_range() {
            return Err(ReceiptError::OutOfRange(template_class.0));
        }
        Ok(Self {
            commitment,
            instance_id,
            template_class,
            deployer,
            fee_paid,
            deployed_at_epoch,
            block_height,
        })
    }

    /// Canonical bytes for the receipt — what the chain hashes for
    /// the event-log Merkle inclusion. Layout:
    ///
    /// ```text
    /// b"evaporchain:receipt:v1\0"   (23 B, fixed)
    /// commitment                    (32 B)
    /// instance_id                   (32 B)
    /// template_class                (u32 LE, 4 B)
    /// deployer                      (32 B)
    /// fee_paid                      (u64 LE, 8 B)
    /// deployed_at_epoch             (u64 LE, 8 B)
    /// block_height                  (u64 LE, 8 B)
    /// ```
    ///
    /// All fields fixed-width — no length prefix needed (the
    /// receipt has no variable-shape data).
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(RECEIPT_DOMAIN_TAG.len() + 32 + 32 + 4 + 32 + 8 + 8 + 8);
        out.extend_from_slice(RECEIPT_DOMAIN_TAG);
        out.extend_from_slice(&self.commitment);
        out.extend_from_slice(&self.instance_id);
        out.extend_from_slice(&self.template_class.0.to_le_bytes());
        out.extend_from_slice(&self.deployer);
        out.extend_from_slice(&self.fee_paid.to_le_bytes());
        out.extend_from_slice(&self.deployed_at_epoch.to_le_bytes());
        out.extend_from_slice(&self.block_height.to_le_bytes());
        out
    }

    /// BLAKE3 commitment of the receipt — usable as the event id.
    pub fn event_id(&self) -> [u8; 32] {
        *blake3::hash(&self.canonical_bytes()).as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_app_templates::class::{MAYFLY, SINGH_SABI};

    fn r() -> DeployReceipt {
        DeployReceipt::new(
            [0x11; 32], [0x22; 32], SINGH_SABI, [0x33; 32], 1_400, 500, 12_345,
        )
        .unwrap()
    }

    #[test]
    fn rejects_out_of_range_class() {
        let err = DeployReceipt::new(
            [0; 32],
            [0; 32],
            TemplateClass(0x9999_9999),
            [0; 32],
            0,
            0,
            0,
        )
        .unwrap_err();
        assert_eq!(err, ReceiptError::OutOfRange(0x9999_9999));
    }

    #[test]
    fn canonical_bytes_starts_with_domain_tag() {
        let bytes = r().canonical_bytes();
        assert!(bytes.starts_with(RECEIPT_DOMAIN_TAG));
    }

    #[test]
    fn canonical_bytes_layout_is_locked() {
        // Anti-regression: lock the byte layout so future refactors
        // can't silently shuffle the fields.
        let bytes = r().canonical_bytes();
        let tag = RECEIPT_DOMAIN_TAG.len();
        assert_eq!(&bytes[..tag], RECEIPT_DOMAIN_TAG);
        // commitment (32 B)
        assert_eq!(&bytes[tag..tag + 32], &[0x11; 32]);
        // instance_id (32 B)
        assert_eq!(&bytes[tag + 32..tag + 64], &[0x22; 32]);
        // template_class (u32 LE, 4 B)
        let cls = u32::from_le_bytes(bytes[tag + 64..tag + 68].try_into().unwrap());
        assert_eq!(cls, SINGH_SABI.0);
        // deployer (32 B)
        assert_eq!(&bytes[tag + 68..tag + 100], &[0x33; 32]);
        // fee_paid (u64 LE)
        let fee = u64::from_le_bytes(bytes[tag + 100..tag + 108].try_into().unwrap());
        assert_eq!(fee, 1_400);
        // deployed_at_epoch (u64 LE)
        let epoch = u64::from_le_bytes(bytes[tag + 108..tag + 116].try_into().unwrap());
        assert_eq!(epoch, 500);
        // block_height (u64 LE)
        let h = u64::from_le_bytes(bytes[tag + 116..tag + 124].try_into().unwrap());
        assert_eq!(h, 12_345);
    }

    #[test]
    fn canonical_bytes_total_length_is_fixed() {
        // Receipt has no variable-shape data; total bytes is
        // domain_tag + 32 + 32 + 4 + 32 + 8 + 8 + 8 = tag + 124.
        let bytes = r().canonical_bytes();
        assert_eq!(bytes.len(), RECEIPT_DOMAIN_TAG.len() + 124);
    }

    #[test]
    fn canonical_bytes_are_deterministic() {
        // Same receipt → byte-identical canonical bytes.
        assert_eq!(r().canonical_bytes(), r().canonical_bytes());
    }

    #[test]
    fn event_id_is_blake3_of_canonical_bytes() {
        let receipt = r();
        let expected: [u8; 32] = *blake3::hash(&receipt.canonical_bytes()).as_bytes();
        assert_eq!(receipt.event_id(), expected);
    }

    #[test]
    fn different_commitments_produce_different_event_ids() {
        let mut a = r();
        let mut b = r();
        a.commitment = [0xAA; 32];
        b.commitment = [0xBB; 32];
        assert_ne!(a.event_id(), b.event_id());
    }

    #[test]
    fn different_block_heights_produce_different_event_ids() {
        // Same deploy retried at a different height yields a
        // different event id — useful for validators distinguishing
        // double-finalisations.
        let mut a = r();
        let mut b = r();
        a.block_height = 12_345;
        b.block_height = 12_346;
        assert_ne!(a.event_id(), b.event_id());
    }

    #[test]
    fn different_classes_produce_different_event_ids() {
        let mut a = r();
        let mut b = r();
        a.template_class = SINGH_SABI;
        b.template_class = MAYFLY;
        assert_ne!(a.event_id(), b.event_id());
    }

    #[test]
    fn different_instance_ids_produce_different_event_ids() {
        let mut a = r();
        let mut b = r();
        a.instance_id = [0xC1; 32];
        b.instance_id = [0xC2; 32];
        assert_ne!(a.event_id(), b.event_id());
    }

    #[test]
    fn different_deployers_produce_different_event_ids() {
        let mut a = r();
        let mut b = r();
        a.deployer = [0xD1; 32];
        b.deployer = [0xD2; 32];
        assert_ne!(a.event_id(), b.event_id());
    }

    #[test]
    fn different_fee_paids_produce_different_event_ids() {
        let mut a = r();
        let mut b = r();
        a.fee_paid = 1_000;
        b.fee_paid = 1_001;
        assert_ne!(a.event_id(), b.event_id());
    }

    #[test]
    fn class_at_app_range_boundaries_accepted() {
        use evaporchain_app_templates::class::{APP_TEMPLATE_RANGE_END, APP_TEMPLATE_RANGE_START};
        let lo = DeployReceipt::new(
            [0; 32],
            [0; 32],
            TemplateClass(APP_TEMPLATE_RANGE_START),
            [0; 32],
            0,
            0,
            0,
        );
        assert!(lo.is_ok());
        let hi = DeployReceipt::new(
            [0; 32],
            [0; 32],
            TemplateClass(APP_TEMPLATE_RANGE_END),
            [0; 32],
            0,
            0,
            0,
        );
        assert!(hi.is_ok());
        // Off-by-one just below should reject.
        let below = DeployReceipt::new(
            [0; 32],
            [0; 32],
            TemplateClass(APP_TEMPLATE_RANGE_START - 1),
            [0; 32],
            0,
            0,
            0,
        );
        assert!(below.is_err());
        // Off-by-one just above should reject.
        let above = DeployReceipt::new(
            [0; 32],
            [0; 32],
            TemplateClass(APP_TEMPLATE_RANGE_END + 1),
            [0; 32],
            0,
            0,
            0,
        );
        assert!(above.is_err());
    }

    #[test]
    fn domain_tag_actually_separates_from_naive_hash() {
        // Receipt event_id MUST differ from a naive BLAKE3 of the
        // body without the tag — otherwise some other layer hashing
        // the same fields could collide.
        let receipt = r();
        let with_tag = receipt.event_id();
        let mut naive = blake3::Hasher::new();
        naive.update(&receipt.commitment);
        naive.update(&receipt.instance_id);
        naive.update(&receipt.template_class.0.to_le_bytes());
        naive.update(&receipt.deployer);
        naive.update(&receipt.fee_paid.to_le_bytes());
        naive.update(&receipt.deployed_at_epoch.to_le_bytes());
        naive.update(&receipt.block_height.to_le_bytes());
        let without_tag: [u8; 32] = *naive.finalize().as_bytes();
        assert_ne!(with_tag, without_tag);
    }

    #[test]
    fn round_trip_serde() {
        let receipt = r();
        let s = serde_json::to_string(&receipt).unwrap();
        let back: DeployReceipt = serde_json::from_str(&s).unwrap();
        assert_eq!(receipt, back);
    }

    #[test]
    fn dapp_can_poll_by_commitment() {
        // The whole point: a dApp that submitted a deploy with
        // commitment X can find the receipt by matching on its
        // `commitment` field. Verify the field round-trips through
        // construction.
        let request_commitment = [0xAB; 32];
        let receipt = DeployReceipt::new(
            request_commitment,
            [0x22; 32],
            MAYFLY,
            [0x33; 32],
            1_200,
            100,
            500,
        )
        .unwrap();
        assert_eq!(receipt.commitment, request_commitment);
    }
}
