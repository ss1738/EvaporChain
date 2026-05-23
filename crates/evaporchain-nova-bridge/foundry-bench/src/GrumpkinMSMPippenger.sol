// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Grumpkin} from "./Grumpkin.sol";

/// @title Production-shape windowed Pippenger MSM on Grumpkin.
/// @notice Bucket-method MSM with window width `c`. For each window
/// of `c` consecutive bits in the scalars (processed MSB-first):
///   1. Each base is added to bucket[window_value - 1].
///   2. Buckets are summed using the running-sum trick:
///        sum_{k=1..2^c-1} k · B_k = R_1 + R_2 + ... + R_{2^c-1}
///      where R_k = B_k + B_{k+1} + ... + B_{2^c-1}.
///   3. Accumulator is shifted by 2^c (i.e. doubled `c` times) and
///      the window sum is added.
///
/// At width c, n bases, 256-bit scalars:
///   num_windows  = ⌈256/c⌉
///   per-window   = n point-adds (bucket assign) + 2·(2^c - 1)
///                  point-adds (running-sum) + c point-doubles (shift)
///   total        ≈ ⌈256/c⌉ · (n + 2^{c+1} - 2 + c) point-adds.
///
/// Optimal c grows with n; for n = 16,384 the empirical sweet-spot
/// is c = 8 (32 windows × 511 bucket-sum ops). The (c)-1c naive
/// measurement was 37 BILLION gas at n=16,384; this measures the
/// realistic Pippenger gas to anchor (6-α)'s analytical 62.7M
/// (which used the optimistic `n × point-add` lower bound).
library GrumpkinMSMPippenger {
    function msm_pippenger(
        Grumpkin.Point[] memory bases,
        uint256[] memory scalars,
        uint256 c
    ) internal view returns (Grumpkin.Point memory acc) {
        require(c >= 1 && c <= 8, "c in [1,8]");
        require(bases.length == scalars.length, "len mismatch");
        uint256 n = bases.length;
        uint256 num_windows = (256 + c - 1) / c;
        uint256 num_buckets = (1 << c) - 1;
        uint256 mask = num_buckets;

        acc = Grumpkin.Point({x: 0, y: 0, inf: true});

        // Process windows MSB → LSB. For the first window the
        // `acc * 2^c` shift is a no-op (identity stays identity).
        for (uint256 w = num_windows; w > 0; w--) {
            uint256 shift = (w - 1) * c;

            // Bucket array — initialized as identity.
            Grumpkin.Point[] memory buckets =
                new Grumpkin.Point[](num_buckets);
            for (uint256 b = 0; b < num_buckets; b++) {
                buckets[b] = Grumpkin.Point({x: 0, y: 0, inf: true});
            }

            // Assign each base to its bucket for this window.
            for (uint256 i = 0; i < n; i++) {
                uint256 v = (scalars[i] >> shift) & mask;
                if (v != 0) {
                    buckets[v - 1] =
                        Grumpkin.add(buckets[v - 1], bases[i]);
                }
            }

            // Sum buckets via running-sum trick (high → low).
            Grumpkin.Point memory running =
                Grumpkin.Point({x: 0, y: 0, inf: true});
            Grumpkin.Point memory window_sum =
                Grumpkin.Point({x: 0, y: 0, inf: true});
            for (uint256 b = num_buckets; b > 0; b--) {
                running = Grumpkin.add(running, buckets[b - 1]);
                window_sum = Grumpkin.add(window_sum, running);
            }

            // Shift acc by 2^c (c doublings) — skip on first window
            // (acc is identity, the doublings would be no-ops anyway,
            // but explicit-skip saves real gas vs branch-in-loop).
            if (w < num_windows) {
                for (uint256 d = 0; d < c; d++) {
                    acc = Grumpkin.add(acc, acc);
                }
            }
            acc = Grumpkin.add(acc, window_sum);
        }
    }
}
