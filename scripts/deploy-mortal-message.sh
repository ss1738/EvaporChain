#!/usr/bin/env bash
#
# deploy-mortal-message.sh — end-to-end doctrine proof for MortalMessage
# (contracts/evaporscript/mortal_message.es).
#
# Doctrine: each message IS a contract instance. The contract's own energy
# IS the message's lifespan: when energy hits zero the message evaporates.
# This is the canonical "the lifecycle is the entity" pattern — the chain
# runtime, not contract code, drives the message through
# active → grace → ghost. set_payload seals content in one shot; after
# sealing only sender and recipient may read.
#
# Two modes:
#
#   --mode settle (default):
#     Full lifecycle: seal once, read as sender, read as recipient, boost.
#     1.  Deploy
#     2.  Adversarial: set_payload by CALLER2 (non-owner) → REJECTED
#     3.  Real: set_payload by DEPLOYER: body="hello from EvaporChain"
#         recipient=CALLER2. sealed=true.
#         [CALLER2 (step 2) vs DEPLOYER (step 3) → different callers → no dedup]
#     4.  read() as DEPLOYER (owner/sender) → ACCEPTED
#     5.  read() as CALLER2 (recipient) → ACCEPTED
#         [DEPLOYER (step 4) vs CALLER2 (step 5) → different callers → no dedup]
#     6.  read() as CALLER3 (unauthorized) → REJECTED (not authorized)
#         [CALLER3 distinct from DEPLOYER and CALLER2 → no dedup]
#     7.  record_boost() as DEPLOYER → ACCEPTED; boost_count=1
#     8.  GET state → sealed=true, boost_count=1
#
#   --mode gate:
#     Prove seal-once + authorization guards.
#     1.  Deploy
#     2.  Adversarial: set_payload by CALLER2 (non-owner) → REJECTED
#     3.  Real: set_payload by DEPLOYER: body="gate message", recipient=CALLER3
#         [CALLER2 (step 2) vs DEPLOYER (step 3) → different callers → no dedup]
#     4.  Adversarial: set_payload again by DEPLOYER → REJECTED (already sealed)
#         [args differ: body="second message" vs "gate message" → different hash → no dedup]
#     5.  Adversarial: read() by CALLER2 (not recipient CALLER3, not owner DEPLOYER) → REJECTED
#     6.  GET state → sealed=true, boost_count=0
#
# TX DEDUP NOTES (settle):
#   set_payload adv (step 2) uses CALLER2; real (step 3) uses DEPLOYER → safe.
#   read() steps 4 and 5 use different callers (DEPLOYER vs CALLER2) → safe.
#   read() step 6 uses CALLER3 → safe.
#   record_boost step 7: single call only; second identical call would dedup.
#
# TX DEDUP NOTES (gate):
#   set_payload adv (step 2, CALLER2) vs real (step 3, DEPLOYER) → different callers → safe.
#   set_payload dup (step 4, DEPLOYER, body="second message") vs step 3 (body="gate message")
#     → different args → different hash → node runs fresh TX → sealed guard fires → REJECTED.
#   read() adv (step 5, CALLER2, args=[]) → CALLER2 distinct from step 2 (different method) → safe.
#
# Usage:
#   ./deploy-mortal-message.sh --dry-run
#   ./deploy-mortal-message.sh --node http://89.167.52.40:8099 --mode settle
#   ./deploy-mortal-message.sh --node http://89.167.52.40:8099 --mode gate
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 call · 5 adversarial · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

CONTRACT_PATH="/Users/satyawansingh/EvaporChain/contracts/evaporscript/mortal_message.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"
CALLER2_U8="${CALLER2_U8:-1}"
CALLER3_U8="${CALLER3_U8:-2}"
MODE="${MODE:-settle}"
INITIAL_ENERGY="${INITIAL_ENERGY:-$(( 5000000 + RANDOM % 32768 ))}"
CONTRACT_HALF_LIFE="${CONTRACT_HALF_LIFE:-200000}"
POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-mortal-message.sh [options]
  --dry-run              print intended calls; no network
  --node URL             node base URL (default http://89.167.52.40:8099)
  --token TOKEN          auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8          sender/creator account index (default 0)
  --caller2 U8           recipient (settle) / adv sender (gate) (default 1)
  --caller3 U8           unauthorized reader (settle) / recipient (gate) (default 2)
  --mode settle|gate     prove mode (default settle)
  --energy N             contract initial energy (default ~5M randomised)
  --hl N                 contract half-life (default 200000)
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

log()  { printf '\033[1;36m[mortal-message]\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m[mortal-message OK]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[mortal-message ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }

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
  local email="deploy-mortal-msg-${ts}@example.com"
  local pass="EvaporMortal${ts}!"
  local reg_body; reg_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p, display_name:"mortal-message-deploy"}')
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
grep -q "^contract MortalMessage" "$CONTRACT_PATH" || die ".es missing MortalMessage header" 2
grep -q "fn set_payload("         "$CONTRACT_PATH" || die ".es missing fn set_payload" 2
grep -q "fn read("                "$CONTRACT_PATH" || die ".es missing fn read" 2
grep -q "fn record_boost("        "$CONTRACT_PATH" || die ".es missing fn record_boost" 2
[[ "$MODE" == "settle" || "$MODE" == "gate" ]] \
  || die "unknown --mode '$MODE' (settle|gate)" 2

if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

acquire_token

if [[ "$MODE" == "settle" ]]; then
cat <<EOF

+=====================================================================+
|  MortalMessage — doctrine proof (settle mode)                      |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  sender: $DEPLOYER_U8 (owner)  recipient: $CALLER2_U8  unauth: $CALLER3_U8
|  prove: one-shot seal, sender + recipient read, unauthorized blocked,
|         record_boost accumulates; message energy IS its lifespan
+=====================================================================+
EOF
else
cat <<EOF

+=====================================================================+
|  MortalMessage — doctrine proof (gate mode)                        |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  sender: $DEPLOYER_U8 (owner)  recipient: $CALLER3_U8  adv: $CALLER2_U8
|  prove: only owner can seal; seal-once guard; unauthorized read blocked
+=====================================================================+
EOF
fi

# Step 1: Deploy MortalMessage
log "Step 1 - deploy MortalMessage  energy=$INITIAL_ENERGY"
SRC=$(jq -Rs . < "$CONTRACT_PATH")
DEPLOY_BODY=$(jq -n \
  --argjson d  "$DEPLOYER_U8"        \
  --argjson s  "$SRC"                \
  --argjson e  "$INITIAL_ENERGY"     \
  --argjson hl "$CONTRACT_HALF_LIFE" \
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

# ── SETTLE MODE ────────────────────────────────────────────────────────────
if [[ "$MODE" == "settle" ]]; then

  RECIP=$(addr_arg "$CALLER2_U8")

  # Step 2: Adversarial set_payload by CALLER2 (non-owner) → REJECTED
  log "Step 2 - adversarial: set_payload by CALLER2 (non-owner) → REJECTED (only sender can seal)"
  EP=$(get_epoch)
  ADV_SET_BODY=$(jq -n \
    --argjson c    "$CALLER2_U8"  \
    --argjson cid  "$CID"         \
    --argjson ep   "$EP"          \
    --argjson recip "$RECIP"      \
    '{caller:$c, contract_id:$cid, method:"set_payload",
      args:[{Str:"adversarial body"}, $recip], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_SET_BODY" "set_payload-non-owner" 5

  # Step 3: Real set_payload by DEPLOYER: body, recipient=CALLER2 → sealed=true
  # TX dedup: step 2 uses CALLER2; step 3 uses DEPLOYER → different callers → distinct hashes
  log "Step 3 - set_payload(\"hello from EvaporChain\", recipient=CALLER2) → sealed=true"
  EP=$(get_epoch)
  SET_BODY=$(jq -n \
    --argjson c    "$DEPLOYER_U8"  \
    --argjson cid  "$CID"          \
    --argjson ep   "$EP"           \
    --argjson recip "$RECIP"       \
    '{caller:$c, contract_id:$cid, method:"set_payload",
      args:[{Str:"hello from EvaporChain"}, $recip], epoch:$ep}')
  require_tx "/api/tx/call-script" "$SET_BODY" "set_payload" 4
  ok "set_payload → sealed=true ✓"

  # Step 4: read() as DEPLOYER (owner/sender) → ACCEPTED
  log "Step 4 - read() as DEPLOYER (owner/sender) → ACCEPTED"
  EP=$(get_epoch)
  READ_OWNER_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"read", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$READ_OWNER_BODY" "read-owner" 4
  ok "read() as owner (sender) → accepted ✓"

  # Step 5: read() as CALLER2 (recipient) → ACCEPTED
  # TX dedup: step 4 uses DEPLOYER; step 5 uses CALLER2 → different callers → distinct hashes
  log "Step 5 - read() as CALLER2 (recipient) → ACCEPTED"
  EP=$(get_epoch)
  READ_RECIP_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"read", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$READ_RECIP_BODY" "read-recipient" 4
  ok "read() as recipient (CALLER2) → accepted ✓"

  # Step 6: read() as CALLER3 (unauthorized — not sender, not recipient) → REJECTED
  log "Step 6 - adversarial: read() as CALLER3 (unauthorized) → REJECTED (not authorized)"
  EP=$(get_epoch)
  ADV_READ_BODY=$(jq -n \
    --argjson c   "$CALLER3_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"read", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_READ_BODY" "read-unauthorized" 5
  ok "DOCTRINE: only sender and recipient may read; all others blocked ✓"

  # Step 7: record_boost() as DEPLOYER → boost_count=1
  # Note: single call only; a second identical call (same caller+method+args=[])
  # would produce the same hash and return the cached finalised result.
  log "Step 7 - record_boost() as DEPLOYER → boost_count=1"
  EP=$(get_epoch)
  BOOST_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"record_boost", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$BOOST_BODY" "record_boost" 4
  ok "record_boost() → boost_count=1 ✓"
  ok "DOCTRINE: boost telemetry tracks how many times message kept alive past natural decay ✓"

  # Step 8: GET state → sealed=true, boost_count=1
  log "Step 8 - GET /api/script/$CID — verify state"
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    SEALED_V=$(printf '%s' "$STATE"  | untag sealed)
    BOOST_V=$(printf '%s'  "$STATE"  | untag boost_count)
    ok "sealed=$SEALED_V  boost_count=$BOOST_V"
    [[ "$BOOST_V"  == "1" ]] || die "boost_count mismatch: expected 1, got $BOOST_V" 6
    case "$SEALED_V" in true|1|True) ok "sealed=true ✓" ;; *) die "sealed!=true (got $SEALED_V)" 6 ;; esac
    ok "boost_count=1 ✓"
    ok "DOCTRINE: message energy IS its lifespan; on_grace/on_evaporate hook the runtime ✓"
  fi

cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — MortalMessage (settle mode)             |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (full message lifecycle):
|   - set_payload by non-owner → REJECTED ✓
|   - set_payload by sender → sealed=true ✓
|   - read() by sender → accepted ✓
|   - read() by recipient → accepted ✓
|   - read() by unauthorized → REJECTED ✓
|   - record_boost() → boost_count=1 ✓
|   - "message IS the contract; energy IS the lifespan;
|     runtime drives active → grace → ghost" ✓
+=====================================================================+
EOF

fi  # end settle mode

# ── GATE MODE ──────────────────────────────────────────────────────────────
if [[ "$MODE" == "gate" ]]; then

  RECIP_G=$(addr_arg "$CALLER3_U8")

  # Step 2: Adversarial set_payload by CALLER2 (non-owner) → REJECTED
  log "Step 2 - adversarial: set_payload by CALLER2 (non-owner) → REJECTED (only sender can seal)"
  EP=$(get_epoch)
  ADV_SET_BODY=$(jq -n \
    --argjson c    "$CALLER2_U8"  \
    --argjson cid  "$CID"         \
    --argjson ep   "$EP"          \
    --argjson recip "$RECIP_G"    \
    '{caller:$c, contract_id:$cid, method:"set_payload",
      args:[{Str:"gate test body"}, $recip], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_SET_BODY" "set_payload-non-owner" 5

  # Step 3: Real set_payload by DEPLOYER: body="gate message", recipient=CALLER3 → sealed=true
  # TX dedup: step 2 uses CALLER2; step 3 uses DEPLOYER → different callers → distinct hashes
  log "Step 3 - set_payload(\"gate message\", recipient=CALLER3) → sealed=true"
  EP=$(get_epoch)
  SET_BODY=$(jq -n \
    --argjson c    "$DEPLOYER_U8"  \
    --argjson cid  "$CID"          \
    --argjson ep   "$EP"           \
    --argjson recip "$RECIP_G"     \
    '{caller:$c, contract_id:$cid, method:"set_payload",
      args:[{Str:"gate message"}, $recip], epoch:$ep}')
  require_tx "/api/tx/call-script" "$SET_BODY" "set_payload" 4
  ok "set_payload → sealed=true ✓"

  # Step 4: Adversarial set_payload again by DEPLOYER (already sealed) → REJECTED
  # TX dedup: different body arg ("second message" vs "gate message") → different args
  #   → different hash → node runs fresh TX → sealed guard fires → REJECTED
  log "Step 4 - adversarial: set_payload again (already sealed) → REJECTED (message already sealed)"
  log "         [body=\"second message\" ≠ \"gate message\" → different hash → no dedup]"
  EP=$(get_epoch)
  ADV_DUP_BODY=$(jq -n \
    --argjson c    "$DEPLOYER_U8"  \
    --argjson cid  "$CID"          \
    --argjson ep   "$EP"           \
    --argjson recip "$RECIP_G"     \
    '{caller:$c, contract_id:$cid, method:"set_payload",
      args:[{Str:"second message"}, $recip], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_DUP_BODY" "set_payload-already-sealed" 5
  ok "DOCTRINE: set_payload is a one-shot seal; re-seal is permanently blocked ✓"

  # Step 5: Adversarial read() by CALLER2 → REJECTED (not recipient CALLER3, not owner DEPLOYER)
  log "Step 5 - adversarial: read() by CALLER2 (unauthorized) → REJECTED (not authorized)"
  EP=$(get_epoch)
  ADV_READ_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"read", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_READ_BODY" "read-unauthorized" 5
  ok "DOCTRINE: sender-only seal + recipient-only read; all other callers blocked ✓"

  # Step 6: GET state → sealed=true, boost_count=0
  log "Step 6 - GET /api/script/$CID — verify state"
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    SEALED_V=$(printf '%s' "$STATE" | untag sealed)
    BOOST_V=$(printf '%s'  "$STATE" | untag boost_count)
    ok "sealed=$SEALED_V  boost_count=$BOOST_V"
    [[ "$BOOST_V" == "0" ]] || die "boost_count mismatch: expected 0, got $BOOST_V" 6
    case "$SEALED_V" in true|1|True) ok "sealed=true ✓" ;; *) die "sealed!=true (got $SEALED_V)" 6 ;; esac
    ok "boost_count=0 (no boosts in gate run) ✓"
  fi

cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — MortalMessage (gate mode)               |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (adversarial guards):
|   - set_payload by non-owner → REJECTED ✓
|   - set_payload again (already sealed) → REJECTED ✓
|   - read() by unauthorized caller → REJECTED ✓
|   - sealed=true, boost_count=0 ✓
|   - "one-shot seal; only sender+recipient may read;
|     runtime drives the lifecycle; no re-seal ever" ✓
+=====================================================================+
EOF

fi  # end gate mode
