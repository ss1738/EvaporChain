# EvaporChain Dependency Baseline — 2026-04-27

A snapshot of the project's third-party dependency surface as of `Cargo.lock` on 2026-04-27. This is the baseline auditors will compare against; future drift should be reviewed in the same way.

## 1. Workspace topology

From `Cargo.toml`:

```
resolver = "2"
edition  = "2021"
license  = "MIT"
repo     = github.com/ss1738/EvaporChain
```

**Members (16 in workspace):** evaporchain-types, evaporchain-crypto, evaporchain-state, evaporchain-consensus, evaporchain-contracts, evaporchain-execution, evaporchain-proving, evaporchain-network, evaporchain-node, evaporchain-da, evaporchain-cli, evaporchain-script, evaporchain-mcp, evaporchain-oracle, evaporchain-sharding, plus `wallet`, `prototypes/fold-a-block`, `tests/integration`.

**Excluded:** `crates/evaporchain-crypto-wasm` (WASM target — separate build).

## 2. Critical dependencies (security-relevant)

| Package | Version | Purpose | Audit status / notes |
|---|---|---|---|
| `pqc_dilithium` | 0.2.0 | ML-DSA Dilithium3 post-quantum signatures | **Upstream UNAUDITED.** Tracked as known issue H-13. Pin the exact commit before mainnet; document the upstream-audit risk explicitly to the engaged auditor. |
| `blst` | 0.3.16 | BLS12-381 pairing-based signatures (consensus) | Supranational reference impl; widely deployed (Ethereum, Filecoin). Subgroup checks rely on `from_bytes` — verify it enforces prime-order subgroup, not just curve. |
| `nova-snark` | 0.68.0 | Nova IVC recursive proofs | Active research code, upstream changes frequent. Pin and re-verify on bumps. HyperKZG transcript binding is the soundness anchor. |
| `rocksdb` | 0.22.0 | Persistent state backend | Vendored bindings to upstream RocksDB C++. Crash-safety relies on WAL — verify atomicity of multi-CF batch writes. |
| `chacha20poly1305` | 0.10.1 | XChaCha20-Poly1305 AEAD (wallet-key encryption) | RustCrypto canonical impl. Constant-time correctness depends on platform; validate. |
| `libp2p` | 0.54.1 | P2P networking (GossipSub, Kademlia, Noise) | Large surface. Audit specific transports enabled (TCP/QUIC), GossipSub config, peer scoring. |
| `bcrypt` | 0.15.1 | Password hashing for wallet user-auth | cost=10 in source. Standard impl. Off the consensus critical path. |
| `axum` | 0.7.9 | API HTTP server | tokio-based. Off the consensus critical path. |
| `tokio` | 1.50.0 | Async runtime | Pervasive. Standard. |
| `blake3` | (workspace) | Cryptographic hashing | Reference impl. |
| `rand` | 0.8 (workspace) | RNG. Source must be `OsRng` for security-critical paths | Verified clean in `evaporchain-crypto`; uncommitted hardening swaps remaining `thread_rng` to `OsRng` |

Pinning policy for these critical deps before mainnet: **commit-pin** rather than version-range pin. `Cargo.lock` is the canonical lock; commit it (it already is).

## 3. License coverage

From `deny.toml`:

```toml
allow = [
    "MIT", "Apache-2.0",
    "BSD-2-Clause", "BSD-3-Clause",
    "ISC",
    "Unicode-3.0", "Unicode-DFS-2016",
    "Zlib",
    "BSL-1.0",
    "CC0-1.0",
    "OpenSSL",
    "MPL-2.0",
]
copyleft = "deny"
```

**Risk:** MPL-2.0 is on the allow list; auditors sometimes flag this for product builds (file-level copyleft). Confirm the MPL-2.0 deps in the tree are acceptable for the project license posture (MIT). If the project plans a closed-source fork, MPL-2.0 obligations apply to modifications of the licensed file.

`unlicensed = "deny"` — strict, good. `confidence-threshold = 0.8` — tolerable.

## 4. Source policy

```toml
unknown-registry = "warn"
unknown-git = "warn"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
allow-git = []
```

**Risk:** `allow-git = []` means any git-source dependency is a warning. If `nova-snark` or `pqc_dilithium` is git-pinned for a fix not yet released, this would surface here. Confirm by running `cargo-deny check sources` on a Mini.

## 5. Known issues to flag

- **`unknown-git = "warn"` rather than `"deny"`** — escalate to deny for mainnet builds, with explicit allowlist for any git deps that exist.
- **`copyleft = "deny"` is correct** but `MPL-2.0` is in the allow list. Internally inconsistent if treating MPL as copyleft. Decide one way.
- **`vulnerability = "deny"`** — good, will fail CI on RUSTSEC advisories. Run weekly to surface new advisories.
- `wildcards = "allow"` — should be `"deny"` for a security-sensitive project; confirm no `version = "*"` deps in `Cargo.toml`.
- `multiple-versions = "warn"` — acceptable in the short term; resolve duplicates progressively.

## 6. Suggested commands to run before audit kickoff

Run on a Mini (per project rule — no cargo on MacBook):

```sh
# Resolve all advisories, license issues, source issues
cargo deny check
cargo deny check advisories
cargo deny check licenses
cargo deny check sources
cargo deny check bans

# Independent advisory check
cargo install cargo-audit
cargo audit

# Find unused deps
cargo install cargo-machete
cargo machete

# SBOM
cargo install cargo-cyclonedx
cargo cyclonedx --format json --output-pattern bom.json
```

The SBOM (`bom.json`) goes in the auditor packet alongside `Cargo.lock`. Auditors often request it explicitly under SLSA / NIST 800-218 SSDF compliance.

## 7. Top-of-tree dep counts (informational)

From `Cargo.lock`, total package count is in the hundreds — `cargo tree --workspace --depth 1 | wc -l` on a Mini will produce the exact number. Direct workspace dependencies (declared in `[workspace.dependencies]` of the root `Cargo.toml`):

- serde, serde_json, tokio, tracing, tracing-subscriber, anyhow, thiserror, rand, hex, rocksdb, bincode, blake3 (visible in workspace section)

Per-crate `[dependencies]` blocks add additional direct deps; full audit requires reading each `crates/*/Cargo.toml`.

## 8. Recommended pinning for security-critical

Before mainnet RFP issue, change in workspace root `Cargo.toml`:

```toml
[workspace.dependencies]
pqc_dilithium = { version = "=0.2.0", default-features = false }   # commit-pin via Cargo.lock
blst = { version = "=0.3.16", default-features = false }
nova-snark = { version = "=0.68.0", default-features = false }
rocksdb = { version = "=0.22.0", default-features = false }
```

Rationale: `=` constraint prevents semver drift on `cargo update`. Default features off, opt-in only — reduces unexpected attack surface.

## 9. What this baseline does NOT cover

- **Dependency-of-dependency (transitive) analysis.** The hundreds of indirect deps in `Cargo.lock` need `cargo tree` + `cargo audit` to evaluate. Out of scope for this baseline; auditors should run on first kickoff call.
- **Build-time vs runtime separation.** No analysis of which deps are build-only (proc macros) vs runtime — relevant for supply-chain attack surface.
- **Network requests during build.** No verification that the build is hermetic.

These belong in the audit firm's standard SDLC review.

---

**Owner:** review and update this file every time `Cargo.lock` changes substantially. Auditors will diff against this baseline to score dependency-discipline drift.
