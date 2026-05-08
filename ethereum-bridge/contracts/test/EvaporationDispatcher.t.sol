// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";

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
        vm.expectRevert(
            abi.encodeWithSelector(
                EvaporationDispatcher.MmrRootMissingForHeight.selector, uint64(99_999)
            )
        );
        dispatcher.dispatch(objectId, 99_999, 0, 0, 0, 1, empty, empty, empty);
    }
}
