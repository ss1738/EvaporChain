// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";

import {BridgeTypes} from "../src/BridgeTypes.sol";
import {CommitCertVerifier} from "../src/CommitCertVerifier.sol";
import {ValidatorSetRegistry} from "../src/ValidatorSetRegistry.sol";
import {ICommitCertVerifier} from "../src/interfaces/ICommitCertVerifier.sol";

/// @notice End-to-end Phase 2 test. Loads the BLS-signed fixture
///         produced by `tests/commit_cert_fixture.rs`, deploys the
///         registry + real EIP-2537 verifier, runs `genesisInit`
///         followed by `updateValset`, and asserts that the supplied
///         aggregate signature was accepted.
///
///         If this passes, an EvaporChain BLS commit-cert verifies on
///         Ethereum. That is the Phase 2 acceptance criterion.
contract CommitCertVerifierTest is Test {
    using stdJson for string;

    CommitCertVerifier verifier;
    ValidatorSetRegistry registry;

    string fixture;

    function setUp() public {
        verifier = new CommitCertVerifier();
        registry = new ValidatorSetRegistry(ICommitCertVerifier(address(verifier)));

        // Load fixture once.
        fixture = vm.readFile("./fixtures/commit_cert_5.json");
    }

    function _readValidator(string memory key) internal view returns (BridgeTypes.Validator memory) {
        bytes memory pkRaw = vm.parseJsonBytes(fixture, string.concat(key, ".pubkey_compressed"));
        uint256 stake = vm.parseJsonUint(fixture, string.concat(key, ".stake"));
        return BridgeTypes.Validator({pubkey: pkRaw, stake: uint128(stake)});
    }

    function _readValidatorArray(string memory key) internal view returns (BridgeTypes.Validator[] memory) {
        BridgeTypes.Validator[] memory out = new BridgeTypes.Validator[](5);
        for (uint256 i = 0; i < 5; i++) {
            out[i] =
                _readValidator(string.concat(key, "[", vm.toString(i), "]"));
        }
        return out;
    }

    function test_realBlsCommitCertVerifies() public {
        BridgeTypes.Validator[] memory prev = _readValidatorArray(".validators");
        BridgeTypes.Validator[] memory next = _readValidatorArray(".next_validators");
        bytes memory prevPksUncomp = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory aggSig = vm.parseJsonBytes(fixture, ".agg_signature_uncompressed");
        uint256 prevEpoch = vm.parseJsonUint(fixture, ".prev_epoch");
        uint256 nextEpoch = vm.parseJsonUint(fixture, ".next_epoch");

        assertEq(prev.length, 5);
        assertEq(prevPksUncomp.length, 5 * 128);
        assertEq(aggSig.length, 256);

        registry.genesisInit(uint64(prevEpoch), prev);
        bytes32 prevRoot = registry.valsetRoot();

        uint256 g0 = gasleft();
        registry.updateValset(
            uint64(nextEpoch),
            next,
            prev,
            prevPksUncomp,
            bitmap,
            aggSig
        );
        uint256 used = g0 - gasleft();

        emit log_named_uint("updateValset gas (5 validators, 5 signers)", used);

        assertEq(registry.epoch(), nextEpoch);
        assertTrue(registry.valsetRoot() != prevRoot);

        // Total `updateValset` gas with 5 signers ≈ 840k (witness recompute,
        // bitmap walk, stake sum, hash-to-G2 ~280k, MSM ~50k, pairing ~104k,
        // plus storage writes). The verifier-only budget is the lower-half
        // of this; we lock the full path at ≤ 1.2M to catch regressions.
        assertLt(used, 1_200_000);
    }

    function test_realBlsCommitCert_rejectsBadAggSig() public {
        BridgeTypes.Validator[] memory prev = _readValidatorArray(".validators");
        BridgeTypes.Validator[] memory next = _readValidatorArray(".next_validators");
        bytes memory prevPksUncomp = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory bitmap = vm.parseJsonBytes(fixture, ".signed_bitmap");
        bytes memory aggSig = vm.parseJsonBytes(fixture, ".agg_signature_uncompressed");

        // Flip one byte in the aggregate signature.
        aggSig[200] = bytes1(uint8(aggSig[200]) ^ 0x01);

        registry.genesisInit(1, prev);

        vm.expectRevert(ValidatorSetRegistry.VerifierRejected.selector);
        registry.updateValset(2, next, prev, prevPksUncomp, bitmap, aggSig);
    }

    function test_realBlsCommitCert_rejectsTamperedBitmap() public {
        BridgeTypes.Validator[] memory prev = _readValidatorArray(".validators");
        BridgeTypes.Validator[] memory next = _readValidatorArray(".next_validators");
        bytes memory prevPksUncomp = vm.parseJsonBytes(fixture, ".prev_pubkeys_uncompressed");
        bytes memory aggSig = vm.parseJsonBytes(fixture, ".agg_signature_uncompressed");

        // Pretend only 3 of 5 signed (bits 0,1,2 = 0x07 = 60% < 2/3).
        bytes memory bitmap = hex"07";

        registry.genesisInit(1, prev);

        // Insufficient stake: registry gates BEFORE the verifier is even called.
        vm.expectRevert();
        registry.updateValset(2, next, prev, prevPksUncomp, bitmap, aggSig);
    }
}
