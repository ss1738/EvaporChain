#!/usr/bin/env bash
# run.sh — Drive Maestro flows for the EvaporChain mobile wallet.
#
# Validates preconditions (Maestro CLI installed, node reachable,
# emulator/simulator booted) and then runs every flow in `.maestro/`.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}/.."

# 1. Maestro CLI installed?
if ! command -v maestro >/dev/null 2>&1; then
  echo "error: maestro CLI not found on PATH." >&2
  echo "" >&2
  echo "Install on macOS via Homebrew:" >&2
  echo "  brew install maestro" >&2
  echo "" >&2
  echo "Or via the official installer:" >&2
  echo "  curl -Ls https://get.maestro.mobile.dev | bash" >&2
  exit 1
fi

# 2. WALLET_NODE_URL set + reachable?
if [[ -z "${WALLET_NODE_URL:-}" ]]; then
  echo "error: WALLET_NODE_URL is not set." >&2
  echo "  export WALLET_NODE_URL=http://satyawan.local:8080" >&2
  exit 1
fi

if ! curl -fs --max-time 5 "${WALLET_NODE_URL%/}/api/status" >/dev/null; then
  echo "error: WALLET_NODE_URL=${WALLET_NODE_URL} is unreachable." >&2
  echo "  Start the EvaporChain node and retry." >&2
  exit 1
fi

# 3. An iOS simulator OR Android emulator running?
ios_booted=""
if command -v xcrun >/dev/null 2>&1; then
  ios_booted="$(xcrun simctl list devices | grep -E '\(Booted\)' || true)"
fi

android_running=""
if command -v adb >/dev/null 2>&1; then
  android_running="$(adb devices | awk 'NR>1 && /device$/' || true)"
fi

if [[ -z "${ios_booted}" && -z "${android_running}" ]]; then
  echo "error: no iOS simulator or Android emulator detected." >&2
  echo "" >&2
  echo "Boot an iOS simulator:" >&2
  echo "  xcrun simctl boot 'iPhone 15 Pro'  # or any device name" >&2
  echo "  open -a Simulator" >&2
  echo "" >&2
  echo "Boot an Android emulator:" >&2
  echo "  emulator -list-avds                 # list available AVDs" >&2
  echo "  emulator -avd <avd-name> &" >&2
  exit 1
fi

# 4. Run every flow in .maestro/.
echo "→ maestro test .maestro/"
exec maestro test .maestro/
