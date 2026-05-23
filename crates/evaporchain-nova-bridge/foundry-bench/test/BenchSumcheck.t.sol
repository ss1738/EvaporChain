// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Bn254Fq} from "../src/Bn254Fq.sol";
import {SumcheckRound} from "../src/SumcheckRound.sol";

/// @notice 1C increment (c)-1: gas anchors for Spartan sumcheck
/// verification in pure-EVM Bn254Fq arithmetic. Measures one
/// cubic-sumcheck round, then extrapolates to the full ppsnark
/// verifier's 3 sumchecks × log(N) rounds.
contract BenchSumcheck is Test {
    function testGas_Bn254Fq_Mul() public {
        uint256 a = 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef;
        uint256 b = 0xfedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321;
        uint256 g0 = gasleft();
        uint256 r = Bn254Fq.mul_q(a, b);
        uint256 used = g0 - gasleft();
        emit log_named_uint("BN254FQ_MUL_GAS", used);
        assertTrue(r != 0);
    }

    function testGas_Bn254Fq_Inv() public {
        uint256 a = 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef;
        uint256 g0 = gasleft();
        uint256 r = Bn254Fq.inv_q(a);
        uint256 used = g0 - gasleft();
        emit log_named_uint("BN254FQ_INV_GAS", used);
        assertTrue(r != 0);
    }

    function testGas_Bn254Fq_EvalDeg3() public {
        uint256 c0 = 0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef;
        uint256 c1 = 0xcafebabecafebabecafebabecafebabecafebabecafebabecafebabecafebabe;
        uint256 c2 = 0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef;
        uint256 c3 = 0xfedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210;
        uint256 x = 0x1111111122222222333333334444444455555555666666667777777788888888;
        uint256 g0 = gasleft();
        uint256 r = Bn254Fq.eval_deg3(c0, c1, c2, c3, x);
        uint256 used = g0 - gasleft();
        emit log_named_uint("BN254FQ_EVAL_DEG3_GAS", used);
        assertTrue(r != 0);
    }

    function testGas_Sumcheck_OneCubicRound() public {
        uint256 prev_claim = 0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef;
        uint256 c0 = 0x1111111111111111111111111111111111111111111111111111111111111111;
        uint256 c1 = 0x2222222222222222222222222222222222222222222222222222222222222222;
        uint256 c2 = 0x3333333333333333333333333333333333333333333333333333333333333333;
        uint256 c3 = 0x4444444444444444444444444444444444444444444444444444444444444444;
        uint256 challenge = 0xabcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789;
        uint256 g0 = gasleft();
        (uint256 new_claim, ) = SumcheckRound.verify_round_cubic(
            prev_claim, c0, c1, c2, c3, challenge
        );
        uint256 used = g0 - gasleft();
        emit log_named_uint("SUMCHECK_ONE_CUBIC_ROUND_GAS", used);
        assertTrue(new_claim != 0);
    }

    /// EXTRAPOLATION: full ppsnark verifier's sumcheck cost.
    /// ppsnark verify (per `spartan/ppsnark.rs::verify`) runs:
    ///   - num_rounds_outer = log2(num_cons) rounds (cubic).
    ///   - num_rounds_inner = log2(S_comm.N) rounds (quadratic; we
    ///     conservatively bound with cubic cost too).
    ///   - sc_batch: another sumcheck (treat as cubic).
    /// For n_aux=16,384 secondary: num_cons≈1,985 → log2≈11 rounds
    /// outer; S_comm.N=16,384 → log2=14 inner. Total ~25-40 rounds
    /// across the 3 sumcheck proofs. Use 40 as a conservative
    /// upper bound for extrapolation.
    function testGas_PpsnarkSumcheck_Extrapolation() public {
        uint256 prev = 1;
        uint256 c0 = 1; uint256 c1 = 2; uint256 c2 = 3; uint256 c3 = 4;
        uint256 ch = 5;
        uint256 g0 = gasleft();
        (uint256 nc, ) = SumcheckRound.verify_round_cubic(prev, c0, c1, c2, c3, ch);
        uint256 round_gas = g0 - gasleft();

        // Conservative: 40 rounds total across all 3 sumcheck
        // proofs at n_aux=16,384 secondary shape.
        uint256 rounds = 40;
        uint256 sumcheck_total = round_gas * rounds;
        emit log_named_uint("ONE_ROUND_GAS", round_gas);
        emit log_named_uint("SUMCHECK_TOTAL_GAS_40_ROUNDS", sumcheck_total);

        // For context: compare to (6-α) IPA MSM at n=16,384
        // Pippenger best ~62.7M.
        emit log_named_uint("CONTEXT_IPA_MSM_GAS_BEST", 62_734_336);
        emit log_named_uint("ETHEREUM_L1_BLOCK_LIMIT_GAS", 30_000_000);

        assertTrue(nc != 0);
    }
}
