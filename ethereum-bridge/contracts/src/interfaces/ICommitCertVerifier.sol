// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {BridgeTypes} from "../BridgeTypes.sol";

/// @title  ICommitCertVerifier
/// @notice Verifies a BLS aggregate commit-certificate over a valset
///         transition. Phase 2 lands the real EIP-2537 implementation;
///         Phase 1 ships only `MockCommitCertVerifier` (in test/lib/) so
///         the registry can be exercised independently.
interface ICommitCertVerifier {
    /// @notice Verify the previous valset's signed approval of the next root.
    ///
    /// @dev    The caller (ValidatorSetRegistry) is expected to have ALREADY
    ///         confirmed `_computeRoot(prevEpoch, prevValidators) == prevValsetRoot`
    ///         and that `signedBitmap` corresponds to ≥ 2/3 stake of
    ///         `prevTotalStake`. The verifier itself only checks the BLS
    ///         math (aggregation + pairing) for those validators marked
    ///         in `signedBitmap`.
    ///
    /// @param prevValidators        full prev set (witness; ordering matches signedBitmap MSB→LSB).
    /// @param prevPubkeysUncompressed 128-byte uncompressed G1 point per validator,
    ///                              concatenated in the same order as prevValidators.
    /// @param signedBitmap          big-endian byte-packed bitmap; bit `i` = validator `i` signed.
    /// @param messageHash           keccak256 commitment to (DOMAIN_TAG_COMMIT,
    ///                              prevEpoch, nextValsetRoot, nextEpoch).
    /// @param aggSignature          BLS-G2 aggregate signature, EIP-2537 uncompressed (256 bytes).
    /// @return ok                   pairing check passed.
    function verifyCommitCert(
        BridgeTypes.Validator[] calldata prevValidators,
        bytes calldata prevPubkeysUncompressed,
        bytes calldata signedBitmap,
        bytes32 messageHash,
        bytes calldata aggSignature
    ) external view returns (bool ok);
}
