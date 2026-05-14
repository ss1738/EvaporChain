// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {ValidatorSetRegistry} from "../src/ValidatorSetRegistry.sol";
import {BridgeTypes} from "../src/BridgeTypes.sol";
import {ICommitCertVerifier} from "../src/interfaces/ICommitCertVerifier.sol";
import {MockCommitCertVerifier} from "./lib/MockCommitCertVerifier.sol";

contract ValidatorSetRegistryTest is Test {
    ValidatorSetRegistry registry;
    MockCommitCertVerifier verifier;

    function setUp() public {
        verifier = new MockCommitCertVerifier();
        registry = new ValidatorSetRegistry(ICommitCertVerifier(address(verifier)));
    }

    // ─── helpers ────────────────────────────────────────────────────

    function _v(uint8 seed, uint128 stake)
        internal
        pure
        returns (BridgeTypes.Validator memory)
    {
        bytes memory pk = new bytes(48);
        for (uint256 i = 0; i < 48; i++) pk[i] = bytes1(seed);
        return BridgeTypes.Validator({pubkey: pk, stake: stake});
    }

    function _genesisFive() internal returns (BridgeTypes.Validator[] memory) {
        BridgeTypes.Validator[] memory vs = new BridgeTypes.Validator[](5);
        vs[0] = _v(0x11, 100);
        vs[1] = _v(0x22, 100);
        vs[2] = _v(0x33, 100);
        vs[3] = _v(0x44, 100);
        vs[4] = _v(0x55, 100);
        return vs;
    }

    // ─── tests ──────────────────────────────────────────────────────

    function test_genesisInit_setsState() public {
        BridgeTypes.Validator[] memory vs = _genesisFive();
        registry.genesisInit(1, vs);

        assertEq(registry.epoch(), 1);
        assertEq(registry.totalStake(), 500);
        assertTrue(registry.valsetRoot() != bytes32(0));
        assertEq(registry.owner(), address(0)); // owner burned
    }

    function test_genesisInit_revertsOnSecondCall() public {
        BridgeTypes.Validator[] memory vs = _genesisFive();
        registry.genesisInit(1, vs);
        vm.expectRevert(ValidatorSetRegistry.AlreadyInitialised.selector);
        registry.genesisInit(2, vs);
    }

    function test_genesisInit_revertsOnNonOwner() public {
        BridgeTypes.Validator[] memory vs = _genesisFive();
        vm.prank(address(0xBEEF));
        vm.expectRevert(ValidatorSetRegistry.NotOwner.selector);
        registry.genesisInit(1, vs);
    }

    function test_genesisInit_rejectsBadPubkeyLen() public {
        BridgeTypes.Validator[] memory vs = new BridgeTypes.Validator[](1);
        bytes memory shortPk = new bytes(32); // wrong length
        vs[0] = BridgeTypes.Validator({pubkey: shortPk, stake: 1});
        vm.expectRevert(ValidatorSetRegistry.InvalidPubkeyLength.selector);
        registry.genesisInit(1, vs);
    }

    function test_genesisInit_rejectsEmpty() public {
        BridgeTypes.Validator[] memory vs = new BridgeTypes.Validator[](0);
        vm.expectRevert(ValidatorSetRegistry.EmptyValset.selector);
        registry.genesisInit(1, vs);
    }

    /// @dev 128 zero bytes per prev validator; only used by the verifier
    ///      (mock here ignores it). The real verifier will require these
    ///      to be the EIP-2537 uncompressed encodings of each pubkey.
    function _zeroPubkeysFor(uint256 n) internal pure returns (bytes memory) {
        return new bytes(n * 128);
    }

    function test_updateValset_acceptsSignedTransition() public {
        BridgeTypes.Validator[] memory prev = _genesisFive();
        registry.genesisInit(1, prev);
        bytes32 prevRoot = registry.valsetRoot();

        BridgeTypes.Validator[] memory next = _genesisFive();
        next[4] = _v(0x66, 200); // rotate one validator, bump stake

        bytes memory bitmap = hex"1F"; // all 5 prev validators signed (LSB-first)
        bytes memory sig = "OK"; // mock accepts this

        registry.updateValset(2, next, prev, _zeroPubkeysFor(5), bitmap, sig);

        assertEq(registry.epoch(), 2);
        assertEq(registry.totalStake(), 600);
        assertTrue(registry.valsetRoot() != prevRoot);
    }

    function test_updateValset_rejectsBadEpochOrder() public {
        BridgeTypes.Validator[] memory prev = _genesisFive();
        registry.genesisInit(1, prev);
        BridgeTypes.Validator[] memory next = _genesisFive();
        bytes memory bitmap = hex"1F";
        bytes memory sig = "OK";

        vm.expectRevert(ValidatorSetRegistry.EpochMustIncrement.selector);
        registry.updateValset(3, next, prev, _zeroPubkeysFor(5), bitmap, sig);

        vm.expectRevert(ValidatorSetRegistry.EpochMustIncrement.selector);
        registry.updateValset(1, next, prev, _zeroPubkeysFor(5), bitmap, sig);
    }

    function test_updateValset_rejectsWhenVerifierFails() public {
        BridgeTypes.Validator[] memory prev = _genesisFive();
        registry.genesisInit(1, prev);
        BridgeTypes.Validator[] memory next = _genesisFive();
        bytes memory bitmap = hex"1F";
        bytes memory badSig = "NO";

        vm.expectRevert(ValidatorSetRegistry.VerifierRejected.selector);
        registry.updateValset(2, next, prev, _zeroPubkeysFor(5), bitmap, badSig);
    }

    function test_updateValset_rejectsBadPrevWitness() public {
        BridgeTypes.Validator[] memory prev = _genesisFive();
        registry.genesisInit(1, prev);
        BridgeTypes.Validator[] memory next = _genesisFive();

        // Tamper with prev witness — change one pubkey.
        BridgeTypes.Validator[] memory wrongPrev = _genesisFive();
        wrongPrev[0] = _v(0x99, 100);

        vm.expectRevert(ValidatorSetRegistry.PrevValsetWitnessMismatch.selector);
        registry.updateValset(2, next, wrongPrev, _zeroPubkeysFor(5), hex"1F", "OK");
    }

    function test_updateValset_rejectsInsufficientStake() public {
        BridgeTypes.Validator[] memory prev = _genesisFive();
        registry.genesisInit(1, prev);
        BridgeTypes.Validator[] memory next = _genesisFive();

        // Only 3/5 = 60% < 2/3.
        bytes memory bitmap = hex"07"; // bits 0,1,2

        vm.expectRevert();
        registry.updateValset(2, next, prev, _zeroPubkeysFor(5), bitmap, "OK");
    }

    function test_updateValset_rejectsBadPubkeyArity() public {
        BridgeTypes.Validator[] memory prev = _genesisFive();
        registry.genesisInit(1, prev);
        BridgeTypes.Validator[] memory next = _genesisFive();

        bytes memory wrongLen = new bytes(127); // not a multiple of 128
        vm.expectRevert(ValidatorSetRegistry.PubkeyArityMismatch.selector);
        registry.updateValset(2, next, prev, wrongLen, hex"1F", "OK");
    }

    function test_isActiveValset_view() public {
        BridgeTypes.Validator[] memory vs = _genesisFive();
        registry.genesisInit(1, vs);
        assertTrue(registry.isActiveValset(1, vs));

        BridgeTypes.Validator[] memory other = _genesisFive();
        other[0] = _v(0x99, 100); // different pubkey
        assertFalse(registry.isActiveValset(1, other));
        assertFalse(registry.isActiveValset(2, vs)); // wrong epoch
    }

    // ── L4 (audit 2026-05-13): duplicate-pubkey rejection ──

    /// Pre-fix this valset would have been accepted: pubkey 0x33
    /// listed twice would double-count that signer's stake in
    /// `_sumSignedStake`, dropping the effective 2/3 quorum
    /// threshold. Post-fix `_computeRoot` rejects with
    /// `DuplicatePubkey(firstIndex, duplicateIndex)`.
    function test_audit_l4_genesisInit_rejectsDuplicateAdjacent() public {
        BridgeTypes.Validator[] memory vs = new BridgeTypes.Validator[](3);
        vs[0] = _v(0x11, 100);
        vs[1] = _v(0x33, 100);
        vs[2] = _v(0x33, 100); // duplicate of index 1
        vm.expectRevert(
            abi.encodeWithSelector(
                ValidatorSetRegistry.DuplicatePubkey.selector,
                uint256(1),
                uint256(2)
            )
        );
        registry.genesisInit(1, vs);
    }

    /// Non-adjacent duplicates (the pre-fix exploit shape — sneaking
    /// a copy of validator 0 into position 4 of a longer list) are
    /// also caught.
    function test_audit_l4_genesisInit_rejectsDuplicateNonAdjacent() public {
        BridgeTypes.Validator[] memory vs = new BridgeTypes.Validator[](5);
        vs[0] = _v(0x11, 100);
        vs[1] = _v(0x22, 100);
        vs[2] = _v(0x33, 100);
        vs[3] = _v(0x44, 100);
        vs[4] = _v(0x11, 100); // duplicate of index 0
        vm.expectRevert(
            abi.encodeWithSelector(
                ValidatorSetRegistry.DuplicatePubkey.selector,
                uint256(0),
                uint256(4)
            )
        );
        registry.genesisInit(1, vs);
    }

    /// `updateValset`'s next-set is also subject to the same gate.
    /// Pre-fix an adversarial relayer could have shipped a next-set
    /// with duplicates and inflated quorum for the next epoch.
    function test_audit_l4_updateValset_rejectsDuplicateInNextSet() public {
        BridgeTypes.Validator[] memory prev = _genesisFive();
        registry.genesisInit(1, prev);

        BridgeTypes.Validator[] memory bad = new BridgeTypes.Validator[](3);
        bad[0] = _v(0x10, 100);
        bad[1] = _v(0x20, 100);
        bad[2] = _v(0x20, 100); // duplicate
        vm.expectRevert(
            abi.encodeWithSelector(
                ValidatorSetRegistry.DuplicatePubkey.selector,
                uint256(1),
                uint256(2)
            )
        );
        registry.updateValset(
            2,
            bad,
            prev,
            _zeroPubkeysFor(prev.length),
            hex"1F",
            "OK"
        );
    }

    /// Singleton valsets are accepted (no pairs to compare).
    function test_audit_l4_genesisInit_acceptsSingletonValset() public {
        BridgeTypes.Validator[] memory vs = new BridgeTypes.Validator[](1);
        vs[0] = _v(0x42, 1000);
        registry.genesisInit(1, vs);
        assertEq(registry.totalStake(), 1000);
    }

    /// Un-sorted (but non-duplicate) valsets are still accepted —
    /// the audit's narrow ask was duplicate-rejection only. Sorting
    /// is a doctrine convention that the producer follows but the
    /// contract doesn't yet enforce.
    function test_audit_l4_genesisInit_acceptsUnsortedNonDuplicate() public {
        BridgeTypes.Validator[] memory vs = new BridgeTypes.Validator[](3);
        vs[0] = _v(0x55, 100);
        vs[1] = _v(0x22, 100); // out of order
        vs[2] = _v(0x99, 100);
        registry.genesisInit(1, vs);
        assertEq(registry.totalStake(), 300);
    }
}
