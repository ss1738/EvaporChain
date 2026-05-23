// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";
import {VerkleProofVerifier} from "../src/VerkleProofVerifier.sol";

/// @notice B-1/B-2 1C (e)-2 EVM round-trip — Foundry test loading
///         the `recursion_decider_smoke.json` fixture (emitted by
///         `cargo run --bin recursion-decider-fixture-emit`) and
///         checking the existing Solidity `VerkleProofVerifier`
///         accepts a REAL Groth16 proof from the live
///         `RecursionDeciderCircuit` (Section A, n=4, sections B/C/D
///         deferred).
///
///         Section A binds only via witness-commit
///         (`sections_bcd_wired=false`), so `public_inputs.len() == 0`
///         and `IC` array has exactly 1 element (`IC[0]`, the
///         constant-1 term).
///
///         This is the symmetric closure of (d)-4: (d)-4 validated
///         the off-chain pipeline at production n_aux=16,384; this
///         closes the on-chain side at smoke n=4 by showing a real
///         proof EVM-verifies via EIP-197 ecPairing.
contract RecursionDeciderVerifierTest is Test {
    using stdJson for string;

    /// IC length = 1 + num_public_inputs = 1 + 0 = 1 for Section A only.
    uint256 constant IC_LEN = 1;

    VerkleProofVerifier internal verifier;
    string              internal fix;
    bytes               internal proofBytes;
    uint256[]           internal pis;

    function setUp() public {
        fix = vm.readFile("./fixtures/recursion_decider_smoke.json");

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

        // Section A: zero public inputs.
        pis = new uint256[](0);
    }

    function test_recursionDecider_proofAccepted() public view {
        assertTrue(
            verifier.verify(proofBytes, pis),
            "real RecursionDeciderCircuit proof must verify on-chain"
        );
    }

    function test_recursionDecider_tamperedProofByte_rejected() public {
        bytes memory bad = abi.encodePacked(proofBytes);
        bad[0] = bytes1(uint8(bad[0]) ^ 1);
        // Tampered proof rejected either by `false` return or by
        // PairingFailed revert (off-curve tamper → EIP-197 errors).
        try verifier.verify(bad, pis) returns (bool accepted) {
            assertFalse(
                accepted,
                "tampered proof byte must NOT verify on-chain"
            );
        } catch {
            // PairingFailed revert is also acceptable rejection.
        }
    }
}
