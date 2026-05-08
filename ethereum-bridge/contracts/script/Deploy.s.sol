// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";

import {BridgeTypes} from "../src/BridgeTypes.sol";
import {CommitCertVerifier} from "../src/CommitCertVerifier.sol";
import {EvaporationDispatcher} from "../src/EvaporationDispatcher.sol";
import {EvaporHeaderInbox} from "../src/EvaporHeaderInbox.sol";
import {ICommitCertVerifier} from "../src/interfaces/ICommitCertVerifier.sol";
import {StateMembershipAttester} from "../src/StateMembershipAttester.sol";
import {ValidatorSetRegistry} from "../src/ValidatorSetRegistry.sol";

/// @title  Deploy
/// @notice Deploys the full bridge stack (all 5 contracts) and optionally
///         seeds the genesis validator set in the same transaction bundle.
///
/// @dev    Usage — local Anvil (default):
///           $ forge script script/Deploy.s.sol --broadcast --rpc-url $RPC \
///                           --private-key $PRIVATE_KEY
///
///         Usage — Sepolia / Holesky (use `--broadcast` + `--verify` for Etherscan):
///           $ ETHEREUM_RPC=https://rpc.sepolia.org \
///             PRIVATE_KEY=0x… \
///             forge script script/Deploy.s.sol --broadcast \
///               --rpc-url $ETHEREUM_RPC --private-key $PRIVATE_KEY \
///               --verify --etherscan-api-key $ETHERSCAN_KEY
///
///         Genesis init — set GENESIS_VALIDATORS_JSON before running to seed
///         the valset in the same deploy transaction bundle.  The env var must
///         hold an ABI-encoded `(uint64 epoch, bytes[] pubkeys, uint128[] stakes)`
///         produced by `scripts/genesis-init-calldata.py`.
///
///         If GENESIS_VALIDATORS_JSON is not set, only the contracts are
///         deployed; you must call `genesisInit` separately (via Cast or the
///         Rust helper in `scripts/genesis_init.rs`).
///
/// @dev    Outputs all five contract addresses to stdout in the form
///         `KEY=0x…` so they can be eval'd into the environment:
///           eval $(forge script script/Deploy.s.sol … 2>&1 | grep ^0x | ...)
///         or parsed by the relayer / CI scripts.
contract Deploy is Script {
    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(deployerKey);

        // ── Core stack ───────────────────────────────────────────────
        CommitCertVerifier verifier = new CommitCertVerifier();
        ValidatorSetRegistry registry =
            new ValidatorSetRegistry(ICommitCertVerifier(address(verifier)));
        EvaporHeaderInbox inbox = new EvaporHeaderInbox(registry);

        // ── Phase 4 MVP + Phase 5 ────────────────────────────────────
        StateMembershipAttester attester = new StateMembershipAttester(inbox);
        EvaporationDispatcher dispatcher  = new EvaporationDispatcher(inbox);

        vm.stopBroadcast();

        // ── Print addresses (parseable by scripts) ───────────────────
        console2.log("VERIFIER_ADDR=%s",    vm.toString(address(verifier)));
        console2.log("REGISTRY_ADDR=%s",    vm.toString(address(registry)));
        console2.log("INBOX_ADDR=%s",       vm.toString(address(inbox)));
        console2.log("ATTESTER_ADDR=%s",    vm.toString(address(attester)));
        console2.log("DISPATCHER_ADDR=%s",  vm.toString(address(dispatcher)));

        // ── Optional genesis init ────────────────────────────────────
        // Skipped when GENESIS_CALLDATA is not set. The relayer's
        // `scripts/genesis_init.py` produces the right calldata from
        // the live node's `/api/bridge/validators` endpoint.
        string memory calldata_ = vm.envOr("GENESIS_CALLDATA", string(""));
        if (bytes(calldata_).length > 0) {
            bytes memory cd = vm.parseBytes(calldata_);
            vm.broadcast(deployerKey);
            (bool ok,) = address(registry).call(cd);
            require(ok, "genesisInit failed — check GENESIS_CALLDATA");
            console2.log("genesis validator set committed");
        } else {
            console2.log("(genesis init skipped — set GENESIS_CALLDATA to seed valset)");
        }
    }
}
