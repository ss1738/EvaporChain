// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title Minimal pure-EVM Bn254Fq field library for the ppsnark
/// Solidity verifier. Bn254Fq is GrumpkinEngine::Scalar — the
/// scalar field over which the secondary Spartan ppsnark verifies.
/// All sumcheck rounds + IPA opening checks evaluate polynomials
/// in this field.
///
/// Bn254Fq modulus
/// (`21888242871839275222246405745257275088696311157297823662689037894645226208583`)
/// is slightly larger than Bn254Fr. mulmod / addmod opcodes apply.
library Bn254Fq {
    uint256 internal constant Q = 21888242871839275222246405745257275088696311157297823662689037894645226208583;

    function add_q(uint256 a, uint256 b) internal pure returns (uint256) {
        return addmod(a, b, Q);
    }

    function sub_q(uint256 a, uint256 b) internal pure returns (uint256) {
        return addmod(a, Q - b, Q);
    }

    function mul_q(uint256 a, uint256 b) internal pure returns (uint256) {
        return mulmod(a, b, Q);
    }

    /// Fermat-inverse via ModExp precompile at 0x05. EIP-2565
    /// pricing: ~700 gas for 256-bit ModExp.
    function inv_q(uint256 a) internal view returns (uint256 r) {
        uint256[6] memory input;
        input[0] = 32;
        input[1] = 32;
        input[2] = 32;
        input[3] = a;
        input[4] = Q - 2;
        input[5] = Q;
        uint256[1] memory out;
        assembly {
            if iszero(staticcall(gas(), 0x05, input, 0xC0, out, 0x20)) {
                revert(0, 0)
            }
        }
        r = out[0];
    }

    /// Evaluate a degree-3 univariate polynomial
    /// P(x) = c0 + c1·x + c2·x² + c3·x³ at point `x` via Horner.
    /// Used inside sumcheck rounds (outer sumcheck uses deg-3
    /// polys; inner/batch use deg-2 — separate eval fn or just
    /// pass c3=0).
    function eval_deg3(
        uint256 c0,
        uint256 c1,
        uint256 c2,
        uint256 c3,
        uint256 x
    ) internal pure returns (uint256) {
        // Horner: ((c3·x + c2)·x + c1)·x + c0
        uint256 acc = mulmod(c3, x, Q);
        acc = addmod(acc, c2, Q);
        acc = mulmod(acc, x, Q);
        acc = addmod(acc, c1, Q);
        acc = mulmod(acc, x, Q);
        acc = addmod(acc, c0, Q);
        return acc;
    }
}
