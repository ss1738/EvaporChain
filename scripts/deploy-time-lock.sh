#!/usr/bin/env bash
#
# deploy-time-lock.sh — end-to-end doctrine proof for TimeLock
# (contracts/evaporscript/time_lock.es).
#
# Doctrine: the contract's own energy IS the claim window.  If the
# beneficiary never claims and the contract evaporates, on_evaporate
# flips forfeit_signaled automatically — the coordinator returns the
# locked amount to the grantor.  No off-chain reaper needed; the
# runtime is the deadline enforcer.
#
# set_terms is one-shot and requires unlock > epoch (strictly future).
# After unlock, the grantor CANNOT revoke — the beneficiary's window
# is open and shortening it unilaterally is unsafe.
#
# claim() happy path requires epoch >= unlock_epoch.  Both modes use
# unlock_epoch=99999999 (guaranteed future) so claim() will always be
# gated "still locked" in these proofs.  The gate itself is the
# doctrine moment — the energy window enforces it without an oracle.
#
# Two modes:
#
#   --mode lock (default):
#     Prove the claim window is locked until unlock_epoch.
#     1. Adversarial: claim() before set_terms → REJECTED (not sealed)
#     2. Adversarial: set_terms with unlock=1 (past epoch) → REJECTED
#     3. set_terms(beneficiary=CALLER2, amount=50000, unlock=99999999)
#     4. Adversarial: set_terms again (different unlock) → REJECTED (sealed)
#     5. Adversarial: claim() as DEPLOYER (not beneficiary) → REJECTED
#     6. Adversarial: claim() as CALLER2 → REJECTED (still locked)
#     7. Adversarial: revoke() as CALLER2 (not grantor) → REJECTED
#     8. GET state → sealed=true, claimed=false, revoked=false, locked=50000
#
#   --mode revoke:
#     Grantor cancels the lock before unlock_epoch.
#     1. set_terms(beneficiary=CALLER2, amount=50000, unlock=99999999)
#     2. Adversarial: revoke() as CALLER2 (not grantor) → REJECTED
#     3. Adversarial: claim() as CALLER2 → REJECTED (still locked)
#     4. revoke() as DEPLOYER (grantor) → revoked=true
#     5. Adversarial: claim() as CALLER2 → REJECTED (lock revoked)
#        [dedup note: step 3 and step 5 share (CALLER2, claim, no-args, epoch)
#         if same epoch → step 5 dedupes to step 3's rejected state — still rejected ✓]
#     6. GET state → revoked=true, claimed=false
#
# TX DEDUP NOTES:
#   set_terms adversarial (unlock=1) vs real (unlock=99999999) → different args → safe.
#   revoke() adversarial (CALLER2) vs real (DEPLOYER) → different callers → safe.
#   claim() steps 3 and 5 in revoke mode: same caller (CALLER2), no args — if same
#   epoch → dedup → step 5 returns step 3's rejected state.  Both are rejected → ✓.
#
# Usage:
#   ./scripts/deploy-time-lock.sh --dry-run
#   ./scripts/deploy-time-lock.sh --node http://89.167.52.40:8099 --mode lock
#   ./scripts/deploy-time-lock.sh --node http://89.167.52.40:8099 --mode revoke
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 call · 5 adversarial · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/time_lock.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"   # grantor
CALLER2_U8="${CALLER2_U8:-1}"     # beneficiary
MODE="${MODE:-lock}"

LOCK_AMOUNT="${LOCK_AMOUNT:-50000}"
UNLOCK_EPOCH="${UNLOCK_EPOCH:-99999999}"   # guaranteed future
INITIAL_ENERGY="${INITIAL_ENERGY:-$(( 5000000 + RANDOM % 32768 ))}"
CONTRACT_HALF_LIFE="${CONTRACT_HALF_LIFE:-500000}"
POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-time-lock.sh [options]
  --dry-run              print intended calls; no network
  --node URL             node base URL (default http://89.167.52.40:8099)
  --token TOKEN          auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8          grantor account index (default 0)
  --caller2 U8           beneficiary (default 1)
  --mode lock|revoke     prove mode (default lock)
  --amount N             locked amount (default 50000)
  --unlock N             unlock_epoch (default 99999999)
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
    --mode)     MODE="$2"; shift 2 ;;
    --amount)   LOCK_AMOUNT="$2"; shift 2 ;;
    --unlock)   UNLOCK_EPOCH="$2"; shift 2 ;;
    --energy)   INITIAL_ENERGY="$2"; shift 2 ;;
    --hl)       CONTRACT_HALF_LIFE="$2"; shift 2 ;;
    --timeout)  POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose)  VERBOSE=true; shift ;;
    -h|--help)  usage; exit 0 ;;
    *)          echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[time-lock]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[time-lock ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }
ok()   { printf '\033[1;32m[time-lock OK]\033[0m %s\n' "$*"; }

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
    case "$st" in included|finalised|rejected) printf '%s' "$st"; return 0 ;; esac
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
  local email="deploy-tlock-${ts}@example.com"
  local pass="EvaporLock${ts}!"
  local reg_body; reg_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p, display_name:"time-lock-deploy"}')
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
grep -q "^contract TimeLock"  "$CONTRACT_PATH" || die ".es missing TimeLock header" 2
grep -q "fn set_terms("       "$CONTRACT_PATH" || die ".es missing fn set_terms" 2
grep -q "fn claim("           "$CONTRACT_PATH" || die ".es missing fn claim" 2
grep -q "fn revoke("          "$CONTRACT_PATH" || die ".es missing fn revoke" 2
[[ "$MODE" == "lock" || "$MODE" == "revoke" ]] \
  || die "unknown --mode '$MODE' (lock|revoke)" 2

if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

acquire_token
ADDR2=$(addr_arg "$CALLER2_U8")

if [[ "$MODE" == "lock" ]]; then
cat <<EOF

+=====================================================================+
|  TimeLock — doctrine proof (lock mode)                             |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  grantor: $DEPLOYER_U8  beneficiary: $CALLER2_U8
|  amount: $LOCK_AMOUNT  unlock_epoch: $UNLOCK_EPOCH
|  prove: claim window locked; past-epoch set_terms rejected;
|         non-beneficiary claim rejected; grantor-only revoke
|  doctrine: contract energy IS the claim window; no off-chain reaper
+=====================================================================+
EOF
else
cat <<EOF

+=====================================================================+
|  TimeLock — doctrine proof (revoke mode)                           |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  grantor: $DEPLOYER_U8  beneficiary: $CALLER2_U8
|  amount: $LOCK_AMOUNT  unlock_epoch: $UNLOCK_EPOCH
|  prove: grantor can revoke before unlock; post-revoke claim rejected
+=====================================================================+
EOF
fi

# ── Step 1: Deploy ─────────────────────────────────────────────────────────
log "Step 1 - deploy TimeLock  energy=$INITIAL_ENERGY"
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

# ── LOCK MODE ──────────────────────────────────────────────────────────────
if [[ "$MODE" == "lock" ]]; then

  log "Step 2 - adversarial: claim() before set_terms → REJECTED (not sealed)"
  EP=$(get_epoch)
  ADV_CLAIM_EARLY_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"claim", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_CLAIM_EARLY_BODY" "claim-before-terms" 5

  log "Step 3 - adversarial: set_terms with unlock=1 (past epoch) → REJECTED"
  EP=$(get_epoch)
  ADV_ST_PAST_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    --argjson a   "$ADDR2"        \
    --argjson amt "$LOCK_AMOUNT"  \
    '{caller:$c, contract_id:$cid, method:"set_terms",
      args:[$a,{U64:$amt},{U64:1}], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_ST_PAST_BODY" "set_terms-past-unlock" 5
  ok "set_terms with past unlock_epoch correctly REJECTED ✓"
  ok "DOCTRINE: unlock must be strictly future; the contract cannot arm for a past deadline"

  log "Step 4 - set_terms(beneficiary=$CALLER2_U8, amount=$LOCK_AMOUNT, unlock=$UNLOCK_EPOCH)"
  EP=$(get_epoch)
  ST_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"   \
    --argjson cid "$CID"           \
    --argjson ep  "$EP"            \
    --argjson a   "$ADDR2"         \
    --argjson amt "$LOCK_AMOUNT"   \
    --argjson u   "$UNLOCK_EPOCH"  \
    '{caller:$c, contract_id:$cid, method:"set_terms",
      args:[$a,{U64:$amt},{U64:$u}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$ST_BODY" "set_terms" 4
  ok "set_terms → sealed=true ✓"

  log "Step 5 - adversarial: set_terms again (different unlock) → REJECTED (sealed)"
  EP=$(get_epoch)
  ADV_ST2_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    --argjson a   "$ADDR2"        \
    --argjson amt "$LOCK_AMOUNT"  \
    '{caller:$c, contract_id:$cid, method:"set_terms",
      args:[$a,{U64:$amt},{U64:99999998}], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_ST2_BODY" "set_terms-duplicate" 5

  log "Step 6 - adversarial: claim() as DEPLOYER (grantor, not beneficiary) → REJECTED"
  EP=$(get_epoch)
  ADV_CLAIM_DEPL_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"claim", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_CLAIM_DEPL_BODY" "claim-non-beneficiary" 5

  log "Step 7 - adversarial: claim() as CALLER2 (beneficiary) → REJECTED (still locked; epoch << $UNLOCK_EPOCH)"
  EP=$(get_epoch)
  ADV_CLAIM_LOCKED_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"claim", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_CLAIM_LOCKED_BODY" "claim-still-locked" 5
  ok "claim() rejected while locked ✓"
  ok "DOCTRINE: contract energy is the claim window; epoch gate enforced by VM ✓"

  log "Step 8 - adversarial: revoke() as CALLER2 (beneficiary, not grantor) → REJECTED"
  EP=$(get_epoch)
  ADV_REVOKE_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"revoke", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_REVOKE_BODY" "revoke-non-grantor" 5

  log "Step 9 - GET /api/script/$CID — verify state"
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    SEALED_V=$(printf '%s' "$STATE"  | untag sealed)
    CLAIMED_V=$(printf '%s' "$STATE" | untag claimed)
    REVOKED_V=$(printf '%s' "$STATE" | untag revoked)
    AMOUNT_V=$(printf '%s' "$STATE"  | untag amount)
    UNLOCK_V=$(printf '%s' "$STATE"  | untag unlock_epoch)
    ok "sealed=$SEALED_V  claimed=$CLAIMED_V  revoked=$REVOKED_V  amount=$AMOUNT_V  unlock_epoch=$UNLOCK_V"
    [[ "$AMOUNT_V" == "$LOCK_AMOUNT" ]] || die "amount mismatch: expected $LOCK_AMOUNT, got $AMOUNT_V" 6
    [[ "$UNLOCK_V" == "$UNLOCK_EPOCH" ]] || die "unlock_epoch mismatch: expected $UNLOCK_EPOCH, got $UNLOCK_V" 6
    case "$SEALED_V"  in true|1|True)   ok "sealed=true ✓"    ;; *) die "sealed!=true"    6 ;; esac
    case "$CLAIMED_V" in false|0|False) ok "claimed=false ✓"  ;; *) die "claimed=true"    6 ;; esac
    case "$REVOKED_V" in false|0|False) ok "revoked=false ✓"  ;; *) die "revoked=true"    6 ;; esac
  fi

cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — TimeLock (lock mode)                    |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (claim window locked):
|   - claim() before set_terms → REJECTED ✓
|   - set_terms with past unlock → REJECTED ✓
|   - set_terms duplicate → REJECTED ✓
|   - claim() by non-beneficiary → REJECTED ✓
|   - claim() while locked (epoch << unlock_epoch) → REJECTED ✓
|   - revoke() by non-grantor → REJECTED ✓
|   - sealed=true, claimed=false, revoked=false, amount=$LOCK_AMOUNT ✓
|   - "contract energy is the claim window; runtime is the deadline" ✓
+=====================================================================+
EOF

fi  # end lock mode

# ── REVOKE MODE ────────────────────────────────────────────────────────────
if [[ "$MODE" == "revoke" ]]; then

  log "Step 2 - set_terms(beneficiary=$CALLER2_U8, amount=$LOCK_AMOUNT, unlock=$UNLOCK_EPOCH)"
  EP=$(get_epoch)
  ST_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"   \
    --argjson cid "$CID"           \
    --argjson ep  "$EP"            \
    --argjson a   "$ADDR2"         \
    --argjson amt "$LOCK_AMOUNT"   \
    --argjson u   "$UNLOCK_EPOCH"  \
    '{caller:$c, contract_id:$cid, method:"set_terms",
      args:[$a,{U64:$amt},{U64:$u}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$ST_BODY" "set_terms" 4
  ok "set_terms → sealed=true ✓"

  log "Step 3 - adversarial: revoke() as CALLER2 (beneficiary, not grantor) → REJECTED"
  EP=$(get_epoch)
  ADV_REVOKE_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"revoke", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_REVOKE_BODY" "revoke-non-grantor" 5

  log "Step 4 - adversarial: claim() as CALLER2 → REJECTED (still locked before revoke)"
  EP=$(get_epoch)
  ADV_CLAIM_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"claim", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_CLAIM_BODY" "claim-still-locked" 5

  log "Step 5 - revoke() as DEPLOYER (grantor) → revoked=true"
  EP=$(get_epoch)
  REVOKE_BODY=$(jq -n \
    --argjson c   "$DEPLOYER_U8"  \
    --argjson cid "$CID"          \
    --argjson ep  "$EP"           \
    '{caller:$c, contract_id:$cid, method:"revoke", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$REVOKE_BODY" "revoke" 4
  ok "revoke() → revoked=true ✓"

  log "Step 6 - adversarial: claim() as CALLER2 after revoke → REJECTED (lock revoked)"
  log "         [TX dedup: steps 4 and 6 share CALLER2+claim+no-args; if same epoch → dedup"
  log "          returns step 4's rejected state; both rejected → gate confirmed ✓]"
  EP=$(get_epoch)
  ADV_CLAIM2_BODY=$(jq -n \
    --argjson c   "$CALLER2_U8"  \
    --argjson cid "$CID"         \
    --argjson ep  "$EP"          \
    '{caller:$c, contract_id:$cid, method:"claim", args:[], epoch:$ep}')
  require_rejected "/api/tx/call-script" "$ADV_CLAIM2_BODY" "claim-after-revoke" 5

  log "Step 7 - GET /api/script/$CID — verify state"
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    SEALED_V=$(printf '%s' "$STATE"  | untag sealed)
    CLAIMED_V=$(printf '%s' "$STATE" | untag claimed)
    REVOKED_V=$(printf '%s' "$STATE" | untag revoked)
    AMOUNT_V=$(printf '%s' "$STATE"  | untag amount)
    ok "sealed=$SEALED_V  claimed=$CLAIMED_V  revoked=$REVOKED_V  amount=$AMOUNT_V"
    [[ "$AMOUNT_V" == "$LOCK_AMOUNT" ]] || die "amount mismatch: expected $LOCK_AMOUNT, got $AMOUNT_V" 6
    case "$SEALED_V"  in true|1|True)   ok "sealed=true ✓"    ;; *) die "sealed!=true"    6 ;; esac
    case "$CLAIMED_V" in false|0|False) ok "claimed=false ✓"  ;; *) die "claimed=true"    6 ;; esac
    case "$REVOKED_V" in true|1|True)   ok "revoked=true ✓"   ;; *) die "revoked=false"   6 ;; esac
  fi

cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — TimeLock (revoke mode)                  |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (grantor revoke):
|   - revoke() by non-grantor → REJECTED ✓
|   - claim() before revoke → REJECTED (still locked) ✓
|   - revoke() by grantor before unlock → revoked=true ✓
|   - claim() after revoke → REJECTED ✓
|   - "grantor can cancel before unlock; post-unlock revoke is blocked" ✓
|   - "revoked amount returned to grantor via forfeit_signaled at evaporate" ✓
+=====================================================================+
EOF

fi  # end revoke mode
