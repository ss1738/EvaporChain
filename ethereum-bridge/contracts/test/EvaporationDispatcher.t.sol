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

    // ─── Lane T0.11 sub-A — helpers (avoid stack-too-deep) ───────────

    struct DispatchArgs {
        bytes32 objectId;
        uint64 height;
        uint64 evaporatedAtHeight;
        uint128 finalEnergy;
        uint64 leafIndex;
        uint64 treeSize;
        bytes mmrPath;
        bytes peaksLeft;
        bytes peaksRight;
    }

    function _readDispatchArgs() internal view returns (DispatchArgs memory) {
        return DispatchArgs({
            objectId: vm.parseJsonBytes32(fixture, ".object_id"),
            height: uint64(vm.parseJsonUint(fixture, ".height")),
            evaporatedAtHeight:
                uint64(vm.parseJsonUint(fixture, ".target_evaporated_at_height")),
            finalEnergy:
                uint128(vm.parseJsonUint(fixture, ".target_final_energy")),
            leafIndex: uint64(vm.parseJsonUint(fixture, ".leaf_index")),
            treeSize: uint64(vm.parseJsonUint(fixture, ".tree_size")),
            mmrPath: vm.parseJsonBytes(fixture, ".mmr_path"),
            peaksLeft: vm.parseJsonBytes(fixture, ".peaks_left"),
            peaksRight: vm.parseJsonBytes(fixture, ".peaks_right")
        });
    }

    function _doDispatch(
        EvaporationDispatcher d,
        DispatchArgs memory a
    ) internal {
        d.dispatch(
            a.objectId, a.height, a.evaporatedAtHeight, a.finalEnergy,
            a.leafIndex, a.treeSize, a.mmrPath, a.peaksLeft, a.peaksRight
        );
    }

    // ─── Lane T0.11 sub-A — reorg + replay scenario coverage ─────────
    //
    // Acceptance from MAINNET_READINESS.md T0.11:
    //   "forge tests exercise reorg scenarios; dispatcher rejects
    //    replay."
    //
    // The contract-side T0.11 work (l1AcceptedAt, MIN_FINALIZATION_DEPTH
    // gate, HeaderTooRecent + HeaderNotAccepted reverts) has been in
    // place since the depth-gate tests above. This sub-A bundle pins
    // five additional scenarios:
    //
    //   1. Leaf-binding integrity — mutating any of objectId,
    //      evaporatedAtHeight, finalEnergy in the dispatch args
    //      changes leafHash and the MMR walk fails closed.
    //   2. Cancel-then-rebind succeeds — cancelling an unfired hook
    //      frees its slot for fresh registration with new params.
    //   3. Fired slot is sticky — a fired hook CANNOT be re-registered
    //      via cancel, because cancel reverts HookAlreadyFired.
    //   4. Multi-deployment isolation — two independent inbox+
    //      dispatcher deployments don't share fired state for the
    //      same objectId.
    //   5. L1 reorg wipes acceptance — vm.snapshotState before
    //      submitHeader, then revertToState simulates the reorg that
    //      removes the storage write; dispatch correctly reverts
    //      HeaderNotAccepted.

    /// Mutating `evaporatedAtHeight` while keeping every other dispatch
    /// arg identical changes `leafHash`, so the MMR walk produces a
    /// root that doesn't match `mmrRoot`. Replay-with-mutation is
    /// rejected by the cryptographic binding, not just by the per-hook
    /// fired flag.
    function test_dispatch_rejectsLeafFieldMutation_evaporatedAtHeight() public {
        DispatchArgs memory a = _readDispatchArgs();
        dispatcher.registerHook(a.objectId, address(target), abi.encode(), 50_000);

        // Off by one — different leafHash, MMR walk fails.
        a.evaporatedAtHeight = a.evaporatedAtHeight + 1;
        vm.expectRevert(EvaporationDispatcher.MmrInclusionFailed.selector);
        _doDispatch(dispatcher, a);
    }

    /// Same shape as the previous test, but mutating `finalEnergy`.
    /// Pins that the cryptographic binding covers all three fields
    /// keccak256'd into `leafHash`.
    function test_dispatch_rejectsLeafFieldMutation_finalEnergy() public {
        DispatchArgs memory a = _readDispatchArgs();
        dispatcher.registerHook(a.objectId, address(target), abi.encode(), 50_000);

        // ^1 flips a single bit — guaranteed != trueEnergy.
        a.finalEnergy = a.finalEnergy ^ 1;
        vm.expectRevert(EvaporationDispatcher.MmrInclusionFailed.selector);
        _doDispatch(dispatcher, a);
    }

    /// Cancelling an unfired hook frees the slot. A fresh
    /// `registerHook` for the same objectId then succeeds and (when
    /// dispatched) fires the NEW hook, not the cancelled one. Pins the
    /// happy-path slot recycling that the cancel flow promises.
    function test_cancelThenRegister_firesNewHook() public {
        DispatchArgs memory a = _readDispatchArgs();

        // First registration carries an old marker; cancel frees slot.
        {
            bytes memory firstData = abi.encodeWithSelector(
                GhostTokenMinter.mintBecauseEvaporated.selector,
                bytes("first-marker")
            );
            dispatcher.registerHook(a.objectId, address(target), firstData, 200_000);
            dispatcher.cancelHook(a.objectId);
        }

        // Second registration with a NEW marker — pins that the
        // dispatcher reads from the post-cancel slot, not the wiped one.
        bytes memory secondData = abi.encodeWithSelector(
            GhostTokenMinter.mintBecauseEvaporated.selector,
            bytes("second-marker")
        );
        dispatcher.registerHook(a.objectId, address(target), secondData, 200_000);

        _doDispatch(dispatcher, a);
        assertTrue(dispatcher.isFired(a.objectId));
        assertEq(target.minted(), 1);
        assertEq(
            target.lastData(), bytes("second-marker"),
            "second registration's payload must be the one delivered"
        );
    }

    /// Once a hook has fired, its slot is sticky: cancel reverts
    /// HookAlreadyFired, and (transitively) registerHook reverts
    /// HookAlreadyRegistered. Replay attempts via "cancel + re-register"
    /// are blocked at the cancel step.
    function test_firedHook_cannotBeRebound() public {
        DispatchArgs memory a = _readDispatchArgs();

        dispatcher.registerHook(a.objectId, address(target), abi.encode(), 50_000);
        _doDispatch(dispatcher, a);
        assertTrue(dispatcher.isFired(a.objectId));

        // Cancel after fire is forbidden — slot stays sticky.
        vm.expectRevert(
            abi.encodeWithSelector(
                EvaporationDispatcher.HookAlreadyFired.selector, a.objectId
            )
        );
        dispatcher.cancelHook(a.objectId);

        // Direct re-register also fails — registrar slot is non-zero.
        vm.expectRevert(
            abi.encodeWithSelector(
                EvaporationDispatcher.HookAlreadyRegistered.selector, a.objectId
            )
        );
        dispatcher.registerHook(a.objectId, address(target), abi.encode(), 50_000);
    }

    /// Two independently-deployed (inbox, dispatcher) pairs share NO
    /// per-objectId fired state. Firing the hook on dispatcher A leaves
    /// dispatcher B's hook (registered for the same objectId, against
    /// dispatcher B's fresh inbox) ready to fire on its own. Pins that
    /// the bridge's replay protection is per-deployment, not global.
    function test_dispatcherIsolation_acrossInboxes() public {
        DispatchArgs memory a = _readDispatchArgs();

        // Stand up a parallel inbox/dispatcher/target on the same
        // registry (so the BLS valset signs both alike).
        EvaporHeaderInbox inboxB = new EvaporHeaderInbox(registry);
        EvaporationDispatcher dispatcherB = new EvaporationDispatcher(inboxB);
        GhostTokenMinter targetB = new GhostTokenMinter();

        {
            EvaporHeaderInbox.Header memory header = _readHeader();
            BridgeTypes.Validator[] memory vs = _readValidators();
            bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
            bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
            bytes memory aggSig = vm.parseJsonBytes(fixture, ".agg_signature_uncompressed");
            inboxB.submitHeader(header, vs, pks, bitmap, aggSig);
        }
        vm.roll(block.number + BridgeConstants.MIN_FINALIZATION_DEPTH);

        // Register on BOTH dispatchers for the same objectId.
        dispatcher.registerHook(a.objectId, address(target), abi.encode(), 50_000);
        dispatcherB.registerHook(a.objectId, address(targetB), abi.encode(), 50_000);

        // Fire on dispatcher A.
        _doDispatch(dispatcher, a);
        assertTrue(dispatcher.isFired(a.objectId));
        assertFalse(dispatcherB.isFired(a.objectId), "dispatcher B must be unaffected");

        // dispatcher B fires independently with the same proof.
        _doDispatch(dispatcherB, a);
        assertTrue(dispatcherB.isFired(a.objectId));
    }

    /// L1 reorg simulation: take a state snapshot BEFORE submitHeader,
    /// submit, observe l1AcceptedAt is set; revert state (reorg wipes
    /// the storage write); observe l1AcceptedAt is zero again. A
    /// dispatch attempted after the reorg correctly reverts
    /// HeaderNotAccepted — the dispatcher does NOT fire on a header
    /// whose acceptance was rolled back.
    ///
    /// This is the contract-level analogue of the MIN_FINALIZATION_DEPTH
    /// gate: depth waits long enough for reorgs to become economically
    /// infeasible; this test pins what happens if a reorg DOES land
    /// before depth elapses.
    function test_l1Reorg_wipesAcceptance_dispatcherRefuses() public {
        EvaporHeaderInbox freshInbox = new EvaporHeaderInbox(registry);
        EvaporationDispatcher freshDispatcher =
            new EvaporationDispatcher(freshInbox);

        EvaporHeaderInbox.Header memory header = _readHeader();
        BridgeTypes.Validator[] memory vs = _readValidators();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory aggSig = vm.parseJsonBytes(fixture, ".agg_signature_uncompressed");

        // Snapshot state BEFORE the submit; the reorg in this scenario
        // unwinds the chain segment that contains the submitHeader tx.
        uint256 sid = vm.snapshotState();

        freshInbox.submitHeader(header, vs, pks, bitmap, aggSig);
        assertTrue(
            freshInbox.l1AcceptedAt(header.height) != 0,
            "post-submit l1AcceptedAt must be set"
        );

        // Reorg fires — every storage write inside the reorged segment
        // is gone. revertToState reverts the inbox's mapping write.
        vm.revertToState(sid);
        assertEq(
            freshInbox.l1AcceptedAt(header.height),
            uint64(0),
            "post-reorg l1AcceptedAt must be cleared"
        );

        // Roll forward as if the post-reorg chain is mining new blocks.
        vm.roll(block.number + BridgeConstants.MIN_FINALIZATION_DEPTH);

        // Dispatch attempt against the orphaned header — must refuse.
        bytes32 objectId = vm.parseJsonBytes32(fixture, ".object_id");
        freshDispatcher.registerHook(objectId, address(target), abi.encode(), 50_000);
        bytes memory empty = new bytes(0);
        vm.expectRevert(
            abi.encodeWithSelector(
                EvaporationDispatcher.HeaderNotAccepted.selector, header.height
            )
        );
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
}
