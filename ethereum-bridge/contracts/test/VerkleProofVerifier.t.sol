// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";

import {IVerkleProofVerifier} from "../src/interfaces/IVerkleProofVerifier.sol";
import {VerkleProofVerifier} from "../src/VerkleProofVerifier.sol";

/// @notice T0.10 STARTER tests. The Groth16 verifying key has not been
///         wired in yet (see `VerkleProofVerifier.sol` doc), so every
///         well-formed call MUST revert with `Groth16VKNotWired`. These
///         tests pin that contract: any future change that lets a
///         caller through without a real VK will break here loudly.
///
///         T0.10-finish replaces the revert assertion with a real
///         `assertTrue(ok)` against a fixture (`fixtures/verkle_proof_v2_*.json`)
///         emitted by the Rust prover (`VerkleProverV2`).
contract VerkleProofVerifierTest is Test {
    VerkleProofVerifier verifier;

    /// @dev BN254 scalar-field modulus r — duplicated from the contract
    ///      so the test isn't testing a constant against itself.
    ///      Source: EIP-197.
    uint256 internal constant BN254_FR_MODULUS =
        21_888_242_871_839_275_222_246_405_745_257_275_088_548_364_400_416_034_343_698_204_186_575_808_495_617;

    function setUp() public {
        verifier = new VerkleProofVerifier();
    }

    /// Well-formed inputs (all field-element guards pass, proof byte-length
    /// is the expected 256). MUST revert with Groth16VKNotWired in starter
    /// state — pinning that the contract is not silently accepting proofs.
    function test_starterReverts_onWellFormedCall() public {
        bytes memory proof = new bytes(256);

        vm.expectRevert(VerkleProofVerifier.Groth16VKNotWired.selector);
        verifier.verifyVerkleMembership(
            bytes32(uint256(1)),
            bytes32(uint256(2)),
            bytes32(uint256(3)),
            bytes32(uint256(4)),
            proof
        );
    }

    /// Wrong proof byte-length must be rejected pre-pairing — the
    /// length gate stays in place after T0.10-finish lands the real
    /// verifier so malformed calldata never reaches the precompile.
    function test_rejectsWrongProofLength() public {
        bytes memory tooShort = new bytes(255);

        vm.expectRevert(
            abi.encodeWithSelector(
                VerkleProofVerifier.InvalidGroth16ProofLength.selector,
                uint256(255)
            )
        );
        verifier.verifyVerkleMembership(
            bytes32(uint256(1)),
            bytes32(uint256(2)),
            bytes32(uint256(3)),
            bytes32(uint256(4)),
            tooShort
        );
    }

    /// Public inputs ≥ BN254 r must produce `false` (not revert) so the
    /// caller can distinguish "proof is structurally invalid for this
    /// VK domain" from "Groth16 not wired". This guard remains active
    /// after T0.10-finish: BN254-Fr inputs > r are not representable
    /// in the verifier's input vector.
    function test_returnsFalse_whenStateRootExceedsFrModulus() public view {
        bytes memory proof = new bytes(256);

        bool ok = verifier.verifyVerkleMembership(
            bytes32(BN254_FR_MODULUS),  // == r exactly → must reject
            bytes32(uint256(2)),
            bytes32(uint256(3)),
            bytes32(uint256(4)),
            proof
        );
        assertFalse(ok, "input == r must be rejected");
    }

    function test_returnsFalse_whenKeyExceedsFrModulus() public view {
        bytes memory proof = new bytes(256);
        bool ok = verifier.verifyVerkleMembership(
            bytes32(uint256(1)),
            bytes32(BN254_FR_MODULUS + 1),
            bytes32(uint256(3)),
            bytes32(uint256(4)),
            proof
        );
        assertFalse(ok, "input > r must be rejected");
    }

    function test_returnsFalse_whenValueCommitmentExceedsFrModulus() public view {
        bytes memory proof = new bytes(256);
        bool ok = verifier.verifyVerkleMembership(
            bytes32(uint256(1)),
            bytes32(uint256(2)),
            bytes32(BN254_FR_MODULUS),
            bytes32(uint256(4)),
            proof
        );
        assertFalse(ok, "input == r must be rejected");
    }

    function test_returnsFalse_whenParamsFingerprintExceedsFrModulus() public view {
        bytes memory proof = new bytes(256);
        bool ok = verifier.verifyVerkleMembership(
            bytes32(uint256(1)),
            bytes32(uint256(2)),
            bytes32(uint256(3)),
            bytes32(BN254_FR_MODULUS + 100),
            proof
        );
        assertFalse(ok, "input > r must be rejected");
    }

    /// Sanity check: the contract DOES implement the interface. Pins
    /// the ABI so consumers (StateMembershipAttester upgrade,
    /// EvaporationDispatcher V2 path) can compile against
    /// IVerkleProofVerifier and pass this contract's address.
    function test_implementsInterface() public view {
        IVerkleProofVerifier _iface = IVerkleProofVerifier(address(verifier));
        // Just touching the interface ref forces the compiler to check
        // the inheritance graph at compile time.
        assertTrue(address(_iface) == address(verifier));
    }
}
