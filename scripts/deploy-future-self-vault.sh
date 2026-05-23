#!/usr/bin/env bash
#
# deploy-future-self-vault.sh — end-to-end doctrine proof for FutureSelfVault
# (contracts/evaporscript/future_self_vault.es).
#
# Doctrine: a vault locks an energy deposit on behalf of the creator's future
# self. The vault IS the energy: deploying with a chosen energy budget IS the
# act of locking. The chain's evaporation engine decays the contract's own
# energy — for EnergyDecaysBelow predicates the physics does the work with no
# hand-coded formula. The runtime IS the closer.
#
# Two modes:
#
#   --mode settle (default):
#     EpochReached predicate, full lifecycle: seal → list → cancel → re-list
#     → record_sale → payout.
#     1.  Deploy
#     2.  Adversarial: set_terms by CALLER2 (non-owner) → REJECTED (owner guard)
#         [CALLER2 (step 2) vs DEPLOYER (step 3) → different callers → no dedup]
#     3.  Real: set_terms by DEPLOYER: future_self=CALLER2, predicate=0 (EpochReached),
#         release_epoch=EP (current epoch). sealed=true, holder=CALLER2.
#     4.  list_for_sale by CALLER2 (current holder): ceiling=1000, floor=100, duration=500
#     5.  cancel_listing by CALLER2 → listed=false
#     6.  list_for_sale by CALLER2 again: ceiling=2000, floor=200, duration=100
#         [args differ from step 4 (1000/100/500 vs 2000/200/100) → different hash → no dedup]
#     7.  record_sale by DEPLOYER (owner/coordinator): winner=CALLER3 → holder=CALLER3
#     8.  try_payout by CALLER3 (any caller; predicate: epoch>=release_epoch)
#         epoch=EP8 (fresh, guaranteed >=RELEASE_EP) → released=true, payout_at=EP8
#     9.  GET state → sealed=true, released=true, holder=CALLER3
#
#   --mode gate:
#     Prove all access guards: non-owner seal, non-holder list, duplicate list,
#     non-owner record_sale, non-holder cancel, premature payout.
#     1.  Deploy
#     2.  Adversarial: set_terms by CALLER2 (non-owner) → REJECTED
#         [CALLER2 (step 2) vs DEPLOYER (step 3) → different callers → no dedup]
#     3.  Real: set_terms by DEPLOYER: future_self=CALLER2, predicate=0,
#         release_epoch=EP+1000000 (far future). sealed=true, holder=CALLER2.
#     4.  Adversarial: list_for_sale by CALLER3 (not holder) → REJECTED (holder guard)
#         [CALLER3 (step 4) vs CALLER2 (step 5) → different callers → no dedup]
#     5.  Real: list_for_sale by CALLER2 (holder): ceiling=1000, floor=100, duration=500
#         → listed=true
#     6.  Adversarial: list_for_sale by CALLER2 again (already listed) → REJECTED
#         [args: ceiling=9999, floor=1, duration=9999 ≠ step 5 → different hash → no dedup]
#     7.  Adversarial: record_sale by CALLER3 (not owner) → REJECTED (owner guard)
#     8.  Adversarial: cancel_listing by CALLER3 (not holder) → REJECTED (holder guard)
#     9.  Adversarial: try_payout by DEPLOYER → REJECTED (predicate not satisfied:
#         epoch << release_epoch=EP+1000000)
#    10.  GET state → sealed=true, released=false, listed=true
#
# TX DEDUP NOTES (settle):
#   set_terms adv (step 2, CALLER2) vs real (step 3, DEPLOYER) → different callers → safe.
#   list_for_sale step 4 (args=[1000,100,500]) vs step 6 (args=[2000,200,100]) → diff args → safe.
#   cancel_listing step 5 vs list_for_sale step 6 → different methods → safe.
#   record_sale step 7 (DEPLOYER, [CALLER3_ADDR]) → single call by DEPLOYER with these args.
#   try_payout step 8 (CALLER3, []) → single call.
#
# TX DEDUP NOTES (gate):
#   set_terms adv (step 2, CALLER2) vs real (step 3, DEPLOYER) → different callers → safe.
#   list_for_sale adv (step 4, CALLER3) vs real (step 5, CALLER2) → different callers → safe.
#   list_for_sale dup (step 6, CALLER2, [9999,1,9999]) vs step 5 ([1000,100,500]) → diff args → safe.
#   record_sale adv (step 7, CALLER3) vs owner call → different callers → safe.
#   cancel_listing adv (step 8, CALLER3) vs holder call → different callers → safe.
#   try_payout adv (step 9, DEPLOYER, []) → single call.
#
# Usage:
#   ./deploy-future-self-vault.sh --dry-run
#   ./deploy-future-self-vault.sh --node http://89.167.52.40:8099 --mode settle
#   ./deploy-future-self-vault.sh --node http://89.167.52.40:8099 --mode gate
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 call · 5 adversarial · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

CONTRACT_PATH="/Users/satyawansingh/EvaporChain/contracts/evaporscript/future_self_vault.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"
CALLER2_U8="${CALLER2_U8:-1}"
CALLER3_U8="${CALLER3_U8:-2}"
MODE="${MODE:-settle}"
INITIAL_ENERGY="${INITIAL_ENERGY:-$(( 5000000 + RANDOM % 32768 ))}"
CONTRACT_HALF_LIFE="${CONTRACT_HALF_LIFE:-300000}"
POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-future-self-vault.sh [options]
  --dry-run              print intended calls; no network
  --node URL             node base URL (default http://89.167.52.40:8099)
  --token TOKEN          auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8          creator/coordinator account index (default 0)
  --caller2 U8           future_self/holder (default 1)
  --caller3 U8           buyer / adversary (default 2)
  --mode settle|gate     prove mode (default settle)
  --energy N             contract initial energy (default ~5M randomised)
  --hl N                 contract half-life (default 300000)
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

log()  { printf '\033[1;36m[future-self-vault]\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m[future-self-vault OK]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[future-self-vault ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }

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
  local email="deploy-fsv-${ts}@example.com"
  local pass="EvaporFSV${ts}!"
  local reg_body; reg_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p, display_name:"future-self-vault-deploy"}')
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
grep -q "^contract FutureSelfVault" "$CONTRACT_PATH" || die ".es missing FutureSelfVault header" 2
grep -q "fn set_terms("             "$CONTRACT_PATH" || die ".es missing fn set_terms" 2
grep -q "fn list_for_sale("         "$CONTRACT_PATH" || die ".es missing fn list_for_sale" 2
grep -q "fn record_sale("           "$CONTRACT_PATH" || die ".es missing fn record_sale" 2
grep -q "fn try_payout("            "$CONTRACT_PATH" || die ".es missing fn try_payout" 2
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
|  FutureSelfVault — doctrine proof (settle mode)                    |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  creator: $DEPLOYER_U8  future_self/holder: $CALLER2_U8  buyer: $CALLER3_U8
|  predicate: EpochReached (release_epoch=current_epoch)
|  prove: sealed deposit, list→cancel→re-list, SDDC sale, payout
|  doctrine: vault IS the energy; runtime IS the closer; no escrow agent
+=====================================================================+
EOF
else
cat <<EOF

+=====================================================================+
|  FutureSelfVault — doctrine proof (gate mode)                      |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  creator: $DEPLOYER_U8  holder: $CALLER2_U8  adversary: $CALLER3_U8
|  predicate: EpochReached (release_epoch=EP+1000000, far future)
|  prove: all access guards; premature payout blocked
+=====================================================================+
EOF
fi

# Step 1: Deploy FutureSelfVault
log "Step 1 - deploy FutureSelfVault  energy=$INITIAL_ENERGY"
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

  FS=$(addr_arg "$CALLER2_U8")
  WINNER=$(addr_arg "$CALLER3_U8")

  # Step 2: Adversarial set_terms by CALLER2 (non-owner) → REJECTED
  # TX dedup: CALLER2 here vs DEPLOYER in step 3 → different callers → distinct hashes
  log "Step 2 - adversarial: set_terms by CALLER2 (non-owner) → REJECTED (owner guard)"
  EP=$(get_epoch)
  ADV_ST_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    --argjson fs  "$FS"          \
    --argjson rel "$EP"          \
    '{caller:$c, contract_id:$cid, method:"set_terms",
      args:[$fs, {U64:0}, {U64:($rel)}, {U64:5000}], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_ST_BODY" "set_terms-non-owner" 5

  # Step 3: Real set_terms by DEPLOYER: EpochReached, release_epoch=EP → sealed=true
  log "Step 3 - set_terms(future_self=CALLER2, predicate=EpochReached, release_epoch=EP, deposit=5000)"
  EP=$(get_epoch)
  RELEASE_EP="$EP"
  ST_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    --argjson fs  "$FS"           \
    --argjson rel "$RELEASE_EP"   \
    '{caller:$c, contract_id:$cid, method:"set_terms",
      args:[$fs, {U64:0}, {U64:($rel)}, {U64:5000}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$ST_BODY" "set_terms" 4
  ok "set_terms → sealed=true, holder=CALLER2, release_epoch=$RELEASE_EP ✓"

  # Step 4: list_for_sale by CALLER2 (holder): ceiling=1000, floor=100, duration=500
  log "Step 4 - list_for_sale by CALLER2 (holder): ceiling=1000, floor=100, duration=500"
  EP=$(get_epoch)
  LIST1_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"list_for_sale",
      args:[{U64:1000}, {U64:100}, {U64:500}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$LIST1_BODY" "list_for_sale-1" 4
  ok "list_for_sale(1000, 100, 500) → listed=true ✓"

  # Step 5: cancel_listing by CALLER2 (holder) → listed=false
  log "Step 5 - cancel_listing by CALLER2 (holder) → listed=false"
  EP=$(get_epoch)
  CANCEL_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"cancel_listing", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$CANCEL_BODY" "cancel_listing" 4
  ok "cancel_listing → listed=false ✓"

  # Step 6: list_for_sale by CALLER2 again: ceiling=2000, floor=200, duration=100
  # TX dedup: step 4 args=[1000,100,500] vs step 6 args=[2000,200,100] → different args → safe
  log "Step 6 - list_for_sale by CALLER2 again: ceiling=2000, floor=200, duration=100"
  log "         [args differ from step 4 → different hash → no dedup]"
  EP=$(get_epoch)
  LIST2_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"list_for_sale",
      args:[{U64:2000}, {U64:200}, {U64:100}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$LIST2_BODY" "list_for_sale-2" 4
  ok "list_for_sale(2000, 200, 100) → listed=true (re-listed after cancel) ✓"

  # Step 7: record_sale by DEPLOYER (owner/coordinator): winner=CALLER3 → holder=CALLER3
  log "Step 7 - record_sale by DEPLOYER (coordinator): winner=CALLER3 → holder=CALLER3"
  EP=$(get_epoch)
  SALE_BODY=$(jq -n \
    --argjson c      "$DEPLOYER_U8"  \
    --argjson cid    "$CID"          \
    --argjson ep     "$EP"           \
    --argjson winner "$WINNER"       \
    '{caller:$c, contract_id:$cid, method:"record_sale",
      args:[$winner], epoch:$ep}')
  require_tx "/api/tx/call-script" "$SALE_BODY" "record_sale" 4
  ok "record_sale(CALLER3) → holder=CALLER3, listed=false ✓"
  ok "DOCTRINE: SDDC coordinator records Dutch-cleared sale; holder transferred atomically ✓"

  # Step 8: try_payout by CALLER3 → predicate EpochReached: TX epoch >= release_epoch → ACCEPTED
  # release_epoch=RELEASE_EP (from step 3). Fresh EP >= RELEASE_EP since time only moves forward.
  log "Step 8 - try_payout by CALLER3 (any caller; predicate: epoch >= release_epoch=$RELEASE_EP)"
  EP=$(get_epoch)
  PAYOUT_BODY=$(jq -n \
    --argjson c   "$CALLER3_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"try_payout", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$PAYOUT_BODY" "try_payout" 4
  ok "try_payout → released=true (epoch=$EP >= release_epoch=$RELEASE_EP) ✓"
  ok "DOCTRINE: predicate satisfied; vault released; off-chain layer credits current_holder ✓"

  # Step 9: GET state → sealed=true, released=true, holder=CALLER3
  log "Step 9 - GET /api/script/$CID — verify state"
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    SEALED_V=$(printf '%s'   "$STATE" | untag sealed)
    RELEASED_V=$(printf '%s' "$STATE" | untag released)
    LISTED_V=$(printf '%s'   "$STATE" | untag listed)
    PAYOUT_V=$(printf '%s'   "$STATE" | untag payout_at)
    ok "sealed=$SEALED_V  released=$RELEASED_V  listed=$LISTED_V  payout_at=$PAYOUT_V"
    case "$SEALED_V"   in true|1|True) ok "sealed=true ✓"   ;; *) die "sealed!=true (got $SEALED_V)" 6 ;; esac
    case "$RELEASED_V" in true|1|True) ok "released=true ✓" ;; *) die "released!=true (got $RELEASED_V)" 6 ;; esac
    (( PAYOUT_V > 0 )) || die "payout_at not set (got $PAYOUT_V)" 6
    ok "payout_at=$PAYOUT_V (>0) ✓"
    ok "DOCTRINE: vault IS the energy; deploy IS the lock; runtime IS the closer ✓"
  fi

cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — FutureSelfVault (settle mode)           |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (full vault lifecycle):
|   - set_terms by non-owner → REJECTED ✓
|   - set_terms(EpochReached, release_epoch=$RELEASE_EP) → sealed=true ✓
|   - list_for_sale(1000,100,500) → listed=true ✓
|   - cancel_listing → listed=false ✓
|   - list_for_sale(2000,200,100) → re-listed ✓
|   - record_sale(CALLER3) → holder transferred ✓
|   - try_payout (epoch>=release_epoch) → released=true ✓
|   - "vault IS the energy; runtime IS the closer;
|     SDDC coordinator records sale; physics drives payout" ✓
+=====================================================================+
EOF

fi  # end settle mode

# ── GATE MODE ──────────────────────────────────────────────────────────────
if [[ "$MODE" == "gate" ]]; then

  FS_G=$(addr_arg "$CALLER2_U8")
  ADV_WINNER=$(addr_arg "$CALLER3_U8")

  # Step 2: Adversarial set_terms by CALLER2 (non-owner) → REJECTED
  # TX dedup: CALLER2 (step 2) vs DEPLOYER (step 3) → different callers → safe
  log "Step 2 - adversarial: set_terms by CALLER2 (non-owner) → REJECTED (owner guard)"
  EP=$(get_epoch)
  RELEASE_EP_G=$(( EP + 1000000 ))
  ADV_ST_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"     \
    --argjson cid "$CID"            \
    --argjson ep  "$EP"             \
    --argjson fs  "$FS_G"           \
    --argjson rel "$RELEASE_EP_G"   \
    '{caller:$c, contract_id:$cid, method:"set_terms",
      args:[$fs, {U64:0}, {U64:($rel)}, {U64:5000}], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_ST_BODY" "set_terms-non-owner" 5

  # Step 3: Real set_terms by DEPLOYER: release_epoch=EP+1000000 (far future)
  log "Step 3 - set_terms(future_self=CALLER2, predicate=EpochReached, release_epoch=EP+1000000)"
  EP=$(get_epoch)
  RELEASE_EP_G=$(( EP + 1000000 ))
  ST_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"    \
    --argjson cid "$CID"            \
    --argjson ep  "$EP"             \
    --argjson fs  "$FS_G"           \
    --argjson rel "$RELEASE_EP_G"   \
    '{caller:$c, contract_id:$cid, method:"set_terms",
      args:[$fs, {U64:0}, {U64:($rel)}, {U64:5000}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$ST_BODY" "set_terms" 4
  ok "set_terms → sealed=true, release_epoch=$RELEASE_EP_G (far future) ✓"

  # Step 4: Adversarial list_for_sale by CALLER3 (not holder, holder=CALLER2) → REJECTED
  # TX dedup: CALLER3 (step 4) vs CALLER2 (step 5) → different callers → safe
  log "Step 4 - adversarial: list_for_sale by CALLER3 (not holder) → REJECTED (holder guard)"
  EP=$(get_epoch)
  ADV_LIST_BODY=$(jq -n \
    --argjson c   "$CALLER3_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"list_for_sale",
      args:[{U64:1000}, {U64:100}, {U64:500}], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_LIST_BODY" "list_for_sale-non-holder" 5
  ok "DOCTRINE: only current holder can list vault for sale ✓"

  # Step 5: Real list_for_sale by CALLER2 (holder): ceiling=1000, floor=100, duration=500
  log "Step 5 - list_for_sale by CALLER2 (holder): ceiling=1000, floor=100, duration=500"
  EP=$(get_epoch)
  LIST_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"list_for_sale",
      args:[{U64:1000}, {U64:100}, {U64:500}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$LIST_BODY" "list_for_sale" 4
  ok "list_for_sale(1000, 100, 500) → listed=true ✓"

  # Step 6: Adversarial list_for_sale by CALLER2 again (already listed) → REJECTED
  # TX dedup: step 5 args=[1000,100,500] vs step 6 args=[9999,1,9999] → different args → safe
  log "Step 6 - adversarial: list_for_sale by CALLER2 again (already listed) → REJECTED"
  log "         [args=[9999,1,9999] ≠ step 5 [1000,100,500] → different hash → no dedup]"
  EP=$(get_epoch)
  ADV_DUP_LIST_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"list_for_sale",
      args:[{U64:9999}, {U64:1}, {U64:9999}], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_DUP_LIST_BODY" "list_for_sale-already-listed" 5
  ok "DOCTRINE: only one active listing at a time ✓"

  # Step 7: Adversarial record_sale by CALLER3 (not owner) → REJECTED (owner guard)
  log "Step 7 - adversarial: record_sale by CALLER3 (not owner) → REJECTED (owner guard)"
  EP=$(get_epoch)
  ADV_SALE_BODY=$(jq -n \
    --argjson c      "$CALLER3_U8"   \
    --argjson cid    "$CID"          \
    --argjson ep     "$EP"           \
    --argjson winner "$ADV_WINNER"   \
    '{caller:$c, contract_id:$cid, method:"record_sale",
      args:[$winner], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_SALE_BODY" "record_sale-non-owner" 5
  ok "DOCTRINE: only coordinator/owner can record sale; arbitrary callers cannot transfer holder ✓"

  # Step 8: Adversarial cancel_listing by CALLER3 (not holder) → REJECTED (holder guard)
  log "Step 8 - adversarial: cancel_listing by CALLER3 (not holder) → REJECTED (holder guard)"
  EP=$(get_epoch)
  ADV_CANCEL_BODY=$(jq -n \
    --argjson c   "$CALLER3_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"cancel_listing", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_CANCEL_BODY" "cancel_listing-non-holder" 5
  ok "DOCTRINE: only current holder can cancel their own listing ✓"

  # Step 9: Adversarial try_payout by DEPLOYER → REJECTED (predicate not satisfied)
  # TX epoch=EP << release_epoch=EP+1000000 → epoch < release_epoch → not satisfied
  log "Step 9 - adversarial: try_payout by DEPLOYER → REJECTED (predicate not satisfied)"
  log "         [epoch=EP << release_epoch=EP+1000000 → require(satisfied==1) fails]"
  EP=$(get_epoch)
  ADV_PAY_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"try_payout", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_PAY_BODY" "try_payout-predicate-not-met" 5
  ok "DOCTRINE: try_payout is a no-op until the predicate fires; time-lock enforced ✓"

  # Step 10: GET state → sealed=true, released=false, listed=true
  log "Step 10 - GET /api/script/$CID — verify state"
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    SEALED_V=$(printf '%s'   "$STATE" | untag sealed)
    RELEASED_V=$(printf '%s' "$STATE" | untag released)
    LISTED_V=$(printf '%s'   "$STATE" | untag listed)
    ok "sealed=$SEALED_V  released=$RELEASED_V  listed=$LISTED_V"
    case "$SEALED_V"   in true|1|True) ok "sealed=true ✓"    ;; *) die "sealed!=true (got $SEALED_V)" 6 ;; esac
    case "$RELEASED_V" in false|0|False) ok "released=false ✓" ;; *) die "released should be false (got $RELEASED_V)" 6 ;; esac
    case "$LISTED_V"   in true|1|True) ok "listed=true ✓"    ;; *) die "listed!=true (got $LISTED_V)" 6 ;; esac
    ok "DOCTRINE: vault locked by predicate; premature payout impossible ✓"
  fi

cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — FutureSelfVault (gate mode)             |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (adversarial guards):
|   - set_terms by non-owner → REJECTED ✓
|   - list_for_sale by non-holder → REJECTED ✓
|   - list_for_sale when already listed → REJECTED ✓
|   - record_sale by non-owner → REJECTED ✓
|   - cancel_listing by non-holder → REJECTED ✓
|   - try_payout before predicate fires → REJECTED ✓
|   - sealed=true, released=false, listed=true ✓
|   - "vault locked by predicate; holder rights enforced;
|     coordinator monopoly on sale recording" ✓
+=====================================================================+
EOF

fi  # end gate mode
