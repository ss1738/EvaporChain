// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title Thin wrapper around EIP-196/197 BN254 G1 precompiles.
/// @notice (c)-2c gas anchor — tests if the (R4) BN254-precompile
/// re-routing path actually delivers the analytical ~22× speedup
/// vs pure-EVM Grumpkin (Jacobian floor 1.77B at n=16,384).
///
/// Precompiles (post-EIP-1108):
///   - 0x06 ECADD: 150 gas, 128-byte input (x1,y1,x2,y2), 64-byte out
///   - 0x07 ECMUL: 6,000 gas, 96-byte input (x,y,scalar), 64-byte out
///   - 0x08 PAIRING: 45,000 + 34,000·k gas, 192·k input
///
/// CAVEAT (architectural, NOT performance): BN254 is the OTHER
/// curve in the cycle. The current Nova architecture has the
/// SECONDARY on Grumpkin (so Grumpkin's base field = BN254-Fr makes
/// in-circuit IPA cheap). Switching the secondary to BN254 breaks
/// that cycle. This anchor measures the gas-floor IF the architecture
/// were redesigned to put the on-chain MSM on BN254 — it is NOT a
/// drop-in replacement for the existing Grumpkin secondary.
library BN254G1 {
    /// BN254 G1 generator: (1, 2). Curve y² = x³ + 3 over Fp where
    /// p = 21888242871839275222246405745257275088696311157297823662689037894645226208583.
    function generator() internal pure returns (uint256 x, uint256 y) {
        return (1, 2);
    }

    /// BN254 ECADD via precompile 0x06.
    function ecAdd(
        uint256 x1, uint256 y1,
        uint256 x2, uint256 y2
    ) internal view returns (uint256 x3, uint256 y3) {
        uint256[4] memory input;
        input[0] = x1; input[1] = y1; input[2] = x2; input[3] = y2;
        uint256[2] memory out;
        assembly {
            if iszero(staticcall(gas(), 0x06, input, 0x80, out, 0x40)) {
                revert(0, 0)
            }
        }
        x3 = out[0]; y3 = out[1];
    }

    /// BN254 ECMUL via precompile 0x07.
    function ecMul(uint256 x, uint256 y, uint256 s)
        internal
        view
        returns (uint256 rx, uint256 ry)
    {
        uint256[3] memory input;
        input[0] = x; input[1] = y; input[2] = s;
        uint256[2] memory out;
        assembly {
            if iszero(staticcall(gas(), 0x07, input, 0x60, out, 0x40)) {
                revert(0, 0)
            }
        }
        rx = out[0]; ry = out[1];
    }
}
