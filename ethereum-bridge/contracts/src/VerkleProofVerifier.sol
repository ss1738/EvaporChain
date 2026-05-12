// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {IVerkleProofVerifier} from "./interfaces/IVerkleProofVerifier.sol";

/// @title  VerkleProofVerifier
/// @notice T0.10 STARTER. On-chain Groth16 wrap of the Halo2 IPA Verkle
///         membership proof produced by `VerkleProverV2` (Rust).
///
///         **Status:** the verifying key, IC-points, and pairing call
///         are NOT YET WIRED. Every call to `verifyVerkleMembership`
///         currently reverts with `Groth16VKNotWired`. T0.10-finish
///         lands:
///           1. The Halo2-IPA-verifier-in-BN254 circuit (arkworks /
///              circom) and its trusted-setup ceremony output.
///           2. The verifying-key constants below (alpha, beta, gamma,
///              delta, IC[]).
///           3. The pairing-check body inside `_verifyGroth16`.
///
///         Until then this contract exists so that consumers
///         (StateMembershipAttester upgrade, EvaporationDispatcher V2
///         path) can be written against the final interface, but
///         attempting to validate a real proof reverts loudly. This is
///         deliberate — a "always-true" stub would be a silent footgun.
///
/// @dev    Public-input layout (passed into the BN254 Groth16 verifier
///         as Fr elements, big-endian, reduced mod r):
///         input[0] = uint256(stateRoot)
///         input[1] = uint256(key)
///         input[2] = uint256(valueCommitment)
///         input[3] = uint256(paramsFingerprint)
///
///         The Halo2 IPA proof bytes are NOT a public input — they are
///         the witness consumed by the wrapper circuit during proving;
///         only the bound (stateRoot, key, value, params) end up on
///         chain.
contract VerkleProofVerifier is IVerkleProofVerifier {
    /// @notice Reverts when the Groth16 verifying key has not been
    ///         wired in (T0.10 starter state).
    error Groth16VKNotWired();

    /// @notice Reverts when the Groth16 proof byte-length is not the
    ///         expected 256 bytes (8 × uint256: A, B, C).
    /// @param  got the actual proof length in bytes.
    error InvalidGroth16ProofLength(uint256 got);

    /// @notice The BN254 prime field modulus r (scalar field). Public
    ///         inputs must be reduced mod r before being fed to the
    ///         pairing check; if any input is ≥ r, the verifier MUST
    ///         reject. Source: EIP-197.
    uint256 internal constant BN254_FR_MODULUS =
        21_888_242_871_839_275_222_246_405_745_257_275_088_548_364_400_416_034_343_698_204_186_575_808_495_617;

    /// @notice Expected Groth16 proof byte-length (A: 64B, B: 128B, C: 64B).
    uint256 internal constant GROTH16_PROOF_BYTES = 256;

    /// @inheritdoc IVerkleProofVerifier
    function verifyVerkleMembership(
        bytes32 stateRoot,
        bytes32 key,
        bytes32 valueCommitment,
        bytes32 paramsFingerprint,
        bytes calldata groth16Proof
    ) external view returns (bool ok) {
        // Cheap structural checks first — these will remain in T0.10-finish
        // as a pre-pairing gate so malformed calldata reverts before any
        // pairing-precompile gas is burned.
        if (groth16Proof.length != GROTH16_PROOF_BYTES) {
            revert InvalidGroth16ProofLength(groth16Proof.length);
        }

        // Public inputs must be < r. uint256(bytes32(...)) can equal up
        // to 2^256 − 1, which is > r, so reject early.
        if (uint256(stateRoot) >= BN254_FR_MODULUS) return false;
        if (uint256(key) >= BN254_FR_MODULUS) return false;
        if (uint256(valueCommitment) >= BN254_FR_MODULUS) return false;
        if (uint256(paramsFingerprint) >= BN254_FR_MODULUS) return false;

        // T0.10 STARTER GATE — the verifying key is not yet provided.
        // T0.10-finish replaces this revert with the real pairing call.
        // (All four field-element inputs were already consumed by the
        // FR_MODULUS bounds checks above.)
        revert Groth16VKNotWired();
    }

    /// @dev Placeholder for the Groth16 pairing call. T0.10-finish lands
    ///      the body: (alpha, beta, gamma, delta, IC[0..N]) constants
    ///      from the trusted setup, then
    ///        e(A, B) == e(alpha, beta)
    ///                · e(IC[0] + Σ IC[i]·input[i-1], gamma)
    ///                · e(C, delta)
    ///      via the EIP-197 pairing precompile at 0x08.
    function _verifyGroth16(
        uint256[2] memory, /* a */
        uint256[2][2] memory, /* b */
        uint256[2] memory, /* c */
        uint256[4] memory /* publicInputs */
    ) internal pure returns (bool) {
        revert Groth16VKNotWired();
    }
}
