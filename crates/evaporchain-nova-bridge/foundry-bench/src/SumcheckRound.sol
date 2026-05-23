// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Bn254Fq} from "./Bn254Fq.sol";

/// @title One round of Spartan sumcheck verification (the inner
/// loop of the ppsnark verifier's sumcheck checks).
///
/// Per round, given previous claim `s` and the prover-supplied
/// univariate polynomial P(X) (degree ≤ 3 for outer sumcheck,
/// ≤ 2 for inner/batch — verifier passes c3=0 for the lower-
/// degree variants), verify:
///   1. P(0) + P(1) == s  (sum consistency).
///   2. Generate Fiat-Shamir challenge r from a hash of state
///      (here a placeholder — production uses Neptune sponge
///      absorb of the coefficients).
///   3. New claim = P(r).
/// Returns (new_claim, challenge_used). Production verifier
/// chains these calls log(N) times per sumcheck.
library SumcheckRound {
    /// One round of a cubic-degree sumcheck (outer sumcheck).
    /// Production version absorbs (c0,c1,c2,c3) into a transcript;
    /// for the benchmark we pass a deterministic challenge to
    /// isolate the per-round arithmetic cost.
    function verify_round_cubic(
        uint256 prev_claim,
        uint256 c0,
        uint256 c1,
        uint256 c2,
        uint256 c3,
        uint256 challenge
    ) internal pure returns (uint256 new_claim, bool consistent) {
        // P(0) = c0; P(1) = c0+c1+c2+c3. Sum consistency: P(0)+P(1)
        // = 2·c0 + c1 + c2 + c3 must equal prev_claim.
        uint256 sum = addmod(c0, c0, Bn254Fq.Q);
        sum = addmod(sum, c1, Bn254Fq.Q);
        sum = addmod(sum, c2, Bn254Fq.Q);
        sum = addmod(sum, c3, Bn254Fq.Q);
        consistent = (sum == prev_claim);
        new_claim = Bn254Fq.eval_deg3(c0, c1, c2, c3, challenge);
    }
}
