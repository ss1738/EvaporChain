// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";
import {VerkleProofVerifier} from "../src/VerkleProofVerifier.sol";

/// @notice Phase 2.5 smoke test.
///
///         Loads the deterministic seed-0 fixture produced by
///         `cargo run --bin smoke-fixture-emit`, deploys
///         `VerkleProofVerifier` with the fixture VK, then checks:
///           1. Valid proof + matching public inputs → accepted.
///           2. Tampered first public input → rejected.
///           3. Tampered first proof byte → rejected.
///           4. Wrong proof length → InvalidProofLength revert.
///           5. Wrong public-input count → InvalidPublicInputCount revert.
///
///         The dummy circuit has vacuous Section 2/3 constraints so
///         the proof is sound for the current circuit shape.  This test
///         contract stays unchanged when the fixture is regenerated after
///         Section 2/3 are wired.
///
/// @dev    IC length is 5 (1 + 4 public inputs) for TrivialIncrementCircuit.
///         If the circuit shape changes, regenerate the fixture and update IC_LEN.
contract VerkleProofVerifierTest is Test {
    using stdJson for string;

    /// Number of IC points (1 + num_public_inputs).
    uint256 constant IC_LEN = 5;

    VerkleProofVerifier internal verifier;
    string              internal fix;
    bytes               internal proofBytes;
    uint256[]           internal pis; // public inputs

    function setUp() public {
        fix = vm.readFile("./fixtures/verkle_proof_smoke.json");

        // ── VK: alpha (G1), beta/gamma/delta (G2) ────────────────────────
        bytes memory alpha = vm.parseJsonBytes(fix, ".vk.alpha");
        bytes memory beta  = vm.parseJsonBytes(fix, ".vk.beta");
        bytes memory gamma = vm.parseJsonBytes(fix, ".vk.gamma");
        bytes memory delta = vm.parseJsonBytes(fix, ".vk.delta");

        // IC points: parse ic[0]…ic[IC_LEN-1] individually
        bytes[] memory ic = new bytes[](IC_LEN);
        for (uint256 i = 0; i < IC_LEN; ++i) {
            ic[i] = vm.parseJsonBytes(fix, string.concat(".vk.ic[", vm.toString(i), "]"));
        }

        verifier = new VerkleProofVerifier(alpha, beta, gamma, delta, ic);

        // ── Proof ─────────────────────────────────────────────────────────
        proofBytes = vm.parseJsonBytes(fix, ".proof");

        // ── Public inputs: hex strings → uint256 via bytes32 ─────────────
        // public_inputs is a JSON array of "0x<64 hex>" strings (32 bytes each).
        uint256 piLen = IC_LEN - 1;
        pis = new uint256[](piLen);
        for (uint256 i = 0; i < piLen; ++i) {
            bytes memory raw = vm.parseJsonBytes(
                fix, string.concat(".public_inputs[", vm.toString(i), "]")
            );
            // raw is exactly 32 bytes; decode as a big-endian uint256
            require(raw.length == 32, "VerkleProofVerifierTest: PI not 32 bytes");
            uint256 v;
            assembly { v := mload(add(raw, 32)) }
            pis[i] = v;
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    function test_realProofAccepted() public view {
        assertTrue(verifier.verify(proofBytes, pis), "real proof must verify");
    }

    function test_tamperedPublicInput_rejected() public view {
        uint256[] memory tampered = new uint256[](pis.length);
        for (uint256 i = 0; i < pis.length; ++i) tampered[i] = pis[i];
        tampered[0] = tampered[0] ^ 1;
        assertFalse(verifier.verify(proofBytes, tampered), "tampered PI must be rejected");
    }

    function test_tamperedProofByte_rejected() public {
        bytes memory bad = abi.encodePacked(proofBytes);
        bad[0] = bytes1(uint8(bad[0]) ^ 1);
        // A tampered proof is rejected either by returning false (on-curve tamper)
        // or by reverting PairingFailed (off-curve tamper — EIP-197 errors).
        // Both are valid rejections; treat both as a pass.
        try verifier.verify(bad, pis) returns (bool accepted) {
            assertFalse(accepted, "on-curve tampered proof must not be accepted");
        } catch {
            // Off-curve tamper: PairingFailed revert is also a valid rejection.
        }
    }

    function test_wrongProofLength_reverts() public {
        bytes memory short = new bytes(255);
        vm.expectRevert(
            abi.encodeWithSelector(VerkleProofVerifier.InvalidProofLength.selector, uint256(255))
        );
        verifier.verify(short, pis);
    }

    function test_wrongPublicInputCount_reverts() public {
        uint256[] memory tooMany = new uint256[](pis.length + 1);
        vm.expectRevert(
            abi.encodeWithSelector(
                VerkleProofVerifier.InvalidPublicInputCount.selector,
                uint256(pis.length + 1),
                uint256(pis.length)
            )
        );
        verifier.verify(proofBytes, tooMany);
    }
}
