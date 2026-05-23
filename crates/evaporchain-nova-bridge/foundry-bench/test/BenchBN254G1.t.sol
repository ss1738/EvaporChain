// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {BN254G1} from "../src/BN254G1.sol";

/// @notice 1C (c)-2c: BN254 precompile gas anchor. Tests the (R4)
/// "BN254 precompile re-routing" mitigation claim from (c)-2b's
/// dossier — analytical projection was ~22× vs Grumpkin Jacobian
/// (150 / 3,271 ≈ 22×). This measures whether that analytical
/// claim survives Foundry contact.
contract BenchBN254G1 is Test {
    function testGas_BN254_ECADD() public {
        // G + G (well-defined, both points on curve, distinct after
        // doubling logic). Precompile 0x06 handles doubling
        // case internally.
        (uint256 gx, uint256 gy) = BN254G1.generator();
        uint256 g0 = gasleft();
        (uint256 rx, uint256 ry) = BN254G1.ecAdd(gx, gy, gx, gy);
        uint256 used = g0 - gasleft();
        emit log_named_uint("BN254_ECADD_GAS", used);
        // Reference: Grumpkin Jacobian add measured 3,271 gas.
        emit log_named_uint("GRUMPKIN_JACOBIAN_ADD_GAS_REF", 3_271);
        // 2G x-coord well-known.
        assertTrue(rx != 0 && ry != 0, "2G must be non-identity");
    }

    function testGas_BN254_ECADD_Distinct() public {
        // G + 2G = 3G, distinct points (avoid doubling branch).
        (uint256 gx, uint256 gy) = BN254G1.generator();
        (uint256 x2, uint256 y2) = BN254G1.ecAdd(gx, gy, gx, gy);
        uint256 g0 = gasleft();
        BN254G1.ecAdd(gx, gy, x2, y2);
        uint256 used = g0 - gasleft();
        emit log_named_uint("BN254_ECADD_DISTINCT_GAS", used);
    }

    function testGas_BN254_ECMUL() public {
        (uint256 gx, uint256 gy) = BN254G1.generator();
        uint256 s = uint256(
            keccak256(abi.encodePacked("bn254-bench-scalar"))
        );
        uint256 g0 = gasleft();
        BN254G1.ecMul(gx, gy, s);
        uint256 used = g0 - gasleft();
        emit log_named_uint("BN254_ECMUL_GAS", used);
        // Reference: Grumpkin pure-EVM scalar-mul 1,545,603 gas.
        emit log_named_uint("GRUMPKIN_SCALARMUL_GAS_REF", 1_545_603);
    }

    /// Pippenger MSM at n=16,384 with BN254 precompile add.
    /// Same formula, different per-op anchor.
    function testGas_BN254_Pippenger_Extrapolation() public {
        // Re-measure the per-op cost in this same test for
        // determinism in extrapolation.
        (uint256 gx, uint256 gy) = BN254G1.generator();
        (uint256 x2, uint256 y2) = BN254G1.ecAdd(gx, gy, gx, gy);
        uint256 g0 = gasleft();
        BN254G1.ecAdd(gx, gy, x2, y2);
        uint256 measured_add = g0 - gasleft();
        emit log_named_uint("BN254_ECADD_MEASURED", measured_add);

        // Pippenger: ⌈256/c⌉ × (n + 2^{c+1} - 2 + c) × add_gas.
        uint256 n_aux = 16_384;
        uint256 c = 8;
        uint256 num_windows = (256 + c - 1) / c;
        uint256 per_window = n_aux + (1 << (c + 1)) - 2 + c;
        uint256 pip = num_windows * per_window * measured_add;
        emit log_named_uint(
            "PIPPENGER_BN254_PRECOMPILE_n16384_c8_GAS", pip
        );

        // Also c=10 (theoretical sweet spot) and c=12.
        uint256 c10 = 10;
        uint256 num_w10 = (256 + c10 - 1) / c10;
        uint256 pip_c10 =
            num_w10 * (n_aux + (1 << 11) - 2 + c10) * measured_add;
        emit log_named_uint(
            "PIPPENGER_BN254_PRECOMPILE_n16384_c10_GAS", pip_c10
        );
        uint256 c12 = 12;
        uint256 num_w12 = (256 + c12 - 1) / c12;
        uint256 pip_c12 =
            num_w12 * (n_aux + (1 << 13) - 2 + c12) * measured_add;
        emit log_named_uint(
            "PIPPENGER_BN254_PRECOMPILE_n16384_c12_GAS", pip_c12
        );

        // Comparison anchors.
        emit log_named_uint("GRUMPKIN_JACOBIAN_PIP_n16384_c8", 1_769_166_144);
        emit log_named_uint("GRUMPKIN_AFFINE_PIP_n16384_c8", 2_073_672_576);
        emit log_named_uint("NAIVE_MSM_n16384_GAS", 36_948_066_304);
        emit log_named_uint("L1_BLOCK_LIMIT_GAS", 30_000_000);
        emit log_named_uint("L2_BLOCK_LIMIT_GAS", 30_000_000);

        // Speedup vs Jacobian floor (basis points).
        uint256 speedup_bp = 1_769_166_144 * 10_000 / pip;
        emit log_named_uint("BN254_VS_JACOBIAN_SPEEDUP_BP", speedup_bp);
    }

    /// Per-base anchor: extract just the n-dependent term.
    /// Pippenger's n-amortised cost ≈ (num_windows × add_gas) per
    /// base, ignoring bucket-sum overhead which is n-independent.
    function testGas_BN254_PerBase_Anchor() public {
        // Measure single add (= per-base per-window cost).
        (uint256 gx, uint256 gy) = BN254G1.generator();
        (uint256 x2, uint256 y2) = BN254G1.ecAdd(gx, gy, gx, gy);
        uint256 g0 = gasleft();
        BN254G1.ecAdd(gx, gy, x2, y2);
        uint256 add_gas = g0 - gasleft();

        // c=8: 32 windows × add_gas per base.
        uint256 per_base_c8 = 32 * add_gas;
        // c=10: 26 windows × add_gas per base.
        uint256 per_base_c10 = 26 * add_gas;
        emit log_named_uint("BN254_PERBASE_c8", per_base_c8);
        emit log_named_uint("BN254_PERBASE_c10", per_base_c10);
        // Reference: Grumpkin affine per-base @ c=4, n=16: 736,364.
        emit log_named_uint("GRUMPKIN_AFFINE_PERBASE_REF", 736_364);
    }
}
