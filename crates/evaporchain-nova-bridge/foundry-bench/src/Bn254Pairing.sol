// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title Thin wrapper around the EVM BN254 pairing precompile
/// at address 0x08 (EIP-197). Used to verify the primary
/// HyperKZG opening in the ppsnark Solidity verifier.
///
/// EIP-197 contract: takes 192 bytes per pair (G1 = 64, G2 = 128)
/// and returns 32 bytes (`0x...01` if product of pairings == 1,
/// else `0x...00`). Standard KZG opening check uses 2 pairs:
///   e(A, [β]_2) · e(-B, [1]_2) ?= 1
/// (or similar — exact pair structure depends on the KZG
/// variant).
///
/// EIP-1108 (Istanbul) reformed pricing: 45_000 + 34_000 per pair.
/// For 2 pairs (standard KZG check): 45_000 + 2 × 34_000 = 113_000
/// gas nominal, plus calldata + ABI handling.
library Bn254Pairing {
    /// Verify the standard 2-pair check: e(a1, b1) · e(a2, b2) = 1.
    /// `input` is 384 bytes: a1 (64) + b1 (128) + a2 (64) + b2 (128).
    /// Returns true if the product is the identity.
    function ec_pairing_check_2pair(bytes memory input)
        internal
        view
        returns (bool ok)
    {
        require(input.length == 384, "expected 2-pair input (384 bytes)");
        uint256[1] memory out;
        assembly {
            // staticcall(gas, addr, in, insize, out, outsize)
            // `input` is a bytes pointer; add 0x20 to skip length.
            if iszero(
                staticcall(
                    gas(),
                    0x08,
                    add(input, 0x20),
                    384,
                    out,
                    0x20
                )
            ) {
                revert(0, 0)
            }
        }
        ok = out[0] == 1;
    }
}
