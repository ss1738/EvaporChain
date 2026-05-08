// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Test} from "forge-std/Test.sol";
import {ValidatorSetRegistry} from "../src/ValidatorSetRegistry.sol";
import {BridgeTypes} from "../src/BridgeTypes.sol";
import {ICommitCertVerifier} from "../src/interfaces/ICommitCertVerifier.sol";
import {MockCommitCertVerifier} from "./lib/MockCommitCertVerifier.sol";

/// @notice Cross-side hash agreement: this test deploys the registry,
///         seeds it with a fixed valset, and asserts that the resulting
///         `valsetRoot` matches the byte-identical hash computed by the
///         Rust mirror in `crates/evaporchain-eth-bridge/src/valset.rs::cross_side_test_vector`.
///
///         If you change the pre-image format on either side and forget
///         to update the other, both tests fail together.
contract ValsetAgreementTest is Test {
    ValidatorSetRegistry registry;

    /// @dev keccak256 of the same pre-image computed by Rust:
    ///        epoch=7, 5 validators with pubkey [0x11..0x55] × 48 and
    ///        stakes 100/200/300/400/500.
    bytes32 internal constant EXPECTED_ROOT =
        0xd9772b11c3a1277e03d3e44f3bee65806a0360c27ae1b98fab1ccb1ccc4a8a2b;

    function setUp() public {
        MockCommitCertVerifier v = new MockCommitCertVerifier();
        registry = new ValidatorSetRegistry(ICommitCertVerifier(address(v)));
    }

    function test_rootMatchesRustMirror() public {
        BridgeTypes.Validator[] memory vs =
            new BridgeTypes.Validator[](5);

        bytes memory pk1 = new bytes(48);
        bytes memory pk2 = new bytes(48);
        bytes memory pk3 = new bytes(48);
        bytes memory pk4 = new bytes(48);
        bytes memory pk5 = new bytes(48);
        for (uint256 i = 0; i < 48; i++) {
            pk1[i] = 0x11;
            pk2[i] = 0x22;
            pk3[i] = 0x33;
            pk4[i] = 0x44;
            pk5[i] = 0x55;
        }

        vs[0] = BridgeTypes.Validator({pubkey: pk1, stake: 100});
        vs[1] = BridgeTypes.Validator({pubkey: pk2, stake: 200});
        vs[2] = BridgeTypes.Validator({pubkey: pk3, stake: 300});
        vs[3] = BridgeTypes.Validator({pubkey: pk4, stake: 400});
        vs[4] = BridgeTypes.Validator({pubkey: pk5, stake: 500});

        registry.genesisInit(7, vs);

        assertEq(registry.valsetRoot(), EXPECTED_ROOT, "Rust/Sol root disagree");
        assertEq(registry.totalStake(), 1500);
        assertEq(registry.epoch(), 7);
    }
}
