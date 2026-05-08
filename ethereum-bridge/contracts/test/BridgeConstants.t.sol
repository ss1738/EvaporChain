// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {BridgeConstants} from "../src/BridgeConstants.sol";

/// @notice Phase 0 smoke test. Asserts the constants are non-zero and
///         that the EIP-2537 precompile addresses are wired correctly.
contract BridgeConstantsTest is Test {
    function test_domainTagsDistinct() public pure {
        assertTrue(BridgeConstants.DOMAIN_TAG_COMMIT != bytes32(0));
        assertTrue(BridgeConstants.DOMAIN_TAG_VERKLE_ROOT != bytes32(0));
        assertTrue(BridgeConstants.DOMAIN_TAG_GHOST_RECORD != bytes32(0));
        assertTrue(BridgeConstants.DOMAIN_TAG_COMMIT != BridgeConstants.DOMAIN_TAG_VERKLE_ROOT);
        assertTrue(BridgeConstants.DOMAIN_TAG_COMMIT != BridgeConstants.DOMAIN_TAG_GHOST_RECORD);
        assertTrue(BridgeConstants.DOMAIN_TAG_VERKLE_ROOT != BridgeConstants.DOMAIN_TAG_GHOST_RECORD);
    }

    function test_eip2537PrecompileAddresses() public pure {
        // Per EIP-2537 final spec.
        assertEq(uint160(uint160(BridgeConstants.BLS12_G1ADD)), 0x0b);
        assertEq(uint160(uint160(BridgeConstants.BLS12_G1MSM)), 0x0c);
        assertEq(uint160(uint160(BridgeConstants.BLS12_G2ADD)), 0x0d);
        assertEq(uint160(uint160(BridgeConstants.BLS12_G2MSM)), 0x0e);
        assertEq(uint160(uint160(BridgeConstants.BLS12_PAIRING)), 0x0f);
        assertEq(uint160(uint160(BridgeConstants.BLS12_MAP_FP_TO_G1)), 0x10);
        assertEq(uint160(uint160(BridgeConstants.BLS12_MAP_FP2_TO_G2)), 0x11);
    }

    function test_stakeThreshold() public pure {
        assertEq(BridgeConstants.STAKE_NUM, 2);
        assertEq(BridgeConstants.STAKE_DEN, 3);
        assertGt(BridgeConstants.MAX_VALIDATORS, 0);
    }
}
