// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

/// @title  MmrInclusion
/// @notice Verifies that a leaf is in a Merkle Mountain Range (MMR)
///         identified by its bagged peak root.
///
///         EvaporChain ghost records (§17.4 of the whitepaper) accumulate
///         into an MMR. Each block header commits the MMR root. To prove
///         an object evaporated, the relayer supplies the leaf hash + the
///         sibling path (peaks + path-up).
///
/// @dev    Hash function is `keccak256` (chosen to keep on-chain
///         verification cheap; the bridge-layer MMR is a parallel
///         construction to whatever the EvaporChain consensus persists,
///         signed by validators alongside the block header).
///
///         Leaf encoding (caller's responsibility):
///             leafHash = keccak256(abi.encode(objectId, evaporatedAtHeight, finalEnergy))
///         The dispatcher calls `verify(leafHash, leafIndex, peakBitmap, path, root)`.
///
///         Format of `path`:
///             concatenation of 32-byte sibling hashes, in order from
///             leaf-level up to the peak that contains the leaf.
///
///         Format of `peaks`:
///             concatenation of 32-byte peak hashes for the OTHER peaks
///             of the bagged MMR (i.e. all peaks except the one containing
///             the leaf), in left-to-right order.
///
///         The bagged root is computed as
///             root = keccak256(peak_n || keccak256(peak_{n-1} || …))
///         right-to-left bagging — the standard Open Timestamps MMR
///         convention.
library MmrInclusion {
    error MalformedPath();
    error PeakIndexOutOfRange();

    /// @notice Verify a leaf's inclusion in an MMR.
    ///
    /// @param leafHash       keccak256 of the leaf payload.
    /// @param leafIndex      zero-based MMR position of the leaf.
    /// @param treeSize       total number of leaves in the MMR (so peak
    ///                       structure is determined).
    /// @param path           concatenated 32-byte sibling hashes.
    /// @param peaksLeft      32-byte peaks to the *left* of the leaf's
    ///                       peak, in MMR order (left → right).
    /// @param peaksRight     32-byte peaks to the *right* of the leaf's
    ///                       peak, in MMR order (left → right).
    /// @param expectedRoot   the bagged-peak root committed in the header.
    function verify(
        bytes32 leafHash,
        uint64 leafIndex,
        uint64 treeSize,
        bytes calldata path,
        bytes calldata peaksLeft,
        bytes calldata peaksRight,
        bytes32 expectedRoot
    ) internal pure returns (bool) {
        if (path.length % 32 != 0) revert MalformedPath();
        if (peaksLeft.length % 32 != 0) revert MalformedPath();
        if (peaksRight.length % 32 != 0) revert MalformedPath();
        if (leafIndex >= treeSize) revert PeakIndexOutOfRange();

        // 1. Walk up the path computing the peak that contains our leaf.
        bytes32 cur = leafHash;
        uint256 idx = uint256(leafIndex);
        uint256 nibbles = path.length / 32;
        for (uint256 i = 0; i < nibbles; i++) {
            bytes32 sibling;
            uint256 off = i * 32;
            // load sibling
            assembly {
                sibling := calldataload(add(path.offset, off))
            }
            // Lower bit of `idx` decides left/right.
            if (idx & 1 == 0) {
                cur = keccak256(abi.encodePacked(cur, sibling));
            } else {
                cur = keccak256(abi.encodePacked(sibling, cur));
            }
            idx >>= 1;
        }
        // `cur` is now the peak hash for the subtree containing the leaf.

        // 2. Bag all peaks (peaksLeft, cur, peaksRight) right-to-left.
        bytes32 acc;
        bool started = false;

        // bag peaksRight in reverse, then cur, then peaksLeft in reverse.
        uint256 rightCount = peaksRight.length / 32;
        for (uint256 i = rightCount; i > 0; i--) {
            bytes32 p;
            uint256 off = (i - 1) * 32;
            assembly {
                p := calldataload(add(peaksRight.offset, off))
            }
            if (!started) {
                acc = p;
                started = true;
            } else {
                acc = keccak256(abi.encodePacked(p, acc));
            }
        }
        if (!started) {
            acc = cur;
            started = true;
        } else {
            acc = keccak256(abi.encodePacked(cur, acc));
        }

        uint256 leftCount = peaksLeft.length / 32;
        for (uint256 i = leftCount; i > 0; i--) {
            bytes32 p;
            uint256 off = (i - 1) * 32;
            assembly {
                p := calldataload(add(peaksLeft.offset, off))
            }
            acc = keccak256(abi.encodePacked(p, acc));
        }

        return acc == expectedRoot;
    }
}
