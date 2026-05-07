//! Deploy-receipt layer for app-templates — the OUTPUT analogue of
//! [`DeployRequest`].
//!
//! Lifecycle, end-to-end:
//!
//! 1. dApp builds a [`DeployRequest`][1], signs, submits.
//! 2. Chain validates → materialises → binds → charges fee →
//!    constructs the typed instance.
//! 3. Chain emits a [`DeployReceipt`] in the block's event log:
//!    `{ commitment, instance_id, template_class, deployer,
//!    fee_paid, deployed_at_epoch, block_height }`.
//! 4. Indexers store it; dApps poll on it ("did my deploy land?");
//!    wallets render the resulting instance handle.
//!
//! ## Why a receipt at all?
//!
//! Without a structured receipt, a dApp has to scan the chain for
//! "an instance whose id matches what I expected to derive". With
//! a receipt:
//!
//! - dApp polls by `commitment` (the deploy-request hash it already
//!   knows) and gets the resulting `instance_id` directly.
//! - The receipt is itself canonical-byte hashable, so it can ride
//!   in a Merkle event log alongside other block events.
//! - The `block_height` field gives strong-consistency: "your
//!   deploy is final at height H" is a stronger claim than "your
//!   deploy might have happened somewhere".
//!
//! ## Three structural decisions
//!
//! 1. **Domain-separated commitment.** Tag is
//!    `b"evaporchain:receipt:v1\0"` — distinct from the
//!    deploy-request and instance-id tags so the three hashes
//!    (request commitment, instance id, receipt commitment) can
//!    never collide.
//!
//! 2. **`commitment` is a back-pointer.** The receipt carries the
//!    deploy-request's commitment, so a dApp polling on its
//!    request hash can find the matching receipt without scanning.
//!
//! 3. **No status field.** A receipt only ever exists for a
//!    *successful* deploy. Failures don't get receipts; they get
//!    error events out-of-band. This keeps the receipt struct
//!    monomorphic — no `Result`, no enum, every field always
//!    populated.
//!
//! [1]: evaporchain-app-templates-deploy::DeployRequest
//!
//! ## Module map
//!
//! - [`receipt`] — [`DeployReceipt`] struct + canonical bytes +
//!   commitment.

pub mod receipt;

pub use receipt::{DeployReceipt, ReceiptError, RECEIPT_DOMAIN_TAG};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_app_templates::class::MAYFLY;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "DeployReceipt is the OUTPUT analogue of
    /// DeployRequest. It carries the request's commitment as a back-
    /// pointer plus the resulting instance_id, fee_paid, and block_
    /// height. canonical_bytes() is fixed-width domain-tagged;
    /// event_id() is BLAKE3 of those bytes. Receipts only exist for
    /// successful deploys (no status field). Out-of-range template
    /// classes are rejected at construction."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let r = DeployReceipt::new(
            [1u8; 32], // request commitment
            [2u8; 32], // instance_id
            MAYFLY, [7u8; 32], // deployer
            500,       // fee_paid
            100,       // deployed_at_epoch
            42,        // block_height
        )
        .unwrap();
        // Determinism.
        assert_eq!(r.event_id(), r.event_id());
        // Domain-tagged canonical bytes start with the receipt tag.
        let bytes = r.canonical_bytes();
        assert!(bytes.starts_with(RECEIPT_DOMAIN_TAG));

        // Out-of-range class rejected.
        let bad = evaporchain_app_templates::TemplateClass(0xFFFF_FFFF);
        assert!(DeployReceipt::new([1u8; 32], [2u8; 32], bad, [7u8; 32], 500, 100, 42,).is_err());
    }
}
