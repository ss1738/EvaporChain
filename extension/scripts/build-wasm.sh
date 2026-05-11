#!/usr/bin/env bash
#
# build-wasm.sh — reproducible WASM build for the EvaporChain wallet extension.
#
# This script is the *only* supported way to (re)build the post-quantum
# crypto WASM blob shipped in the browser extension. It:
#
#   1. Verifies the local toolchain matches the pinned versions in
#      scripts/wasm-build-versions.json. If not, it errors with the exact
#      install commands the developer needs to run. It does NOT auto-install.
#   2. Builds the crate with `wasm-pack build --target web --release` and
#      RUSTFLAGS that scrub local paths.
#   3. Runs `wasm-opt -O3 --strip-debug --strip-producers` for deterministic
#      post-processing. Stripping producer metadata is critical: that section
#      embeds the toolchain version string, which would otherwise drift across
#      machines and break reproducibility.
#   4. Copies the artifacts into extension/src/crypto/wasm/.
#   5. Writes extension/src/crypto/wasm/checksums.json with sha256 hashes
#      of the .wasm + .js, the toolchain versions used, the build timestamp,
#      and the parent repo's git HEAD.
#   6. Prints the new checksums and a copy-pastable diff against any prior
#      checksums.json so the developer can confirm a clean rebuild matches
#      the previously-pinned hash.
#
# This script must be run from any path inside the EvaporChain repo. It will
# resolve paths relative to the script location, NOT the caller's CWD.
#
set -euo pipefail

# -----------------------------------------------------------------------------
# Resolve paths
# -----------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$EXT_DIR/.." && pwd)"
CRATE_DIR="$REPO_ROOT/crates/evaporchain-crypto-wasm"
WASM_OUT_DIR="$EXT_DIR/src/crypto/wasm"
PKG_DIR="$CRATE_DIR/pkg"
VERSIONS_FILE="$SCRIPT_DIR/wasm-build-versions.json"
CHECKSUMS_FILE="$WASM_OUT_DIR/checksums.json"

WASM_FILE_NAME="evaporchain_crypto_wasm_bg.wasm"
JS_FILE_NAME="evaporchain_crypto_wasm.js"
DTS_FILE_NAME="evaporchain_crypto_wasm.d.ts"
DTS_BG_FILE_NAME="evaporchain_crypto_wasm_bg.wasm.d.ts"

# -----------------------------------------------------------------------------
# Helpers
# -----------------------------------------------------------------------------
red() { printf '\033[0;31m%s\033[0m\n' "$*"; }
green() { printf '\033[0;32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[0;33m%s\033[0m\n' "$*"; }
bold() { printf '\033[1m%s\033[0m\n' "$*"; }

die() {
  red "ERROR: $*"
  exit 1
}

# Read a string field from the pinned versions JSON. Pure bash — no jq dep.
json_field() {
  local field="$1"
  local file="$2"
  # Match "field": "value" — trim whitespace + quotes.
  python3 -c "
import json, sys
with open('$file') as f:
    data = json.load(f)
print(data['$field'])
"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    die "Neither sha256sum nor shasum is available."
  fi
}

# -----------------------------------------------------------------------------
# Load pinned versions
# -----------------------------------------------------------------------------
[ -f "$VERSIONS_FILE" ] || die "Missing $VERSIONS_FILE"

PINNED_RUST="$(json_field rust "$VERSIONS_FILE")"
PINNED_WASM_PACK="$(json_field wasm_pack "$VERSIONS_FILE")"
PINNED_WASM_OPT="$(json_field wasm_opt_human "$VERSIONS_FILE")"

bold "EvaporChain WASM reproducible build"
echo "  pinned rust:      $PINNED_RUST"
echo "  pinned wasm-pack: $PINNED_WASM_PACK"
echo "  pinned wasm-opt:  $PINNED_WASM_OPT"
echo "  crate:            $CRATE_DIR"
echo "  output dir:       $WASM_OUT_DIR"
echo

# -----------------------------------------------------------------------------
# Verify toolchain
# -----------------------------------------------------------------------------
verify_rust() {
  command -v rustup >/dev/null 2>&1 \
    || die "rustup is not installed. Install: https://rustup.rs/"
  command -v rustc >/dev/null 2>&1 \
    || die "rustc is not on PATH after rustup install."

  local actual
  actual="$(rustc --version | awk '{print $2}')"
  if [ "$actual" != "$PINNED_RUST" ]; then
    red "rustc version mismatch: got $actual, expected $PINNED_RUST"
    echo
    echo "Fix:"
    echo "  rustup toolchain install $PINNED_RUST"
    echo "  rustup default $PINNED_RUST"
    echo "  rustup target add wasm32-unknown-unknown --toolchain $PINNED_RUST"
    exit 1
  fi

  if ! rustup target list --installed | grep -q '^wasm32-unknown-unknown$'; then
    red "wasm32-unknown-unknown target is not installed."
    echo "Fix:"
    echo "  rustup target add wasm32-unknown-unknown --toolchain $PINNED_RUST"
    exit 1
  fi
  green "  ok rustc $actual"
}

verify_wasm_pack() {
  command -v wasm-pack >/dev/null 2>&1 \
    || { red "wasm-pack is not installed."; \
         echo "Fix:"; \
         echo "  cargo install --locked wasm-pack@$PINNED_WASM_PACK"; \
         exit 1; }

  local actual
  actual="$(wasm-pack --version | awk '{print $2}')"
  if [ "$actual" != "$PINNED_WASM_PACK" ]; then
    red "wasm-pack version mismatch: got $actual, expected $PINNED_WASM_PACK"
    echo "Fix:"
    echo "  cargo install --locked --force wasm-pack@$PINNED_WASM_PACK"
    exit 1
  fi
  green "  ok wasm-pack $actual"
}

verify_wasm_opt() {
  command -v wasm-opt >/dev/null 2>&1 \
    || { red "wasm-opt (binaryen) is not installed."; \
         echo "Fix:"; \
         echo "  # macOS:  brew install binaryen"; \
         echo "  # then verify: wasm-opt --version  # must report 'wasm-opt version $PINNED_WASM_OPT'"; \
         exit 1; }

  # `wasm-opt --version` prints e.g. "wasm-opt version 121"
  local actual_line actual
  actual_line="$(wasm-opt --version | head -n1)"
  actual="$(echo "$actual_line" | awk '{print $3}')"
  if [ "$actual" != "$PINNED_WASM_OPT" ]; then
    red "wasm-opt version mismatch: got '$actual_line', expected version $PINNED_WASM_OPT"
    echo "Fix:"
    echo "  # macOS: install binaryen $PINNED_WASM_OPT"
    echo "  brew install binaryen"
    echo "  # If brew lags behind, install from the matching binaryen release:"
    echo "  #   https://github.com/WebAssembly/binaryen/releases/tag/version_$PINNED_WASM_OPT"
    exit 1
  fi
  green "  ok wasm-opt $actual"
}

bold "Verifying toolchain versions..."
verify_rust
verify_wasm_pack
verify_wasm_opt
echo

# -----------------------------------------------------------------------------
# Capture pre-build state for diff
# -----------------------------------------------------------------------------
PREV_CHECKSUMS=""
if [ -f "$CHECKSUMS_FILE" ]; then
  PREV_CHECKSUMS="$(cat "$CHECKSUMS_FILE")"
fi

# -----------------------------------------------------------------------------
# Build
# -----------------------------------------------------------------------------
bold "Building $CRATE_DIR with wasm-pack..."

# RUSTFLAGS:
#   -C link-arg=-s             — strip linker symbols
#   --remap-path-prefix $HOME/=~/  — scrub absolute home-directory paths from any
#                                    debug-info / panic strings that survive
export RUSTFLAGS="-C link-arg=-s --remap-path-prefix=$HOME/=~/"

# Force a clean output dir so stale artifacts can't sneak through.
rm -rf "$PKG_DIR"

(
  cd "$REPO_ROOT"
  # `--features extension-context` enables the SK-touching wasm_bindgen
  # exports (`mlDsaKeygen`, `mlDsaSign`). Without this flag the build
  # produces a verifier-only WASM that cannot expose secret keys to JS.
  # See AUDIT_2026_05_06.md "WASM secret-key JS exposure" and
  # docs/runbooks/wasm-crypto-csp.md for the threat model.
  wasm-pack build --target web --release "$CRATE_DIR" \
    -- --features extension-context
)

[ -f "$PKG_DIR/$WASM_FILE_NAME" ] || die "wasm-pack did not produce $WASM_FILE_NAME"
[ -f "$PKG_DIR/$JS_FILE_NAME" ] || die "wasm-pack did not produce $JS_FILE_NAME"

# -----------------------------------------------------------------------------
# Deterministic post-processing with wasm-opt
# -----------------------------------------------------------------------------
bold "Running wasm-opt -O3 --strip-debug --strip-producers..."
# --enable-bulk-memory keeps wasm-opt happy with rustc-emitted memory.copy /
# memory.fill ops (Rust 1.85+ stable defaults to bulk-memory). The runtime
# (browser WASM engine) has supported it since 2018; no compatibility cost.
wasm-opt -O3 --enable-bulk-memory --enable-mutable-globals \
  --strip-debug --strip-producers \
  -o "$PKG_DIR/$WASM_FILE_NAME.opt" \
  "$PKG_DIR/$WASM_FILE_NAME"
mv "$PKG_DIR/$WASM_FILE_NAME.opt" "$PKG_DIR/$WASM_FILE_NAME"

# -----------------------------------------------------------------------------
# Copy artifacts into extension
# -----------------------------------------------------------------------------
bold "Copying artifacts into $WASM_OUT_DIR..."
mkdir -p "$WASM_OUT_DIR"

cp "$PKG_DIR/$WASM_FILE_NAME"     "$WASM_OUT_DIR/$WASM_FILE_NAME"
cp "$PKG_DIR/$JS_FILE_NAME"       "$WASM_OUT_DIR/$JS_FILE_NAME"
[ -f "$PKG_DIR/$DTS_FILE_NAME" ]    && cp "$PKG_DIR/$DTS_FILE_NAME"    "$WASM_OUT_DIR/$DTS_FILE_NAME"    || true
[ -f "$PKG_DIR/$DTS_BG_FILE_NAME" ] && cp "$PKG_DIR/$DTS_BG_FILE_NAME" "$WASM_OUT_DIR/$DTS_BG_FILE_NAME" || true

# -----------------------------------------------------------------------------
# Compute checksums + write checksums.json
# -----------------------------------------------------------------------------
bold "Computing sha256 checksums..."
WASM_HASH="$(sha256_of "$WASM_OUT_DIR/$WASM_FILE_NAME")"
JS_HASH="$(sha256_of "$WASM_OUT_DIR/$JS_FILE_NAME")"
BUILT_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

PINNED_COMMIT="unset"
if command -v git >/dev/null 2>&1; then
  if git -C "$REPO_ROOT" rev-parse HEAD >/dev/null 2>&1; then
    PINNED_COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD)"
  fi
fi

cat > "$CHECKSUMS_FILE" <<EOF
{
  "evaporchain_crypto_wasm_bg.wasm": "$WASM_HASH",
  "evaporchain_crypto_wasm.js": "$JS_HASH",
  "built_at": "$BUILT_AT",
  "rust": "$PINNED_RUST",
  "wasm_pack": "$PINNED_WASM_PACK",
  "wasm_opt": "$PINNED_WASM_OPT",
  "pinned_commit": "$PINNED_COMMIT"
}
EOF

green "Wrote $CHECKSUMS_FILE"
echo
bold "New checksums:"
cat "$CHECKSUMS_FILE"
echo

# -----------------------------------------------------------------------------
# Diff against previous pin (if any)
# -----------------------------------------------------------------------------
if [ -n "$PREV_CHECKSUMS" ]; then
  bold "Diff vs previous checksums.json (copy-pastable):"
  PREV_TMP="$(mktemp)"
  printf '%s\n' "$PREV_CHECKSUMS" > "$PREV_TMP"
  if diff -u "$PREV_TMP" "$CHECKSUMS_FILE" >/dev/null 2>&1; then
    green "  no change — rebuild matches the previously-pinned hashes."
  else
    diff -u "$PREV_TMP" "$CHECKSUMS_FILE" || true
    echo
    yellow "  hashes differ from the previous pin. If this is unexpected,"
    yellow "  do NOT commit — investigate first (toolchain drift, source change)."
  fi
  rm -f "$PREV_TMP"
else
  yellow "No previous checksums.json existed — nothing to diff against."
fi

echo
green "Done. Review checksums.json and the artifacts in $WASM_OUT_DIR before committing."
