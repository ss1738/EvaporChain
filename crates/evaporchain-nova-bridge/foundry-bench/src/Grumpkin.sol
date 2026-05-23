// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title Minimal pure-EVM Grumpkin curve library — gas-anchor
/// benchmark for the EvaporChain CycleFold L1 verifier path.
///
/// @notice Grumpkin has NO EVM precompile (BN254 precompiles at
/// 0x06/07/08 don't apply). All field ops use the EVM `mulmod`
/// opcode against the BN254-Fr modulus (which IS Grumpkin's BASE
/// field). The curve equation is `y^2 = x^3 - 17` (a=0, b=-17 mod
/// p) per `crate::grumpkin_config`.
///
/// This library is INTENTIONALLY MINIMAL: just the per-op
/// primitives Foundry needs to measure (1) one point addition and
/// (2) one scalar-mul of a base point by a 256-bit scalar. The
/// MSM-at-n_aux gas extrapolation derives from these anchors;
/// implementing a full Pippenger MSM is a separate later step.
library Grumpkin {
    /// BN254-Fr modulus (Grumpkin BASE field).
    uint256 internal constant P = 21888242871839275222246405745257275088548364400416034343698204186575808495617;
    /// Curve coefficient `b = -17 mod p`. (`a = 0`).
    uint256 internal constant B = 21888242871839275222246405745257275088548364400416034343698204186575808495600;

    struct Point {
        uint256 x;
        uint256 y;
        bool inf;
    }

    /// Generator: `x = 1`, `y = 17631683881184975370165255887551781615748388533673675138860` (per crate).
    function generator() internal pure returns (Point memory) {
        return Point({
            x: 1,
            y: 17631683881184975370165255887551781615748388533673675138860,
            inf: false
        });
    }

    function addmod_p(uint256 a, uint256 b) internal pure returns (uint256) {
        return addmod(a, b, P);
    }

    function submod_p(uint256 a, uint256 b) internal pure returns (uint256) {
        return addmod(a, P - b, P);
    }

    function mulmod_p(uint256 a, uint256 b) internal pure returns (uint256) {
        return mulmod(a, b, P);
    }

    /// Modular inverse via Fermat: `a^(p-2) mod p`, calls the
    /// ModExp precompile at 0x05. ~7k gas typical.
    function inv_p(uint256 a) internal view returns (uint256 r) {
        uint256[6] memory input;
        input[0] = 32; // base length
        input[1] = 32; // exp length
        input[2] = 32; // mod length
        input[3] = a;
        input[4] = P - 2;
        input[5] = P;
        uint256[1] memory out;
        assembly {
            if iszero(
                staticcall(gas(), 0x05, input, 0xC0, out, 0x20)
            ) {
                revert(0, 0)
            }
        }
        r = out[0];
    }

    /// Affine Grumpkin point addition (with full case-handling).
    /// Distinct generic points: ~14-16 field mults + a few adds +
    /// ONE inversion (~7k gas). Doubling: similar. Identity / equal
    /// / negation handled inline. Returns `Point.inf=true` for the
    /// identity (no separate sentinel).
    function add(Point memory a, Point memory b) internal view returns (Point memory r) {
        if (a.inf) return b;
        if (b.inf) return a;
        if (a.x == b.x) {
            if (a.y == b.y) {
                // Doubling: lambda = (3 x^2) / (2 y).
                uint256 num = mulmod_p(mulmod_p(a.x, a.x), 3);
                uint256 den = inv_p(addmod_p(a.y, a.y));
                uint256 lam = mulmod_p(num, den);
                uint256 rx = submod_p(mulmod_p(lam, lam), addmod_p(a.x, a.x));
                uint256 ry = submod_p(mulmod_p(lam, submod_p(a.x, rx)), a.y);
                return Point({x: rx, y: ry, inf: false});
            } else {
                // a + (-a) = identity.
                return Point({x: 0, y: 0, inf: true});
            }
        }
        // Distinct x: lambda = (b.y - a.y) / (b.x - a.x).
        uint256 num2 = submod_p(b.y, a.y);
        uint256 den2 = inv_p(submod_p(b.x, a.x));
        uint256 lam2 = mulmod_p(num2, den2);
        uint256 rx2 = submod_p(submod_p(mulmod_p(lam2, lam2), a.x), b.x);
        uint256 ry2 = submod_p(mulmod_p(lam2, submod_p(a.x, rx2)), a.y);
        r = Point({x: rx2, y: ry2, inf: false});
    }

    /// Naive double-and-add scalar mul (256-bit). NOT optimised
    /// (no windowing, no NAF) — represents the pessimistic gas
    /// ceiling for an unoptimised in-circuit-style verifier. Real
    /// production verifiers use windowing for ~2-4× speedup.
    function scalarMul(Point memory p, uint256 s) internal view returns (Point memory r) {
        r = Point({x: 0, y: 0, inf: true});
        Point memory base = p;
        for (uint256 i = 0; i < 256; i++) {
            if (s & 1 == 1) {
                r = add(r, base);
            }
            base = add(base, base);
            s = s >> 1;
            if (s == 0) {
                // Optimisation: bail out when remaining bits are
                // zero. Matches the natural double-and-add cost
                // ceiling for typical (not-extremal) scalars.
                break;
            }
        }
    }
}
