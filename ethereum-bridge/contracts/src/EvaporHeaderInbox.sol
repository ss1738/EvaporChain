// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {BridgeConstants} from "./BridgeConstants.sol";
import {BridgeTypes} from "./BridgeTypes.sol";
import {ICommitCertVerifier} from "./interfaces/ICommitCertVerifier.sol";
import {ValidatorSetRegistry} from "./ValidatorSetRegistry.sol";

/// @title  EvaporHeaderInbox
/// @notice Stores EvaporChain block headers that have been finalised by
///         a BLS commit-certificate from ≥ 2/3 stake of the active
///         validator set. Anyone can submit; verification is on-chain.
///
///         Once accepted, a header tuple `(height, state_root, mmr_root)`
///         is permanently committed and addressable from any other
///         Ethereum contract via [`stateRootAt`] / [`mmrRootAt`].
///         Subsequent phases (Verkle proof verifier, evaporation
///         dispatcher) consume these.
///
/// @dev    Phase 3a deliverable. The relayer service (`ethereum-bridge/relayer/`)
///         is what actually invokes `submitHeader` — for tests we
///         construct headers + signatures by hand.
contract EvaporHeaderInbox {
    /// @notice Commitment to a finalised EvaporChain block.
    struct Header {
        uint64 height;
        bytes32 blockHash;
        bytes32 stateRoot;
        bytes32 mmrRoot;
        uint64 evaporchainEpoch; // valset epoch at finalisation time
    }

    // ─── State ──────────────────────────────────────────────────────

    /// @notice The validator-set registry whose verifier validates inbound headers.
    ValidatorSetRegistry public immutable registry;

    /// @notice Highest height accepted so far. Headers must arrive in order
    ///         to keep the storage layout simple. Re-org tolerance lives at
    ///         a higher layer (relayer reorg handling).
    uint64 public latestHeight;

    /// @notice Sparse mapping height → committed header.
    mapping(uint64 => Header) internal _headers;

    // ─── Errors ─────────────────────────────────────────────────────

    error HeightNotMonotonic(uint64 supplied, uint64 latest);
    error PrevWitnessMismatch();
    error PubkeyArityMismatch();
    error InsufficientStake();
    error VerifierRejected();

    // ─── Events ─────────────────────────────────────────────────────

    event HeaderAccepted(
        uint64 indexed height,
        bytes32 blockHash,
        bytes32 stateRoot,
        bytes32 mmrRoot,
        uint64 evaporchainEpoch
    );

    // ─── Construction ───────────────────────────────────────────────

    constructor(ValidatorSetRegistry _registry) {
        registry = _registry;
    }

    // ─── Submission ─────────────────────────────────────────────────

    /// @notice Submit a finalised EvaporChain header.
    ///
    /// @param header                  the (height, blockHash, stateRoot, mmrRoot, epoch) tuple.
    /// @param prevValidators          the active validator set at `header.evaporchainEpoch`.
    /// @param prevPubkeysUncompressed concat of EIP-2537 G1 points (128 B each), same order.
    /// @param signedBitmap            LSB-first bitmap; bit `i` = validator `i` signed.
    /// @param aggSignature            BLS-G2 aggregate, EIP-2537 uncompressed (256 B).
    function submitHeader(
        Header calldata header,
        BridgeTypes.Validator[] calldata prevValidators,
        bytes calldata prevPubkeysUncompressed,
        bytes calldata signedBitmap,
        bytes calldata aggSignature
    ) external {
        if (header.height <= latestHeight) {
            revert HeightNotMonotonic(header.height, latestHeight);
        }

        // Witness: registry must agree on the active valset.
        if (!registry.isActiveValset(header.evaporchainEpoch, prevValidators)) {
            revert PrevWitnessMismatch();
        }

        if (prevPubkeysUncompressed.length != prevValidators.length * 128) {
            revert PubkeyArityMismatch();
        }

        // Quorum check.
        uint128 signedStake = _sumSignedStake(prevValidators, signedBitmap);
        uint128 totalStake = registry.totalStake();
        if (uint256(signedStake) * BridgeConstants.STAKE_DEN
            <= uint256(totalStake) * BridgeConstants.STAKE_NUM) {
            revert InsufficientStake();
        }

        // Build the header message hash.
        bytes32 messageHash = keccak256(
            abi.encodePacked(
                BridgeConstants.DOMAIN_TAG_HEADER,
                header.height,
                header.blockHash,
                header.stateRoot,
                header.mmrRoot,
                header.evaporchainEpoch
            )
        );

        // Verify via the same EIP-2537 verifier that gates valset transitions.
        ICommitCertVerifier verifier = registry.verifier();
        bool ok = verifier.verifyCommitCert(
            prevValidators,
            prevPubkeysUncompressed,
            signedBitmap,
            messageHash,
            aggSignature
        );
        if (!ok) revert VerifierRejected();

        // Commit.
        _headers[header.height] = header;
        latestHeight = header.height;

        emit HeaderAccepted(
            header.height,
            header.blockHash,
            header.stateRoot,
            header.mmrRoot,
            header.evaporchainEpoch
        );
    }

    // ─── Reads ──────────────────────────────────────────────────────

    function headerAt(uint64 height) external view returns (Header memory) {
        return _headers[height];
    }

    function stateRootAt(uint64 height) external view returns (bytes32) {
        return _headers[height].stateRoot;
    }

    function mmrRootAt(uint64 height) external view returns (bytes32) {
        return _headers[height].mmrRoot;
    }

    function blockHashAt(uint64 height) external view returns (bytes32) {
        return _headers[height].blockHash;
    }

    // ─── Internals ──────────────────────────────────────────────────

    function _sumSignedStake(
        BridgeTypes.Validator[] calldata validators,
        bytes calldata signedBitmap
    ) internal pure returns (uint128 sum) {
        uint256 n = validators.length;
        for (uint256 i = 0; i < n; i++) {
            uint256 byteIdx = i / 8;
            uint256 bitIdx = i % 8;
            if (byteIdx >= signedBitmap.length) break;
            if ((uint8(signedBitmap[byteIdx]) >> bitIdx) & 1 == 1) {
                sum += validators[i].stake;
            }
        }
    }
}
