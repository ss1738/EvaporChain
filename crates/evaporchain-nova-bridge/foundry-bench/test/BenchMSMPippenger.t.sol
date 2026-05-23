// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Grumpkin} from "../src/Grumpkin.sol";
import {GrumpkinMSM} from "../src/GrumpkinMSM.sol";
import {GrumpkinMSMPippenger} from "../src/GrumpkinMSMPippenger.sol";

/// @notice 1C (c)-2a: realistic Pippenger MSM anchor.
/// (c)-1c gave the worst-case naive ceiling (37B gas at n=16,384,
/// 590× over (6-α)'s analytical Pippenger estimate). This measures
/// the REALISTIC Pippenger gas — turning (6-α)'s `n × point-add`
/// optimistic lower bound into a measured number.
contract BenchMSMPippenger is Test {
    /// Correctness: Pippenger must equal naive on a small case.
    function testCorrectness_Pippenger_vs_Naive_n4() public view {
        uint256 n = 4;
        Grumpkin.Point[] memory bases = new Grumpkin.Point[](n);
        uint256[] memory scalars = new uint256[](n);
        Grumpkin.Point memory g = Grumpkin.generator();
        Grumpkin.Point memory cur = g;
        for (uint256 i = 0; i < n; i++) {
            bases[i] = cur;
            scalars[i] = uint256(
                keccak256(abi.encodePacked(i, "pippenger-correctness"))
            ) >> 200; // ~56-bit scalars so naive stays fast
            cur = Grumpkin.add(cur, g);
        }
        Grumpkin.Point memory naive = GrumpkinMSM.msm_naive(bases, scalars);
        Grumpkin.Point memory pip4 =
            GrumpkinMSMPippenger.msm_pippenger(bases, scalars, 4);
        Grumpkin.Point memory pip2 =
            GrumpkinMSMPippenger.msm_pippenger(bases, scalars, 2);

        // Identity ⇒ both inf, x/y don't matter.
        assertEq(pip4.inf, naive.inf, "pip4 inf mismatch");
        if (!naive.inf) {
            assertEq(pip4.x, naive.x, "pip4 x mismatch");
            assertEq(pip4.y, naive.y, "pip4 y mismatch");
        }
        assertEq(pip2.inf, naive.inf, "pip2 inf mismatch");
        if (!naive.inf) {
            assertEq(pip2.x, naive.x, "pip2 x mismatch");
            assertEq(pip2.y, naive.y, "pip2 y mismatch");
        }
    }

    function bench_pippenger_at(uint256 n, uint256 c)
        internal
        returns (uint256)
    {
        Grumpkin.Point[] memory bases = new Grumpkin.Point[](n);
        uint256[] memory scalars = new uint256[](n);
        Grumpkin.Point memory g = Grumpkin.generator();
        Grumpkin.Point memory cur = g;
        for (uint256 i = 0; i < n; i++) {
            bases[i] = cur;
            scalars[i] = uint256(
                keccak256(abi.encodePacked(i, "pippenger-bench-scalar"))
            );
            cur = Grumpkin.add(cur, g);
        }
        uint256 g0 = gasleft();
        GrumpkinMSMPippenger.msm_pippenger(bases, scalars, c);
        return g0 - gasleft();
    }

    function testGas_Pippenger_n8_c4() public {
        uint256 used = bench_pippenger_at(8, 4);
        emit log_named_uint("PIPPENGER_n8_c4_GAS", used);
    }

    function testGas_Pippenger_n16_c4() public {
        uint256 used = bench_pippenger_at(16, 4);
        emit log_named_uint("PIPPENGER_n16_c4_GAS", used);
    }

    function testGas_Pippenger_n16_c2() public {
        uint256 used = bench_pippenger_at(16, 2);
        emit log_named_uint("PIPPENGER_n16_c2_GAS", used);
    }

    /// EXTRAPOLATION: realistic Pippenger at n=16,384 with c=8.
    /// Per-base cost ≈ (num_windows × c) point-adds for the bucket
    /// assignment step alone — at c=8, that's 32 × 8 = 256 point-adds
    /// per base × ~3.8k gas ≈ 970k gas/base (with bucket-sum and
    /// shift amortised over n).
    ///
    /// More precisely, total ≈ ⌈256/c⌉ · (n + 2^{c+1} - 2 + c) · ~3.8k
    /// At n=16,384, c=8:
    ///   32 · (16,384 + 510 + 8) · 3,834 ≈ 32 · 16,902 · 3,834 ≈ 2.07B
    /// At n=16,384, c=10 (theoretical, not measured):
    ///   ⌈256/10⌉=26 · (16,384 + 2046 + 10) · 3,834 ≈ 1.84B
    /// At n=16,384, c=12:
    ///   ⌈256/12⌉=22 · (16,384 + 8190 + 12) · 3,834 ≈ 2.08B
    ///
    /// Sweet-spot c ≈ 9-10 → ~1.8 BILLION gas. STILL ~60× over L1
    /// block limit, but ~20× better than naive (37B).
    function testGas_Pippenger_Extrapolation() public {
        uint256 g16_c4 = bench_pippenger_at(16, 4);
        uint256 per_base_c4 = g16_c4 / 16;
        emit log_named_uint("PIPPENGER_n16_c4_GAS", g16_c4);
        emit log_named_uint("PIPPENGER_PER_BASE_GAS_c4", per_base_c4);

        // c=4 has 64 windows; c=8 has 32 windows but 2^c=256 buckets
        // (vs 15). For large n the per-base term dominates so the
        // per-base extrapolation overcounts for c=8 — use the
        // analytical formula instead for n=16,384.
        uint256 n_aux = 16_384;
        uint256 point_add_gas = 3_834; // (6-α)
        // c=8 analytical: 32 × (16,384 + 510 + 8) × 3,834
        uint256 pip_c8 = 32 * (n_aux + 510 + 8) * point_add_gas;
        emit log_named_uint(
            "PIPPENGER_ANALYTICAL_n16384_c8_GAS", pip_c8
        );

        // Compare with the 3 reference points.
        emit log_named_uint("NAIVE_MSM_n16384_GAS_CEILING", 36_948_066_304);
        emit log_named_uint("ANALYTICAL_6_ALPHA_BEST_GAS", 62_734_336);
        emit log_named_uint("L1_BLOCK_LIMIT_GAS", 30_000_000);
        // L2 block limits vary; Optimism ~30M, Arbitrum ~32M.
        emit log_named_uint("L2_BLOCK_LIMIT_OP_GAS", 30_000_000);
    }
}
