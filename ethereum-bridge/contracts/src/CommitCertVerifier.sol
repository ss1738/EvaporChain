// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {BridgeConstants} from "./BridgeConstants.sol";
import {BridgeTypes} from "./BridgeTypes.sol";
import {BLS381} from "./lib/BLS381.sol";
import {HashToCurve} from "./lib/HashToCurve.sol";
import {ICommitCertVerifier} from "./interfaces/ICommitCertVerifier.sol";

/// @title  CommitCertVerifier
/// @notice The real Phase 2 verifier. Aggregates the supplied signed-set
///         pubkeys via G1MSM, hashes the commit message to G2 via
///         HashToCurve, and pairing-checks against the aggregate signature.
///
///         Verification equation:
///             e(-G1_gen, aggSignature) · e(aggPubkey, hashToG2(msg)) == 1
///
///         which is equivalent to the standard BLS check
///             e(G1_gen, aggSignature) == e(aggPubkey, hashToG2(msg)).
///
/// @dev    Per `evaporchain-crypto::bls_portable`, EvaporChain validators
///         sign with `BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_` as DST.
///         Same DST is hardcoded in `HashToCurve.DST`.
contract CommitCertVerifier is ICommitCertVerifier {
    error PubkeyXMismatch(uint256 index);
    error UncompressedHighBits();

    /// @dev EIP-2537 uncompressed encoding of -G1::generator() on
    ///      BLS12-381. 128 bytes: [pad16 || x_be_48 || pad16 || (p-y)_be_48].
    ///      Computed by `tests/g1_generator_constants.rs` (see the
    ///      sibling Rust crate). DO NOT EDIT BY HAND.
    bytes internal constant NEG_G1_GEN_UNCOMP =
        hex"0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0f"
        hex"c3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb"
        hex"00000000000000000000000000000000114d1d6855d545a8aa7d76c8cf2e21f2"
        hex"67816aef1db507c96655b9d5caac42364e6f38ba0ecb751bad54dcd6b939c2ca";

    /// @inheritdoc ICommitCertVerifier
    function verifyCommitCert(
        BridgeTypes.Validator[] calldata prevValidators,
        bytes calldata prevPubkeysUncompressed,
        bytes calldata signedBitmap,
        bytes32 messageHash,
        bytes calldata aggSignature
    ) external view returns (bool) {
        // 1. Aggregate the signed-set pubkeys via G1MSM (scalar=1 for each).
        //
        //    We build the MSM input as concatenated (G1_uncomp || scalar=1)
        //    pairs. As we walk, we also enforce that each supplied
        //    uncompressed pubkey's x-coordinate matches the stored
        //    compressed pubkey's x-coordinate. Y-coordinate consistency
        //    is enforced indirectly by the pairing check (a point with
        //    the same x but flipped y — i.e. -P — produces e(-P, h) =
        //    e(P, h)^-1, which fails the verification equation).
        bytes memory msmInput = _buildMsmInput(
            prevValidators, prevPubkeysUncompressed, signedBitmap
        );
        if (msmInput.length == 0) {
            // No signers in the bitmap, or bitmap completely invalid.
            return false;
        }
        bytes memory aggPubkey = BLS381.g1MsmNoRevert(msmInput);
        if (aggPubkey.length == 0) return false;

        // 2. Hash message to G2.
        bytes memory msgG2 = HashToCurve.hashToG2(abi.encodePacked(messageHash));
        if (aggSignature.length != BLS381.G2_LEN) return false;

        // 3. Pairing check (no-revert: malformed inputs from an adversarial
        //    relayer must NOT bubble up as a contract revert).
        bytes memory pairs = bytes.concat(
            NEG_G1_GEN_UNCOMP, aggSignature, aggPubkey, msgG2
        );
        return BLS381.pairingCheckNoRevert(pairs);
    }

    // ─── Internals ──────────────────────────────────────────────────

    function _buildMsmInput(
        BridgeTypes.Validator[] calldata prevValidators,
        bytes calldata prevPubkeysUncompressed,
        bytes calldata signedBitmap
    ) internal pure returns (bytes memory msmInput) {
        uint256 n = prevValidators.length;
        // Walk once to count signers.
        uint256 signers = 0;
        for (uint256 i = 0; i < n; i++) {
            if (_bitSet(signedBitmap, i)) signers++;
        }
        if (signers == 0) return msmInput;

        msmInput = new bytes(signers * BLS381.MSM_PAIR_LEN);
        uint256 cursor = 0;
        for (uint256 i = 0; i < n; i++) {
            if (!_bitSet(signedBitmap, i)) continue;

            // Read the i-th uncompressed pubkey (128 bytes).
            uint256 off = i * 128;
            // x-coordinate consistency check: prevValidators[i].pubkey is the
            // stored compressed (48-byte) form. Bytes 16..64 of the
            // uncompressed point are the same x in BE.
            _checkXMatches(prevPubkeysUncompressed, off, prevValidators[i].pubkey, i);

            // Copy 128 bytes of pubkey + 32 bytes of scalar=1 into msmInput.
            for (uint256 j = 0; j < 128; j++) {
                msmInput[cursor + j] = prevPubkeysUncompressed[off + j];
            }
            // scalar = 1 (32-byte big-endian, only last byte is 1).
            msmInput[cursor + 128 + 31] = 0x01;
            cursor += BLS381.MSM_PAIR_LEN;
        }
    }

    /// @dev Compare x-coordinates between uncompressed (128 B EIP-2537
    ///      layout) and compressed (48 B Zcash layout). The compressed
    ///      form has its top 3 bits used as flags; we mask them off
    ///      before comparing the 381-bit x value.
    function _checkXMatches(
        bytes calldata uncompressed,
        uint256 off,
        bytes memory compressed,
        uint256 index
    ) internal pure {
        // Uncompressed first 16 bytes must be zero (FP element pad).
        for (uint256 i = 0; i < 16; i++) {
            if (uncompressed[off + i] != 0) revert UncompressedHighBits();
        }
        // Compare byte 0 with the 3 flag bits masked.
        uint8 uncompB0 = uint8(uncompressed[off + 16]);
        uint8 compB0 = uint8(compressed[0]) & 0x1F; // clear flags (top 3 bits)
        if (uncompB0 != compB0) revert PubkeyXMismatch(index);
        // Compare bytes 1..47 of x.
        for (uint256 i = 1; i < 48; i++) {
            if (uncompressed[off + 16 + i] != compressed[i]) {
                revert PubkeyXMismatch(index);
            }
        }
    }

    function _bitSet(bytes calldata bitmap, uint256 i) internal pure returns (bool) {
        uint256 byteIdx = i / 8;
        uint256 bitIdx = i % 8;
        if (byteIdx >= bitmap.length) return false;
        return (uint8(bitmap[byteIdx]) >> bitIdx) & 1 == 1;
    }
}
