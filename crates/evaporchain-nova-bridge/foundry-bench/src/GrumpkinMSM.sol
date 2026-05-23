// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Grumpkin} from "./Grumpkin.sol";

/// @title Naive Grumpkin MSM (sum of scalar_i · base_i) — the
/// UPPER-BOUND cost. Real ppsnark Solidity verifiers use Pippenger
/// for asymptotically-optimal n · point_add cost; this naive
/// version measures n · scalar_mul + (n-1) · add, which is the
/// pessimistic ceiling. Useful as a sanity check on (6-α)'s
/// Pippenger best-case extrapolation: any production MSM gas
/// must lie between naive (here) and Pippenger best (~62.7M for
/// n=16,384).
library GrumpkinMSM {
    function msm_naive(
        Grumpkin.Point[] memory bases,
        uint256[] memory scalars
    ) internal view returns (Grumpkin.Point memory acc) {
        require(bases.length == scalars.length, "len mismatch");
        acc = Grumpkin.Point({x: 0, y: 0, inf: true});
        for (uint256 i = 0; i < bases.length; i++) {
            Grumpkin.Point memory term =
                Grumpkin.scalarMul(bases[i], scalars[i]);
            acc = Grumpkin.add(acc, term);
        }
    }
}
