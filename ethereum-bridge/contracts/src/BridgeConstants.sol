// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

/// @title  EvaporChain ↔ Ethereum bridge constants
/// @notice Single source of truth for protocol-level identifiers shared
///         between the EvaporChain side (`crates/evaporchain-eth-bridge`)
///         and the Ethereum side (this Foundry project).
///
///         Phase 0 placeholder: only constants and bytecode-size sanity.
///         Phases 1+ will populate validator-set, commit-cert, and
///         Verkle membership logic.
library BridgeConstants {
    /// @dev Domain-separation tag for BLS commit-certificate hashing.
    ///      MUST match `evaporchain_consensus::bridge::DOMAIN_TAG_COMMIT`.
    bytes32 internal constant DOMAIN_TAG_COMMIT =
        keccak256("EvaporChain/v1/commit-cert");

    /// @dev Domain-separation tag for Verkle root commitments.
    bytes32 internal constant DOMAIN_TAG_VERKLE_ROOT =
        keccak256("EvaporChain/v1/verkle-root");

    /// @dev Domain-separation tag for ghost-record (evaporated object) commitments.
    bytes32 internal constant DOMAIN_TAG_GHOST_RECORD =
        keccak256("EvaporChain/v1/ghost-record");

    /// @dev Domain-separation tag for finalised block-header commitments.
    ///      Used by `EvaporHeaderInbox.sol` to bind a relayer-submitted
    ///      header tuple to the BLS-signed message validators attest to.
    bytes32 internal constant DOMAIN_TAG_HEADER =
        keccak256("EvaporChain/v1/header");

    /// @dev Domain-separation tag for state-membership attestations.
    ///      Validators sign `(DOMAIN_TAG_STATE_MEMBERSHIP, height, key, valueHash)`
    ///      to attest that `value` is committed at `key` under the state root
    ///      committed at `height`. This is the Phase 4 MVP path — a fallback
    ///      to a Halo2 → Groth16 wrap of the Verkle-IPA proof, shipping today.
    bytes32 internal constant DOMAIN_TAG_STATE_MEMBERSHIP =
        keccak256("EvaporChain/v1/state-membership");

    /// @dev EIP-2537 precompile addresses (Prague hard fork).
    address internal constant BLS12_G1ADD = address(0x0b);
    address internal constant BLS12_G1MSM = address(0x0c);
    address internal constant BLS12_G2ADD = address(0x0d);
    address internal constant BLS12_G2MSM = address(0x0e);
    address internal constant BLS12_PAIRING = address(0x0f);
    address internal constant BLS12_MAP_FP_TO_G1 = address(0x10);
    address internal constant BLS12_MAP_FP2_TO_G2 = address(0x11);

    /// @dev Maximum validator-set size we will ever accept on-chain.
    ///      Caps gas of a worst-case `verifyCommit` MSM. Sized for
    ///      EvaporChain's published validator-cap.
    uint256 internal constant MAX_VALIDATORS = 1024;

    /// @dev Stake threshold numerator/denominator for BFT 2/3+.
    uint256 internal constant STAKE_NUM = 2;
    uint256 internal constant STAKE_DEN = 3;
}
