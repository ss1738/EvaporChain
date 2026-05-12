// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

/// @title  IVerkleProofVerifier
/// @notice On-chain consumer of EvaporChain's Phase 4 V2 Verkle-membership
///         proofs. The Rust prover (`VerkleProverV2` in
///         `ethereum-bridge/circuits/src/circuit_v2.rs`) emits Halo2-IPA
///         proof bytes over Pallas/Vesta. Pallas curve operations are not
///         available as Ethereum precompiles, so direct on-chain IPA
///         verification is prohibitively expensive (~10M+ gas per proof).
///
///         The shipping path therefore wraps the Halo2-IPA proof in a
///         Groth16 SNARK over BN254 (verifying-circuit input = the IPA
///         proof; verifying-circuit output = the bound public inputs).
///         BN254 pairings are EIP-197 precompiles, so the on-chain
///         verifier reduces to a standard Groth16 pairing check.
///
///         This interface is the abstract contract; the impl
///         (`VerkleProofVerifier.sol`) is currently a T0.10 starter that
///         reverts with `Groth16VKNotWired` until the trusted-setup
///         verifying key is supplied. T0.10-finish wires the real VK
///         and turns these calls live.
///
/// @dev    Public-input binding (in the order they appear in the
///         Groth16 input vector):
///         - `stateRoot`         — 32 bytes anchored by EvaporHeaderInbox
///         - `key`               — 32 bytes Verkle path key
///         - `valueCommitment`   — keccak256(value) (so values >32 bytes
///                                 are still bindable as a single Fr)
///         - `paramsFingerprint` — anchors the IPA Params used during
///                                 proving (prevents cross-circuit replay)
interface IVerkleProofVerifier {
    /// @notice Verify a Verkle-membership proof anchoring (key, value)
    ///         under stateRoot.
    ///
    /// @param stateRoot         the EvaporChain state root the proof binds to.
    /// @param key               32-byte Verkle path key.
    /// @param valueCommitment   keccak256(value), domain-bound by the prover.
    /// @param paramsFingerprint blake3 fingerprint of the Halo2 IPA Params
    ///                          (matches `VerkleProofV2.params_fingerprint_hex`).
    /// @param groth16Proof      256-byte standard Groth16 proof:
    ///                          `abi.encode(uint256[2] a, uint256[2][2] b, uint256[2] c)`.
    ///                          Snarkjs / circom produce this layout.
    /// @return ok               true iff the Groth16 pairing check passes
    ///                          AND all public inputs match the supplied
    ///                          (stateRoot, key, valueCommitment, paramsFingerprint).
    function verifyVerkleMembership(
        bytes32 stateRoot,
        bytes32 key,
        bytes32 valueCommitment,
        bytes32 paramsFingerprint,
        bytes calldata groth16Proof
    ) external view returns (bool ok);
}
