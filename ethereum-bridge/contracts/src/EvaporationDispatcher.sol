// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {BridgeConstants} from "./BridgeConstants.sol";
import {EvaporHeaderInbox} from "./EvaporHeaderInbox.sol";
import {MmrInclusion} from "./lib/MmrInclusion.sol";

/// @title  EvaporationDispatcher
/// @notice Fires a user-registered Ethereum action when (and only when)
///         a specific EvaporChain object has *evaporated* — i.e. been
///         demoted to a ghost record committed in the MMR root of a
///         finalised header.
///
///         This is the §17.4 cross-chain primitive of the EvaporChain
///         whitepaper made operational. Nothing on Ethereum has done it
///         before, because no other L1 has thermodynamic state decay.
///
/// @dev    Workflow:
///           1. User calls `registerHook(objectId, target, calldata, gasCap)`.
///           2. EvaporChain object decays past zero and becomes a ghost record.
///           3. The relayer publishes the finalised header to `EvaporHeaderInbox`.
///           4. Anyone calls `dispatch(objectId, height, leaf, leafIndex,
///              treeSize, mmrPath, peaksLeft, peaksRight)`.
///           5. This contract:
///                - Reads `mmrRoot` for `height` from the inbox.
///                - Verifies the leaf hash binds `objectId` to the
///                  ghost-record commitment.
///                - Verifies the leaf is in the MMR via `MmrInclusion.verify`.
///                - Marks the hook fired (one-shot — replay-immune).
///                - Calls `target` with `calldata` under `gasCap`.
contract EvaporationDispatcher {
    /// @notice One registered evaporation hook.
    struct Hook {
        address registrar;   // who set it (refunds on cancel)
        address target;      // contract to call when objectId evaporates
        bytes data;          // calldata to pass
        uint96 gasCap;       // gas given to the target call (bounded)
        bool fired;          // one-shot
    }

    /// @notice The header inbox we trust for finalised (mmrRoot, height) pairs.
    EvaporHeaderInbox public immutable inbox;

    /// @notice objectId → registered hook.
    mapping(bytes32 => Hook) public hooks;

    error HookAlreadyRegistered(bytes32 objectId);
    error HookNotFound(bytes32 objectId);
    error HookAlreadyFired(bytes32 objectId);
    error MmrInclusionFailed();
    error MmrRootMissingForHeight(uint64 height);
    error LeafDoesNotBindObject(bytes32 objectId);
    error TargetCallReverted();
    /// Lane T0.11 — header was accepted by inbox but not yet far enough
    /// behind L1 head to be safe against reorgs that revert the
    /// inbox's storage write.
    error HeaderTooRecent(
        uint64 height,
        uint64 l1AcceptedAt,
        uint64 currentL1Block,
        uint256 minDepth
    );
    /// Lane T0.11 — header was never accepted by the inbox (the
    /// dispatcher refuses to consult `mmrRootAt` because the
    /// `l1AcceptedAt` mapping returns 0 for never-accepted heights).
    /// Distinct from `MmrRootMissingForHeight` so callers can tell
    /// "wrong height" from "not yet finalised".
    error HeaderNotAccepted(uint64 height);

    event HookRegistered(bytes32 indexed objectId, address indexed target, address indexed registrar);
    event HookFired(
        bytes32 indexed objectId,
        uint64 indexed height,
        address indexed target,
        bool targetSuccess
    );
    event HookCancelled(bytes32 indexed objectId);

    constructor(EvaporHeaderInbox _inbox) {
        inbox = _inbox;
    }

    // ─── Registration ───────────────────────────────────────────────

    function registerHook(
        bytes32 objectId,
        address target,
        bytes calldata data,
        uint96 gasCap
    ) external {
        if (hooks[objectId].registrar != address(0)) {
            revert HookAlreadyRegistered(objectId);
        }
        hooks[objectId] = Hook({
            registrar: msg.sender,
            target: target,
            data: data,
            gasCap: gasCap,
            fired: false
        });
        emit HookRegistered(objectId, target, msg.sender);
    }

    function cancelHook(bytes32 objectId) external {
        Hook storage h = hooks[objectId];
        if (h.registrar == address(0)) revert HookNotFound(objectId);
        if (h.fired) revert HookAlreadyFired(objectId);
        require(msg.sender == h.registrar, "only registrar can cancel");
        delete hooks[objectId];
        emit HookCancelled(objectId);
    }

    // ─── Dispatch ───────────────────────────────────────────────────

    /// @notice Verify that `objectId` has evaporated at `height` and fire its hook.
    ///
    /// @param objectId      32-byte EvaporChain object identifier.
    /// @param height        finalised header height where the ghost record committed.
    /// @param evaporatedAtHeight  height EvaporChain marked the object evaporated.
    /// @param finalEnergy   the energy value at evaporation (typically zero).
    /// @param leafIndex     MMR position of the ghost-record leaf.
    /// @param treeSize      total leaves in the MMR at `height`.
    /// @param mmrPath       sibling path (concatenated 32-byte hashes).
    /// @param peaksLeft     other peaks left of the leaf's peak.
    /// @param peaksRight    other peaks right of the leaf's peak.
    function dispatch(
        bytes32 objectId,
        uint64 height,
        uint64 evaporatedAtHeight,
        uint128 finalEnergy,
        uint64 leafIndex,
        uint64 treeSize,
        bytes calldata mmrPath,
        bytes calldata peaksLeft,
        bytes calldata peaksRight
    ) external {
        Hook storage h = hooks[objectId];
        if (h.registrar == address(0)) revert HookNotFound(objectId);
        if (h.fired) revert HookAlreadyFired(objectId);

        // Lane T0.11 — finalization-depth gate. The inbox records the
        // L1 block.number at which `submitHeader` accepted each
        // EvaporChain header. Refuse to dispatch until enough L1 blocks
        // have passed that an L1 reorg reverting the inbox's storage
        // write becomes economically infeasible (default
        // BridgeConstants.MIN_FINALIZATION_DEPTH = 12 blocks ≈ 2.5 min
        // post-merge).
        //
        // The mmrRoot != 0 check below remains the second line of
        // defense — if the inbox returned a zero mmrRoot the header
        // wasn't accepted at all. The l1AcceptedAt check fires first
        // and provides a more specific revert reason.
        uint64 acceptedAt = inbox.l1AcceptedAt(height);
        if (acceptedAt == 0) revert HeaderNotAccepted(height);
        uint256 depth = block.number - uint256(acceptedAt);
        if (depth < BridgeConstants.MIN_FINALIZATION_DEPTH) {
            revert HeaderTooRecent(
                height,
                acceptedAt,
                uint64(block.number),
                BridgeConstants.MIN_FINALIZATION_DEPTH
            );
        }

        bytes32 mmrRoot = inbox.mmrRootAt(height);
        if (mmrRoot == bytes32(0)) revert MmrRootMissingForHeight(height);

        // Leaf binding: leafHash = keccak256(objectId, evaporatedAtHeight, finalEnergy).
        bytes32 leafHash = keccak256(
            abi.encodePacked(objectId, evaporatedAtHeight, finalEnergy)
        );

        bool included = MmrInclusion.verify(
            leafHash,
            leafIndex,
            treeSize,
            mmrPath,
            peaksLeft,
            peaksRight,
            mmrRoot
        );
        if (!included) revert MmrInclusionFailed();

        // Mark fired BEFORE the external call (re-entrancy safe).
        h.fired = true;

        // Make the bounded external call. The hook is "best-effort": we
        // still emit the event and mark fired even if the target reverts,
        // so a buggy target can't trap evaporation events forever.
        (bool ok,) = h.target.call{gas: h.gasCap}(h.data);
        emit HookFired(objectId, height, h.target, ok);
    }

    /// @notice Read-only helper: has the hook for this object fired?
    function isFired(bytes32 objectId) external view returns (bool) {
        return hooks[objectId].fired;
    }
}
