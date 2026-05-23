// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Grumpkin} from "../src/Grumpkin.sol";

/// @notice 1C increment 6-α: Foundry gas anchors for pure-EVM
/// Grumpkin operations. Measures (a) one point-add and (b) one
/// 256-bit scalar-mul. From these per-op anchors + a Pippenger
/// term-count formula, the n_aux=16,384 MSM gas extrapolates with
/// defensible bounds (no full MSM contract required at this
/// sub-step).
contract BenchGrumpkin is Test {
    function testGas_PointAddDistinct() public {
        Grumpkin.Point memory g = Grumpkin.generator();
        // Pre-compute 2G out-of-band so the add path is the
        // distinct-x branch (not the doubling branch).
        Grumpkin.Point memory two_g = Grumpkin.add(g, g);
        uint256 g0 = gasleft();
        Grumpkin.Point memory r = Grumpkin.add(g, two_g);
        uint256 used = g0 - gasleft();
        emit log_named_uint("GRUMPKIN_ADD_DISTINCT_GAS", used);
        // Sanity: returned point not infinity.
        assertEq(r.inf, false);
    }

    function testGas_PointAddDoubling() public {
        Grumpkin.Point memory g = Grumpkin.generator();
        uint256 g0 = gasleft();
        Grumpkin.Point memory r = Grumpkin.add(g, g);
        uint256 used = g0 - gasleft();
        emit log_named_uint("GRUMPKIN_ADD_DOUBLING_GAS", used);
        assertEq(r.inf, false);
    }

    function testGas_ScalarMul256() public {
        Grumpkin.Point memory g = Grumpkin.generator();
        // Use a generic 256-bit scalar (no special bit-pattern
        // optimisations); ~half the bits set.
        uint256 s = 0x9d3f6c8a5b2e4a7c81d63f9e2c5a8b4f7e1d6c9a3b5e8f2d4a7c1b6e9d3f5a82;
        uint256 g0 = gasleft();
        Grumpkin.Point memory r = Grumpkin.scalarMul(g, s);
        uint256 used = g0 - gasleft();
        emit log_named_uint("GRUMPKIN_SCALARMUL_256_GAS", used);
        assertEq(r.inf, false);
    }

    /// EXTRAPOLATION: print the n_aux=16,384 Pippenger best-case
    /// estimate from the measured point-add gas anchor. Pippenger
    /// at the best-case bound is ~n point-additions for windowed
    /// MSM (assuming optimal window size). Realistic
    /// implementations carry ~30-50% overhead from bucket
    /// management.
    function testGas_PippengerExtrapolation_n16384() public {
        Grumpkin.Point memory g = Grumpkin.generator();
        Grumpkin.Point memory two_g = Grumpkin.add(g, g);
        uint256 g0 = gasleft();
        Grumpkin.add(g, two_g);
        uint256 add_gas = g0 - gasleft();
        uint256 n = 16384;
        uint256 best_case = add_gas * n;
        // Realistic-with-overhead: +40% (Pippenger bucket
        // management + windowing scaffolding).
        uint256 realistic = best_case + (best_case * 40) / 100;
        emit log_named_uint("PIPPENGER_BEST_CASE_GAS_N16384", best_case);
        emit log_named_uint("PIPPENGER_REALISTIC_GAS_N16384", realistic);
        emit log_named_uint("ETHEREUM_L1_BLOCK_LIMIT_GAS", 30_000_000);
    }
}
