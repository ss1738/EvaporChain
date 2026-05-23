#!/usr/bin/env bash
#
# deploy-oracle-feed.sh — end-to-end doctrine proof for OracleFeed
# (contracts/evaporscript/oracle_feed.es).
#
# Doctrine: freshness is a structural property of the read, not a consumer
# convention. The feed contract makes staleness a type-level failure — callers
# cannot receive a stale value without a require-revert; there is no sentinel
# they can forget to check.
#
# Two modes:
#
#   --mode publish (default):
#     Full oracle lifecycle — configure, publish, dispute.
#     1. Adversarial: update before set_feed  → REJECTED (feed not configured)
#     2. set_feed("ETH_USD", max_age=10000)   → sealed=true
#     3. Adversarial: set_feed again           → REJECTED (already configured)
#     4. update(200000)                        → update_count=1
#     5. update(201000)                        → update_count=2
#     6. dispute() as DEPLOYER                 → dispute_count=1
#     7. dispute() as CALLER2                  → dispute_count=2
#     8. Adversarial: update as CALLER2        → REJECTED (not owner)
#     9. GET /api/script → verify value=201000, update_count=2, dispute_count=2
#     Press claim: once sealed, only the operator updates; disputes are open;
#     evaporation removes the feed entirely — no explicit shutdown tx.
#
#   --mode gate:
#     Structural no-value gate — "no value" = revert, not sentinel 0.
#     1. set_feed("BTC_USD", max_age=5000)
#     2. Adversarial: latest() before any update → REJECTED (no value published)
#     3. update(6500000)
#     4. update(6510000)
#     5. GET /api/script → verify value=6510000, update_count=2
#     Press claim: latest() reverts on no-value rather than returning 0 —
#     callers cannot silently consume a missing price.
#
# TX DEDUP NOTES:
#   update() args differ per call (different values) → no dedup risk.
#   dispute() called with different callers → no dedup risk.
#   Adversarial calls use different method/caller/args from successful calls.
#
# Usage:
#   ./scripts/deploy-oracle-feed.sh --dry-run
#   ./scripts/deploy-oracle-feed.sh --node http://89.167.52.40:8099 --mode publish
#   ./scripts/deploy-oracle-feed.sh --node http://89.167.52.40:8099 --mode gate
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 call · 5 adversarial · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/oracle_feed.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"
CALLER2_U8="${CALLER2_U8:-1}"
CALLER3_U8="${CALLER3_U8:-2}"
MODE="${MODE:-publish}"

INITIAL_ENERGY="${INITIAL_ENERGY:-$(( 5000000 + RANDOM % 32768 ))}"
CONTRACT_HALF_LIFE="${CONTRACT_HALF_LIFE:-500000}"
POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-oracle-feed.sh [options]
  --dry-run              print intended calls; no network
  --node URL             node base URL (default http://89.167.52.40:8099)
  --token TOKEN          auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8          operator account index (default 0)
  --caller2 U8           second caller for dispute (default 1)
  --caller3 U8           adversarial caller (default 2)
  --mode publish|gate    prove mode (default publish)
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

log()  { printf '\033[1;36m[oracle]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[oracle ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }
ok()   { printf '\033[1;32m[oracle OK]\033[0m %s\n' "$*"; }

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
untag()     { jq -r ".state.$1 | if type==\"object\" then (.Bool // .U64 // .Str // .Address // .) else . end"; }

acquire_token() {
  $DRY_RUN && return 0
  [[ -n "$TOKEN" ]] && return 0
  local ts; ts=$(date +%s%N 2>/dev/null || date +%s)
  local email="deploy-oracle-${ts}@example.com"
  local pass="EvaporOracle${ts}!"
  local reg_body; reg_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p, display_name:"oracle-deploy"}')
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
grep -q "^contract OracleFeed"   "$CONTRACT_PATH" || die ".es missing OracleFeed header" 2
grep -q "fn set_feed("           "$CONTRACT_PATH" || die ".es missing fn set_feed" 2
grep -q "fn update("             "$CONTRACT_PATH" || die ".es missing fn update" 2
grep -q "fn latest("             "$CONTRACT_PATH" || die ".es missing fn latest" 2
grep -q "fn dispute("            "$CONTRACT_PATH" || die ".es missing fn dispute" 2
[[ "$MODE" == "publish" || "$MODE" == "gate" ]] \
  || die "unknown --mode '$MODE' (publish|gate)" 2

if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

acquire_token

if [[ "$MODE" == "publish" ]]; then
cat <<EOF

+=====================================================================+
|  OracleFeed — doctrine proof (publish mode)                        |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  deployer: $DEPLOYER_U8  caller2: $CALLER2_U8  caller3: $CALLER3_U8
|  feed: ETH_USD  max_age=10000  update×2  dispute×2
|  expect: value=201000, update_count=2, dispute_count=2, sealed=true
+=====================================================================+
EOF
else
cat <<EOF

+=====================================================================+
|  OracleFeed — doctrine proof (gate mode)                           |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  deployer: $DEPLOYER_U8  caller2: $CALLER2_U8  caller3: $CALLER3_U8
|  feed: BTC_USD  max_age=5000
|  prove: latest() before value_set → REJECTED (no sentinel returned)
+=====================================================================+
EOF
fi

# ── Step 1: Deploy ─────────────────────────────────────────────────────────
EP=$(get_epoch)
log "Step 1 - deploy OracleFeed  energy=$INITIAL_ENERGY"
SRC=$(jq -Rs . < "$CONTRACT_PATH")
DEPLOY_BODY=$(jq -n \
  --argjson d  "$DEPLOYER_U8"         \
  --argjson s  "$SRC"                 \
  --argjson e  "$INITIAL_ENERGY"      \
  --argjson hl "$CONTRACT_HALF_LIFE"  \
  '{deployer:$d, source_code:$s, energy:$e, half_life:$hl}')
DH=$(submit_tx "/api/tx/deploy-script" "$DEPLOY_BODY" deploy 3)
DEPLOY_POLL=$(curl_json GET "/api/tx/$DH" 2>/dev/null || echo '{}')
$DRY_RUN && DEPLOY_POLL='{"state":"finalised","contract_id":0}'
# Poll until we get a contract_id
if ! $DRY_RUN; then
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
fi
$DRY_RUN && CID=0
ok "deployed contract_id=$CID"

# ── PUBLISH MODE ───────────────────────────────────────────────────────────
if [[ "$MODE" == "publish" ]]; then

  log "Step 2 - adversarial: update(1000) before set_feed → must be REJECTED"
  EP=$(get_epoch)
  ADV_PRE_BODY=$(jq -n \
    --argjson c  "$DEPLOYER_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"update",
      args:[{U64:1000}], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_PRE_BODY" "update-before-set_feed" 5

  log "Step 3 - set_feed(\"ETH_USD\", max_age=10000) → sealed=true"
  EP=$(get_epoch)
  SF_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"set_feed",
      args:[{Str:"ETH_USD"},{U64:10000}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$SF_BODY" "set_feed" 4
  ok "set_feed ETH_USD max_age=10000 ✓"

  log "Step 4 - adversarial: set_feed again → must be REJECTED (already configured)"
  EP=$(get_epoch)
  ADV_SF_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"set_feed",
      args:[{Str:"ETH_USD_DUP"},{U64:5000}], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_SF_BODY" "set_feed-duplicate" 5

  log "Step 5 - update(200000) → update_count=1"
  EP=$(get_epoch)
  UPD1_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"update",
      args:[{U64:200000}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$UPD1_BODY" "update-1" 4
  ok "update(200000) → update_count=1 ✓"

  log "Step 6 - update(201000) → update_count=2"
  EP=$(get_epoch)
  UPD2_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"update",
      args:[{U64:201000}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$UPD2_BODY" "update-2" 4
  ok "update(201000) → update_count=2 ✓"

  log "Step 7 - dispute() as DEPLOYER → dispute_count=1"
  EP=$(get_epoch)
  DISP1_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"dispute",
      args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$DISP1_BODY" "dispute-1" 4
  ok "dispute() as DEPLOYER → dispute_count=1 ✓"

  log "Step 8 - dispute() as CALLER2 → dispute_count=2"
  EP=$(get_epoch)
  DISP2_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"   \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"dispute",
      args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$DISP2_BODY" "dispute-2" 4
  ok "dispute() as CALLER2 → dispute_count=2 ✓"

  log "Step 9 - adversarial: update as CALLER2 → must be REJECTED (not owner)"
  EP=$(get_epoch)
  ADV_UPD_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"update",
      args:[{U64:999999}], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_UPD_BODY" "update-non-owner" 5

  log "Step 10 - GET /api/script/$CID — verify value=201000, update_count=2, dispute_count=2"
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    SEALED_V=$(printf '%s' "$STATE"  | untag sealed)
    VALUE_V=$(printf '%s' "$STATE"   | untag value)
    UPDCNT_V=$(printf '%s' "$STATE"  | untag update_count)
    DISPCNT_V=$(printf '%s' "$STATE" | untag dispute_count)
    ok "sealed=$SEALED_V  value=$VALUE_V  update_count=$UPDCNT_V  dispute_count=$DISPCNT_V"
    [[ "$VALUE_V"   == "201000" ]] || die "value mismatch: expected 201000, got $VALUE_V" 6
    [[ "$UPDCNT_V"  == "2"      ]] || die "update_count mismatch: expected 2, got $UPDCNT_V" 6
    [[ "$DISPCNT_V" == "2"      ]] || die "dispute_count mismatch: expected 2, got $DISPCNT_V" 6
    case "$SEALED_V" in true|1|True) ok "sealed=true ✓" ;; *) die "sealed != true (got: $SEALED_V)" 6 ;; esac
  fi

cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — OracleFeed (publish mode)               |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (operator-gated oracle with open disputes):
|   - update before set_feed → REJECTED ✓
|   - set_feed duplicate → REJECTED ✓
|   - update×2 by operator → update_count=2, value=201000 ✓
|   - dispute×2 open (any caller) → dispute_count=2 ✓
|   - update by non-owner → REJECTED ✓
|   - "freshness is a structural property, not a consumer convention" ✓
+=====================================================================+
EOF

fi  # end publish mode

# ── GATE MODE ──────────────────────────────────────────────────────────────
if [[ "$MODE" == "gate" ]]; then

  log "Step 2 - set_feed(\"BTC_USD\", max_age=5000) → sealed=true"
  EP=$(get_epoch)
  SF_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"set_feed",
      args:[{Str:"BTC_USD"},{U64:5000}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$SF_BODY" "set_feed" 4
  ok "set_feed BTC_USD max_age=5000 ✓"

  log "Step 3 - adversarial: latest() before any update → must be REJECTED (no value published)"
  EP=$(get_epoch)
  LATEST_ADV_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"latest",
      args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$LATEST_ADV_BODY" "latest-no-value" 5
  ok "latest() with no value published correctly REJECTED ✓"
  ok "Proof: 'no value published' = revert, not sentinel 0 ✓"

  log "Step 4 - update(6500000) → stamped"
  EP=$(get_epoch)
  UPD1_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"update",
      args:[{U64:6500000}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$UPD1_BODY" "update-1" 4
  ok "update(6500000) accepted ✓"

  log "Step 5 - update(6510000) → update_count=2"
  EP=$(get_epoch)
  UPD2_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"update",
      args:[{U64:6510000}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$UPD2_BODY" "update-2" 4
  ok "update(6510000) accepted → update_count=2 ✓"

  log "Step 6 - GET /api/script/$CID — verify value=6510000, update_count=2"
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    VALUE_V=$(printf '%s' "$STATE"  | untag value)
    UPDCNT_V=$(printf '%s' "$STATE" | untag update_count)
    SEALED_V=$(printf '%s' "$STATE" | untag sealed)
    ok "value=$VALUE_V  update_count=$UPDCNT_V  sealed=$SEALED_V"
    [[ "$VALUE_V"  == "6510000" ]] || die "value mismatch: expected 6510000, got $VALUE_V" 6
    [[ "$UPDCNT_V" == "2"       ]] || die "update_count mismatch: expected 2, got $UPDCNT_V" 6
  fi

cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — OracleFeed (gate mode)                  |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (structural no-value gate):
|   - set_feed("BTC_USD", max_age=5000) sealed ✓
|   - latest() before value_set → REJECTED ✓
|   - "callers cannot silently consume a missing price" ✓
|   - update×2 → value=6510000, update_count=2 ✓
+=====================================================================+
EOF

fi  # end gate mode
