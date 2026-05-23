// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Grumpkin} from "./Grumpkin.sol";

/// @title Jacobian-projective Grumpkin arithmetic.
/// @notice Affine Grumpkin `add` costs ~3,834 gas — dominated by a
/// single ModExp inversion (~7k nominal, ~3k observed) per op.
/// Jacobian coordinates `(X, Y, Z)` representing affine `(X/Z², Y/Z³)`
/// eliminate the inversion: all internal ops are mulmod-only, with a
/// single inversion at the very END to project back to affine.
///
/// For curve `y² = x³ - 17` (a=0):
///
/// ### `addJ` (P1 ≠ P2, both non-identity):
///   U1 = X1·Z2², U2 = X2·Z1², S1 = Y1·Z2³, S2 = Y2·Z1³
///   H = U2 - U1, R = S2 - S1
///   H2 = H², H3 = H·H2, U1H2 = U1·H2
///   X3 = R² - H3 - 2·U1H2
///   Y3 = R·(U1H2 - X3) - S1·H3
///   Z3 = H·Z1·Z2
/// 11 mulmods + 4 addmods. No inversion.
///
/// ### `doubleJ` (a = 0):
///   A = X², B = Y², C = B²
///   D = 2·((X+B)² - A - C)
///   E = 3·A, F = E²
///   X' = F - 2·D
///   Y' = E·(D - X') - 8·C
///   Z' = 2·Y·Z
/// 8 mulmods + ~6 addmods. No inversion.
///
/// Per-mulmod EVM cost ≈ 8 gas; Jacobian add target ≈ 88 + overhead
/// ≈ 200-400 gas measured (vs affine 3,834). Final affine projection:
/// ONE ModExp inv + 4 mulmods ≈ 3,200 gas amortised over the whole
/// MSM.
library GrumpkinJacobian {
    uint256 internal constant P = 21888242871839275222246405745257275088548364400416034343698204186575808495617;

    struct PointJ {
        uint256 X;
        uint256 Y;
        uint256 Z; // Z == 0 ⇒ identity (point at infinity)
    }

    function identity() internal pure returns (PointJ memory) {
        return PointJ({X: 0, Y: 1, Z: 0});
    }

    function fromAffine(Grumpkin.Point memory p)
        internal
        pure
        returns (PointJ memory)
    {
        if (p.inf) return identity();
        return PointJ({X: p.x, Y: p.y, Z: 1});
    }

    function addmodP(uint256 a, uint256 b) internal pure returns (uint256) {
        return addmod(a, b, P);
    }
    function submodP(uint256 a, uint256 b) internal pure returns (uint256) {
        return addmod(a, P - b, P);
    }
    function mulmodP(uint256 a, uint256 b) internal pure returns (uint256) {
        return mulmod(a, b, P);
    }

    /// Jacobian point-doubling (curve a = 0).
    function doubleJ(PointJ memory p) internal pure returns (PointJ memory r) {
        if (p.Z == 0) return identity();
        if (p.Y == 0) return identity();

        uint256 A = mulmodP(p.X, p.X);              // X²
        uint256 B = mulmodP(p.Y, p.Y);              // Y²
        uint256 C = mulmodP(B, B);                  // B² = Y⁴
        // D = 2·((X+B)² - A - C)
        uint256 t = addmodP(p.X, B);
        t = mulmodP(t, t);
        t = submodP(t, A);
        t = submodP(t, C);
        uint256 D = addmodP(t, t);
        uint256 E = mulmodP(A, 3);                  // 3·X²
        uint256 F = mulmodP(E, E);                  // E²
        uint256 X3 = submodP(F, addmodP(D, D));     // F - 2D
        // Y3 = E·(D - X3) - 8·C
        uint256 c8 = mulmodP(C, 8);
        uint256 Y3 = submodP(mulmodP(E, submodP(D, X3)), c8);
        uint256 Z3 = mulmodP(mulmodP(p.Y, p.Z), 2); // 2·Y·Z
        r = PointJ({X: X3, Y: Y3, Z: Z3});
    }

    /// Jacobian point-addition. Handles identity / equal / negation.
    function addJ(PointJ memory p1, PointJ memory p2)
        internal
        pure
        returns (PointJ memory r)
    {
        if (p1.Z == 0) return p2;
        if (p2.Z == 0) return p1;

        // U1, U2, S1, S2 via Z-powers; written into r to reduce
        // stack pressure (Solidity stack-too-deep otherwise).
        uint256 U1;
        uint256 U2;
        uint256 S1;
        uint256 S2;
        {
            uint256 Z1Z1 = mulmodP(p1.Z, p1.Z);
            uint256 Z2Z2 = mulmodP(p2.Z, p2.Z);
            U1 = mulmodP(p1.X, Z2Z2);
            U2 = mulmodP(p2.X, Z1Z1);
            S1 = mulmodP(p1.Y, mulmodP(Z2Z2, p2.Z));
            S2 = mulmodP(p2.Y, mulmodP(Z1Z1, p1.Z));
        }

        if (U1 == U2) {
            if (S1 == S2) return doubleJ(p1);
            return identity();
        }

        uint256 H = submodP(U2, U1);
        uint256 R = submodP(S2, S1);
        uint256 X3;
        uint256 Y3;
        {
            uint256 H2 = mulmodP(H, H);
            uint256 H3 = mulmodP(H, H2);
            uint256 U1H2 = mulmodP(U1, H2);
            X3 = submodP(submodP(mulmodP(R, R), H3), addmodP(U1H2, U1H2));
            Y3 = submodP(mulmodP(R, submodP(U1H2, X3)), mulmodP(S1, H3));
        }
        uint256 Z3 = mulmodP(H, mulmodP(p1.Z, p2.Z));
        r = PointJ({X: X3, Y: Y3, Z: Z3});
    }

    /// Project Jacobian → affine. ONE inversion (~3k gas observed
    /// for ModExp on EIP-2565 path) + 4 mulmods.
    function toAffine(PointJ memory p)
        internal
        view
        returns (Grumpkin.Point memory r)
    {
        if (p.Z == 0) {
            return Grumpkin.Point({x: 0, y: 0, inf: true});
        }
        uint256 Zinv = Grumpkin.inv_p(p.Z);
        uint256 Zinv2 = mulmodP(Zinv, Zinv);
        uint256 Zinv3 = mulmodP(Zinv2, Zinv);
        uint256 x = mulmodP(p.X, Zinv2);
        uint256 y = mulmodP(p.Y, Zinv3);
        r = Grumpkin.Point({x: x, y: y, inf: false});
    }
}
