// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";

import {BridgeTypes} from "../src/BridgeTypes.sol";
import {CommitCertVerifier} from "../src/CommitCertVerifier.sol";
import {EvaporHeaderInbox} from "../src/EvaporHeaderInbox.sol";
import {ICommitCertVerifier} from "../src/interfaces/ICommitCertVerifier.sol";
import {ValidatorSetRegistry} from "../src/ValidatorSetRegistry.sol";

/// @title  Deploy
/// @notice Deploys the full bridge stack (verifier + registry + inbox)
///         and writes the addresses to stdout for the relayer to pick up.
///
/// @dev    Usage:
///           $ forge script script/Deploy.s.sol --broadcast --rpc-url $RPC --private-key $PK
///
///         For the local Anvil-based integration test, the test spawns
///         Anvil and invokes this directly via `forge script ... --broadcast`.
contract Deploy is Script {
    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(deployerKey);

        CommitCertVerifier verifier = new CommitCertVerifier();
        ValidatorSetRegistry registry =
            new ValidatorSetRegistry(ICommitCertVerifier(address(verifier)));
        EvaporHeaderInbox inbox = new EvaporHeaderInbox(registry);

        vm.stopBroadcast();

        console2.log("VERIFIER", address(verifier));
        console2.log("REGISTRY", address(registry));
        console2.log("INBOX", address(inbox));
    }
}
