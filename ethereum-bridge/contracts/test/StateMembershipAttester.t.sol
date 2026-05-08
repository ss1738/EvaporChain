// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";

import {BridgeTypes} from "../src/BridgeTypes.sol";
import {CommitCertVerifier} from "../src/CommitCertVerifier.sol";
import {EvaporHeaderInbox} from "../src/EvaporHeaderInbox.sol";
import {ICommitCertVerifier} from "../src/interfaces/ICommitCertVerifier.sol";
import {StateMembershipAttester} from "../src/StateMembershipAttester.sol";
import {ValidatorSetRegistry} from "../src/ValidatorSetRegistry.sol";

/// @notice Phase 4 MVP end-to-end. Loads a real BLS-signed
///         state-membership attestation, verifies that
///         `(key=keccak("account_balance/0xCAFEBABE"), value="1e18")`
///         is committed at height 12_345 — without a Verkle/Groth16
///         proof, just a 2/3+ stake validator-multisig attestation.
contract StateMembershipAttesterTest is Test {
    using stdJson for string;

    ValidatorSetRegistry registry;
    CommitCertVerifier verifier;
    EvaporHeaderInbox inbox;
    StateMembershipAttester attester;

    string fixture;

    function setUp() public {
        verifier = new CommitCertVerifier();
        registry = new ValidatorSetRegistry(ICommitCertVerifier(address(verifier)));
        inbox = new EvaporHeaderInbox(registry);
        attester = new StateMembershipAttester(inbox);

        fixture = vm.readFile("./fixtures/state_membership_5.json");

        // Seed the validator set + submit the header so the inbox has
        // a stateRoot at the height the attestation refers to.
        BridgeTypes.Validator[] memory vs = _readValidators();
        uint64 epoch = uint64(vm.parseJsonUint(fixture, ".epoch"));
        registry.genesisInit(epoch, vs);

        EvaporHeaderInbox.Header memory header = _readHeader();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory headerSig = vm.parseJsonBytes(fixture, ".header_agg_signature");
        inbox.submitHeader(header, vs, pks, bitmap, headerSig);
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

    function test_realStateMembershipVerifies() public {
        BridgeTypes.Validator[] memory vs = _readValidators();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory attestSig = vm.parseJsonBytes(fixture, ".attestation_agg_signature");
        uint64 height = uint64(vm.parseJsonUint(fixture, ".height"));
        bytes32 key = vm.parseJsonBytes32(fixture, ".key");
        bytes memory value = vm.parseJsonBytes(fixture, ".value");

        uint256 g0 = gasleft();
        bool ok = attester.verifyStateMembership(
            height, key, value, vs, pks, bitmap, attestSig
        );
        uint256 used = g0 - gasleft();
        emit log_named_uint("verifyStateMembership gas (5 signers)", used);
        assertTrue(ok);
    }

    function test_rejectsBadAttestation() public {
        BridgeTypes.Validator[] memory vs = _readValidators();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory attestSig = vm.parseJsonBytes(fixture, ".attestation_agg_signature");
        uint64 height = uint64(vm.parseJsonUint(fixture, ".height"));
        bytes32 key = vm.parseJsonBytes32(fixture, ".key");
        bytes memory value = vm.parseJsonBytes(fixture, ".value");

        // Flip a byte in the attestation signature.
        attestSig[120] = bytes1(uint8(attestSig[120]) ^ 0x01);

        vm.expectRevert(StateMembershipAttester.VerifierRejected.selector);
        attester.verifyStateMembership(
            height, key, value, vs, pks, bitmap, attestSig
        );
    }

    function test_rejectsValueTampering() public {
        BridgeTypes.Validator[] memory vs = _readValidators();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory attestSig = vm.parseJsonBytes(fixture, ".attestation_agg_signature");
        uint64 height = uint64(vm.parseJsonUint(fixture, ".height"));
        bytes32 key = vm.parseJsonBytes32(fixture, ".key");

        // Validators signed value=1e18 — try to claim 2e18 instead.
        bytes memory tamperedValue = bytes("2000000000000000000");

        vm.expectRevert(StateMembershipAttester.VerifierRejected.selector);
        attester.verifyStateMembership(
            height, key, tamperedValue, vs, pks, bitmap, attestSig
        );
    }

    function test_rejectsHeightWithoutHeader() public {
        BridgeTypes.Validator[] memory vs = _readValidators();
        bytes memory pks = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory attestSig = vm.parseJsonBytes(fixture, ".attestation_agg_signature");
        bytes32 key = vm.parseJsonBytes32(fixture, ".key");
        bytes memory value = vm.parseJsonBytes(fixture, ".value");

        vm.expectRevert(
            abi.encodeWithSelector(
                StateMembershipAttester.HeaderMissingForHeight.selector, uint64(99_999)
            )
        );
        attester.verifyStateMembership(
            99_999, key, value, vs, pks, bitmap, attestSig
        );
    }
}
