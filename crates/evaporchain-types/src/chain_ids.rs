//! Typed chain-id constants — the single source of truth for the
//! canonical environment names.
//!
//! Before this module existed, the strings `"evaporchain-mainnet-1"`,
//! `"evaporchain-testnet-1"`, and `"evaporchain-devnet-1"` lived as
//! literals scattered across the genesis defaults, the node and CLI
//! `--chain-id` flag defaults, several `--mainnet` strict-mode pre-flight
//! checks, and an assortment of tests. The doctrine cost of that drift
//! was real: chain-id is bound into the validator's BLS signing message
//! (`Block::signing_message`), the VRF leader-selection input
//! (`leader_vrf_input(height, round, chain_id)`), the paymaster
//! sponsorship payload (`TransferTx::paymaster_sponsorship_payload`),
//! and the gossipsub topic namespace — a one-character typo in any of
//! those literals quietly creates a partition.
//!
//! Pin them here. Anywhere a chain-id literal would have been hard-coded,
//! prefer `evaporchain_types::chain_ids::MAINNET` (etc.) instead.
//!
//! ## Suffix convention
//!
//! The trailing `-1` is the chain-id **version**. A future hard fork that
//! breaks state compatibility increments the suffix
//! (`evaporchain-mainnet-2`) and the constant is added alongside, keeping
//! the previous one for historical-archive readers and indexers. The
//! constants do NOT bake in `pub const MAINNET_V2`; that's done at the
//! release commit when V2 is actually shipped.

/// Production mainnet chain-id.
///
/// This is the value baked into `genesis::ChainParams::default()`. The
/// `--mainnet` strict-mode pre-flight at `evaporchain-node/src/main.rs`
/// requires the genesis-config's chain_id to match this constant (modulo
/// future hard-fork suffix bumps).
pub const MAINNET: &str = "evaporchain-mainnet-1";

/// Public-facing testnet chain-id.
///
/// The current default for the node binary's `--chain-id` flag and the
/// genesis `ChainParams::testnet()` factory. Use for incentivised + un-
/// incentivised public testnets; do NOT use for local development nodes
/// (use [`DEVNET`] there so wallet auth tokens / faucet credits don't
/// accidentally cross-pollinate).
pub const TESTNET: &str = "evaporchain-testnet-1";

/// Local-development chain-id.
///
/// Use this for any node intended only for the developer's own machine
/// or a private CI run. Distinct from [`TESTNET`] so that a wallet auth
/// token issued against a local devnet won't accidentally be replayed
/// against a public testnet of the same operator and vice-versa.
pub const DEVNET: &str = "evaporchain-devnet-1";

/// Returns true iff `id` is one of the canonical chain-ids defined here.
///
/// Useful for early-startup pre-flight checks ("don't let the binary boot
/// with a chain-id we don't recognise") without forcing the rest of the
/// system to hard-code a closed set — chain-id remains a `String` in
/// `ChainParams` so private testnets are still expressible.
pub fn is_canonical(id: &str) -> bool {
    matches!(id, MAINNET | TESTNET | DEVNET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ids_are_distinct() {
        // A typo in any of these would silently partition validators.
        assert_ne!(MAINNET, TESTNET);
        assert_ne!(MAINNET, DEVNET);
        assert_ne!(TESTNET, DEVNET);
    }

    #[test]
    fn canonical_ids_have_v1_suffix() {
        // Future hard-fork bumps increment the suffix; pin the current
        // version so the bump is a visible diff, not a silent drift.
        assert!(MAINNET.ends_with("-1"));
        assert!(TESTNET.ends_with("-1"));
        assert!(DEVNET.ends_with("-1"));
    }

    #[test]
    fn canonical_ids_share_evaporchain_prefix() {
        // The `evaporchain-` prefix anchors the namespace; a chain-id
        // without it shouldn't pass `is_canonical`.
        for id in &[MAINNET, TESTNET, DEVNET] {
            assert!(
                id.starts_with("evaporchain-"),
                "chain-id {id} missing canonical prefix"
            );
        }
    }

    #[test]
    fn is_canonical_recognises_the_three_and_only_the_three() {
        assert!(is_canonical(MAINNET));
        assert!(is_canonical(TESTNET));
        assert!(is_canonical(DEVNET));
        assert!(!is_canonical("evaporchain-mainnet-2"));
        assert!(!is_canonical("mainnet-1"));
        assert!(!is_canonical(""));
        assert!(!is_canonical("some-private-testnet"));
    }
}
