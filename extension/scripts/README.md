# Extension build scripts

Reproducible WASM build pipeline for the EvaporChain wallet extension. The
post-quantum signing surface ships as a pre-built `.wasm` blob; if the build
pipeline is compromised, every user is owned. These scripts pin the toolchain,
strip non-deterministic metadata, and verify checksums at extension build time.

## Files

| File | Purpose |
|---|---|
| `wasm-build-versions.json` | Pinned versions of `rust`, `wasm-pack`, `wasm-opt`. |
| `build-wasm.sh` | Verifies toolchain → builds with `wasm-pack` → post-processes with `wasm-opt` → writes `src/crypto/wasm/checksums.json`. |
| `verify-wasm.mjs` | Recomputes sha256 of the shipped artifacts and compares against `checksums.json`. Wired in as `prebuild`. |
| `ci-verify.sh` | Thin wrapper that runs `verify-wasm.mjs`. Drop-in for GitHub Actions. |

## How to build the WASM (once per source change)

```sh
cd extension
npm run build:wasm
```

The script will refuse to run unless your local `rustc`, `wasm-pack`, and
`wasm-opt` exactly match the pinned versions. It does **not** auto-install
anything — it prints the exact commands you need to run.

After it finishes, review the rebuild diff:

```sh
git diff src/crypto/wasm/
```

Then commit `src/crypto/wasm/` and `scripts/wasm-build-versions.json` together.

## How to verify

```sh
npm run verify:wasm
```

Run automatically before every `npm run build` via the `prebuild` hook.

## What to do if `prebuild` fails

1. **You did not change the WASM source.** A tampered or stale artifact is in
   `src/crypto/wasm/`. Restore from git: `git checkout -- src/crypto/wasm/`.
2. **You did change the WASM source (or bumped the toolchain pin).** Rebuild:
   `npm run build:wasm`, review the diff, commit.
3. **Fresh clone, never built.** The placeholder `checksums.json` is the all-zero
   sentinel. Run `npm run build:wasm` once on a machine with the pinned toolchain.

## Why we strip producer metadata

`wasm-opt --strip-producers` removes the WebAssembly `producers` custom section,
which embeds the toolchain name + version that produced the module. That string
drifts across machines (e.g. `rustc 1.83.0 (host x86_64-…)` vs `aarch64-…`) and
would otherwise break byte-identical reproducibility. We also pass `-O3
--strip-debug` for deterministic optimisation and to drop debug info.

## Rebuilding from a clean clone (third-party verification)

Anyone can verify the shipped WASM matches the source:

```sh
git clone https://github.com/ss1738/EvaporChain.git
cd EvaporChain/extension

# Install the pinned toolchain (versions in scripts/wasm-build-versions.json):
rustup toolchain install 1.83.0
rustup default 1.83.0
rustup target add wasm32-unknown-unknown
cargo install --locked wasm-pack@0.13.1
brew install binaryen   # or install binaryen 121 from the GitHub release

# Rebuild and verify:
bash scripts/build-wasm.sh
git diff src/crypto/wasm/checksums.json   # must be empty for byte-identical reproduction
```

## Provenance: `pinned_commit` field

`checksums.json.pinned_commit` records `git rev-parse HEAD` of the parent repo
at the time of the build. It is *provenance only* — `verify-wasm.mjs` does not
enforce a match because the commit will naturally drift as unrelated files
change. Only the sha256 hashes gate the build.

## Note on the placeholder

The `checksums.json` checked in alongside this README contains all-zero hashes
and `built_at: null` until the user runs `build-wasm.sh` once on a Mini with
the pinned toolchain. Until then, `npm run build` will fail at the `prebuild`
step. This is intentional — it forces an explicit, audited first build.
