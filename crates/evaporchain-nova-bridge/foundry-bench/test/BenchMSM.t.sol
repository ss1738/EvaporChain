// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Grumpkin} from "../src/Grumpkin.sol";
import {GrumpkinMSM} from "../src/GrumpkinMSM.sol";

/// @notice 1C (c)-1c: naive MSM gas anchor (worst-case ceiling).
/// Per (6-α): point-add ~3,834 gas, scalar-mul ~1.5M gas.
/// Naive MSM at n: ~n × 1.5M + (n-1) × 3.8k.
/// Pippenger best at n: ~n × 3.8k. Real production: somewhere in
/// between, closer to Pippenger.
contract BenchMSM is Test {
    function bench_naive_msm_at(uint256 n) internal returns (uint256) {
        Grumpkin.Point[] memory bases = new Grumpkin.Point[](n);
        uint256[] memory scalars = new uint256[](n);
        Grumpkin.Point memory g = Grumpkin.generator();
        // Synthesize n distinct bases via doubling chain; scalars
        // are pseudo-random non-trivial values.
        Grumpkin.Point memory cur = g;
        for (uint256 i = 0; i < n; i++) {
            bases[i] = cur;
            scalars[i] =
                uint256(keccak256(abi.encodePacked(i, "msm-bench-scalar")));
            cur = Grumpkin.add(cur, g);
        }
        uint256 g0 = gasleft();
        GrumpkinMSM.msm_naive(bases, scalars);
        return g0 - gasleft();
    }

    function testGas_MSM_Naive_n8() public {
        uint256 used = bench_naive_msm_at(8);
        emit log_named_uint("MSM_NAIVE_n8_GAS", used);
    }

    function testGas_MSM_Naive_n16() public {
        uint256 used = bench_naive_msm_at(16);
        emit log_named_uint("MSM_NAIVE_n16_GAS", used);
    }

    /// EXTRAPOLATION: naive MSM at n=16,384 from per-base cost.
    /// Naive grows linearly with n (no asymptotic improvement);
    /// at small n the per-base setup overhead inflates it, so
    /// linear extrapolation from n=16 is conservative.
    function testGas_MSM_Naive_Extrapolation() public {
        uint256 g16 = bench_naive_msm_at(16);
        uint256 per_base = g16 / 16;
        uint256 n_aux = 16_384;
        uint256 naive_total = per_base * n_aux;
        emit log_named_uint("MSM_NAIVE_n16_GAS", g16);
        emit log_named_uint("MSM_NAIVE_PER_BASE_GAS", per_base);
        emit log_named_uint(
            "MSM_NAIVE_EXTRAPOLATED_n16384_GAS", naive_total
        );
        // (6-α) Pippenger best-case anchor for comparison.
        emit log_named_uint("MSM_PIPPENGER_BEST_n16384_GAS", 62_734_336);
        emit log_named_uint("L1_BLOCK_LIMIT_GAS", 30_000_000);
    }
}
