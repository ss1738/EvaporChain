// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Grumpkin} from "../src/Grumpkin.sol";
import {GrumpkinJacobian} from "../src/GrumpkinJacobian.sol";

/// @notice 1C (c)-2b: Jacobian-projective Grumpkin gas anchor.
/// Box-validates the affine→Jacobian ~40× per-add speedup claim
/// from (c)-2a, the architectural prerequisite for L2 single-tx
/// fit (~2B → ~50M projected gas at n=16,384).
contract BenchJacobian is Test {
    /// Correctness: Jacobian add of G + G must equal affine 2G.
    function testCorrectness_JacobianDouble_vs_Affine() public view {
        Grumpkin.Point memory g = Grumpkin.generator();
        // Affine 2G via the affine library's doubling branch.
        Grumpkin.Point memory twoG_aff = Grumpkin.add(g, g);

        GrumpkinJacobian.PointJ memory gJ = GrumpkinJacobian.fromAffine(g);
        GrumpkinJacobian.PointJ memory twoGJ =
            GrumpkinJacobian.addJ(gJ, gJ);
        Grumpkin.Point memory twoG_proj = GrumpkinJacobian.toAffine(twoGJ);

        assertEq(twoG_proj.inf, twoG_aff.inf, "2G inf mismatch");
        assertEq(twoG_proj.x, twoG_aff.x, "2G x mismatch");
        assertEq(twoG_proj.y, twoG_aff.y, "2G y mismatch");
    }

    /// Correctness: Jacobian add of G + 2G must equal affine 3G.
    function testCorrectness_JacobianAdd_vs_Affine() public view {
        Grumpkin.Point memory g = Grumpkin.generator();
        Grumpkin.Point memory twoG = Grumpkin.add(g, g);
        Grumpkin.Point memory threeG_aff = Grumpkin.add(twoG, g);

        GrumpkinJacobian.PointJ memory gJ = GrumpkinJacobian.fromAffine(g);
        GrumpkinJacobian.PointJ memory twoGJ =
            GrumpkinJacobian.addJ(gJ, gJ);
        GrumpkinJacobian.PointJ memory threeGJ =
            GrumpkinJacobian.addJ(twoGJ, gJ);
        Grumpkin.Point memory threeG_proj =
            GrumpkinJacobian.toAffine(threeGJ);

        assertEq(threeG_proj.inf, threeG_aff.inf, "3G inf mismatch");
        assertEq(threeG_proj.x, threeG_aff.x, "3G x mismatch");
        assertEq(threeG_proj.y, threeG_aff.y, "3G y mismatch");
    }

    /// Correctness: extended chain — 5G via Jacobian == 5G via affine.
    function testCorrectness_5G_Chain() public view {
        Grumpkin.Point memory g = Grumpkin.generator();
        Grumpkin.Point memory acc_aff = Grumpkin.Point({x: 0, y: 0, inf: true});
        for (uint256 i = 0; i < 5; i++) {
            acc_aff = Grumpkin.add(acc_aff, g);
        }

        GrumpkinJacobian.PointJ memory gJ = GrumpkinJacobian.fromAffine(g);
        GrumpkinJacobian.PointJ memory accJ = GrumpkinJacobian.identity();
        for (uint256 i = 0; i < 5; i++) {
            accJ = GrumpkinJacobian.addJ(accJ, gJ);
        }
        Grumpkin.Point memory acc_proj = GrumpkinJacobian.toAffine(accJ);

        assertEq(acc_proj.inf, acc_aff.inf, "5G inf mismatch");
        assertEq(acc_proj.x, acc_aff.x, "5G x mismatch");
        assertEq(acc_proj.y, acc_aff.y, "5G y mismatch");
    }

    function testGas_JacobianAdd_Distinct() public {
        Grumpkin.Point memory g = Grumpkin.generator();
        Grumpkin.Point memory twoG = Grumpkin.add(g, g);
        GrumpkinJacobian.PointJ memory gJ = GrumpkinJacobian.fromAffine(g);
        GrumpkinJacobian.PointJ memory twoGJ =
            GrumpkinJacobian.fromAffine(twoG);

        uint256 g0 = gasleft();
        GrumpkinJacobian.addJ(twoGJ, gJ);
        uint256 used = g0 - gasleft();
        emit log_named_uint("JACOBIAN_ADD_DISTINCT_GAS", used);
        // Reference: affine add ~3,834 gas.
        emit log_named_uint("AFFINE_ADD_GAS_REF", 3_834);
    }

    function testGas_JacobianDouble() public {
        Grumpkin.Point memory g = Grumpkin.generator();
        GrumpkinJacobian.PointJ memory gJ = GrumpkinJacobian.fromAffine(g);

        uint256 g0 = gasleft();
        GrumpkinJacobian.doubleJ(gJ);
        uint256 used = g0 - gasleft();
        emit log_named_uint("JACOBIAN_DOUBLE_GAS", used);
    }

    function testGas_JacobianToAffine() public {
        Grumpkin.Point memory g = Grumpkin.generator();
        Grumpkin.Point memory twoG = Grumpkin.add(g, g);
        GrumpkinJacobian.PointJ memory gJ = GrumpkinJacobian.fromAffine(g);
        GrumpkinJacobian.PointJ memory twoGJ = GrumpkinJacobian.addJ(gJ, gJ);
        // Need a non-Z=1 point for realistic toAffine cost.
        GrumpkinJacobian.PointJ memory threeGJ =
            GrumpkinJacobian.addJ(twoGJ, gJ);

        uint256 g0 = gasleft();
        GrumpkinJacobian.toAffine(threeGJ);
        uint256 used = g0 - gasleft();
        emit log_named_uint("JACOBIAN_TO_AFFINE_GAS", used);
        // Avoid silencing unused-warning on twoG.
        assertTrue(twoG.x != 0 || twoG.y != 0);
    }

    /// EXTRAPOLATION: Pippenger MSM gas with Jacobian arithmetic at
    /// n=16,384, c=8. Per-op savings flow through to the whole MSM.
    function testGas_Pippenger_Jacobian_Extrapolation() public {
        Grumpkin.Point memory g = Grumpkin.generator();
        Grumpkin.Point memory twoG = Grumpkin.add(g, g);
        GrumpkinJacobian.PointJ memory gJ = GrumpkinJacobian.fromAffine(g);
        GrumpkinJacobian.PointJ memory twoGJ =
            GrumpkinJacobian.fromAffine(twoG);

        uint256 g0 = gasleft();
        GrumpkinJacobian.addJ(twoGJ, gJ);
        uint256 jacAdd = g0 - gasleft();
        emit log_named_uint("JACOBIAN_ADD_GAS_MEASURED", jacAdd);

        // Pippenger formula: ⌈256/c⌉ · (n + 2^{c+1} - 2 + c) · add_gas
        uint256 n_aux = 16_384;
        uint256 c = 8;
        uint256 num_windows = (256 + c - 1) / c;
        uint256 per_window = n_aux + (1 << (c + 1)) - 2 + c;
        uint256 pip_jac = num_windows * per_window * jacAdd;
        emit log_named_uint(
            "PIPPENGER_JACOBIAN_ANALYTICAL_n16384_c8_GAS", pip_jac
        );

        // Compare with affine-Pippenger floor (~2.07B).
        emit log_named_uint("PIPPENGER_AFFINE_n16384_c8_GAS", 2_073_672_576);
        emit log_named_uint("L1_BLOCK_LIMIT_GAS", 30_000_000);
        emit log_named_uint("L2_BLOCK_LIMIT_OP_GAS", 30_000_000);
        // Speedup ratio.
        emit log_named_uint("SPEEDUP_RATIO_X100", 2_073_672_576 * 100 / pip_jac);
    }
}
