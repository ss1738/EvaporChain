// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {BridgeConstants} from "../BridgeConstants.sol";

/// @title  BLS381
/// @notice Thin wrapper around the EIP-2537 BLS12-381 precompiles
///         (Prague hard fork, mainnet since May 2025).
///
/// @dev    All point inputs/outputs use the EIP-2537 *uncompressed* encoding:
///           - G1 point  : 128 bytes (2 × 64-byte field elements: x, y)
///           - G2 point  : 256 bytes (2 × 128-byte FP2 elements: x, y)
///           - FP element: 64 bytes (16-byte zero pad || 48-byte big-endian value)
///           - Scalar    : 32 bytes (big-endian)
///
///         Compressed encoding (48-byte G1, 96-byte G2) is what
///         EvaporChain transmits over the wire and stores in its valset
///         commitment, but the Solidity verifier only accepts uncompressed.
///         Decompression happens off-chain in the relayer; on-chain we
///         only check that the supplied uncompressed point is consistent
///         with its stored compressed counterpart (see
///         `CommitCertVerifier.sol`).
library BLS381 {
    error InvalidG1Length(uint256 got);
    error InvalidG2Length(uint256 got);
    error InvalidScalarLength(uint256 got);
    error PrecompileFailed();

    uint256 internal constant G1_LEN = 128;
    uint256 internal constant G2_LEN = 256;
    uint256 internal constant FP_LEN = 64;
    uint256 internal constant SCALAR_LEN = 32;
    uint256 internal constant MSM_PAIR_LEN = G1_LEN + SCALAR_LEN; // 160
    uint256 internal constant PAIRING_PAIR_LEN = G1_LEN + G2_LEN; // 384

    // ─── G1 ─────────────────────────────────────────────────────────

    /// @notice G1 point addition: a + b.
    /// @dev    EIP-2537 G1ADD precompile (0x0b). Cost: 375 gas.
    function g1Add(bytes memory a, bytes memory b) internal view returns (bytes memory out) {
        if (a.length != G1_LEN) revert InvalidG1Length(a.length);
        if (b.length != G1_LEN) revert InvalidG1Length(b.length);
        bytes memory input = abi.encodePacked(a, b);
        out = _staticcall(BridgeConstants.BLS12_G1ADD, input, G1_LEN);
    }

    /// @notice G1 multi-scalar-multiplication: Σ s_i × P_i for i = 0..N.
    /// @dev    EIP-2537 G1MSM precompile (0x0c). Discounted curve;
    ///         cost ≈ 12_000 + 22_500 × N × discount(N).
    /// @param  pairs concatenation of (G1 point ‖ scalar) tuples.
    function g1Msm(bytes memory pairs) internal view returns (bytes memory out) {
        if (pairs.length == 0 || pairs.length % MSM_PAIR_LEN != 0) {
            revert InvalidG1Length(pairs.length);
        }
        out = _staticcall(BridgeConstants.BLS12_G1MSM, pairs, G1_LEN);
    }

    // ─── G2 ─────────────────────────────────────────────────────────

    /// @notice G2 point addition: a + b. Precompile 0x0d.
    function g2Add(bytes memory a, bytes memory b) internal view returns (bytes memory out) {
        if (a.length != G2_LEN) revert InvalidG2Length(a.length);
        if (b.length != G2_LEN) revert InvalidG2Length(b.length);
        bytes memory input = abi.encodePacked(a, b);
        out = _staticcall(BridgeConstants.BLS12_G2ADD, input, G2_LEN);
    }

    // ─── Pairing ────────────────────────────────────────────────────

    /// @notice Multi-pairing check: returns true iff Π e(G1_i, G2_i) == 1.
    /// @dev    EIP-2537 PAIRING precompile (0x0f). Cost ≈ 37_700 + 32_600 × N.
    /// @param  pairs concatenation of (G1 ‖ G2) pairs.
    function pairingCheck(bytes memory pairs) internal view returns (bool) {
        if (pairs.length == 0 || pairs.length % PAIRING_PAIR_LEN != 0) {
            revert InvalidG1Length(pairs.length);
        }
        bytes memory ret = _staticcall(BridgeConstants.BLS12_PAIRING, pairs, 32);
        // Output is a 32-byte big-endian word: 0x...01 on success, 0x...00 on failure.
        uint256 v;
        assembly { v := mload(add(ret, 32)) }
        return v == 1;
    }

    /// @notice Like `pairingCheck`, but returns `false` on any precompile
    ///         revert (off-curve point, wrong subgroup, etc.) instead of
    ///         bubbling. Use in verifiers that take adversarially supplied
    ///         points and treat malformed input as "verification failed".
    function pairingCheckNoRevert(bytes memory pairs) internal view returns (bool) {
        if (pairs.length == 0 || pairs.length % PAIRING_PAIR_LEN != 0) {
            return false;
        }
        bytes memory ret = new bytes(32);
        bool ok;
        assembly {
            ok :=
                staticcall(
                    gas(),
                    0x0f,
                    add(pairs, 32),
                    mload(pairs),
                    add(ret, 32),
                    32
                )
        }
        if (!ok) return false;
        uint256 v;
        assembly {
            v := mload(add(ret, 32))
        }
        return v == 1;
    }

    /// @notice Like `g1Msm`, but returns an empty `bytes` if the
    ///         precompile reverts. Caller treats empty as "invalid input".
    function g1MsmNoRevert(bytes memory pairs) internal view returns (bytes memory out) {
        if (pairs.length == 0 || pairs.length % MSM_PAIR_LEN != 0) {
            return out;
        }
        bytes memory buf = new bytes(G1_LEN);
        bool ok;
        assembly {
            ok :=
                staticcall(
                    gas(),
                    0x0c,
                    add(pairs, 32),
                    mload(pairs),
                    add(buf, 32),
                    G1_LEN
                )
        }
        if (!ok) return out;
        return buf;
    }

    // ─── Map-to-curve ───────────────────────────────────────────────

    /// @notice Map an FP element to G1 (RFC 9380 SSWU). Precompile 0x10.
    function mapFpToG1(bytes memory fp) internal view returns (bytes memory out) {
        if (fp.length != FP_LEN) revert InvalidG1Length(fp.length);
        out = _staticcall(BridgeConstants.BLS12_MAP_FP_TO_G1, fp, G1_LEN);
    }

    /// @notice Map an FP2 element to G2. Precompile 0x11.
    function mapFp2ToG2(bytes memory fp2) internal view returns (bytes memory out) {
        if (fp2.length != 2 * FP_LEN) revert InvalidG2Length(fp2.length);
        out = _staticcall(BridgeConstants.BLS12_MAP_FP2_TO_G2, fp2, G2_LEN);
    }

    // ─── Internal ───────────────────────────────────────────────────

    function _staticcall(address precompile, bytes memory input, uint256 outLen)
        private
        view
        returns (bytes memory out)
    {
        out = new bytes(outLen);
        bool ok;
        assembly {
            ok :=
                staticcall(
                    gas(),
                    precompile,
                    add(input, 32),
                    mload(input),
                    add(out, 32),
                    outLen
                )
        }
        if (!ok) revert PrecompileFailed();
    }
}
