# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| Testnet (current) | Yes |

## Reporting a Vulnerability

EvaporChain takes security seriously. If you discover a security vulnerability,
please report it responsibly.

### Reporting Process

1. **Do NOT** open a public GitHub issue for security vulnerabilities.
2. Email your findings to **security@evaporchain.io** with:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact assessment
   - Suggested fix (if any)
3. You will receive an acknowledgment within **48 hours**.
4. We will provide a detailed response within **7 business days**, including:
   - Confirmation of the vulnerability
   - Severity assessment
   - Planned fix timeline

### Scope

The following are in scope for security reports:

- **Consensus** — Byzantine fault tolerance, finality guarantees, validator set management
- **Cryptography** — Poseidon hash, Verkle trie commitments, MMR accumulator, Nova IVC proofs
- **Signatures** — ML-DSA (FIPS 204) transaction signing, BLS12-381 consensus attestations
- **State management** — Energy decay formula, evaporation lifecycle, ghost record integrity
- **Execution** — Transaction validation, nonce management, balance conservation, gas metering
- **Network** — Gossip protocol, message validation, DoS resistance, peer management
- **Smart contracts** — EvaporScript VM, template contracts, gas/iteration limits

### Out of Scope

- Website UI/UX issues (non-security)
- Spam or social engineering attacks
- Denial of service via normal network load (below design capacity)
- Issues in third-party dependencies (report upstream)

### Severity Levels

| Level | Description | Response Time |
|-------|-------------|---------------|
| Critical | Consensus break, fund theft, state corruption | 24 hours |
| High | Signature bypass, DoS on validators, proof forgery | 72 hours |
| Medium | Information leaks, non-critical state issues | 7 days |
| Low | Minor issues, hardening suggestions | 14 days |

### Safe Harbor

We consider security research conducted in accordance with this policy to be:

- Authorized and not subject to legal action
- Helpful and conducted in good faith
- Exempt from any terms that would restrict security testing

We will not pursue legal action against researchers who:

- Act in good faith and follow this disclosure policy
- Avoid privacy violations, data destruction, or service disruption
- Provide sufficient information to reproduce the issue

### Bug Bounty

A formal bug bounty program will be announced prior to mainnet launch.
During testnet, we acknowledge all valid reports and credit researchers
in our security advisories.

## Cryptographic Assumptions

EvaporChain's security relies on the following hardness assumptions:

| Primitive | Assumption | Standard |
|-----------|-----------|----------|
| ML-DSA (Dilithium3) | Module-LWE, Module-SIS | NIST FIPS 204 |
| BLS12-381 | Discrete Log in G1/G2, CDH | RFC draft-irtf-cfrg-bls |
| BLAKE3 | Collision resistance | CFRG RFC 7693 variant |
| Poseidon | Algebraic attack resistance | Grassi et al. 2021 |
| Pallas curve | ECDLP (254-bit) | Pasta curves (Zcash) |
| Nova IVC | Knowledge soundness (HyperKZG) | KST 2022 |

## Audits

| Date | Auditor | Scope | Status |
|------|---------|-------|--------|
| 2026-04-24 | Internal (12-agent suite) | Full codebase + consensus + crypto + DA + execution + proving + scripting + governance + persistence + network + standards + tests | **Complete** — see `FULL_AUDIT_2026_04_24.md` (13 CRITICALs identified, all closed) |
| 2026-04-27 | Internal (end-to-end suite) | Re-audit + cross-verification + dependency baseline + audit-readiness pack + external-audit RFP | **Complete** — see `audit/end_to_end_audit_2026_04_27.md` and the four sibling files in `audit/` |
| 2026-05-06 | Internal (full-tree audit) | End-to-end re-audit covering 7/7 historical CRITICALs + 4/4 HIGH + 5/5 MEDIUM substrates + doc-drift sweep | **Complete** — see `AUDIT_2026_05_06.md` |
| TBD | External firm (Trail of Bits / Sigma Prime / Halborn — RFP issued) | Full pre-mainnet audit per scope in `audit/external_audit_rfp_2026_04_27.md` | **Engagement deferred** until mainnet-blocker punch-list at `DOCTRINE_PUNCH_LIST.md` is exhausted |

This document will be updated as audits are completed.
