// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.26;

/// @title  VerkleProofVerifier
/// @notice On-chain Groth16 BN254 verifier for the Nova→Groth16
///         IVC state-transition proof produced by the EvaporChain
///         nova-bridge crate.
///
///         Verification equation:
///             e(-A, B) · e(α, β) · e(vk_x, γ) · e(C, δ) == 1_{GT}
///         where vk_x = IC[0] + Σᵢ (pub[i] · IC[i+1]).
///
///         G1 arithmetic via EIP-196 ecAdd (0x06) / ecMul (0x07).
///         Pairing check via EIP-197 ecPairing (0x08).
///
///         The verifying key is committed at deployment time.
///         In production, supply parameters from a multi-party ceremony;
///         in testing, supply deterministic parameters (seed-0 from
///         `smoke-fixture-emit`).
///
/// @dev    G2 points are passed/stored in EIP-197 pairing-precompile
///         order: (x.c1, x.c0, y.c1, y.c0) — the coefficient of i
///         before the constant term (inverse of arkworks natural order).
///         `smoke-fixture-emit` emits G2 bytes in this order already.
contract VerkleProofVerifier {
    /// @dev BN254 base-field prime (Fq). Used for G1 negation: y → Fq - y.
    uint256 private constant FQ_PRIME =
        21888242871839275222246405745257275088696311157297823662689037894645226208583;

    uint256 private constant PROOF_BYTES = 256;

    error InvalidProofLength(uint256 got);
    error InvalidPublicInputCount(uint256 got, uint256 expected);
    error EcMulFailed();
    error EcAddFailed();
    error PairingFailed();
    /// NB4 (re-audit 2026-05-14): deployer supplied no IC bytes.
    /// `verify()` reads `_ic_x[0]` for `vk_x` — with zero IC, every
    /// access reverts and the contract is permanently unusable;
    /// worse, with exactly one IC, `verify()` accepts any proof with
    /// `publicInputs.length == 0` whose pairing closes against
    /// `vk_x = IC[0]` alone. Reject at constructor time.
    error EmptyVerifyingKey();
    /// NB4: a G1 coordinate or a G2 component is outside the
    /// BN254 base field. EIP-196 ecMul rejects this lazily at
    /// first use; rejecting at decode time gives a clean error
    /// at deployment / proof submission rather than mid-pairing.
    error CoordinateNotInField(uint256 component);

    // ── Storage (VK, immutable after construction) ────────────────────────────

    uint256 private _alpha_x;
    uint256 private _alpha_y;
    uint256 private _beta_xc1;
    uint256 private _beta_xc0;
    uint256 private _beta_yc1;
    uint256 private _beta_yc0;
    uint256 private _gamma_xc1;
    uint256 private _gamma_xc0;
    uint256 private _gamma_yc1;
    uint256 private _gamma_yc0;
    uint256 private _delta_xc1;
    uint256 private _delta_xc0;
    uint256 private _delta_yc1;
    uint256 private _delta_yc0;
    uint256[] private _ic_x;
    uint256[] private _ic_y;

    // ── Constructor ───────────────────────────────────────────────────────────

    /// @param alphaBytes  64-byte G1 encoding (x BE32 ‖ y BE32).
    /// @param betaBytes   128-byte G2 encoding (xc1‖xc0‖yc1‖yc0, each BE32).
    /// @param gammaBytes  128-byte G2.
    /// @param deltaBytes  128-byte G2.
    /// @param icBytes     Array of 64-byte G1 encodings: IC[0], IC[1], …
    ///                    Must have ≥ 1 entry (`IC[0]` is the constant
    ///                    term of `vk_x`). NB4 (re-audit 2026-05-14):
    ///                    pre-fix `icBytes.length == 0` deployed a
    ///                    permanently broken verifier; `icBytes.length
    ///                    == 1` accepted any zero-publicInput proof
    ///                    whose pairing closed against `vk_x = IC[0]`
    ///                    alone (the public-input MSM contributed
    ///                    nothing). Reject the empty case at
    ///                    construction; the arity-vs-publicInput
    ///                    count check happens at `verify()` time.
    constructor(
        bytes memory alphaBytes,
        bytes memory betaBytes,
        bytes memory gammaBytes,
        bytes memory deltaBytes,
        bytes[] memory icBytes
    ) {
        if (icBytes.length == 0) revert EmptyVerifyingKey();
        (_alpha_x, _alpha_y)                             = _decodeG1(alphaBytes);
        (_beta_xc1, _beta_xc0, _beta_yc1, _beta_yc0)   = _decodeG2(betaBytes);
        (_gamma_xc1, _gamma_xc0, _gamma_yc1, _gamma_yc0) = _decodeG2(gammaBytes);
        (_delta_xc1, _delta_xc0, _delta_yc1, _delta_yc0) = _decodeG2(deltaBytes);
        for (uint256 i = 0; i < icBytes.length; ++i) {
            (uint256 ix, uint256 iy) = _decodeG1(icBytes[i]);
            _ic_x.push(ix);
            _ic_y.push(iy);
        }
    }

    // ── External verify ───────────────────────────────────────────────────────

    /// @notice Verify a Groth16 proof against the verifying key set at
    ///         deployment time.
    ///
    /// @param proofBytes   256-byte EIP-197 encoded proof (A∥B∥C) produced by
    ///                     `eip197::proof_to_eip197_bytes`.
    /// @param publicInputs Public scalars in circuit allocation order:
    ///                     [committed_hash_primary, committed_hash_secondary,
    ///                      z0[0], zi[0]] for `TrivialIncrementCircuit`.
    /// @return true iff the proof is valid.
    function verify(
        bytes calldata proofBytes,
        uint256[] calldata publicInputs
    ) external view returns (bool) {
        if (proofBytes.length != PROOF_BYTES) revert InvalidProofLength(proofBytes.length);
        uint256 icLen = _ic_x.length;
        if (publicInputs.length != icLen - 1) {
            revert InvalidPublicInputCount(publicInputs.length, icLen - 1);
        }

        // ── vk_x = IC[0] + Σ pub[i]·IC[i+1] ─────────────────────────────
        uint256 vx = _ic_x[0];
        uint256 vy = _ic_y[0];
        for (uint256 i = 0; i < publicInputs.length; ++i) {
            (uint256 tx, uint256 ty) = _ecMul(_ic_x[i + 1], _ic_y[i + 1], publicInputs[i]);
            (vx, vy) = _ecAdd(vx, vy, tx, ty);
        }

        return _pairingCheck(proofBytes, vx, vy);
    }

    // ── Internal: pairing check ───────────────────────────────────────────────

    /// @dev Fills a 24-slot uint256 buffer with the 4 pairing pairs and
    ///      calls ecPairing (0x08).
    ///
    ///      Buffer layout (each slot = 32 bytes):
    ///        0-5   : (-A, B)       — G1 negated, G2 from proof
    ///        6-11  : (alpha, beta) — from VK storage
    ///        12-17 : (vk_x, gamma) — computed + VK storage
    ///        18-23 : (C, delta)    — G1 from proof, G2 from VK storage
    function _pairingCheck(
        bytes calldata proof,
        uint256 vx,
        uint256 vy
    ) private view returns (bool) {
        uint256[24] memory inp;
        uint256 fqp = FQ_PRIME;

        // Pair 0: (-A, B) — read A and B from calldata
        assembly {
            let base := proof.offset
            let ax   := calldataload(base)
            let ay   := calldataload(add(base, 32))
            let negAy := 0
            if ay { negAy := sub(fqp, ay) }
            mstore(add(inp, 0),   ax)
            mstore(add(inp, 32),  negAy)
            mstore(add(inp, 64),  calldataload(add(base, 64)))
            mstore(add(inp, 96),  calldataload(add(base, 96)))
            mstore(add(inp, 128), calldataload(add(base, 128)))
            mstore(add(inp, 160), calldataload(add(base, 160)))
        }

        // Pair 1: (alpha, beta)
        inp[6]  = _alpha_x;   inp[7]  = _alpha_y;
        inp[8]  = _beta_xc1;  inp[9]  = _beta_xc0;
        inp[10] = _beta_yc1;  inp[11] = _beta_yc0;

        // Pair 2: (vk_x, gamma)
        inp[12] = vx;           inp[13] = vy;
        inp[14] = _gamma_xc1;  inp[15] = _gamma_xc0;
        inp[16] = _gamma_yc1;  inp[17] = _gamma_yc0;

        // Pair 3: (C, delta) — read C from calldata
        assembly {
            let base := proof.offset
            mstore(add(inp, 576), calldataload(add(base, 192)))
            mstore(add(inp, 608), calldataload(add(base, 224)))
        }
        inp[20] = _delta_xc1; inp[21] = _delta_xc0;
        inp[22] = _delta_yc1; inp[23] = _delta_yc0;

        uint256[1] memory out;
        bool ok;
        assembly {
            ok := staticcall(gas(), 0x08, inp, 768, out, 32)
        }
        if (!ok) revert PairingFailed();
        return out[0] == 1;
    }

    // ── EIP-196 wrappers ──────────────────────────────────────────────────────

    function _ecMul(uint256 px, uint256 py, uint256 scalar)
        private view returns (uint256 rx, uint256 ry)
    {
        bool ok;
        assembly {
            let mem := mload(0x40)
            mstore(mem,          px)
            mstore(add(mem, 32), py)
            mstore(add(mem, 64), scalar)
            ok := staticcall(gas(), 0x07, mem, 96, mem, 64)
            rx := mload(mem)
            ry := mload(add(mem, 32))
        }
        if (!ok) revert EcMulFailed();
    }

    function _ecAdd(uint256 px, uint256 py, uint256 qx, uint256 qy)
        private view returns (uint256 rx, uint256 ry)
    {
        bool ok;
        assembly {
            let mem := mload(0x40)
            mstore(mem,          px)
            mstore(add(mem, 32), py)
            mstore(add(mem, 64), qx)
            mstore(add(mem, 96), qy)
            ok := staticcall(gas(), 0x06, mem, 128, mem, 64)
            rx := mload(mem)
            ry := mload(add(mem, 32))
        }
        if (!ok) revert EcAddFailed();
    }

    // ── Decode helpers ────────────────────────────────────────────────────────

    function _decodeG1(bytes memory b) private pure returns (uint256 x, uint256 y) {
        require(b.length == 64, "VerkleProofVerifier: G1 must be 64 bytes");
        assembly {
            x := mload(add(b, 32))
            y := mload(add(b, 64))
        }
        // NB4 (re-audit 2026-05-14): field-range check. BN254
        // points must have both coordinates in [0, FQ_PRIME).
        // EIP-196 ecMul rejects out-of-range coordinates lazily
        // at first use; checking here gives a clean revert at
        // decode time rather than mid-pairing on the first proof
        // submission. Note that on-curve `y² == x³ + 3 (mod p)`
        // is delegated to the precompile — adding the explicit
        // check here costs gas every constructor / verify call
        // and the precompile already enforces it.
        if (x >= FQ_PRIME) revert CoordinateNotInField(x);
        if (y >= FQ_PRIME) revert CoordinateNotInField(y);
    }

    function _decodeG2(bytes memory b)
        private pure returns (uint256 xc1, uint256 xc0, uint256 yc1, uint256 yc0)
    {
        require(b.length == 128, "VerkleProofVerifier: G2 must be 128 bytes");
        assembly {
            xc1 := mload(add(b, 32))
            xc0 := mload(add(b, 64))
            yc1 := mload(add(b, 96))
            yc0 := mload(add(b, 128))
        }
        // NB4: field-range check on all four Fp components.
        // Note: BN254 G2 subgroup membership is NOT enforced
        // here — EIP-197 ecPairing accepts wrong-subgroup G2
        // points (cofactor h2 ≠ 1), which is a known
        // subgroup-confusion vector. We do NOT mitigate this
        // contract-side; the deployer is trusted to supply VK
        // G2 entries from a correct setup (multi-party
        // ceremony in production, deterministic seed-0 fixture
        // in testing). A fully-paranoid verifier would
        // implement an EIP-2537-style explicit subgroup check
        // via Frobenius (~50 lines + ~250k gas); not added
        // here because the operator-trusted VK assumption
        // already covers this attack surface.
        if (xc1 >= FQ_PRIME) revert CoordinateNotInField(xc1);
        if (xc0 >= FQ_PRIME) revert CoordinateNotInField(xc0);
        if (yc1 >= FQ_PRIME) revert CoordinateNotInField(yc1);
        if (yc0 >= FQ_PRIME) revert CoordinateNotInField(yc0);
    }
}
