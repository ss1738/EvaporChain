#!/usr/bin/env bash
# Smoke-test the Singh Pool AMM endpoints (Stages 1-3b) against a
# running node. Exercises the full lifecycle: register/login → create
# pool → mint initial liquidity → list+detail probe → quote → swap →
# /api/swap routing through pool → withdraw → reanchor → final state.
#
# Usage:
#   scripts/test-singh-pool.sh                              # defaults to http://localhost:8081
#   scripts/test-singh-pool.sh http://100.113.253.72:8081   # cluster Mini 2
#
# Required tools: curl, python3.
#
# Exit code: 0 if every step passes, non-zero on first failure.

set -u

NODE="${1:-http://localhost:8081}"
PASS=0
FAIL=0
declare -a FAILED

bold()   { printf '\033[1m%s\033[0m' "$1"; }
green()  { printf '\033[32m%s\033[0m' "$1"; }
red()    { printf '\033[31m%s\033[0m' "$1"; }
yellow() { printf '\033[33m%s\033[0m' "$1"; }

step() {
  local label="$1"; shift
  printf '  %s ' "$(bold "$label")"
  local out
  if out=$("$@" 2>&1); then
    printf '%s\n' "$(green ok)"
    PASS=$((PASS + 1))
    LAST_OUT="$out"
    return 0
  else
    printf '%s\n%s\n' "$(red fail)" "$out"
    FAIL=$((FAIL + 1))
    FAILED+=("$label")
    LAST_OUT="$out"
    return 1
  fi
}

require_field() {
  local label="$1" field="$2" expected="$3"
  local got
  got=$(printf '%s' "$LAST_OUT" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('$field', ''))
except Exception as e:
    print(f'PARSE_ERR:{e}')")
  if [ "$got" = "$expected" ]; then
    return 0
  fi
  printf '    %s expected $field=%s got %s\n' "$(red x)" "$expected" "$got"
  FAIL=$((FAIL + 1))
  FAILED+=("$label:field=$field")
  return 1
}

print_result() {
  printf '\n  result: '
  printf '%s' "$LAST_OUT" | python3 -m json.tool 2>/dev/null | sed 's/^/    /' | head -30
  printf '\n'
}

echo "$(bold 'Singh Pool smoke test') against $NODE"
echo

# ─── auth setup ───
EMAIL="pool-smoke-$(date +%s)@local.test"
PW="SmokeTest123!"
DN="PoolSmoke"

step "register" curl -s --max-time 10 -X POST "$NODE/api/auth/register" \
  -H 'Content-Type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PW\",\"display_name\":\"$DN\"}"

step "login" curl -s --max-time 10 -X POST "$NODE/api/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PW\"}"

TOKEN=$(printf '%s' "$LAST_OUT" | python3 -c "import sys,json;print(json.load(sys.stdin).get('token',''))")
if [ -z "$TOKEN" ]; then
  printf '%s no auth token — aborting\n' "$(red FATAL)"
  exit 1
fi

# Use a unique pool id per run so re-running the script against a
# persistent node doesn't collide with a previous run.
POOL_ID="SMOKE-$(date +%s)"
echo "  pool id for this run: $POOL_ID"
echo

# ─── stage 1: list (empty for this id) ───
echo "$(bold '[stage 1] read-only endpoints')"
step "GET /api/pool/list" curl -s --max-time 5 "$NODE/api/pool/list"

step "GET /api/pool/UNKNOWN (should report found:false)" \
  curl -s --max-time 5 "$NODE/api/pool/DOES-NOT-EXIST-$RANDOM"
require_field "GET unknown pool" "found" "False"

echo
echo "$(bold '[stage 2] create + mint + state')"

# ─── stage 2: create + mint + read state ───
step "POST /api/pool/create" curl -s --max-time 5 -X POST "$NODE/api/pool/create" \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $TOKEN" \
  -d "{\"id\":\"$POOL_ID\",\"fee_bp\":30,\"energy_floor\":0}"
require_field "create" "success" "True"

# Mint initial liquidity. holder = a 32-byte address (val-1 form).
HOLDER="0x0100000000000000000000000000000000000000000000000000000000000000"
step "POST /api/pool/$POOL_ID/mint" curl -s --max-time 5 -X POST "$NODE/api/pool/$POOL_ID/mint" \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $TOKEN" \
  -d "{\"holder\":\"$HOLDER\",\"amount_x\":\"1000000\",\"amount_y\":\"1000000\",\"anchor_energy\":1000,\"epoch\":0}"
require_field "mint initial" "success" "True"

step "GET /api/pool/$POOL_ID detail (after mint)" \
  curl -s --max-time 5 "$NODE/api/pool/$POOL_ID"
require_field "post-mint detail" "found" "True"

print_result

echo
echo "$(bold '[stage 3] swap + /api/swap routing')"

# ─── stage 3a: pool-direct swap ───
step "POST /api/pool/$POOL_ID/swap_x_for_y" curl -s --max-time 5 -X POST \
  "$NODE/api/pool/$POOL_ID/swap_x_for_y" \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"amount_in":"10000"}'
require_field "swap_x_for_y" "success" "True"

# ─── stage 3a: /api/swap routing through pool when pair-id matches ───
# Note: this only routes through the pool if a pool exists with the
# canonical alphabetically-sorted pair id. Our pool is "SMOKE-…"
# which would route iff the swap is between tokens "SMOKE" and "…".
# For a true E2E /api/swap test we'd need to create a pool named
# (e.g.) "EVAP-FLUX". The unit tests already cover pool_id_for_pair.

step "POST /api/swap/quote (oracle fallback expected for unmatched pair)" \
  curl -s --max-time 5 -X POST "$NODE/api/swap/quote" \
  -H 'Content-Type: application/json' \
  -d '{"from_token":"NOTAREAL","to_token":"PAIR","amount":1000}'
# No assertion — just smoke the endpoint responds with a valid envelope.

echo
echo "$(bold '[stage 4] withdraw + reanchor')"

# ─── stage 4: withdraw a small slice ───
step "POST /api/pool/$POOL_ID/withdraw" curl -s --max-time 5 -X POST \
  "$NODE/api/pool/$POOL_ID/withdraw" \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $TOKEN" \
  -d "{\"holder\":\"$HOLDER\",\"shares_to_burn\":\"1000\"}"
# Withdraw may legitimately fail if energy_floor=0 wasn't honoured —
# but we set energy_floor=0 above, so it should succeed.
require_field "withdraw" "success" "True"

step "POST /api/pool/$POOL_ID/reanchor" curl -s --max-time 5 -X POST \
  "$NODE/api/pool/$POOL_ID/reanchor" \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $TOKEN" \
  -d "{\"holder\":\"$HOLDER\",\"anchor_energy\":2000,\"epoch\":1}"
require_field "reanchor" "success" "True"

echo
echo "$(bold '[final state]')"
step "GET /api/pool/$POOL_ID (final)" curl -s --max-time 5 "$NODE/api/pool/$POOL_ID"
print_result

echo
printf '%s pass / %s fail\n' "$(green "$PASS")" "$([ "$FAIL" -eq 0 ] && green 0 || red "$FAIL")"
if [ "$FAIL" -gt 0 ]; then
  echo "  failed steps:"
  for s in "${FAILED[@]}"; do echo "    - $s"; done
  exit 1
fi
echo
echo "$(bold note): persistence is verified by stopping + restarting the node"
echo "      and re-running this script with the same POOL_ID — the pool"
echo "      should be present at /api/pool/list. Bincode round-trip via"
echo "      <data_dir>/singh_pools.bin (covered by the api::singh_pool_helpers"
echo "      unit tests; this script is the live-cluster counterpart)."
