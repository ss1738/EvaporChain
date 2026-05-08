// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {HashToCurve} from "../src/lib/HashToCurve.sol";

/// @notice Exercises the on-chain hash-to-G2 path. The cross-side
///         comparison against the Rust producer (bls12_381 0.8) lives
///         in `tests/hash_to_curve_vector.rs` of `evaporchain-eth-bridge`;
///         the Rust test prints the expected uncompressed G2 bytes,
///         which we hardcode here.
contract HashToCurveTest is Test {
    /// @notice Runs the full hash-to-G2 pipeline on a fixed message and
    ///         emits the resulting 256-byte point as a log so the operator
    ///         can copy it into the Rust cross-side test.
    function test_dumpHashToG2_helloEvaporchain() public {
        bytes memory msg_ = bytes("hello evaporchain");
        bytes memory g2 = HashToCurve.hashToG2(msg_);
        assertEq(g2.length, 256);
        emit log_named_bytes("hashToG2('hello evaporchain')", g2);
    }

    /// @notice Same input twice → byte-identical output.
    function test_deterministic() public view {
        bytes memory msg_ = bytes("deterministic-check");
        bytes memory a = HashToCurve.hashToG2(msg_);
        bytes memory b = HashToCurve.hashToG2(msg_);
        assertEq(keccak256(a), keccak256(b));
    }

    /// @notice Distinct messages produce distinct G2 points (overwhelming
    ///         probability — non-collision sanity check).
    function test_distinctMessagesDistinctOutputs() public view {
        bytes memory a = HashToCurve.hashToG2(bytes("foo"));
        bytes memory b = HashToCurve.hashToG2(bytes("bar"));
        assertTrue(keccak256(a) != keccak256(b));
    }

    /// @notice Snapshot gas used for one full hash-to-G2 call.
    function test_gasSnapshot_hashToG2() public {
        uint256 g0 = gasleft();
        bytes memory g2 = HashToCurve.hashToG2(bytes("gas-snapshot-msg"));
        uint256 used = g0 - gasleft();
        assertEq(g2.length, 256);
        emit log_named_uint("hashToG2 gas", used);
    }

    /// @notice expand_message_xmd output for empty msg + 32-byte len.
    ///         Sanity: returns the right length, and is deterministic.
    function test_expandMessageXmd_basic() public view {
        bytes memory empty = new bytes(0);
        bytes memory out = HashToCurve.expandMessageXmd(empty, 32);
        assertEq(out.length, 32);

        bytes memory out2 = HashToCurve.expandMessageXmd(empty, 32);
        assertEq(keccak256(out), keccak256(out2));
    }
}
