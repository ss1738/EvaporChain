// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Bn254Pairing} from "../src/Bn254Pairing.sol";

/// @notice 1C increment (c)-1b: gas anchor for the EIP-197 BN254
/// pairing precompile at 0x08. Two-pair check is the standard KZG
/// opening verifier shape.
contract BenchPairing is Test {
    /// Two-pair check using the BN254 generator and its negation
    /// (a tautological pair check that exercises the precompile
    /// path without needing a real KZG proof). `e(G, [1]_2) ·
    /// e(-G, [1]_2)` cancels to identity = 1.
    function testGas_BN254Pairing_TwoPair() public {
        // G1 generator: (1, 2). G1 negative generator: (1, p - 2)
        // where p = BN254 base modulus.
        uint256 g1x = 1;
        uint256 g1y = 2;
        uint256 negG1x = 1;
        uint256 negG1y =
            21888242871839275222246405745257275088696311157297823662689037894645226208581; // p - 2

        // G2 generator (4 components — EIP-197 ordering with c1
        // before c0 per Ethereum convention):
        // x.c1 = 11559732032986387107991004021392285783925812861821192530917403151452391805634
        // x.c0 = 10857046999023057135944570762232829481370756359578518086990519993285655852781
        // y.c1 = 4082367875863433681332203403145435568316851327593401208105741076214120093531
        // y.c0 = 8495653923123431417604973247489272438418190587263600148770280649306958101930
        uint256 g2_x_c1 = 11559732032986387107991004021392285783925812861821192530917403151452391805634;
        uint256 g2_x_c0 = 10857046999023057135944570762232829481370756359578518086990519993285655852781;
        uint256 g2_y_c1 = 4082367875863433681332203403145435568316851327593401208105741076214120093531;
        uint256 g2_y_c0 = 8495653923123431417604973247489272438418190587263600148770280649306958101930;

        bytes memory input = abi.encodePacked(
            // Pair 1: (G1, G2)
            g1x, g1y,
            g2_x_c1, g2_x_c0, g2_y_c1, g2_y_c0,
            // Pair 2: (-G1, G2) — product cancels to identity
            negG1x, negG1y,
            g2_x_c1, g2_x_c0, g2_y_c1, g2_y_c0
        );

        uint256 g0 = gasleft();
        bool ok = Bn254Pairing.ec_pairing_check_2pair(input);
        uint256 used = g0 - gasleft();
        emit log_named_uint("BN254_PAIRING_2PAIR_GAS", used);
        // Sanity: a + (-a) cancels, product == 1.
        assertTrue(ok, "2-pair tautology must verify");
    }

    /// EXTRAPOLATION: full ppsnark verifier's pairing portion.
    /// HyperKZG opening verify needs ONE 2-pair check; sumcheck
    /// already characterised ~28k; IPA MSM ~62.7M from (6-α).
    function testGas_PpsnarkVerifier_Composition() public {
        // (Empty — extrapolation lives in the log entries below
        // and the audit dossier.)
        emit log_named_uint(
            "PPSNARK_VERIFIER_COMPONENT_SUMCHECK_GAS", 28_360
        );
        emit log_named_uint(
            "PPSNARK_VERIFIER_COMPONENT_IPA_MSM_GAS", 62_734_336
        );
        // Pairing measured by testGas_BN254Pairing_TwoPair above.
        emit log_named_uint("ETHEREUM_L1_BLOCK_LIMIT", 30_000_000);
    }
}
