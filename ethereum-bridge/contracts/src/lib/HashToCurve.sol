// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {BLS381} from "./BLS381.sol";

/// @title  HashToCurve
/// @notice RFC 9380 hash-to-curve for BLS12-381 G2.
///
///         Implements `expand_message_xmd_sha256` (§5.4.1) followed by
///         two calls to the EIP-2537 MAP_FP2_TO_G2 precompile and a
///         single G2ADD — the standard SSWU-then-clear-cofactor
///         construction for BLS12-381 G2.
///
/// @dev    Domain Separation Tag (DST):
///           "BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_"
///         must match the DST used by EvaporChain validators when signing
///         state-root attestations, commit certificates, and DA
///         attestations. This is the **non-PoP** ciphersuite; proof-of-
///         possession uses a different DST (`BLS_POP_DST` in the Rust
///         side at `evaporchain-crypto::signatures::BLS_POP_DST`,
///         `..._POP_` suffix) — that one is verified off-chain at
///         validator-onboarding time and is NOT what the bridge
///         consumes here.
///         (See `evaporchain-crypto::signatures::BLS_DST`.)
///
///         L6 (audit 2026-05-13): pre-fix this docblock said
///         `..._POP_EVAPORCHAIN_V1` while the constant below was
///         `..._NUL_`. The constant is authoritative — verified
///         against the Rust producer at `signatures.rs:403`. Comment
///         corrected to match.
library HashToCurve {
    /// @dev Domain Separation Tag. Must match the Rust producer in
    ///      `evaporchain-crypto::signatures::BLS_DST`.
    ///      RFC 9380 ciphersuite ID: BLS12381G2_XMD:SHA-256_SSWU_RO_,
    ///      with `_NUL_` application tag for non-PoP signatures.
    bytes internal constant DST =
        bytes("BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_");

    error ExpandMessageBadLen(uint256 got);

    /// @notice RFC 9380 §5.4.1 expand_message_xmd with SHA-256.
    ///
    ///         Returns `len_in_bytes` pseudorandom bytes derived from
    ///         (msg, DST). For our use, len_in_bytes = 256
    ///         (4 × 64 bytes of FP material → 2 FP2 elements).
    function expandMessageXmd(bytes memory msg_, uint256 lenInBytes)
        internal
        view
        returns (bytes memory uniformBytes)
    {
        // Per RFC 9380, with SHA-256 (b_in_bytes = 32, s_in_bytes = 64):
        //   ell = ceil(len_in_bytes / 32)
        //   DST_prime = DST || I2OSP(len(DST), 1)
        //   Z_pad = I2OSP(0, 64)            (s_in_bytes zero bytes)
        //   l_i_b_str = I2OSP(len_in_bytes, 2)
        //   msg_prime = Z_pad || msg || l_i_b_str || I2OSP(0, 1) || DST_prime
        //   b_0 = SHA256(msg_prime)
        //   b_1 = SHA256(b_0 || I2OSP(1, 1) || DST_prime)
        //   b_i = SHA256(strxor(b_0, b_{i-1}) || I2OSP(i, 1) || DST_prime), i = 2..ell
        //   uniform_bytes = b_1 || b_2 || ... || b_ell, truncated to len_in_bytes.

        require(lenInBytes <= 65535, "len too big");
        if (DST.length > 255) revert ExpandMessageBadLen(DST.length);

        uint256 ell = (lenInBytes + 31) / 32;
        require(ell < 256, "ell too big");

        bytes memory dstPrime = abi.encodePacked(DST, uint8(DST.length));
        bytes memory zPad = new bytes(64); // SHA-256 block size
        bytes memory libStr = abi.encodePacked(uint16(lenInBytes));
        bytes memory msgPrime = abi.encodePacked(zPad, msg_, libStr, uint8(0), dstPrime);

        bytes32 b0 = sha256(msgPrime);
        bytes32 bi = sha256(abi.encodePacked(b0, uint8(1), dstPrime));

        uniformBytes = new bytes(lenInBytes);
        uint256 cursor = 0;
        // copy b_1
        cursor = _copy(uniformBytes, cursor, abi.encodePacked(bi), 32, lenInBytes);
        for (uint256 i = 2; i <= ell; i++) {
            bytes32 strxor;
            bytes32 prev = bi;
            assembly {
                strxor := xor(b0, prev)
            }
            bi = sha256(abi.encodePacked(strxor, uint8(i), dstPrime));
            cursor = _copy(uniformBytes, cursor, abi.encodePacked(bi), 32, lenInBytes);
        }
    }

    /// @notice Hash a message to G2 (uncompressed, 256 bytes).
    /// @dev    Returns the standard RFC 9380 hash-to-curve output:
    ///         `clear_cofactor( map(u0) + map(u1) )`. The map and
    ///         clear_cofactor steps are folded into MAP_FP2_TO_G2 by
    ///         the EIP-2537 precompile.
    function hashToG2(bytes memory msg_) internal view returns (bytes memory g2Point) {
        bytes memory uniform = expandMessageXmd(msg_, 256); // 4 × 64-byte FP elements
        bytes memory u0 = _bytesToFp2(uniform, 0);
        bytes memory u1 = _bytesToFp2(uniform, 128);

        bytes memory q0 = BLS381.mapFp2ToG2(u0);
        bytes memory q1 = BLS381.mapFp2ToG2(u1);
        g2Point = BLS381.g2Add(q0, q1);
    }

    // ─── Internals ──────────────────────────────────────────────────

    /// @dev Copy up to `n` bytes from `src` to `dst[cursor..]`,
    ///      capped by `cap`. Returns new cursor.
    function _copy(bytes memory dst, uint256 cursor, bytes memory src, uint256 n, uint256 cap)
        private
        pure
        returns (uint256)
    {
        uint256 take = n;
        if (cursor + take > cap) take = cap - cursor;
        for (uint256 i = 0; i < take; i++) {
            dst[cursor + i] = src[i];
        }
        return cursor + take;
    }

    /// @dev Read 128 bytes at `uniform[off..off+128]` as one FP2 element
    ///      (two FP elements c0, c1), reduced modulo p, in EIP-2537
    ///      encoding (each FP padded to 64 bytes).
    ///
    ///      RFC 9380 says u_i = OS2IP(uniform[i*64..(i+1)*64]) mod p
    ///      for each FP component. We emit the full 128-byte EIP-2537
    ///      FP2 layout: [pad16 || c0_48 || pad16 || c1_48].
    function _bytesToFp2(bytes memory uniform, uint256 off)
        private
        view
        returns (bytes memory fp2)
    {
        fp2 = new bytes(128);
        // c0 = OS2IP(uniform[off .. off+64]) mod p, written as 64-byte BE.
        bytes memory c0 = _reduceFp64(uniform, off);
        // c1 = OS2IP(uniform[off+64 .. off+128]) mod p, written as 64-byte BE.
        bytes memory c1 = _reduceFp64(uniform, off + 64);
        for (uint256 i = 0; i < 64; i++) fp2[i] = c0[i];
        for (uint256 i = 0; i < 64; i++) fp2[64 + i] = c1[i];
    }

    /// @dev Reduce a 64-byte big-endian integer mod the BLS12-381 base
    ///      field prime p, returning a 64-byte EIP-2537-encoded FP
    ///      element (16 bytes zero pad, 48 bytes value).
    ///
    ///      We do this with `expmod` (precompile 0x05): x mod p == x^1 mod p.
    function _reduceFp64(bytes memory src, uint256 off) private view returns (bytes memory fp) {
        // Build the 64-byte input `b`.
        bytes memory b = new bytes(64);
        for (uint256 i = 0; i < 64; i++) b[i] = src[off + i];

        // Build p as 64-byte BE: 16 zero bytes prefix + 48 bytes of p.
        // p (48 bytes) = (P_HI shifted) || P_LO. We'll pack from the constants.
        bytes memory pBytes = abi.encodePacked(
            uint128(0), // 16-byte pad
            uint8(0x1A),
            uint8(0x01),
            uint8(0x11),
            uint8(0xEA),
            uint8(0x39),
            uint8(0x7F),
            uint8(0xE6),
            uint8(0x9A),
            uint8(0x4B),
            uint8(0x1B),
            uint8(0xA7),
            uint8(0xB6),
            uint8(0x43),
            uint8(0x4B),
            uint8(0xAC),
            uint8(0xD7),
            uint8(0x64),
            uint8(0x77),
            uint8(0x4B),
            uint8(0x84),
            uint8(0xF3),
            uint8(0x85),
            uint8(0x12),
            uint8(0xBF),
            uint8(0x67),
            uint8(0x30),
            uint8(0xD2),
            uint8(0xA0),
            uint8(0xF6),
            uint8(0xB0),
            uint8(0xF6),
            uint8(0x24),
            uint8(0x1E),
            uint8(0xAB),
            uint8(0xFF),
            uint8(0xFE),
            uint8(0xB1),
            uint8(0x53),
            uint8(0xFF),
            uint8(0xFF),
            uint8(0xB9),
            uint8(0xFE),
            uint8(0xFF),
            uint8(0xFF),
            uint8(0xFF),
            uint8(0xFF),
            uint8(0xAA),
            uint8(0xAB)
        );

        // expmod input: <length_of_BASE><length_of_EXP><length_of_MOD><BASE><EXP><MOD>
        // BASE = 64 bytes (b), EXP = 1 byte (0x01), MOD = 64 bytes (pBytes).
        bytes memory input = abi.encodePacked(
            uint256(64), uint256(1), uint256(64), b, uint8(1), pBytes
        );
        fp = new bytes(64);
        bool ok;
        assembly {
            ok :=
                staticcall(
                    gas(),
                    0x05, // expmod precompile
                    add(input, 32),
                    mload(input),
                    add(fp, 32),
                    64
                )
        }
        require(ok, "expmod failed");
    }
}
