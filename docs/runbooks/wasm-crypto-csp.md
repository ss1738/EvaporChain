# WASM Crypto — Browser Isolation Runbook

**Audit cross-ref**: `AUDIT_2026_05_06.md` "WASM secret-key JS exposure" + CRITICAL-1 layout hardening.

This runbook captures the threat model for `evaporchain-crypto-wasm` and the
**compile-time + runtime + manifest** controls that make secret-key handling
safe in the browser extension and structurally impossible everywhere else.

---

## Threat model

The WASM bridge exposes ML-DSA (Dilithium3) keygen, sign, and verify to JS via
`wasm-bindgen`. **WASM linear memory is readable by any JS in the same
origin.** This is safe ONLY under the browser-extension isolated execution
context:

1. The extension runs in its own origin (`chrome-extension://<id>`), isolated
   from web-page JS by the Same-Origin Policy.
2. Content scripts run in an isolated world — page JS cannot read extension
   memory or call extension APIs.
3. The extension's `manifest.json` `content_security_policy` forbids
   `unsafe-eval` and restricts script sources to `'self'`.

**If `evaporchain-crypto-wasm` were ever loaded into a regular web page**, any
same-origin script could read WASM linear memory and extract secret keys
between the moment they enter (e.g. via `mlDsaSign(sk_bytes, ...)`) and the
moment `ZeroizingKeypair::drop` overwrites them.

---

## Three layers of defence

### Layer 1 — Cargo feature gate (compile-time, structural)

`evaporchain-crypto-wasm` has a Cargo feature `extension-context` (OFF by
default). The two SK-touching `wasm_bindgen` exports — `mlDsaKeygen` (returns
SK to JS) and `mlDsaSign` (consumes SK from JS) — are gated behind it:

```rust
#[cfg(feature = "extension-context")]
#[wasm_bindgen(js_name = "mlDsaKeygen")]
pub fn ml_dsa_keygen() -> Result<JsValue, JsValue> { ... }
```

A `cargo build` or `wasm-pack build` without `--features extension-context`
produces a WASM that exposes ONLY the public-key surfaces — `mlDsaVerify` and
`deriveAddress`. The functions that would let JS leak secrets *do not exist*
in the binary.

### Layer 2 — Reproducible build pipeline (only extension enables the feature)

`extension/scripts/build-wasm.sh` is the **only supported way** to produce the
WASM blob shipped with the extension. It passes `--features extension-context`
explicitly:

```bash
wasm-pack build --target web --release "$CRATE_DIR" \
  -- --features extension-context
```

The pipeline also pins toolchain versions (`rustc`, `wasm-pack`, `wasm-opt`)
and writes a deterministic `checksums.json`. If the feature flag were ever
removed from the build script, every extension developer would see a hash
diff against the previously-pinned blob and refuse to commit. The feature
flag's presence in the binary is therefore reproducibly auditable.

### Layer 3 — Manifest CSP (runtime, browser-enforced)

`extension/public/manifest.json` sets:

```json
"content_security_policy": {
  "extension_pages": "script-src 'self' 'wasm-unsafe-eval'; object-src 'self'"
}
```

- `script-src 'self'` — only scripts from the extension package can run.
  Page-injected scripts cannot reach the WASM.
- `'wasm-unsafe-eval'` — required for `WebAssembly.compileStreaming` /
  `WebAssembly.instantiate`. Necessary cost for any WASM-loading extension.
- `object-src 'self'` — blocks `<object>`/`<embed>` injection.

---

## Why this combination is load-bearing

Each layer covers a failure mode the others cannot:

| Failure mode | L1 (feature) | L2 (build) | L3 (CSP) |
|---|---|---|---|
| Webapp pipeline accidentally imports the crypto crate | **blocks** (export missing) | n/a | n/a |
| Build script tampered to omit feature | n/a | **blocks** (checksum mismatch) | n/a |
| Web page injects script trying to read WASM memory | n/a | n/a | **blocks** (script-src) |
| Same-origin extension page tries to eval-inject SK exfil | n/a | n/a | **blocks** (`unsafe-eval` forbidden) |

No single layer is sufficient. The combination is.

---

## Verifier-only build (for non-extension consumers)

If a *non-extension* consumer (e.g. a backend that only verifies signatures)
wants to use this crate's WASM:

```bash
wasm-pack build --target web --release crates/evaporchain-crypto-wasm
# (no --features — verifier-only build)
```

The resulting WASM exposes:

| Export | Touches SK? | Safe in webapps? |
|---|---|---|
| `mlDsaVerify(msg, sig, pubkey)` | no | yes |
| `deriveAddress(pubkey)` | no | yes |
| `mlDsaKeygen` | *not present* | n/a |
| `mlDsaSign` | *not present* | n/a |

This is the recommended path for any context outside the audited extension.

---

## CI gate (recommended)

A CI job that builds the crate without the feature and checks the absence of
`mlDsaKeygen` / `mlDsaSign` exports would catch any future PR that accidentally
removes a `#[cfg(feature = "extension-context")]` attribute:

```bash
wasm-pack build --target web --release crates/evaporchain-crypto-wasm
! grep -q 'mlDsaKeygen\|mlDsaSign' crates/evaporchain-crypto-wasm/pkg/evaporchain_crypto_wasm.js \
  || (echo "verifier-only build leaked SK-touching exports"; exit 1)
```

(Not currently wired into CI — defer to whoever owns the bridge-build job.)

---

## When to revisit

- **pqc_dilithium upgrade** — if `pqc_dilithium 0.3+` exposes a constructor
  for raw SK bytes, the `reconstruct_keypair` unsafe block in `lib.rs` can
  be deleted. The feature-gate stays; the underlying SK-exposure threat is
  unchanged.
- **WebAssembly memory isolation proposals** — once the WASM proposal for
  segmented memory / strict memory permissions lands and browsers ship it,
  Layer 1's "function doesn't exist" defence could be replaced with
  per-region memory permissions. Until then, Layer 1 is the strongest
  available defence-in-depth.
- **Audit re-run** — when AUDIT_2026_05_06.md is re-walked, this runbook
  should be cited under the "WASM secret-key JS exposure" entry as the
  documented mitigation.
