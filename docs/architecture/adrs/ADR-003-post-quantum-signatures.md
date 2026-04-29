# ADR-003: ML-DSA (CRYSTALS-Dilithium) as Primary Signature Scheme

**Status:** Accepted  
**Date:** 2026-02-01  
**Deciders:** Satyawan Singh (founder)

---

## Context

NIST finalized FIPS 204 (ML-DSA / Dilithium) in August 2024 as a post-quantum digital signature standard. Cryptographically-relevant quantum computers capable of breaking ECDSA/EdDSA are not expected before 2030–2035, but:

1. Blockchain keys are generated now and may secure value for decades.
2. "Harvest now, decrypt later" attacks are already observed — an adversary can record signatures today and forge them once CRQC capability is available.
3. The mainnet launch window for EvaporChain is 2026-2027; designing in classical signatures now means a costly hard fork later.

## Decision

Use ML-DSA (Mode 3, `pqc_dilithium = "=0.2.0"`) as the primary signature scheme for all user transactions. BLS12-381 (via `blst`) is used for consensus votes and DA attestations where signature aggregation is essential.

The `evaporchain-crypto` crate exposes `HybridVerifier` which accepts either scheme, enabling a migration path from ML-DSA-only to ML-DSA+ECDSA hybrid if needed.

`pqc_dilithium` is pinned to exact version `=0.2.0` pending upstream audit. This is noted as a known risk in `AUDIT_SCOPE.md`.

## Alternatives considered

| Scheme | Why not primary |
|--------|----------------|
| Ed25519 | Classical only; fast but quantum-vulnerable |
| ECDSA secp256k1 | EVM-compatible but quantum-vulnerable; adds no value for a new chain |
| SPHINCS+ (ML-DSA alternative) | Much larger signatures (~8KB vs ~2.4KB for Dilithium Mode 3); impractical for block throughput |
| Falcon | Complex lattice structure; patent concerns resolved only in 2024; less implementation maturity |

## Consequences

- ML-DSA Mode 3 signatures are ~2.4 KB (vs 64 bytes for Ed25519). Block size budget accounts for this.
- `const_assert!` in `signatures.rs` verifies key length assumptions at compile time; runtime panics on malformed keys are impossible for valid Mode 3 keys.
- `pqc_dilithium` upstream has not been externally audited. The external security audit (see `AUDIT_SCOPE.md`) should include review of this dependency.
