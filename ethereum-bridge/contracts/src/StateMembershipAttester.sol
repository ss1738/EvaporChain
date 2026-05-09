// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {BridgeConstants} from "./BridgeConstants.sol";
import {BridgeTypes} from "./BridgeTypes.sol";
import {EvaporHeaderInbox} from "./EvaporHeaderInbox.sol";
import {ICommitCertVerifier} from "./interfaces/ICommitCertVerifier.sol";
import {ValidatorSetRegistry} from "./ValidatorSetRegistry.sol";

/// @title  StateMembershipAttester
/// @notice Phase 4 MVP. Verifies that a `(key, value)` is committed in
///         the EvaporChain state at a given height, via a BLS-aggregate
///         attestation by ≥ 2/3 stake of the active validator set.
///
///         This is the **documented fallback** of the Verkle-membership
///         path (Phase 4 full): instead of verifying a Pallas-IPA proof
///         compressed through a Halo2 → Groth16 wrap, validators sign
///         the membership claim directly. Trades off "any one signer
///         could lie" for "≤ 1/3 stake collusion is the threat model"
///         — same threat model as the rest of the bridge.
///
/// @dev    Validators sign:
///           keccak256(
///             DOMAIN_TAG_STATE_MEMBERSHIP
///             || height (8 BE)
///             || key (32)
///             || valueHash (32)            // = keccak256(value)
///           )
///
///         The contract additionally checks that a finalised header
///         exists at `height` (so an attestation cannot be valid before
///         the chain has visibly committed the state root).
contract StateMembershipAttester {
    EvaporHeaderInbox public immutable inbox;
    ValidatorSetRegistry public immutable registry;

    error HeaderMissingForHeight(uint64 height);
    error PrevWitnessMismatch();
    error PubkeyArityMismatch();
    error InsufficientStake();
    error VerifierRejected();
    /// Lane T0.11b — header was accepted by inbox but not yet far
    /// enough behind L1 head to be safe against reorgs that revert
    /// the inbox's storage write between acceptance and consumption.
    /// Same gate the EvaporationDispatcher uses (lane T0.11).
    error HeaderTooRecent(
        uint64 height,
        uint64 l1AcceptedAt,
        uint64 currentL1Block,
        uint256 minDepth
    );

    event StateMembershipVerified(
        uint64 indexed height,
        bytes32 indexed key,
        bytes32 valueHash,
        uint64 indexed evaporchainEpoch
    );

    constructor(EvaporHeaderInbox _inbox) {
        inbox = _inbox;
        registry = _inbox.registry();
    }

    /// @notice Verify that `value` is the value at `key` at `height`.
    ///
    /// @return ok true iff the BLS aggregate attestation verifies.
    function verifyStateMembership(
        uint64 height,
        bytes32 key,
        bytes calldata value,
        BridgeTypes.Validator[] calldata prevValidators,
        bytes calldata prevPubkeysUncompressed,
        bytes calldata signedBitmap,
        bytes calldata aggSignature
    ) external returns (bool ok) {
        // 1. Header at this height must exist (so the chain finalised the state).
        EvaporHeaderInbox.Header memory h = inbox.headerAt(height);
        if (h.height != height) revert HeaderMissingForHeight(height);

        // 1b. Lane T0.11b — finalization-depth gate. The inbox records
        //     the L1 block.number at which `submitHeader` accepted each
        //     EvaporChain header. Refuse to attest until enough L1
        //     blocks have passed that an L1 reorg reverting the
        //     inbox's storage write becomes economically infeasible
        //     (BridgeConstants.MIN_FINALIZATION_DEPTH = 12 blocks ≈
        //     2.5 min post-merge). Same gate as the dispatcher (T0.11).
        //     Header existence is checked above; here we check depth.
        uint64 acceptedAt = inbox.l1AcceptedAt(height);
        uint256 depth = block.number - uint256(acceptedAt);
        if (depth < BridgeConstants.MIN_FINALIZATION_DEPTH) {
            revert HeaderTooRecent(
                height,
                acceptedAt,
                uint64(block.number),
                BridgeConstants.MIN_FINALIZATION_DEPTH
            );
        }

        // 2. Witness check: prevValidators must hash to the registry's
        //    valsetRoot at the header's epoch. (Re-uses the registry's
        //    canonical commitment so we never reimplement valset hashing.)
        if (!registry.isActiveValset(h.evaporchainEpoch, prevValidators)) {
            revert PrevWitnessMismatch();
        }

        // 3. Arity check on the supplied uncompressed pubkeys.
        if (prevPubkeysUncompressed.length != prevValidators.length * 128) {
            revert PubkeyArityMismatch();
        }

        // 4. Quorum check: signed-stake * STAKE_DEN > total * STAKE_NUM.
        uint128 signedStake = _sumSignedStake(prevValidators, signedBitmap);
        uint128 totalStake = registry.totalStake();
        if (uint256(signedStake) * BridgeConstants.STAKE_DEN
            <= uint256(totalStake) * BridgeConstants.STAKE_NUM) {
            revert InsufficientStake();
        }

        // 5. Build the message hash validators signed.
        bytes32 valueHash = keccak256(value);
        bytes32 messageHash = keccak256(
            abi.encodePacked(
                BridgeConstants.DOMAIN_TAG_STATE_MEMBERSHIP,
                height,
                key,
                valueHash
            )
        );

        // 6. Verify via the same EIP-2537 verifier the inbox uses.
        ICommitCertVerifier verifier = registry.verifier();
        ok = verifier.verifyCommitCert(
            prevValidators,
            prevPubkeysUncompressed,
            signedBitmap,
            messageHash,
            aggSignature
        );
        if (!ok) revert VerifierRejected();

        emit StateMembershipVerified(height, key, valueHash, h.evaporchainEpoch);
    }

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
