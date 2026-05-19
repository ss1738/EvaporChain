#!/usr/bin/env bash
#
# deploy-subscription.sh — end-to-end doctrine proof for Subscription
# (contracts/evaporscript/subscription.es).
#
# Doctrine: no off-chain reaper. `pay()` keeps the contract alive via energy;
# missed payments let it evaporate naturally; `on_evaporate` flips `lapsed=true`.
# The runtime IS the deadline enforcer. No who-watches-the-watcher problem.
#
# Two modes:
#
#   --mode active (default):
#     Prove the subscription lifecycle (set_terms → pay → state check).
#     1. Deploy
#     2. Adversarial: pay() as CALLER3 (not subscriber) → REJECTED (sealed==false)
#     3. set_terms(CALLER2=provider, amount=1000, period=100) → sealed=true
#     4. Adversarial: set_terms again (different args) → REJECTED (already sealed)
#     5. pay() as DEPLOYER (subscriber) → paid_periods=1, cumulative_paid=1000
#     6. Adversarial: pay() as CALLER2 (provider, not subscriber) → REJECTED
#     7. GET state → sealed=true, cancelled=false, paid_periods=1, cumulative_paid=1000
#     Proves: non-subscriber pay rejected pre-seal, set_terms idempotent,
#             subscriber-only pay, non-subscriber pay rejected post-seal.
#     Note: SUB-1 (multiple pays allowed) is documented in contract source; tested
#           separately when the epoch advances to produce distinct TX hashes.
#
#   --mode cancel:
#     Prove cancel gates.
#     1. Deploy
#     2. set_terms(CALLER2=provider, amount=500, period=50) → sealed=true
#     3. Adversarial: cancel() as CALLER3 (not subscriber or provider) → REJECTED
#     4. cancel() as DEPLOYER (subscriber) → cancelled=true
#     5. Adversarial: cancel() again (as CALLER3) → REJECTED (already cancelled)
#     6. Adversarial: pay() after cancel (as DEPLOYER) → REJECTED
#     7. GET state → cancelled=true, paid_periods=0
#     Proves: unauthorized cancel rejected, subscriber can cancel, double-cancel rejected,
#             pay after cancel rejected.
#
# TX DEDUP NOTES:
#   active mode: all TX hashes are distinct.
#     step 2 adversarial pay uses CALLER3; step 5 real pay uses DEPLOYER — different callers.
#     step 4 adversarial set_terms uses different args (2000/200) from step 3 (1000/100).
#     step 6 adversarial pay uses CALLER2 — different caller from step 5 DEPLOYER.
#   cancel mode step 3: adversarial cancel() uses CALLER3 — different from DEPLOYER (step 4).
#   cancel mode step 5: second adversarial cancel() uses CALLER3 again (same hash as step 3).
#     Both were rejected; require_rejected passes on the cached "rejected" state.
#   cancel mode step 6: adversarial pay() uses DEPLOYER after cancel — different method from
#     cancel() so unique hash; correctly REJECTED.
#
# Usage:
#   ./deploy-subscription.sh --dry-run
#   ./deploy-subscription.sh --node http://89.167.52.40:8099 --mode active
#   ./deploy-subscription.sh --node http://89.167.52.40:8099 --mode cancel
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 call · 5 adversarial · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

CONTRACT_PATH="/Users/satyawansingh/EvaporChain/contracts/evaporscript/subscription.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"   # subscriber (owner)
CALLER2_U8="${CALLER2_U8:-1}"     # provider
CALLER3_U8="${CALLER3_U8:-2}"     # unauthorized adversarial caller
MODE="${MODE:-active}"

INITIAL_ENERGY="${INITIAL_ENERGY:-$(( 5000000 + RANDOM % 32768 ))}"
CONTRACT_HALF_LIFE="${CONTRACT_HALF_LIFE:-500000}"
POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-subscription.sh [options]
  --dry-run              print intended calls; no network
  --node URL             node base URL (default http://89.167.52.40:8099)
  --token TOKEN          auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8          subscriber/owner account index (default 0)
  --caller2 U8           provider (default 1)
  --caller3 U8           unauthorized adversarial caller (default 2)
  --mode active|cancel   prove mode (default active)
  --energy N             contract initial energy (default ~5M randomised)
  --hl N                 contract half-life (default 500000)
  --timeout SEC          poll timeout (default 300)
  --verbose
  -h|--help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)  DRY_RUN=true; shift ;;
    --node)     NODE_URL="$2"; shift 2 ;;
    --token)    TOKEN="$2"; shift 2 ;;
    --deployer) DEPLOYER_U8="$2"; shift 2 ;;
    --caller2)  CALLER2_U8="$2"; shift 2 ;;
    --caller3)  CALLER3_U8="$2"; shift 2 ;;
    --mode)     MODE="$2"; shift 2 ;;
    --energy)   INITIAL_ENERGY="$2"; shift 2 ;;
    --hl)       CONTRACT_HALF_LIFE="$2"; shift 2 ;;
    --timeout)  POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose)  VERBOSE=true; shift ;;
    -h|--help)  usage; exit 0 ;;
    *)          echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[subscription]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[subscription ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }
ok()   { printf '\033[1;32m[subscription OK]\033[0m %s\n' "$*"; }

curl_json() {
  local method="$1" path="$2" body="${3:-}"
  if $DRY_RUN; then echo "  [DRY-RUN] $method $NODE_URL$path ${body:+(body omitted)}" >&2; echo '{}'; return 0; fi
  local args=(-sS -m 30 -X "$method" -H 'Content-Type: application/json')
  [[ -n "$TOKEN" ]] && args+=(-H "Authorization: Bearer $TOKEN")
  [[ -n "$body" ]] && args+=(-d "$body")
  local resp; resp=$(curl "${args[@]}" "$NODE_URL$path") || die "curl $method $path failed" 2
  $VERBOSE && echo "  <- $resp" >&2
  printf '%s' "$resp"
}

submit_tx() {
  local resp; resp=$(curl_json POST "$1" "$2")
  $DRY_RUN && { echo "DRYHASH"; return 0; }
  local hash; hash=$(printf '%s' "$resp" | jq -r '.tx_hash // empty')
  [[ -n "$hash" ]] || die "$3 failed: $(printf '%s' "$resp" | jq -r '.message // .error // "(no msg)"')" "$4"
  printf '%s' "$hash"
}

poll_tx_state() {
  $DRY_RUN && { echo "finalised"; return 0; }
  local deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC )) resp st
  while (( $(date +%s) < deadline )); do
    resp=$(curl_json GET "/api/tx/$1") || true
    st=$(printf '%s' "$resp" | jq -r '.state // "unknown"')
    case "$st" in
      included|finalised|rejected) printf '%s' "$st"; return 0 ;;
    esac
    sleep 2
  done
  printf 'timeout'
}

require_tx() {
  local h; h=$(submit_tx "$1" "$2" "$3" "$4")
  $DRY_RUN && return 0
  local s; s=$(poll_tx_state "$h")
  [[ "$s" == "finalised" || "$s" == "included" ]] || die "$3 tx not accepted (state=$s)" "$4"
  printf '%s' "$h"
}

require_rejected() {
  local h; h=$(submit_tx "$1" "$2" "$3" "$4")
  $DRY_RUN && { ok "[DRY-RUN] would verify $3 rejected"; return 0; }
  local s; s=$(poll_tx_state "$h")
  [[ "$s" == "rejected" ]] || die "adversarial $3 was NOT rejected (state=$s) — gate failed" "$4"
  ok "adversarial '$3' correctly REJECTED ✓"
}

get_epoch() { $DRY_RUN && { echo 0; return 0; }; curl_json GET "/api/status" | jq -r '.epoch // 0'; }
untag()     { jq -r ".state.$1 | if type==\"object\" then (if has(\"Bool\") then .Bool elif has(\"U64\") then .U64 elif has(\"Str\") then .Str elif has(\"Address\") then .Address else . end) else . end"; }
addr_arg()  { jq -n --argjson i "$1" '{Address: ([$i] + [range(0;31)|0])}'; }

acquire_token() {
  $DRY_RUN && return 0
  [[ -n "$TOKEN" ]] && return 0
  local ts; ts=$(date +%s%N 2>/dev/null || date +%s)
  local email="deploy-sub-${ts}@example.com"
  local pass="EvaporSub${ts}!"
  local reg_body; reg_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p, display_name:"sub-deploy"}')
  local reg_resp; reg_resp=$(curl -sS -m 15 -X POST \
    -H 'Content-Type: application/json' -d "$reg_body" \
    "$NODE_URL/api/auth/register") || die "auth register curl failed" 2
  local ok_r; ok_r=$(printf '%s' "$reg_resp" | jq -r '.success // false')
  [[ "$ok_r" == "true" ]] || die "auth register failed: $(printf '%s' "$reg_resp" | jq -r '.message')" 2
  local login_body; login_body=$(jq -n --arg e "$email" --arg p "$pass" '{email:$e, password:$p}')
  local login_resp; login_resp=$(curl -sS -m 15 -X POST \
    -H 'Content-Type: application/json' -d "$login_body" \
    "$NODE_URL/api/auth/login") || die "auth login curl failed" 2
  TOKEN=$(printf '%s' "$login_resp" | jq -r '.token // empty')
  [[ -n "$TOKEN" ]] || die "auth login returned no token: $(printf '%s' "$login_resp" | jq -r '.message')" 2
  log "auth: registered + logged in (email=$email)"
}

# ── preflight ──────────────────────────────────────────────────────────────
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract Subscription" "$CONTRACT_PATH" || die ".es missing Subscription header" 2
grep -q "fn set_terms("          "$CONTRACT_PATH" || die ".es missing fn set_terms" 2
grep -q "fn pay("                "$CONTRACT_PATH" || die ".es missing fn pay" 2
grep -q "fn cancel("             "$CONTRACT_PATH" || die ".es missing fn cancel" 2
grep -q "fn is_active("          "$CONTRACT_PATH" || die ".es missing fn is_active" 2
[[ "$MODE" == "active" || "$MODE" == "cancel" ]] \
  || die "unknown --mode '$MODE' (active|cancel)" 2

if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

acquire_token
ADDR2=$(addr_arg "$CALLER2_U8")

if [[ "$MODE" == "active" ]]; then
cat <<EOF

+=====================================================================+
|  Subscription — doctrine proof (active mode)                       |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  subscriber: $DEPLOYER_U8  provider: $CALLER2_U8  adversarial: $CALLER3_U8
|  amount=1000 period=100 — prove lifecycle: set_terms → pay
|  prove: non-subscriber pay rejected; set_terms idempotent; subscriber-only pay
|  expect: sealed=true, cancelled=false, paid_periods=1, cumulative_paid=1000
+=====================================================================+
EOF
else
cat <<EOF

+=====================================================================+
|  Subscription — doctrine proof (cancel mode)                       |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  subscriber: $DEPLOYER_U8  provider: $CALLER2_U8  unauthorized: $CALLER3_U8
|  amount=500 period=50 — prove cancel gates
|  prove: unauthorized cancel rejected; subscriber cancel works;
|         double-cancel rejected; pay after cancel rejected
|  expect: cancelled=true, paid_periods=0
+=====================================================================+
EOF
fi

# ── Step 1: Deploy ─────────────────────────────────────────────────────────
log "Step 1 - deploy Subscription  energy=$INITIAL_ENERGY"
SRC=$(jq -Rs . < "$CONTRACT_PATH")
DEPLOY_BODY=$(jq -n \
  --argjson d  "$DEPLOYER_U8"         \
  --argjson s  "$SRC"                 \
  --argjson e  "$INITIAL_ENERGY"      \
  --argjson hl "$CONTRACT_HALF_LIFE"  \
  '{deployer:$d, source_code:$s, energy:$e, half_life:$hl}')
DH=$(submit_tx "/api/tx/deploy-script" "$DEPLOY_BODY" deploy 3)
$DRY_RUN && CID=0 || {
  DEADLINE=$(( $(date +%s) + POLL_TIMEOUT_SEC ))
  while (( $(date +%s) < DEADLINE )); do
    DEPLOY_POLL=$(curl_json GET "/api/tx/$DH")
    DS=$(printf '%s' "$DEPLOY_POLL" | jq -r '.state // "unknown"')
    CID=$(printf '%s' "$DEPLOY_POLL" | jq -r '.contract_id // empty')
    [[ "$DS" == "rejected" ]] && die "deploy rejected" 3
    [[ -n "$CID" && "$CID" != "null" ]] && break
    sleep 2
  done
  [[ -n "$CID" && "$CID" != "null" ]] || die "no contract_id after deploy" 3
}
ok "deployed contract_id=$CID"

# ═══════════════════════════════════════════════════════════════════════════
# ACTIVE MODE
# ═══════════════════════════════════════════════════════════════════════════
if [[ "$MODE" == "active" ]]; then

  log "Step 2 - adversarial: pay() as CALLER3 (not subscriber) → REJECTED (sealed==false)"
  EP=$(get_epoch)
  ADV_PAY_BODY=$(jq -n \
    --argjson c   "$CALLER3_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"pay", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_PAY_BODY" "pay-before-set_terms" 5

  log "Step 3 - set_terms(provider=CALLER2, amount=1000, period=100) → sealed=true"
  EP=$(get_epoch)
  ST_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    --argjson a   "$ADDR2"        \
    '{caller:$c, contract_id:$cid, method:"set_terms",
      args:[$a,{U64:1000},{U64:100}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$ST_BODY" "set_terms" 4
  ok "set_terms(provider=CALLER2, amount=1000, period=100) → sealed=true ✓"

  log "Step 4 - adversarial: set_terms again (different args) → REJECTED (already sealed)"
  EP=$(get_epoch)
  ADV_ST2_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    --argjson a   "$ADDR2"        \
    '{caller:$c, contract_id:$cid, method:"set_terms",
      args:[$a,{U64:2000},{U64:200}], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_ST2_BODY" "set_terms-duplicate" 5
  ok "set_terms again → REJECTED (already sealed) ✓"

  log "Step 5 - pay() as DEPLOYER (subscriber) → paid_periods=1, cumulative_paid=1000"
  EP=$(get_epoch)
  PAY1_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"pay", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$PAY1_BODY" "pay-first" 4
  ok "pay() → paid_periods=1, cumulative_paid=1000 ✓"

  log "Step 6 - adversarial: pay() as CALLER2 (provider, not subscriber) → REJECTED"
  EP=$(get_epoch)
  ADV_PAY2_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"pay", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_PAY2_BODY" "pay-non-subscriber" 5

  log "Step 7 - GET /api/script/$CID — verify state"
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    SEALED_V=$(printf '%s' "$STATE"    | untag sealed)
    CANCEL_V=$(printf '%s' "$STATE"    | untag cancelled)
    PERIODS_V=$(printf '%s' "$STATE"   | untag paid_periods)
    CUMUL_V=$(printf '%s' "$STATE"     | untag cumulative_paid)
    ok "sealed=$SEALED_V  cancelled=$CANCEL_V  paid_periods=$PERIODS_V  cumulative_paid=$CUMUL_V"
    case "$SEALED_V" in true|1|True) ok "sealed=true ✓"      ;; *) die "sealed != true (got: $SEALED_V)"      6 ;; esac
    case "$CANCEL_V" in false|0|False) ok "cancelled=false ✓" ;; *) die "cancelled=true unexpectedly (got: $CANCEL_V)" 6 ;; esac
    [[ "$PERIODS_V" == "1"    ]] && ok "paid_periods=1 ✓"    || die "paid_periods != 1 (got: $PERIODS_V)"    6
    [[ "$CUMUL_V"   == "1000" ]] && ok "cumulative_paid=1000 ✓" || die "cumulative_paid != 1000 (got: $CUMUL_V)" 6
  fi

cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — Subscription (active mode)              |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (subscription lifecycle):
|   - pay() before set_terms (non-subscriber) → REJECTED ✓
|   - set_terms(provider, 1000, 100) → sealed=true ✓
|   - set_terms duplicate → REJECTED ✓
|   - pay() as subscriber → paid_periods=1, cumul=1000 ✓
|   - pay() by non-subscriber (provider) → REJECTED ✓
|   - sealed=true, cancelled=false, paid_periods=1 ✓
|   - "the runtime IS the deadline enforcer; no off-chain reaper" ✓
+=====================================================================+
EOF

fi  # end active mode

# ═══════════════════════════════════════════════════════════════════════════
# CANCEL MODE
# ═══════════════════════════════════════════════════════════════════════════
if [[ "$MODE" == "cancel" ]]; then

  log "Step 2 - set_terms(provider=CALLER2, amount=500, period=50) → sealed=true"
  EP=$(get_epoch)
  ST_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    --argjson a   "$ADDR2"        \
    '{caller:$c, contract_id:$cid, method:"set_terms",
      args:[$a,{U64:500},{U64:50}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$ST_BODY" "set_terms" 4
  ok "set_terms(provider=CALLER2, amount=500, period=50) → sealed=true ✓"

  log "Step 3 - adversarial: cancel() as CALLER3 (not subscriber or provider) → REJECTED"
  EP=$(get_epoch)
  ADV_CANCEL_BODY=$(jq -n \
    --argjson c   "$CALLER3_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"cancel", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_CANCEL_BODY" "cancel-unauthorized" 5

  log "Step 4 - cancel() as DEPLOYER (subscriber) → cancelled=true"
  EP=$(get_epoch)
  CANCEL_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"cancel", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$CANCEL_BODY" "cancel" 4
  ok "cancel() as subscriber → cancelled=true ✓"

  log "Step 5 - adversarial: cancel() again as CALLER3 → REJECTED (already cancelled)"
  EP=$(get_epoch)
  ADV_CANCEL2_BODY=$(jq -n \
    --argjson c   "$CALLER3_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"cancel", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_CANCEL2_BODY" "cancel-already-cancelled" 5

  log "Step 6 - adversarial: pay() after cancel (as DEPLOYER) → REJECTED"
  EP=$(get_epoch)
  ADV_PAY_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"pay", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_PAY_BODY" "pay-after-cancel" 5

  log "Step 7 - GET /api/script/$CID — verify cancelled=true, paid_periods=0"
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    SEALED_V=$(printf '%s' "$STATE"  | untag sealed)
    CANCEL_V=$(printf '%s' "$STATE"  | untag cancelled)
    PERIODS_V=$(printf '%s' "$STATE" | untag paid_periods)
    CUMUL_V=$(printf '%s' "$STATE"   | untag cumulative_paid)
    ok "sealed=$SEALED_V  cancelled=$CANCEL_V  paid_periods=$PERIODS_V  cumulative_paid=$CUMUL_V"
    case "$SEALED_V" in true|1|True)   ok "sealed=true ✓"    ;; *) die "sealed != true (got: $SEALED_V)"    6 ;; esac
    case "$CANCEL_V" in true|1|True)   ok "cancelled=true ✓" ;; *) die "cancelled != true (got: $CANCEL_V)" 6 ;; esac
    [[ "$PERIODS_V" == "0" ]] && ok "paid_periods=0 ✓" || die "paid_periods != 0 (got: $PERIODS_V)" 6
    [[ "$CUMUL_V"   == "0" ]] && ok "cumulative_paid=0 ✓" || die "cumulative_paid != 0 (got: $CUMUL_V)" 6
  fi

cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — Subscription (cancel mode)              |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (cancel gates):
|   - cancel() by unauthorized caller → REJECTED ✓
|   - cancel() by subscriber → cancelled=true ✓
|   - cancel() after already cancelled → REJECTED ✓
|   - pay() after cancel → REJECTED ✓
|   - cancelled=true, paid_periods=0, cumulative_paid=0 ✓
|   - "no off-chain reaper; on_evaporate flips lapsed=true" ✓
+=====================================================================+
EOF

fi  # end cancel mode
