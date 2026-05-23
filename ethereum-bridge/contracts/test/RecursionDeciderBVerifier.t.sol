// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";
import {VerkleProofVerifier} from "../src/VerkleProofVerifier.sol";

/// @notice B-1/B-2 1C Section B EVM round-trip: extends the
///         (e)-2 RecursionDeciderVerifier test to thread the 11-PI
///         Section B bundle through on-chain Groth16 verify.
///
///         Per dossier §6b, Section B is delegation-bound: on-chain
///         Groth16 binds Section A only; Section B PIs are present
///         but decorative (binding lives in the off-chain
///         `assemble_section_b_pi_bundle` adapter). This Foundry
///         test confirms the on-chain verifier accepts the 11-PI
///         proof emitted by `recursion-decider-b-fixture-emit`.
///
///         IC_LEN = 12 (= 1 + 11 PIs).
contract RecursionDeciderBVerifierTest is Test {
    using stdJson for string;

    uint256 constant IC_LEN = 12;
    uint256 constant PI_COUNT = 11;

    VerkleProofVerifier internal verifier;
    string              internal fix;
    bytes               internal proofBytes;
    uint256[]           internal pis;

    function setUp() public {
        fix = vm.readFile("./fixtures/recursion_decider_b_smoke.json");

        bytes memory alpha = vm.parseJsonBytes(fix, ".vk.alpha");
        bytes memory beta  = vm.parseJsonBytes(fix, ".vk.beta");
        bytes memory gamma = vm.parseJsonBytes(fix, ".vk.gamma");
        bytes memory delta = vm.parseJsonBytes(fix, ".vk.delta");

        bytes[] memory ic = new bytes[](IC_LEN);
        for (uint256 i = 0; i < IC_LEN; ++i) {
            ic[i] = vm.parseJsonBytes(
                fix, string.concat(".vk.ic[", vm.toString(i), "]")
            );
        }

        verifier = new VerkleProofVerifier(alpha, beta, gamma, delta, ic);

        proofBytes = vm.parseJsonBytes(fix, ".proof");

        // Parse 11 PIs from the fixture's public_inputs array.
        pis = new uint256[](PI_COUNT);
        for (uint256 i = 0; i < PI_COUNT; ++i) {
            bytes memory raw = vm.parseJsonBytes(
                fix, string.concat(".public_inputs[", vm.toString(i), "]")
            );
            require(raw.length == 32, "PI not 32 bytes");
            uint256 v;
            assembly { v := mload(add(raw, 32)) }
            pis[i] = v;
        }
    }

    function test_sectionB_proofAccepted() public view {
        assertTrue(
            verifier.verify(proofBytes, pis),
            "Section B real proof must verify on-chain with 11 PIs"
        );
    }

    function test_sectionB_tamperedFirstPI_rejected() public view {
        // Tamper hash_secondary_claimed (PI[0]).
        uint256[] memory tampered = new uint256[](PI_COUNT);
        for (uint256 i = 0; i < PI_COUNT; ++i) tampered[i] = pis[i];
        tampered[0] = tampered[0] ^ 1;
        assertFalse(
            verifier.verify(proofBytes, tampered),
            "tampered PI[0] must NOT verify (Groth16 binds the PI slice)"
        );
    }

    function test_sectionB_tamperedLastPI_rejected() public view {
        // Tamper zn[0] (PI[10]).
        uint256[] memory tampered = new uint256[](PI_COUNT);
        for (uint256 i = 0; i < PI_COUNT; ++i) tampered[i] = pis[i];
        tampered[10] = tampered[10] ^ 1;
        assertFalse(
            verifier.verify(proofBytes, tampered),
            "tampered PI[10] must NOT verify"
        );
    }
}
