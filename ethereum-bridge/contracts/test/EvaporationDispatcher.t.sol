// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";

import {BridgeConstants} from "../src/BridgeConstants.sol";
import {BridgeTypes} from "../src/BridgeTypes.sol";
import {CommitCertVerifier} from "../src/CommitCertVerifier.sol";
import {EvaporationDispatcher} from "../src/EvaporationDispatcher.sol";
import {EvaporHeaderInbox} from "../src/EvaporHeaderInbox.sol";
import {ICommitCertVerifier} from "../src/interfaces/ICommitCertVerifier.sol";
import {ValidatorSetRegistry} from "../src/ValidatorSetRegistry.sol";

/// @notice Sample target contract a user might register an evaporation
///         hook for. Mints a "ghost token" (counter increment) when its
///         twin object on EvaporChain evaporates.
contract GhostTokenMinter {
    uint256 public minted;
    bytes public lastData;

    function mintBecauseEvaporated(bytes calldata data) external {
        minted += 1;
        lastData = data;
    }
}

/// @notice Phase 5 end-to-end. The whole pipeline from
///         a real BLS-signed header → MMR-anchored ghost record → fired
///         user hook on Ethereum.
///
///         If this passes, an EvaporChain object decay event has, for
///         the first time, triggered an Ethereum action with no trusted
///         relayer in between.
contract EvaporationDispatcherTest is Test {
    using stdJson for string;

    ValidatorSetRegistry registry;
    CommitCertVerifier verifier;
    EvaporHeaderInbox inbox;
    EvaporationDispatcher dispatcher;
    GhostTokenMinter target;

    string fixture;

    function setUp() public {
        verifier = new CommitCertVerifier();
        registry = new ValidatorSetRegistry(ICommitCertVerifier(address(verifier)));
        inbox = new EvaporHeaderInbox(registry);
        dispatcher = new EvaporationDispatcher(inbox);
        target = new GhostTokenMinter();

        fixture = vm.readFile("./fixtures/evaporation_dispatch_8.json");

        // Seed registry with the validator set the fixture signed under.
        BridgeTypes.Validator[] memory vs = _readValidators();
        uint64 epoch = uint64(vm.parseJsonUint(fixture, ".epoch"));
        registry.genesisInit(epoch, vs);

        // Submit the finalised header committing the MMR root.
        EvaporHeaderInbox.Header memory header = _readHeader();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory aggSig = vm.parseJsonBytes(fixture, ".agg_signature_uncompressed");
        inbox.submitHeader(header, vs, pks, bitmap, aggSig);

        // Lane T0.11 — advance L1 block.number past the finalization
        // depth so subsequent `dispatch` calls in tests don't trip the
        // HeaderTooRecent gate. Real callers wait for confirmations on
        // the live chain; in forge we fast-forward.
        vm.roll(block.number + BridgeConstants.MIN_FINALIZATION_DEPTH);
    }

    function _readValidator(uint256 i) internal view returns (BridgeTypes.Validator memory) {
        string memory key = string.concat(".validators[", vm.toString(i), "]");
        bytes memory pkRaw = vm.parseJsonBytes(fixture, string.concat(key, ".pubkey_compressed"));
        uint256 stake = vm.parseJsonUint(fixture, string.concat(key, ".stake"));
        return BridgeTypes.Validator({pubkey: pkRaw, stake: uint128(stake)});
    }

    function _readValidators() internal view returns (BridgeTypes.Validator[] memory) {
        BridgeTypes.Validator[] memory out = new BridgeTypes.Validator[](5);
        for (uint256 i = 0; i < 5; i++) out[i] = _readValidator(i);
        return out;
    }

    function _readHeader() internal view returns (EvaporHeaderInbox.Header memory) {
        return EvaporHeaderInbox.Header({
            height: uint64(vm.parseJsonUint(fixture, ".height")),
            blockHash: vm.parseJsonBytes32(fixture, ".block_hash"),
            stateRoot: vm.parseJsonBytes32(fixture, ".state_root"),
            mmrRoot: vm.parseJsonBytes32(fixture, ".mmr_root"),
            evaporchainEpoch: uint64(vm.parseJsonUint(fixture, ".epoch"))
        });
    }

    function test_evaporationFiresHook() public {
        bytes32 objectId = vm.parseJsonBytes32(fixture, ".object_id");
        uint64 evaporatedAtHeight =
            uint64(vm.parseJsonUint(fixture, ".target_evaporated_at_height"));
        uint128 finalEnergy =
            uint128(vm.parseJsonUint(fixture, ".target_final_energy"));
        uint64 leafIndex = uint64(vm.parseJsonUint(fixture, ".leaf_index"));
        uint64 treeSize = uint64(vm.parseJsonUint(fixture, ".tree_size"));
        bytes memory mmrPath = vm.parseJsonBytes(fixture, ".mmr_path");
        bytes memory peaksLeft = vm.parseJsonBytes(fixture, ".peaks_left");
        bytes memory peaksRight = vm.parseJsonBytes(fixture, ".peaks_right");
        uint64 height = uint64(vm.parseJsonUint(fixture, ".height"));

        // 1. Register the hook.
        bytes memory data = abi.encodeWithSelector(
            GhostTokenMinter.mintBecauseEvaporated.selector,
            bytes("evaporated-on-evaporchain")
        );
        dispatcher.registerHook(objectId, address(target), data, 200_000);

        // 2. Sanity: hook is registered, not fired.
        assertFalse(dispatcher.isFired(objectId));
        assertEq(target.minted(), 0);

        // 3. Dispatch.
        dispatcher.dispatch(
            objectId,
            height,
            evaporatedAtHeight,
            finalEnergy,
            leafIndex,
            treeSize,
            mmrPath,
            peaksLeft,
            peaksRight
        );

        // 4. The target was called exactly once.
        assertTrue(dispatcher.isFired(objectId));
        assertEq(target.minted(), 1);
        assertEq(target.lastData(), bytes("evaporated-on-evaporchain"));
    }

    function test_dispatch_revertsOnSecondCall() public {
        bytes32 objectId = vm.parseJsonBytes32(fixture, ".object_id");
        uint64 evaporatedAtHeight =
            uint64(vm.parseJsonUint(fixture, ".target_evaporated_at_height"));
        uint128 finalEnergy =
            uint128(vm.parseJsonUint(fixture, ".target_final_energy"));
        uint64 leafIndex = uint64(vm.parseJsonUint(fixture, ".leaf_index"));
        uint64 treeSize = uint64(vm.parseJsonUint(fixture, ".tree_size"));
        bytes memory mmrPath = vm.parseJsonBytes(fixture, ".mmr_path");
        bytes memory peaksLeft = vm.parseJsonBytes(fixture, ".peaks_left");
        bytes memory peaksRight = vm.parseJsonBytes(fixture, ".peaks_right");
        uint64 height = uint64(vm.parseJsonUint(fixture, ".height"));

        dispatcher.registerHook(objectId, address(target), abi.encode(), 50_000);
        dispatcher.dispatch(
            objectId, height, evaporatedAtHeight, finalEnergy,
            leafIndex, treeSize, mmrPath, peaksLeft, peaksRight
        );
        // Second dispatch on the same object → rejected.
        vm.expectRevert(
            abi.encodeWithSelector(EvaporationDispatcher.HookAlreadyFired.selector, objectId)
        );
        dispatcher.dispatch(
            objectId, height, evaporatedAtHeight, finalEnergy,
            leafIndex, treeSize, mmrPath, peaksLeft, peaksRight
        );
    }

    function test_dispatch_rejectsBadInclusionProof() public {
        bytes32 objectId = vm.parseJsonBytes32(fixture, ".object_id");
        uint64 evaporatedAtHeight =
            uint64(vm.parseJsonUint(fixture, ".target_evaporated_at_height"));
        uint128 finalEnergy =
            uint128(vm.parseJsonUint(fixture, ".target_final_energy"));
        uint64 leafIndex = uint64(vm.parseJsonUint(fixture, ".leaf_index"));
        uint64 treeSize = uint64(vm.parseJsonUint(fixture, ".tree_size"));
        bytes memory mmrPath = vm.parseJsonBytes(fixture, ".mmr_path");
        bytes memory peaksLeft = vm.parseJsonBytes(fixture, ".peaks_left");
        bytes memory peaksRight = vm.parseJsonBytes(fixture, ".peaks_right");
        uint64 height = uint64(vm.parseJsonUint(fixture, ".height"));

        // Flip a byte in the path → MMR walk produces wrong root.
        if (mmrPath.length > 0) {
            mmrPath[0] = bytes1(uint8(mmrPath[0]) ^ 0x01);
        }

        dispatcher.registerHook(objectId, address(target), abi.encode(), 50_000);
        vm.expectRevert(EvaporationDispatcher.MmrInclusionFailed.selector);
        dispatcher.dispatch(
            objectId, height, evaporatedAtHeight, finalEnergy,
            leafIndex, treeSize, mmrPath, peaksLeft, peaksRight
        );
    }

    function test_dispatch_rejectsUnknownObject() public {
        bytes32 unknownObject = bytes32(uint256(0xDEADBEEF));
        bytes memory empty = new bytes(0);
        vm.expectRevert(
            abi.encodeWithSelector(EvaporationDispatcher.HookNotFound.selector, unknownObject)
        );
        dispatcher.dispatch(unknownObject, 9_000, 8_900, 0, 0, 1, empty, empty, empty);
    }

    function test_dispatch_rejectsHeightWithoutHeader() public {
        bytes32 objectId = vm.parseJsonBytes32(fixture, ".object_id");
        dispatcher.registerHook(objectId, address(target), abi.encode(), 50_000);
        bytes memory empty = new bytes(0);
        // Lane T0.11 — the new HeaderNotAccepted check fires first
        // because l1AcceptedAt(99_999) returns 0 (never accepted).
        // Distinct from MmrRootMissingForHeight so callers can tell
        // "wrong height" from "not yet finalised".
        vm.expectRevert(
            abi.encodeWithSelector(
                EvaporationDispatcher.HeaderNotAccepted.selector, uint64(99_999)
            )
        );
        dispatcher.dispatch(objectId, 99_999, 0, 0, 0, 1, empty, empty, empty);
    }

    // ─── Lane T0.11 — finalization-depth gate tests ──────────────────

    /// Calling `dispatch` BEFORE the L1 finalization depth has elapsed
    /// must revert with HeaderTooRecent. This is the protection
    /// against L1 reorgs that revert `submitHeader`'s storage write
    /// between acceptance and consumption — without it, a dispatch
    /// could fire on a header that is later orphaned.
    function test_dispatch_revertsBeforeFinalizationDepth() public {
        // Re-deploy a fresh inbox/dispatcher pair so we control the
        // L1 block.number relationship from scratch (the shared setUp
        // already advanced past finalization depth).
        EvaporHeaderInbox freshInbox = new EvaporHeaderInbox(registry);
        EvaporationDispatcher freshDispatcher =
            new EvaporationDispatcher(freshInbox);

        EvaporHeaderInbox.Header memory header = _readHeader();
        BridgeTypes.Validator[] memory vs = _readValidators();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory aggSig = vm.parseJsonBytes(fixture, ".agg_signature_uncompressed");
        freshInbox.submitHeader(header, vs, pks, bitmap, aggSig);

        // We're at exactly the L1 block where submitHeader landed —
        // depth = 0 < MIN_FINALIZATION_DEPTH = 12. dispatch must revert.

        bytes32 objectId = vm.parseJsonBytes32(fixture, ".object_id");
        freshDispatcher.registerHook(objectId, address(target), abi.encode(), 50_000);
        uint64 leafIndex = uint64(vm.parseJsonUint(fixture, ".leaf_index"));
        uint64 treeSize = uint64(vm.parseJsonUint(fixture, ".tree_size"));
        bytes memory mmrPath = vm.parseJsonBytes(fixture, ".mmr_path");
        bytes memory peaksLeft = vm.parseJsonBytes(fixture, ".peaks_left");
        bytes memory peaksRight = vm.parseJsonBytes(fixture, ".peaks_right");
        uint64 evaporatedAtHeight =
            uint64(vm.parseJsonUint(fixture, ".target_evaporated_at_height"));
        uint128 finalEnergy =
            uint128(vm.parseJsonUint(fixture, ".target_final_energy"));

        vm.expectRevert(
            abi.encodeWithSelector(
                EvaporationDispatcher.HeaderTooRecent.selector,
                header.height,
                uint64(block.number),                    // l1AcceptedAt
                uint64(block.number),                    // currentL1Block
                BridgeConstants.MIN_FINALIZATION_DEPTH
            )
        );
        freshDispatcher.dispatch(
            objectId,
            header.height,
            evaporatedAtHeight,
            finalEnergy,
            leafIndex,
            treeSize,
            mmrPath,
            peaksLeft,
            peaksRight
        );
    }

    /// At depth = MIN_FINALIZATION_DEPTH - 1 still revert (boundary
    /// check: the constant is "minimum depth required", strictly less
    /// is insufficient).
    function test_dispatch_revertsAtDepthMinus1() public {
        EvaporHeaderInbox freshInbox = new EvaporHeaderInbox(registry);
        EvaporationDispatcher freshDispatcher =
            new EvaporationDispatcher(freshInbox);

        EvaporHeaderInbox.Header memory header = _readHeader();
        BridgeTypes.Validator[] memory vs = _readValidators();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory aggSig = vm.parseJsonBytes(fixture, ".agg_signature_uncompressed");
        freshInbox.submitHeader(header, vs, pks, bitmap, aggSig);

        // Advance L1 to MIN_FINALIZATION_DEPTH - 1 blocks past
        // submission. The check uses strict `<`, so this must still
        // revert.
        vm.roll(block.number + BridgeConstants.MIN_FINALIZATION_DEPTH - 1);

        bytes32 objectId = vm.parseJsonBytes32(fixture, ".object_id");
        freshDispatcher.registerHook(objectId, address(target), abi.encode(), 50_000);
        bytes memory empty = new bytes(0);

        vm.expectRevert(); // HeaderTooRecent — args computed at runtime
        freshDispatcher.dispatch(
            objectId, header.height, 0, 0, 0, 1, empty, empty, empty
        );
    }

    /// Crossing the boundary: at depth = MIN_FINALIZATION_DEPTH the
    /// check passes (uses `<`, not `<=`). Use the real fixture so the
    /// MMR proof verifies and we observe the hook actually fires.
    function test_dispatch_succeedsAtExactFinalizationDepth() public {
        EvaporHeaderInbox freshInbox = new EvaporHeaderInbox(registry);
        EvaporationDispatcher freshDispatcher =
            new EvaporationDispatcher(freshInbox);

        EvaporHeaderInbox.Header memory header = _readHeader();
        BridgeTypes.Validator[] memory vs = _readValidators();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory aggSig = vm.parseJsonBytes(fixture, ".agg_signature_uncompressed");
        freshInbox.submitHeader(header, vs, pks, bitmap, aggSig);

        // Roll exactly MIN_FINALIZATION_DEPTH blocks. depth == MIN.
        vm.roll(block.number + BridgeConstants.MIN_FINALIZATION_DEPTH);

        bytes32 objectId = vm.parseJsonBytes32(fixture, ".object_id");
        uint64 evaporatedAtHeight =
            uint64(vm.parseJsonUint(fixture, ".target_evaporated_at_height"));
        uint128 finalEnergy =
            uint128(vm.parseJsonUint(fixture, ".target_final_energy"));
        uint64 leafIndex = uint64(vm.parseJsonUint(fixture, ".leaf_index"));
        uint64 treeSize = uint64(vm.parseJsonUint(fixture, ".tree_size"));
        bytes memory mmrPath = vm.parseJsonBytes(fixture, ".mmr_path");
        bytes memory peaksLeft = vm.parseJsonBytes(fixture, ".peaks_left");
        bytes memory peaksRight = vm.parseJsonBytes(fixture, ".peaks_right");

        bytes memory data = abi.encodeWithSelector(
            GhostTokenMinter.mintBecauseEvaporated.selector,
            bytes("at-exact-depth")
        );
        freshDispatcher.registerHook(objectId, address(target), data, 200_000);

        // Should succeed — the boundary case.
        freshDispatcher.dispatch(
            objectId,
            header.height,
            evaporatedAtHeight,
            finalEnergy,
            leafIndex,
            treeSize,
            mmrPath,
            peaksLeft,
            peaksRight
        );

        assertTrue(freshDispatcher.isFired(objectId));
    }

    // ─── T0.11 — Cross-chain replay protection regression ──────────────
    //
    // The dispatcher already ships defensive code for replay (one-shot
    // `fired` flag) and reorgs (`MIN_FINALIZATION_DEPTH = 12` gate via
    // `l1AcceptedAt`). These tests cover the lifecycle edges:
    // cancel-before-fire, cancel-after-fire, registrar-only cancel,
    // and the re-register-after-fire replay attempt.

    /// T0.11 — cancelHook by registrar pre-fire deletes the hook.
    /// After delete the same objectId can be re-registered by anyone.
    function test_t0_11_cancel_before_fire_allows_reregister() public {
        bytes32 objectId = vm.parseJsonBytes32(fixture, ".object_id");
        bytes memory data = abi.encodeWithSelector(
            GhostTokenMinter.mintBecauseEvaporated.selector,
            bytes("v1")
        );
        // Registrar is address(this) (the test contract).
        dispatcher.registerHook(objectId, address(target), data, 100_000);
        assertFalse(dispatcher.isFired(objectId));

        dispatcher.cancelHook(objectId);
        // After cancel the hook entry is deleted — re-registration must
        // succeed because the registrar field is back to zero.
        bytes memory data2 = abi.encodeWithSelector(
            GhostTokenMinter.mintBecauseEvaporated.selector,
            bytes("v2")
        );
        dispatcher.registerHook(objectId, address(target), data2, 200_000);
        assertFalse(dispatcher.isFired(objectId));
    }

    /// T0.11 — cancelHook by non-registrar reverts. A third party
    /// cannot dump someone else's hook.
    function test_t0_11_only_registrar_can_cancel() public {
        bytes32 objectId = vm.parseJsonBytes32(fixture, ".object_id");
        bytes memory data = abi.encodeWithSelector(
            GhostTokenMinter.mintBecauseEvaporated.selector,
            bytes("v1")
        );
        dispatcher.registerHook(objectId, address(target), data, 100_000);

        // Pretend a different account tries to cancel.
        address attacker = address(0xDEAD);
        vm.prank(attacker);
        vm.expectRevert("only registrar can cancel");
        dispatcher.cancelHook(objectId);

        // Hook still alive.
        assertFalse(dispatcher.isFired(objectId));
    }

    /// T0.11 — cancelHook after fire reverts. The fired flag pins the
    /// hook entry forever — no way to clear and re-use the slot for
    /// the same objectId after it has triggered an Ethereum action.
    function test_t0_11_cancel_after_fire_reverts() public {
        bytes32 objectId = vm.parseJsonBytes32(fixture, ".object_id");
        uint64 evaporatedAtHeight =
            uint64(vm.parseJsonUint(fixture, ".target_evaporated_at_height"));
        uint128 finalEnergy =
            uint128(vm.parseJsonUint(fixture, ".target_final_energy"));
        uint64 leafIndex = uint64(vm.parseJsonUint(fixture, ".leaf_index"));
        uint64 treeSize = uint64(vm.parseJsonUint(fixture, ".tree_size"));
        bytes memory mmrPath = vm.parseJsonBytes(fixture, ".mmr_path");
        bytes memory peaksLeft = vm.parseJsonBytes(fixture, ".peaks_left");
        bytes memory peaksRight = vm.parseJsonBytes(fixture, ".peaks_right");
        uint64 height = uint64(vm.parseJsonUint(fixture, ".height"));

        bytes memory data = abi.encodeWithSelector(
            GhostTokenMinter.mintBecauseEvaporated.selector,
            bytes("evaporated")
        );
        dispatcher.registerHook(objectId, address(target), data, 200_000);

        // Fire it.
        dispatcher.dispatch(
            objectId,
            height,
            evaporatedAtHeight,
            finalEnergy,
            leafIndex,
            treeSize,
            mmrPath,
            peaksLeft,
            peaksRight
        );
        assertTrue(dispatcher.isFired(objectId));

        // Cancel-after-fire must revert.
        vm.expectRevert(
            abi.encodeWithSelector(EvaporationDispatcher.HookAlreadyFired.selector, objectId)
        );
        dispatcher.cancelHook(objectId);
    }

    /// T0.11 — replay via re-register attempt after fire is blocked
    /// by HookAlreadyRegistered. Once fired, the hook entry persists
    /// with `fired = true`; any attempt to re-register the SAME
    /// objectId hits the `registrar != address(0)` gate first.
    /// Closes the attack vector: "fire hook → re-register → fire
    /// again at a later height with a different MMR proof."
    function test_t0_11_replay_via_reregister_after_fire_blocked() public {
        bytes32 objectId = vm.parseJsonBytes32(fixture, ".object_id");
        uint64 evaporatedAtHeight =
            uint64(vm.parseJsonUint(fixture, ".target_evaporated_at_height"));
        uint128 finalEnergy =
            uint128(vm.parseJsonUint(fixture, ".target_final_energy"));
        uint64 leafIndex = uint64(vm.parseJsonUint(fixture, ".leaf_index"));
        uint64 treeSize = uint64(vm.parseJsonUint(fixture, ".tree_size"));
        bytes memory mmrPath = vm.parseJsonBytes(fixture, ".mmr_path");
        bytes memory peaksLeft = vm.parseJsonBytes(fixture, ".peaks_left");
        bytes memory peaksRight = vm.parseJsonBytes(fixture, ".peaks_right");
        uint64 height = uint64(vm.parseJsonUint(fixture, ".height"));

        bytes memory data = abi.encodeWithSelector(
            GhostTokenMinter.mintBecauseEvaporated.selector,
            bytes("v1")
        );
        dispatcher.registerHook(objectId, address(target), data, 200_000);
        dispatcher.dispatch(
            objectId,
            height,
            evaporatedAtHeight,
            finalEnergy,
            leafIndex,
            treeSize,
            mmrPath,
            peaksLeft,
            peaksRight
        );
        assertTrue(dispatcher.isFired(objectId));

        // Attacker tries to re-register the same objectId after fire,
        // hoping to swap target / data and re-fire under a future
        // header. The HookAlreadyRegistered gate (registrar != 0)
        // blocks this regardless of the fired flag.
        vm.expectRevert(
            abi.encodeWithSelector(
                EvaporationDispatcher.HookAlreadyRegistered.selector,
                objectId
            )
        );
        dispatcher.registerHook(objectId, address(target), data, 200_000);
    }
}
