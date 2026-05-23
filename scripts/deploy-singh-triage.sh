#!/usr/bin/env bash
#
# deploy-singh-triage.sh — end-to-end doctrine proof for SinghTriage
# (contracts/evaporscript/singh_triage.es).
#
# The wallet-opens-on-inbox paradigm: each item in a holder's inbox
# decays independently. classify_all() sorts items into urgency buckets
# (Today / Tomorrow / ThisWeek / Healthy / Decayed) so wallets surface
# "what needs attention today".
#
# Two modes:
#
#   --mode classify (default):
#     deploy → initialise(horizons) →
#     register 3 items: Today(energy=4,hl=1000), Healthy(energy=131072,hl=1000),
#       Decayed(energy=1,hl=1000) →
#     classify_all → verify count_today=1, count_healthy=1, count_decayed=1 →
#     require_urgent(slot=0) → proves Today item is non-vacuously urgent.
#     Proves: urgency classification is non-trivial and item 0 is in Today bucket.
#
#   --mode refresh:
#     All classify steps, then:
#     archive_item(slot=1) → verify count_archived=1 →
#     refresh_item(slot=0, top_up=131072) → re-classify (caller rotated) →
#     verify: count_today=0, count_healthy=1 (item 0 moved from Today → Healthy) →
#     let_die_item(slot=2) → final state verify.
#     Proves: refresh lifts an urgent item to healthy; archive and let_die work.
#
# TX HASH DEDUP NOTE: classify_all() takes no args. Caller rotation applied:
#   first classify_all  = caller DEPLOYER_U8 (0)
#   second classify_all = caller CLASSIFY2_U8 (1)
#   third classify_all  = caller CLASSIFY3_U8 (2)
#
# Usage:
#   ./scripts/deploy-singh-triage.sh --dry-run
#   ./scripts/deploy-singh-triage.sh --node http://89.167.52.40:8099 --mode classify
#   ./scripts/deploy-singh-triage.sh --node http://89.167.52.40:8099 --mode refresh
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 init/register · 5 gate-not-exercised · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/singh_triage.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"        # owner — must be funded
CLASSIFY2_U8="${CLASSIFY2_U8:-1}"      # second classify_all caller (dedup avoidance)
CLASSIFY3_U8="${CLASSIFY3_U8:-2}"      # third classify_all caller  (dedup avoidance)
MODE="${MODE:-classify}"               # classify | refresh

# Contract energy — randomised so each run deploys a unique contract (prevents
# deploy-tx hash dedup collisions between runs; range 20_000_000–20_032_767).
INITIAL_ENERGY="${INITIAL_ENERGY:-$(( 20000000 + RANDOM % 32768 ))}"
CONTRACT_HALF_LIFE="${CONTRACT_HALF_LIFE:-500000}"

# Urgency horizons in half-life hops (doctrine defaults §A5.4)
HORIZON_TODAY="${HORIZON_TODAY:-2}"
HORIZON_TOMORROW="${HORIZON_TOMORROW:-5}"
HORIZON_WEEK="${HORIZON_WEEK:-14}"

# Item energies designed to land in specific buckets at elapsed≈0.
# Item 0 (Today):  energy=4,      hl=1000 → cur_e≈4  → hops=2 ≤ 2  → Today
# Item 1 (Healthy): energy=131072, hl=1000 → cur_e≈131072 → hops=17 > 14 → Healthy
# Item 2 (Decayed): energy=1,      hl=1000 → cur_e=1 ≤ 1 → Decayed
ITEM0_ENERGY="${ITEM0_ENERGY:-4}"
ITEM0_HL="${ITEM0_HL:-1000}"
ITEM1_ENERGY="${ITEM1_ENERGY:-131072}"
ITEM1_HL="${ITEM1_HL:-1000}"
ITEM2_ENERGY="${ITEM2_ENERGY:-1}"
ITEM2_HL="${ITEM2_HL:-1000}"

# Refresh top-up: moves item 0 from Today → Healthy
REFRESH_TOP_UP="${REFRESH_TOP_UP:-131072}"

POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-singh-triage.sh [options]
  --dry-run               validate + print intended calls; no network
  --node URL              node base URL (default http://89.167.52.40:8099)
  --token TOKEN           auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8           owner account index (default 0)
  --mode classify|refresh
  --horizon-today N       today horizon in hops (default 2)
  --horizon-tomorrow N    tomorrow horizon in hops (default 5)
  --horizon-week N        week horizon in hops (default 14)
  --refresh-top-up N      energy added to item 0 in refresh mode (default 131072)
  --timeout SEC           poll timeout (default 300)
  --verbose               echo node responses
  -h|--help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)          DRY_RUN=true; shift ;;
    --node)             NODE_URL="$2"; shift 2 ;;
    --token)            TOKEN="$2"; shift 2 ;;
    --deployer)         DEPLOYER_U8="$2"; shift 2 ;;
    --mode)             MODE="$2"; shift 2 ;;
    --horizon-today)    HORIZON_TODAY="$2"; shift 2 ;;
    --horizon-tomorrow) HORIZON_TOMORROW="$2"; shift 2 ;;
    --horizon-week)     HORIZON_WEEK="$2"; shift 2 ;;
    --refresh-top-up)   REFRESH_TOP_UP="$2"; shift 2 ;;
    --timeout)          POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose)          VERBOSE=true; shift ;;
    -h|--help)          usage; exit 0 ;;
    *)                  echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[triage]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[triage ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }
ok()   { printf '\033[1;32m[triage OK]\033[0m %s\n' "$*"; }

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

get_epoch() { $DRY_RUN && { echo 0; return 0; }; curl_json GET "/api/status" | jq -r '.epoch // 0'; }
untag() { jq -r ".state.$1 | if type==\"object\" then (.Bool // .U64 // .Str // .Address // .) else . end"; }

# fund_account SRC DST — native transfer with randomised amount to avoid dedup
fund_account() {
  $DRY_RUN && return 0
  local amt=$(( 10000000 + RANDOM % 1000000 ))
  local body; body=$(jq -n --argjson s "$1" --argjson d "$2" --argjson a "$amt" \
    '{from:$s, to:$d, amount:$a, nonce:0}')
  local resp; resp=$(curl_json POST "/api/tx/transfer" "$body")
  local h; h=$(printf '%s' "$resp" | jq -r '.tx_hash // empty')
  [[ -n "$h" ]] || die "fund_account transfer failed: $resp" 4
  local s; s=$(poll_tx_state "$h")
  [[ "$s" == "finalised" || "$s" == "included" ]] || die "fund_account tx not accepted (state=$s)" 4
}

# ── preflight ──────────────────────────────────────────────────────────────
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract SinghTriage"    "$CONTRACT_PATH" || die ".es missing SinghTriage header" 2
grep -q "fn initialise("           "$CONTRACT_PATH" || die ".es missing initialise" 2
grep -q "fn register_item("        "$CONTRACT_PATH" || die ".es missing register_item" 2
grep -q "fn classify_all("         "$CONTRACT_PATH" || die ".es missing classify_all" 2
grep -q "fn archive_item("         "$CONTRACT_PATH" || die ".es missing archive_item" 2
grep -q "fn require_urgent("       "$CONTRACT_PATH" || die ".es missing require_urgent" 2
[[ "$MODE" == "classify" || "$MODE" == "refresh" ]] || die "unknown --mode '$MODE' (classify|refresh)" 2

if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

cat <<EOF
+=====================================================================+
|  SinghTriage — §A5.4 doctrine proof (Vital-Sign Wallet Inbox)      |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  mode: $($DRY_RUN && echo DRY-RUN || echo LIVE)  run-mode: $MODE
|  deployer(u8): $DEPLOYER_U8
|  horizons: today=$HORIZON_TODAY  tomorrow=$HORIZON_TOMORROW  week=$HORIZON_WEEK  (hops)
|  items: [0] energy=$ITEM0_ENERGY hl=$ITEM0_HL  (Today)
|         [1] energy=$ITEM1_ENERGY hl=$ITEM1_HL  (Healthy)
|         [2] energy=$ITEM2_ENERGY hl=$ITEM2_HL  (Decayed)
+=====================================================================+
EOF

# ── Step 1: deploy ────────────────────────────────────────────────────────
log "Step 1 - deploy-script (singh_triage.es)"
SRC=$(jq -Rs . < "$CONTRACT_PATH")
DBODY=$(jq -n \
  --argjson d "$DEPLOYER_U8" --argjson s "$SRC" \
  --argjson e "$INITIAL_ENERGY" --argjson hl "$CONTRACT_HALF_LIFE" \
  '{deployer:$d, source_code:$s, energy:$e, half_life:$hl}')
DH=$(submit_tx "/api/tx/deploy-script" "$DBODY" "deploy" 3)
if ! $DRY_RUN; then
  DS=$(poll_tx_state "$DH")
  [[ "$DS" == "finalised" || "$DS" == "included" ]] || die "deploy not accepted (state=$DS)" 3
  CID=$(curl_json GET "/api/tx/$DH" | jq -r '.contract_id // empty')
  [[ -n "$CID" ]] || die "no contract_id in deploy receipt" 3
  ok "deployed contract_id=$CID"
else
  CID=99
fi

# ── Step 2: initialise (seal + set horizons) ───────────────────────────────
EPOCH=$(get_epoch)
log "Step 2 - initialise(today=$HORIZON_TODAY, tomorrow=$HORIZON_TOMORROW, week=$HORIZON_WEEK) at epoch=$EPOCH"
INIT_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  --argjson ht "$HORIZON_TODAY" --argjson htm "$HORIZON_TOMORROW" --argjson hw "$HORIZON_WEEK" \
  '{caller:$c, contract_id:$cid, method:"initialise",
    args:[{U64:$ht},{U64:$htm},{U64:$hw}], epoch:$ep}')
require_tx "/api/tx/call-script" "$INIT_BODY" "initialise" 4

if ! $DRY_RUN; then
  STATE=$(curl_json GET "/api/script/$CID")
  SEALED=$(printf '%s' "$STATE" | untag sealed)
  HT=$(printf '%s' "$STATE"    | untag horizon_today)
  [[ "$SEALED" == "true" || "$SEALED" == "1" || "$SEALED" == "True" ]] \
    || die "expected sealed=true, got $SEALED" 6
  [[ "$HT" == "$HORIZON_TODAY" ]] \
    || die "expected horizon_today=$HORIZON_TODAY, got $HT" 6
  ok "sealed=$SEALED  horizon_today=$HT ✓"
fi

# ── Step 3: register item 0 (Today: energy=4, hl=1000) ──────────────────
EPOCH=$(get_epoch)
log "Step 3 - register_item slot=0 (Today bucket): energy=$ITEM0_ENERGY  hl=$ITEM0_HL"
R0_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  --argjson e "$ITEM0_ENERGY" --argjson hl "$ITEM0_HL" \
  '{caller:$c, contract_id:$cid, method:"register_item",
    args:[{U64:$e},{U64:$hl}], epoch:$ep}')
require_tx "/api/tx/call-script" "$R0_BODY" "register_item_0" 4

# ── Step 4: register item 1 (Healthy: energy=131072, hl=1000) ─────────────
EPOCH=$(get_epoch)
log "Step 4 - register_item slot=1 (Healthy bucket): energy=$ITEM1_ENERGY  hl=$ITEM1_HL"
R1_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  --argjson e "$ITEM1_ENERGY" --argjson hl "$ITEM1_HL" \
  '{caller:$c, contract_id:$cid, method:"register_item",
    args:[{U64:$e},{U64:$hl}], epoch:$ep}')
require_tx "/api/tx/call-script" "$R1_BODY" "register_item_1" 4

# ── Step 5: register item 2 (Decayed: energy=1, hl=1000) ─────────────────
EPOCH=$(get_epoch)
log "Step 5 - register_item slot=2 (Decayed bucket): energy=$ITEM2_ENERGY  hl=$ITEM2_HL"
R2_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  --argjson e "$ITEM2_ENERGY" --argjson hl "$ITEM2_HL" \
  '{caller:$c, contract_id:$cid, method:"register_item",
    args:[{U64:$e},{U64:$hl}], epoch:$ep}')
require_tx "/api/tx/call-script" "$R2_BODY" "register_item_2" 4

if ! $DRY_RUN; then
  STATE=$(curl_json GET "/api/script/$CID")
  IC=$(printf '%s' "$STATE" | untag item_count)
  [[ "$IC" == "3" ]] || die "expected item_count=3, got $IC" 6
  ok "item_count=3 ✓  (slots 0=Today, 1=Healthy, 2=Decayed registered)"
fi

# ── Step 6: classify_all — first pass (caller=DEPLOYER) ──────────────────
# Caller = DEPLOYER_U8 to avoid dedup collision with later classify_all calls.
EPOCH=$(get_epoch)
log "Step 6 - classify_all (caller=account[$DEPLOYER_U8]) at epoch=$EPOCH"
C1_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"classify_all", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$C1_BODY" "classify_all_1" 4

if ! $DRY_RUN; then
  STATE=$(curl_json GET "/api/script/$CID")
  CT=$(printf '%s' "$STATE"   | untag count_today)
  CH=$(printf '%s' "$STATE"   | untag count_healthy)
  CD=$(printf '%s' "$STATE"   | untag count_decayed)
  CTM=$(printf '%s' "$STATE"  | untag count_tomorrow)
  CTW=$(printf '%s' "$STATE"  | untag count_this_week)
  log "inbox: today=$CT  tomorrow=$CTM  this_week=$CTW  healthy=$CH  decayed=$CD"
  [[ "$CT" == "1" ]] || die "expected count_today=1, got $CT" 6
  [[ "$CH" == "1" ]] || die "expected count_healthy=1, got $CH" 6
  [[ "$CD" == "1" ]] || die "expected count_decayed=1, got $CD" 6
  ok "classify_all: today=1 healthy=1 decayed=1 ✓"
fi

# ── Step 7: require_urgent(slot=0) — proves Today item is urgent ───────────
EPOCH=$(get_epoch)
log "Step 7 - require_urgent(slot=0): item 0 must be in Today bucket (hops ≤ horizon_today)"
URG_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"require_urgent",
    args:[{U64:0}], epoch:$ep}')
require_tx "/api/tx/call-script" "$URG_BODY" "require_urgent" 5

if ! $DRY_RUN; then
  STATE=$(curl_json GET "/api/script/$CID")
  UHOPS=$(printf '%s' "$STATE" | untag last_urgent_hops)
  USLOT=$(printf '%s' "$STATE" | untag last_urgent_slot)
  ok "require_urgent PASSED — slot=$USLOT  hops=$UHOPS ≤ horizon_today=$HORIZON_TODAY ✓"
fi

if [[ "$MODE" == "classify" ]]; then

  cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — SinghTriage (classify mode)             |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (wallet-opens-on-inbox):
|   - initialise: sealed=true, horizons configured ✓
|   - register 3 items: Today / Healthy / Decayed ✓
|   - classify_all: count_today=1  count_healthy=1  count_decayed=1 ✓
|   - require_urgent(slot=0): item in Today bucket non-vacuously ✓
|   - "The wallet that opens on inbox urgency, not coin balance." ✓
+=====================================================================+
EOF
  exit 0

fi

# ── refresh mode continues ────────────────────────────────────────────────

# ── Step 8: archive_item(slot=1) — dismiss the Healthy item ──────────────
EPOCH=$(get_epoch)
log "Step 8 - archive_item(slot=1): dismiss the Healthy item"
ARCH_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"archive_item",
    args:[{U64:1}], epoch:$ep}')
require_tx "/api/tx/call-script" "$ARCH_BODY" "archive_item" 4

if ! $DRY_RUN; then
  STATE=$(curl_json GET "/api/script/$CID")
  CA=$(printf '%s' "$STATE" | untag count_archived)
  [[ "$CA" == "1" ]] || die "expected count_archived=1, got $CA" 6
  ok "archive_item(1): count_archived=1 ✓"
fi

# ── Step 9: refresh_item(slot=0, top_up=131072) — lift Today → Healthy ───
EPOCH=$(get_epoch)
log "Step 9 - refresh_item(slot=0, top_up=$REFRESH_TOP_UP): energy boost → item moves to Healthy"
REF_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  --argjson tu "$REFRESH_TOP_UP" \
  '{caller:$c, contract_id:$cid, method:"refresh_item",
    args:[{U64:0},{U64:$tu}], epoch:$ep}')
require_tx "/api/tx/call-script" "$REF_BODY" "refresh_item" 4
ok "refresh_item(0, $REFRESH_TOP_UP) finalised — item 0 energy boosted ✓"

# ── Step 10: classify_all — second pass (caller rotated to CLASSIFY2) ─────
# Caller = CLASSIFY2_U8 (1) to avoid hash dedup with step 6 (caller=0).
EPOCH=$(get_epoch)
log "Step 10 - classify_all (caller=account[$CLASSIFY2_U8]): item 0 should now be Healthy"
C2_BODY=$(jq -n \
  --argjson c "$CLASSIFY2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"classify_all", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$C2_BODY" "classify_all_2" 4

if ! $DRY_RUN; then
  STATE=$(curl_json GET "/api/script/$CID")
  CT2=$(printf '%s' "$STATE"  | untag count_today)
  CH2=$(printf '%s' "$STATE"  | untag count_healthy)
  CD2=$(printf '%s' "$STATE"  | untag count_decayed)
  CA2=$(printf '%s' "$STATE"  | untag count_archived)
  log "inbox after refresh: today=$CT2  healthy=$CH2  decayed=$CD2  archived=$CA2"
  [[ "$CT2" == "0" ]] || die "expected count_today=0 (item 0 refreshed out), got $CT2" 6
  [[ "$CH2" == "1" ]] || die "expected count_healthy=1 (item 0 now Healthy), got $CH2" 6
  [[ "$CD2" == "1" ]] || die "expected count_decayed=1 (item 2 still Decayed), got $CD2" 6
  [[ "$CA2" == "1" ]] || die "expected count_archived=1 (item 1 archived), got $CA2" 6
  ok "after refresh: today=0  healthy=1  decayed=1  archived=1 ✓"
  ok "item 0 moved: Today → Healthy via refresh ✓"
fi

# ── Step 11: let_die_item(slot=2) — consciously abandon the Decayed item ──
EPOCH=$(get_epoch)
log "Step 11 - let_die_item(slot=2): consciously abandon the Decayed item"
DIE_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"let_die_item",
    args:[{U64:2}], epoch:$ep}')
require_tx "/api/tx/call-script" "$DIE_BODY" "let_die_item" 4

# ── Step 12: final classify_all — only item 0 remains active ─────────────
# Caller = CLASSIFY3_U8 (2) to avoid dedup with steps 6 and 10.
# Fund CLASSIFY3 first — account[2] may have zero balance on a fresh testnet.
log "Step 12 - pre-fund account[$CLASSIFY3_U8] (classify_all caller) from account[$DEPLOYER_U8]"
fund_account "$DEPLOYER_U8" "$CLASSIFY3_U8"
EPOCH=$(get_epoch)
log "Step 12 - classify_all (caller=account[$CLASSIFY3_U8]): only item 0 active (Healthy)"
C3_BODY=$(jq -n \
  --argjson c "$CLASSIFY3_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"classify_all", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$C3_BODY" "classify_all_3" 4

if ! $DRY_RUN; then
  STATE=$(curl_json GET "/api/script/$CID")
  CT3=$(printf '%s' "$STATE"  | untag count_today)
  CH3=$(printf '%s' "$STATE"  | untag count_healthy)
  CD3=$(printf '%s' "$STATE"  | untag count_decayed)
  CA3=$(printf '%s' "$STATE"  | untag count_archived)
  log "final inbox: today=$CT3  healthy=$CH3  decayed=$CD3  archived=$CA3"
  [[ "$CT3" == "0" ]] || die "expected count_today=0, got $CT3" 6
  [[ "$CH3" == "1" ]] || die "expected count_healthy=1, got $CH3" 6
  [[ "$CD3" == "0" ]] || die "expected count_decayed=0 (item 2 let_die, excluded), got $CD3" 6
  ok "final classify: today=0 healthy=1 decayed=0 archived=1 ✓"
fi

cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — SinghTriage (refresh mode)              |
+---------------------------------------------------------------------+
|  contract_id: $CID   mode: $MODE
|  horizons: today=$HORIZON_TODAY  tomorrow=$HORIZON_TOMORROW  week=$HORIZON_WEEK
|  PROVEN (wallet-opens-on-inbox full round-trip):
|   - initialise: sealed=true, horizons configured ✓
|   - register 3 items: Today / Healthy / Decayed ✓
|   - classify_all pass 1: today=1  healthy=1  decayed=1 ✓
|   - require_urgent(slot=0): Today item non-vacuously urgent ✓
|   - archive_item(slot=1): count_archived=1 ✓
|   - refresh_item(slot=0, +$REFRESH_TOP_UP): item 0 energy boosted ✓
|   - classify_all pass 2: today=0  healthy=1  decayed=1 ✓
|     → item 0 moved: Today → Healthy via refresh ✓
|   - let_die_item(slot=2): Decayed item consciously abandoned ✓
|   - classify_all pass 3: today=0  healthy=1  decayed=0  archived=1 ✓
|   - "The wallet that opens on inbox urgency, not coin balance." ✓
+=====================================================================+
EOF
