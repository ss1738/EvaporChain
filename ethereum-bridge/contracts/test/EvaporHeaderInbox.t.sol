// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";

import {BridgeTypes} from "../src/BridgeTypes.sol";
import {CommitCertVerifier} from "../src/CommitCertVerifier.sol";
import {EvaporHeaderInbox} from "../src/EvaporHeaderInbox.sol";
import {ICommitCertVerifier} from "../src/interfaces/ICommitCertVerifier.sol";
import {ValidatorSetRegistry} from "../src/ValidatorSetRegistry.sol";

/// @notice Phase 3a end-to-end. Loads a real BLS-signed header fixture
///         (see `tests/header_inbox_fixture.rs`), deploys the registry +
///         verifier + inbox, seeds the registry with the matching valset,
///         and asserts that `submitHeader` accepts the header on-chain.
contract EvaporHeaderInboxTest is Test {
    using stdJson for string;

    ValidatorSetRegistry registry;
    CommitCertVerifier verifier;
    EvaporHeaderInbox inbox;

    string fixture;

    function setUp() public {
        verifier = new CommitCertVerifier();
        registry = new ValidatorSetRegistry(ICommitCertVerifier(address(verifier)));
        inbox = new EvaporHeaderInbox(registry);

        fixture = vm.readFile("./fixtures/header_inbox_5.json");

        // Seed the registry with the validator set the fixture signed under.
        BridgeTypes.Validator[] memory vs = _readValidators();
        uint64 epoch = uint64(vm.parseJsonUint(fixture, ".epoch"));
        registry.genesisInit(epoch, vs);
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

    function test_realFinalisedHeaderAccepted() public {
        EvaporHeaderInbox.Header memory header = _readHeader();
        BridgeTypes.Validator[] memory vs = _readValidators();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory aggSig = vm.parseJsonBytes(fixture, ".agg_signature_uncompressed");

        uint256 g0 = gasleft();
        inbox.submitHeader(header, vs, pks, bitmap, aggSig);
        uint256 used = g0 - gasleft();

        emit log_named_uint("submitHeader gas (5 signers)", used);

        assertEq(inbox.latestHeight(), header.height);
        assertEq(inbox.stateRootAt(header.height), header.stateRoot);
        assertEq(inbox.mmrRootAt(header.height), header.mmrRoot);
        assertEq(inbox.blockHashAt(header.height), header.blockHash);
    }

    function test_rejectsNonMonotonic() public {
        EvaporHeaderInbox.Header memory h1 = _readHeader();
        BridgeTypes.Validator[] memory vs = _readValidators();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory aggSig = vm.parseJsonBytes(fixture, ".agg_signature_uncompressed");

        inbox.submitHeader(h1, vs, pks, bitmap, aggSig);

        // Re-submit at the same height: rejected.
        vm.expectRevert();
        inbox.submitHeader(h1, vs, pks, bitmap, aggSig);
    }

    function test_rejectsBadAggSig() public {
        EvaporHeaderInbox.Header memory header = _readHeader();
        BridgeTypes.Validator[] memory vs = _readValidators();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory aggSig = vm.parseJsonBytes(fixture, ".agg_signature_uncompressed");
        // Flip a byte.
        aggSig[150] = bytes1(uint8(aggSig[150]) ^ 0x01);

        vm.expectRevert(EvaporHeaderInbox.VerifierRejected.selector);
        inbox.submitHeader(header, vs, pks, bitmap, aggSig);
    }
}
