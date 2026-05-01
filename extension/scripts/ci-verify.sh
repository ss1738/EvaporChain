#!/usr/bin/env bash
#
# ci-verify.sh — thin wrapper for CI to verify the shipped WASM matches the
# pinned checksums in extension/src/crypto/wasm/checksums.json.
#
# Designed to be wired into a future GitHub Actions workflow without any
# extra setup. Exits 0 on match, 1 otherwise.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$EXT_DIR"

command -v node >/dev/null 2>&1 || {
  echo "ci-verify: node is required" >&2
  exit 1
}

node "$SCRIPT_DIR/verify-wasm.mjs"
