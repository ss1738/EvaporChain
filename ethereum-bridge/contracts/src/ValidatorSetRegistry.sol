// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {BridgeConstants} from "./BridgeConstants.sol";
import {BridgeTypes} from "./BridgeTypes.sol";
import {ICommitCertVerifier} from "./interfaces/ICommitCertVerifier.sol";

/// @title  ValidatorSetRegistry
/// @notice Receiving-side anchor for EvaporChain's validator set.
///         Stores the current valset commitment and accepts updates that
///         carry a BLS aggregate signature from ≥ 2/3 of the previous set.
///
///         The valset is committed by hashing
///         `keccak256(DOMAIN_TAG_COMMIT || epoch || count || (pubkey_i || stake_i)*)`.
///         Identical computation lives in
///         `crates/evaporchain-eth-bridge/src/valset.rs` and the two are
///         pinned together by `tests/cross_side_agreement.rs`.
///
/// @dev    Phase 1: storage + transition logic + ICommitCertVerifier hook.
///         Phase 2 ships the real EIP-2537 verifier. A mock verifier lives
///         in `test/lib/MockCommitCertVerifier.sol`.
contract ValidatorSetRegistry {
    // Validator type lives in BridgeTypes.sol so the verifier and
    // dispatcher can reference the same struct without circular imports.

    // ─── State ──────────────────────────────────────────────────────

    /// @notice Address allowed to call `genesisInit`. Renounced after init.
    address public owner;

    /// @notice keccak256 commitment to the active validator set.
    bytes32 public valsetRoot;

    /// @notice Epoch number of the active validator set.
    uint64 public epoch;

    /// @notice Total stake of the active set, used by quorum checks.
    uint128 public totalStake;

    /// @notice Verifier for BLS commit-certificates over valset transitions.
    ICommitCertVerifier public verifier;

    // ─── Errors ─────────────────────────────────────────────────────

    error NotOwner();
    error AlreadyInitialised();
    error EmptyValset();
    error TooManyValidators();
    error InvalidPubkeyLength();
    error EpochMustIncrement();
    error VerifierRejected();
    error PrevValsetWitnessMismatch();
    error PubkeyArityMismatch();
    error InsufficientStake(uint128 signed, uint128 required);
    /// L4 (audit 2026-05-13): a valset with a duplicate pubkey would
    /// double-count that signer's stake in `_sumSignedStake`,
    /// dropping the effective 2/3 quorum threshold below the real
    /// network stake-weight. Caught at `_computeRoot` time via an
    /// O(n²) pairwise comparison (acceptable at MAX_VALIDATORS=1024
    /// — even the worst case is ~520K comparisons × 48-byte memcmp,
    /// well under genesis/update block gas limits at realistic sizes
    /// of <128 validators).
    error DuplicatePubkey(uint256 firstIndex, uint256 duplicateIndex);

    // ─── Events ─────────────────────────────────────────────────────

    event GenesisInitialised(bytes32 valsetRoot, uint64 epoch, uint128 totalStake);
    event ValsetUpdated(
        bytes32 prevValsetRoot,
        bytes32 nextValsetRoot,
        uint64 prevEpoch,
        uint64 nextEpoch,
        uint128 nextTotalStake
    );

    // ─── Construction ───────────────────────────────────────────────

    constructor(ICommitCertVerifier _verifier) {
        owner = msg.sender;
        verifier = _verifier;
    }

    // ─── Init ───────────────────────────────────────────────────────

    /// @notice Owner-only. Callable once. Burns owner.
    function genesisInit(uint64 _epoch, BridgeTypes.Validator[] calldata validators) external {
        // AlreadyInitialised first so the second-call error is descriptive
        // even after `owner` has been burned to address(0).
        if (valsetRoot != bytes32(0)) revert AlreadyInitialised();
        if (msg.sender != owner) revert NotOwner();
        if (validators.length == 0) revert EmptyValset();
        if (validators.length > BridgeConstants.MAX_VALIDATORS) revert TooManyValidators();

        (bytes32 root, uint128 total) = _computeRoot(_epoch, validators);

        valsetRoot = root;
        epoch = _epoch;
        totalStake = total;
        owner = address(0);

        emit GenesisInitialised(root, _epoch, total);
    }

    // ─── Update ─────────────────────────────────────────────────────

    /// @notice Apply a valset transition signed by ≥ 2/3 stake of the
    ///         previous set. Anyone can call.
    ///
    /// @param nextEpoch                 must equal epoch + 1.
    /// @param nextValidators            full proposed next set (in canonical order).
    /// @param prevValidators            witness for the active set; recomputed root must match storage.
    /// @param prevPubkeysUncompressed   128-byte EIP-2537 G1 point per prev validator,
    ///                                  same order as prevValidators (consumed by verifier).
    /// @param signedBitmap              bit i = prev validator i signed.
    /// @param aggSignature              BLS-G2 aggregate, 256-byte EIP-2537 uncompressed.
    function updateValset(
        uint64 nextEpoch,
        BridgeTypes.Validator[] calldata nextValidators,
        BridgeTypes.Validator[] calldata prevValidators,
        bytes calldata prevPubkeysUncompressed,
        bytes calldata signedBitmap,
        bytes calldata aggSignature
    ) external {
        if (nextEpoch != epoch + 1) revert EpochMustIncrement();
        if (nextValidators.length == 0) revert EmptyValset();
        if (nextValidators.length > BridgeConstants.MAX_VALIDATORS) revert TooManyValidators();

        // Witness: prev set must hash to the active stored root.
        (bytes32 prevRecomputed,) = _computeRoot(epoch, prevValidators);
        if (prevRecomputed != valsetRoot) revert PrevValsetWitnessMismatch();

        // 128-byte uncompressed G1 per validator; verifier checks the
        // compress(uncompressed) == prevValidators[i].pubkey consistency.
        if (prevPubkeysUncompressed.length != prevValidators.length * 128) {
            revert PubkeyArityMismatch();
        }

        // Quorum: signed-stake * STAKE_DEN  >  totalStake * STAKE_NUM
        uint128 signedStake =
            _sumSignedStake(prevValidators, signedBitmap);
        uint128 required = uint128(
            (uint256(totalStake) * BridgeConstants.STAKE_NUM) / BridgeConstants.STAKE_DEN
        );
        if (uint256(signedStake) * BridgeConstants.STAKE_DEN
            <= uint256(totalStake) * BridgeConstants.STAKE_NUM) {
            revert InsufficientStake(signedStake, required);
        }

        (bytes32 nextRoot, uint128 nextTotal) = _computeRoot(nextEpoch, nextValidators);

        bytes32 messageHash = keccak256(
            abi.encodePacked(
                BridgeConstants.DOMAIN_TAG_COMMIT, epoch, nextRoot, nextEpoch
            )
        );

        bool ok = verifier.verifyCommitCert(
            prevValidators,
            prevPubkeysUncompressed,
            signedBitmap,
            messageHash,
            aggSignature
        );
        if (!ok) revert VerifierRejected();

        bytes32 prevRoot = valsetRoot;
        uint64 prevEpoch = epoch;

        valsetRoot = nextRoot;
        epoch = nextEpoch;
        totalStake = nextTotal;

        emit ValsetUpdated(prevRoot, nextRoot, prevEpoch, nextEpoch, nextTotal);
    }

    /// @dev Sum stakes for validators marked in `signedBitmap`. LSB-first
    ///      (bit `i % 8` of byte `i / 8`); matches how the Rust producer
    ///      packs the bitmap.
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

    // ─── Hashing ────────────────────────────────────────────────────

    /// @notice Compute the valset root.
    /// @dev    Pre-image:
    ///           DOMAIN_TAG_COMMIT (32)
    ///         | epoch            (8 BE)
    ///         | count            (4 BE)
    ///         | (pubkey 48 || stake 16 BE)*  in supplied order
    ///
    ///         Order is significant. The producer (Rust mirror) emits the
    ///         same canonical order (sorted by pubkey big-endian).
    function _computeRoot(uint64 _epoch, BridgeTypes.Validator[] calldata validators)
        internal
        pure
        returns (bytes32 root, uint128 total)
    {
        // Build pre-image into memory. 1024 max validators × 64 bytes/entry ≤ 64 KiB.
        bytes memory buf = abi.encodePacked(
            BridgeConstants.DOMAIN_TAG_COMMIT,
            uint64(_epoch),
            uint32(validators.length)
        );

        // L4 (audit 2026-05-13): hash each pubkey once before the
        // main loop so the duplicate check is O(n²) keccak-compares
        // instead of O(n²) 48-byte memcmps. Keeps gas reasonable
        // at the MAX_VALIDATORS=1024 ceiling.
        bytes32[] memory pkHashes = new bytes32[](validators.length);

        for (uint256 i = 0; i < validators.length; i++) {
            if (validators[i].pubkey.length != 48) revert InvalidPubkeyLength();
            pkHashes[i] = keccak256(validators[i].pubkey);
            // Duplicate check against every earlier entry. The doc
            // comment above asks the producer to ship sorted-by-
            // pubkey order, but we don't enforce that here — the
            // L4 audit-required guarantee is just "no duplicates"
            // (and a producer that emits canonical order
            // automatically satisfies it).
            for (uint256 j = 0; j < i; j++) {
                if (pkHashes[j] == pkHashes[i]) {
                    revert DuplicatePubkey(j, i);
                }
            }
            buf = abi.encodePacked(buf, validators[i].pubkey, uint128(validators[i].stake));
            // Saturate-add not needed: uint128 sum of 1024×uint128 fits in uint256
            // headspace, and we cap the array at MAX_VALIDATORS. We assert no
            // overflow into uint128 by reverting if it happens.
            uint256 newTotal = uint256(total) + uint256(validators[i].stake);
            require(newTotal <= type(uint128).max, "stake overflow");
            total = uint128(newTotal);
        }

        root = keccak256(buf);
    }

    // ─── Views ──────────────────────────────────────────────────────

    /// @notice Convenience: do the supplied validators hash to the active root?
    function isActiveValset(uint64 _epoch, BridgeTypes.Validator[] calldata validators)
        external
        view
        returns (bool)
    {
        if (_epoch != epoch) return false;
        (bytes32 root,) = _computeRoot(_epoch, validators);
        return root == valsetRoot;
    }
}
