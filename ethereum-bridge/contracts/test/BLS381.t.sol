// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {BLS381} from "../src/lib/BLS381.sol";

/// @notice Smoke tests for the EIP-2537 precompile wrapper. Verifies the
///         wiring (correct addresses, correct input/output sizes, identity-
///         element behaviour) without yet exercising real BLS sigs from
///         EvaporChain — that lands in CommitCertVerifier.t.sol.
contract BLS381Test is Test {
    /// @dev G1 identity (point at infinity) per EIP-2537: 128 zero bytes.
    function _g1Zero() internal pure returns (bytes memory) {
        return new bytes(BLS381.G1_LEN);
    }

    /// @dev G2 identity: 256 zero bytes.
    function _g2Zero() internal pure returns (bytes memory) {
        return new bytes(BLS381.G2_LEN);
    }

    function test_g1Add_identityPlusIdentity_isIdentity() public view {
        bytes memory zero = _g1Zero();
        bytes memory result = BLS381.g1Add(zero, zero);
        assertEq(result.length, BLS381.G1_LEN);
        for (uint256 i = 0; i < result.length; i++) {
            assertEq(uint8(result[i]), 0, "G1 identity + identity != identity");
        }
    }

    function test_g2Add_identityPlusIdentity_isIdentity() public view {
        bytes memory zero = _g2Zero();
        bytes memory result = BLS381.g2Add(zero, zero);
        assertEq(result.length, BLS381.G2_LEN);
        for (uint256 i = 0; i < result.length; i++) {
            assertEq(uint8(result[i]), 0, "G2 identity + identity != identity");
        }
    }

    function test_g1Add_revertsOnWrongLength() public {
        bytes memory bad = new bytes(64); // 1/2 of G1
        bytes memory ok_ = _g1Zero();
        vm.expectRevert();
        this.callG1Add(bad, ok_);
    }

    function callG1Add(bytes memory a, bytes memory b) external view returns (bytes memory) {
        return BLS381.g1Add(a, b);
    }

    function test_pairingCheck_emptyOrMisaligned_reverts() public {
        bytes memory wrong = new bytes(100); // not a multiple of 384
        vm.expectRevert();
        this.callPairing(wrong);

        bytes memory empty = new bytes(0);
        vm.expectRevert();
        this.callPairing(empty);
    }

    function callPairing(bytes memory pairs) external view returns (bool) {
        return BLS381.pairingCheck(pairs);
    }

    /// @notice Pairing of identity-element pairs trivially equals 1.
    /// @dev    e(0_G1, 0_G2) = 1 in F_p^12, and the precompile returns 1
    ///         for "Π = 1". One pair of identities is enough.
    function test_pairingCheck_oneIdentityPair_returnsTrue() public view {
        bytes memory g1 = _g1Zero();
        bytes memory g2 = _g2Zero();
        bytes memory pairs = abi.encodePacked(g1, g2);
        assertTrue(BLS381.pairingCheck(pairs));
    }

    function test_g1Msm_singleZeroPair_returnsIdentity() public view {
        bytes memory g1 = _g1Zero();
        bytes memory scalar = new bytes(32);
        bytes memory pairs = abi.encodePacked(g1, scalar);
        bytes memory result = BLS381.g1Msm(pairs);
        assertEq(result.length, BLS381.G1_LEN);
        for (uint256 i = 0; i < result.length; i++) {
            assertEq(uint8(result[i]), 0);
        }
    }

    /// @notice Snapshot the gas of a 5-validator pairing check (2 pairs)
    ///         to track how Phase 2 verification compares to the ≤350k budget.
    function test_gasSnapshot_pairing2Pairs() public {
        bytes memory g1 = _g1Zero();
        bytes memory g2 = _g2Zero();
        bytes memory pairs = abi.encodePacked(g1, g2, g1, g2);
        uint256 g0 = gasleft();
        bool ok = BLS381.pairingCheck(pairs);
        uint256 used = g0 - gasleft();
        assertTrue(ok);
        emit log_named_uint("pairing(2) gas", used);
        // EIP-2537 spec: 37_700 + 32_600*N. For N=2 → 102_900. Allow 2x slack
        // for our wrapper + memory expansion.
        assertLt(used, 250_000);
    }
}
