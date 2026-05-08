// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

/// @title  BridgeTypes
/// @notice Shared structs used across the bridge contracts. Lives in a
///         leaf file so verifier, registry, and dispatcher can all
///         depend on the same type without circular imports.
library BridgeTypes {
    /// @notice One validator entry. `pubkey` is the BLS12-381 G1
    ///         compressed encoding (48 bytes). The verifier accepts an
    ///         off-chain-supplied UNCOMPRESSED pubkey (128 bytes,
    ///         EIP-2537 layout) for the BLS aggregation step, and
    ///         confirms it consistent with this 48-byte stored value.
    struct Validator {
        bytes pubkey; // 48 bytes
        uint128 stake;
    }
}
