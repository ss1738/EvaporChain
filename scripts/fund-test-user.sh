#!/usr/bin/env bash
# Fund a test user's address with the standard test bundle:
# - 100k EVP (native, via faucet account → consensus path)
# - 1k of every deployed non-native token (FLUX/HEAT/DEMO/etc)
#
# Calls POST /api/faucet/bundle (admin-key-gated) so this is operator
# tooling, not anonymous-callable. Use during smoke tests, integration
# verification, paymaster setup. NOT a public-facing faucet.
#
# Usage:
#   EVAPORCHAIN_ADMIN_KEY=... \
#     scripts/fund-test-user.sh <RECIPIENT_HEX_ADDR> [URL] [EVP_AMT] [PER_TOKEN_AMT]
#
# Example:
#   export EVAPORCHAIN_ADMIN_KEY=$(cat ~/.evaporchain-admin-key)
#   ./scripts/fund-test-user.sh 0x0500000000000000000000000000000000000000000000000000000000000000
#
#   ./scripts/fund-test-user.sh \
#     0x0500000000000000000000000000000000000000000000000000000000000000 \
#     http://100.113.253.72:8081 \
#     500000 \
#     5000

set -u

if [ $# -lt 1 ]; then
  echo "usage: $0 <RECIPIENT_HEX_ADDR> [URL=http://localhost:8081] [EVP_AMT=100000] [PER_TOKEN_AMT=1000]" >&2
  exit 1
fi

RECIPIENT="$1"
NODE="${2:-http://localhost:8081}"
EVP_AMT="${3:-100000}"
PER_TOKEN_AMT="${4:-1000}"

if [ -z "${EVAPORCHAIN_ADMIN_KEY:-}" ]; then
  echo "FATAL: EVAPORCHAIN_ADMIN_KEY env var is required (admin-key-gated endpoint)" >&2
  exit 2
fi

bold()  { printf '\033[1m%s\033[0m' "$1"; }
green() { printf '\033[32m%s\033[0m' "$1"; }
red()   { printf '\033[31m%s\033[0m' "$1"; }

echo "$(bold 'Fund test user') against $NODE"
echo "  recipient:       $RECIPIENT"
echo "  EVP amount:      $EVP_AMT"
echo "  per-token amt:   $PER_TOKEN_AMT"
echo

RESP=$(curl -s --max-time 10 -X POST "$NODE/api/faucet/bundle" \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $EVAPORCHAIN_ADMIN_KEY" \
  -d "{\"recipient\":\"$RECIPIENT\",\"evp_amount\":$EVP_AMT,\"per_token_amount\":$PER_TOKEN_AMT}")

SUCCESS=$(printf '%s' "$RESP" | python3 -c "import sys,json
try: print(json.load(sys.stdin).get('success', False))
except: print('PARSE_ERR')")

if [ "$SUCCESS" = "True" ]; then
  printf '%s funded\n' "$(green ✓)"
  printf '%s' "$RESP" | python3 -m json.tool | sed 's/^/  /'
  echo
  echo "Note: EVP transfer is queued for next block consensus inclusion."
  echo "      Token credits applied immediately to in-memory token store."
  exit 0
else
  printf '%s failed\n' "$(red ✗)"
  printf '%s' "$RESP" | python3 -m json.tool | sed 's/^/  /'
  exit 1
fi
