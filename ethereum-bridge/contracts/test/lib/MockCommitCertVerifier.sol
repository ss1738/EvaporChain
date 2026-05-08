// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

import {BridgeTypes} from "../../src/BridgeTypes.sol";
import {ICommitCertVerifier} from "../../src/interfaces/ICommitCertVerifier.sol";

/// @notice Test-only verifier. Accepts iff `aggSignature == "OK"` (or
///         `alwaysAccept` is on). Phase 2 replaces this with a real
///         EIP-2537 verifier (`CommitCertVerifier.sol`).
contract MockCommitCertVerifier is ICommitCertVerifier {
    bool public alwaysAccept;

    function setAlwaysAccept(bool v) external {
        alwaysAccept = v;
    }

    function verifyCommitCert(
        BridgeTypes.Validator[] calldata, /* prevValidators */
        bytes calldata, /* prevPubkeysUncompressed */
        bytes calldata, /* signedBitmap */
        bytes32, /* messageHash */
        bytes calldata aggSignature
    ) external view returns (bool) {
        if (alwaysAccept) return true;
        return aggSignature.length == 2 && aggSignature[0] == "O" && aggSignature[1] == "K";
    }
}
