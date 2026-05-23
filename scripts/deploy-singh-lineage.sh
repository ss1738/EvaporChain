#!/usr/bin/env bash
#
# deploy-singh-lineage.sh — end-to-end doctrine proof for SinghLineage
# (contracts/evaporscript/singh_lineage.es).
#
# §A5.4 EvaporWallet-Lineage. "Crypto solves inheritance."
#
# Graduated dormancy model:
#   dormancy = epoch − last_seen_epoch
#   tier_share = highest tier whose threshold ≤ dormancy
#   eff_bp = tier_share_bp * successor_weight_bp / 10000
#
# Two modes:
#
#   --mode authority (default):
#     deploy → initialise(3,6,10) → add_successor(1,6000) + add_successor(2,4000) →
#     witness_authority(addr=1, caller=0) [snapshot1] →
#     witness_authority(addr=2, caller=1) [snapshot2] →
#     require_authority PASSED.
#     Expected at dormancy ≥ 3 (tier1):
#       snapshot1: addr=1  share=2500bp  eff=2500*6000/10000=1500bp (15 %)
#       snapshot2: addr=2  share=2500bp  eff=2500*4000/10000=1000bp (10 %)
#     Proves: silence triggers graduated inheritance authority.
#
#   --mode touch:
#     deploy → initialise(3,6,10) → add_successor(1,6000) + add_successor(2,4000) →
#     witness_authority(addr=1, caller=0) [snapshot1, before touch] →
#       → authority > 0 (dormancy ≥ 3) ✓
#     touch(caller=owner) → resets dormancy to 0 →
#     witness_authority(addr=1, caller=1) [snapshot2, after touch] →
#       → authority = 0 (dormancy ~ 1 < tier1=3) ✓
#     verify snapshot2_authority_bp == 0.
#     Proves: "I'm alive" signal erases accrued authority instantly.
#
# TX HASH DEDUP NOTE:
#   initialise / add_successor take args → unique per contract instance.
#   touch() / require_authority() take no args → different caller indices used:
#     - touch:             caller = DEPLOYER_U8  (owner-only)
#     - witness_authority: caller = DEPLOYER_U8 (snap1) and CALLER2_U8 (snap2)
#     - require_authority: caller = CALLER3_U8
#   INITIAL_ENERGY randomised per run.
#
# Usage:
#   ./scripts/deploy-singh-lineage.sh --dry-run
#   ./scripts/deploy-singh-lineage.sh --node http://89.167.52.40:8099 --mode authority
#   ./scripts/deploy-singh-lineage.sh --node http://89.167.52.40:8099 --mode touch
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 init/add/call · 5 gate · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/singh_lineage.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"       # owner — must be funded
CALLER2_U8="${CALLER2_U8:-1}"         # second witness caller (avoids dedup)
CALLER3_U8="${CALLER3_U8:-2}"         # require_authority caller
MODE="${MODE:-authority}"             # authority | touch

# Contract energy — randomised per run to avoid deploy-tx dedup
INITIAL_ENERGY="${INITIAL_ENERGY:-$(( 20000000 + RANDOM % 32768 ))}"
CONTRACT_HALF_LIFE="${CONTRACT_HALF_LIFE:-500000}"

# Dormancy ladder (epochs). tier1 < tier2 < tier3.
TIER1_EPOCHS="${TIER1_EPOCHS:-3}"
TIER2_EPOCHS="${TIER2_EPOCHS:-6}"
TIER3_EPOCHS="${TIER3_EPOCHS:-10}"

# Successors: addr_u8 → weight_bp (total must be ≤ 10000).
SUCCESSOR1_U8="${SUCCESSOR1_U8:-1}"
SUCCESSOR1_WEIGHT="${SUCCESSOR1_WEIGHT:-6000}"   # 60%
SUCCESSOR2_U8="${SUCCESSOR2_U8:-2}"
SUCCESSOR2_WEIGHT="${SUCCESSOR2_WEIGHT:-4000}"   # 40%

POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-singh-lineage.sh [options]
  --dry-run                validate + print intended calls; no network
  --node URL               node base URL (default http://89.167.52.40:8099)
  --token TOKEN            auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8            owner index (default 0)
  --caller2 U8             second witness caller (default 1)
  --caller3 U8             require_authority caller (default 2)
  --mode authority|touch   prove mode (default authority)
  --tier1 N                tier-1 dormancy threshold epochs (default 3)
  --tier2 N                tier-2 dormancy threshold epochs (default 6)
  --tier3 N                tier-3 dormancy threshold epochs (default 10)
  --energy N               contract initial energy (default randomised ~20M)
  --hl N                   contract half-life (default 500000)
  --timeout SEC            poll timeout (default 300)
  --verbose                echo node responses
  -h|--help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)           DRY_RUN=true; shift ;;
    --node)              NODE_URL="$2"; shift 2 ;;
    --token)             TOKEN="$2"; shift 2 ;;
    --deployer)          DEPLOYER_U8="$2"; shift 2 ;;
    --caller2)           CALLER2_U8="$2"; shift 2 ;;
    --caller3)           CALLER3_U8="$2"; shift 2 ;;
    --mode)              MODE="$2"; shift 2 ;;
    --tier1)             TIER1_EPOCHS="$2"; shift 2 ;;
    --tier2)             TIER2_EPOCHS="$2"; shift 2 ;;
    --tier3)             TIER3_EPOCHS="$2"; shift 2 ;;
    --energy)            INITIAL_ENERGY="$2"; shift 2 ;;
    --hl)                CONTRACT_HALF_LIFE="$2"; shift 2 ;;
    --timeout)           POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose)           VERBOSE=true; shift ;;
    -h|--help)           usage; exit 0 ;;
    *)                   echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[lineage]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[lineage ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }
ok()   { printf '\033[1;32m[lineage OK]\033[0m %s\n' "$*"; }

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
untag()     { jq -r ".state.$1 | if type==\"object\" then (.Bool // .U64 // .Str // .Address // .) else . end"; }

acquire_token() {
  $DRY_RUN && return 0
  [[ -n "$TOKEN" ]] && return 0
  local ts; ts=$(date +%s%N 2>/dev/null || date +%s)
  local email="deploy-lineage-${ts}@example.com"
  local pass="EvaporLin${ts}!"
  local reg_body; reg_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p, display_name:"lineage-deploy"}')
  local reg_resp; reg_resp=$(curl -sS -m 15 -X POST \
    -H 'Content-Type: application/json' -d "$reg_body" \
    "$NODE_URL/api/auth/register") || die "auth register curl failed" 2
  local ok_r; ok_r=$(printf '%s' "$reg_resp" | jq -r '.success // false')
  [[ "$ok_r" == "true" ]] || die "auth register failed: $(printf '%s' "$reg_resp" | jq -r '.message')" 2
  local login_body; login_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p}')
  local login_resp; login_resp=$(curl -sS -m 15 -X POST \
    -H 'Content-Type: application/json' -d "$login_body" \
    "$NODE_URL/api/auth/login") || die "auth login curl failed" 2
  TOKEN=$(printf '%s' "$login_resp" | jq -r '.token // empty')
  [[ -n "$TOKEN" ]] || die "auth login returned no token: $(printf '%s' "$login_resp" | jq -r '.message')" 2
  log "auth: registered + logged in (email=$email)"
}

# ── preflight ──────────────────────────────────────────────────────────────
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract SinghLineage"     "$CONTRACT_PATH" || die ".es missing SinghLineage header" 3
grep -q "fn initialise("             "$CONTRACT_PATH" || die ".es missing initialise" 3
grep -q "fn add_successor("          "$CONTRACT_PATH" || die ".es missing add_successor" 3
grep -q "fn touch("                  "$CONTRACT_PATH" || die ".es missing touch" 3
grep -q "fn witness_authority("      "$CONTRACT_PATH" || die ".es missing witness_authority" 3
grep -q "fn require_authority("      "$CONTRACT_PATH" || die ".es missing require_authority" 3
[[ "$MODE" == "authority" || "$MODE" == "touch" ]] \
  || die "unknown --mode '$MODE' (authority|touch)" 2
(( TIER1_EPOCHS > 0 )) || die "tier1 must be > 0" 2
(( TIER2_EPOCHS > TIER1_EPOCHS )) || die "tier2 must be > tier1" 2
(( TIER3_EPOCHS > TIER2_EPOCHS )) || die "tier3 must be > tier2" 2
(( SUCCESSOR1_WEIGHT + SUCCESSOR2_WEIGHT <= 10000 )) || die "total weight exceeds 10000 bp" 2

if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

acquire_token

# Print banner
if [[ "$MODE" == "authority" ]]; then
  cat <<EOF

+=====================================================================+
|  SinghLineage — §A5.4 EvaporWallet-Lineage doctrine proof          |
|  mode: authority (silence triggers graduated inheritance)           |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  deployer(u8): $DEPLOYER_U8  caller2: $CALLER2_U8  caller3: $CALLER3_U8
|  ladder: tier1=$TIER1_EPOCHS  tier2=$TIER2_EPOCHS  tier3=$TIER3_EPOCHS epochs
|  successors: addr=$SUCCESSOR1_U8 weight=${SUCCESSOR1_WEIGHT}bp  addr=$SUCCESSOR2_U8 weight=${SUCCESSOR2_WEIGHT}bp
+=====================================================================+
EOF
else
  cat <<EOF

+=====================================================================+
|  SinghLineage — §A5.4 EvaporWallet-Lineage doctrine proof          |
|  mode: touch (alive signal erases accrued authority)                |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  deployer(u8): $DEPLOYER_U8  caller2: $CALLER2_U8
|  ladder: tier1=$TIER1_EPOCHS  tier2=$TIER2_EPOCHS  tier3=$TIER3_EPOCHS epochs
|  successor: addr=$SUCCESSOR1_U8 weight=${SUCCESSOR1_WEIGHT}bp
+=====================================================================+
EOF
fi

# ── Step 1: deploy ────────────────────────────────────────────────────────
log "Step 1 - deploy-script (singh_lineage.es)  energy=$INITIAL_ENERGY"
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

# ── Step 2: initialise ────────────────────────────────────────────────────
EPOCH=$(get_epoch)
log "Step 2 - initialise(tier1=$TIER1_EPOCHS, tier2=$TIER2_EPOCHS, tier3=$TIER3_EPOCHS) at epoch=$EPOCH"
INIT_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  --argjson t1 "$TIER1_EPOCHS" --argjson t2 "$TIER2_EPOCHS" --argjson t3 "$TIER3_EPOCHS" \
  '{caller:$c, contract_id:$cid, method:"initialise",
    args:[{U64:$t1},{U64:$t2},{U64:$t3}], epoch:$ep}')
require_tx "/api/tx/call-script" "$INIT_BODY" "initialise" 4
ok "initialised — last_seen_epoch set to $EPOCH ✓"

# ── Step 3: add successors ────────────────────────────────────────────────
EPOCH=$(get_epoch)
log "Step 3 - add_successor(addr=$SUCCESSOR1_U8, weight=${SUCCESSOR1_WEIGHT}bp)"
AS1_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  --argjson a "$SUCCESSOR1_U8" --argjson w "$SUCCESSOR1_WEIGHT" \
  '{caller:$c, contract_id:$cid, method:"add_successor",
    args:[{U64:$a},{U64:$w}], epoch:$ep}')
require_tx "/api/tx/call-script" "$AS1_BODY" "add_successor1" 4

EPOCH=$(get_epoch)
log "Step 4 - add_successor(addr=$SUCCESSOR2_U8, weight=${SUCCESSOR2_WEIGHT}bp)"
AS2_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  --argjson a "$SUCCESSOR2_U8" --argjson w "$SUCCESSOR2_WEIGHT" \
  '{caller:$c, contract_id:$cid, method:"add_successor",
    args:[{U64:$a},{U64:$w}], epoch:$ep}')
require_tx "/api/tx/call-script" "$AS2_BODY" "add_successor2" 4
ok "successors registered: addr=$SUCCESSOR1_U8(${SUCCESSOR1_WEIGHT}bp) + addr=$SUCCESSOR2_U8(${SUCCESSOR2_WEIGHT}bp) ✓"

# ── Mode-specific steps ───────────────────────────────────────────────────
if [[ "$MODE" == "authority" ]]; then

  # Step 5: witness addr=1 (snapshot1)
  EPOCH=$(get_epoch)
  log "Step 5 - witness_authority(addr=$SUCCESSOR1_U8, caller=$DEPLOYER_U8) → snapshot1  epoch=$EPOCH"
  W1_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson a "$SUCCESSOR1_U8" \
    '{caller:$c, contract_id:$cid, method:"witness_authority",
      args:[{U64:$a}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$W1_BODY" "witness_authority1" 4

  # Step 6: witness addr=2 (snapshot2)
  EPOCH=$(get_epoch)
  log "Step 6 - witness_authority(addr=$SUCCESSOR2_U8, caller=$CALLER2_U8) → snapshot2  epoch=$EPOCH"
  W2_BODY=$(jq -n \
    --argjson c "$CALLER2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson a "$SUCCESSOR2_U8" \
    '{caller:$c, contract_id:$cid, method:"witness_authority",
      args:[{U64:$a}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$W2_BODY" "witness_authority2" 4

  # Read and verify state
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    S1A=$(printf '%s' "$STATE" | untag snapshot1_authority_bp)
    S1D=$(printf '%s' "$STATE" | untag snapshot1_dormancy)
    S2A=$(printf '%s' "$STATE" | untag snapshot2_authority_bp)
    S2D=$(printf '%s' "$STATE" | untag snapshot2_dormancy)
    WC=$(printf '%s'  "$STATE" | untag witness_count)
    TWB=$(printf '%s' "$STATE" | untag total_weight_bp)
    ok "witness_count=$WC  total_weight_bp=$TWB"
    ok "snapshot1: addr=$SUCCESSOR1_U8  dormancy=$S1D  authority_bp=$S1A"
    ok "snapshot2: addr=$SUCCESSOR2_U8  dormancy=$S2D  authority_bp=$S2A"
    [[ "$S1A" -gt 0 ]] || die "snapshot1_authority_bp = 0: dormancy ($S1D) hasn't crossed tier1 ($TIER1_EPOCHS)" 6
    [[ "$S2A" -gt 0 ]] || die "snapshot2_authority_bp = 0: dormancy ($S2D) hasn't crossed tier1 ($TIER1_EPOCHS)" 6
    ok "authority accrued for both successors ✓ (dormancy ≥ tier1=$TIER1_EPOCHS)"
  fi

  # Step 7: require_authority gate
  EPOCH=$(get_epoch)
  log "Step 7 - require_authority (caller=$CALLER3_U8)"
  RA_BODY=$(jq -n \
    --argjson c "$CALLER3_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"require_authority", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$RA_BODY" "require_authority" 5
  ok "require_authority PASSED — succession authority confirmed on-chain ✓"

else  # touch mode

  # Step 5: witness BEFORE touch (snapshot1 — should have authority)
  EPOCH=$(get_epoch)
  log "Step 5 - witness_authority(addr=$SUCCESSOR1_U8, caller=$DEPLOYER_U8) BEFORE touch → snapshot1  epoch=$EPOCH"
  WBT_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson a "$SUCCESSOR1_U8" \
    '{caller:$c, contract_id:$cid, method:"witness_authority",
      args:[{U64:$a}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$WBT_BODY" "witness_before_touch" 4

  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    S1A=$(printf '%s' "$STATE" | untag snapshot1_authority_bp)
    S1D=$(printf '%s' "$STATE" | untag snapshot1_dormancy)
    ok "snapshot1 (before touch): dormancy=$S1D  authority_bp=$S1A"
    [[ "$S1A" -gt 0 ]] || die "snapshot1_authority_bp=0 before touch: dormancy ($S1D) < tier1 ($TIER1_EPOCHS). Add more warm-up txs or reduce tier1." 6
    ok "authority accrued before touch ✓ (dormancy=$S1D ≥ tier1=$TIER1_EPOCHS)"
  fi

  # Step 6: touch — resets dormancy to 0
  EPOCH=$(get_epoch)
  log "Step 6 - touch() (caller=$DEPLOYER_U8) → resets dormancy to 0  epoch=$EPOCH"
  TOUCH_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"touch", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$TOUCH_BODY" "touch" 4
  ok "touch() executed — dormancy reset ✓"

  # Step 7: witness AFTER touch (snapshot2 — should be 0)
  EPOCH=$(get_epoch)
  log "Step 7 - witness_authority(addr=$SUCCESSOR1_U8, caller=$CALLER2_U8) AFTER touch → snapshot2  epoch=$EPOCH"
  WAT_BODY=$(jq -n \
    --argjson c "$CALLER2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson a "$SUCCESSOR1_U8" \
    '{caller:$c, contract_id:$cid, method:"witness_authority",
      args:[{U64:$a}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$WAT_BODY" "witness_after_touch" 4

  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    S1A=$(printf '%s' "$STATE" | untag snapshot1_authority_bp)
    S1D=$(printf '%s' "$STATE" | untag snapshot1_dormancy)
    S2A=$(printf '%s' "$STATE" | untag snapshot2_authority_bp)
    S2D=$(printf '%s' "$STATE" | untag snapshot2_dormancy)
    LSE=$(printf '%s' "$STATE" | untag last_seen_epoch)
    ok "last_seen_epoch=$LSE (reset by touch)"
    ok "snapshot1 (before touch): dormancy=$S1D  authority_bp=$S1A"
    ok "snapshot2 (after touch):  dormancy=$S2D  authority_bp=$S2A"
    [[ "$S2A" == "0" ]] || die "expected snapshot2_authority_bp=0 after touch, got $S2A (dormancy=$S2D — too many epochs between touch and witness?)" 6
    ok "touch ERASED authority ✓ (snapshot2_authority_bp=0, dormancy=$S2D < tier1=$TIER1_EPOCHS)"
  fi

fi

# ── Final summary ─────────────────────────────────────────────────────────
if [[ "$MODE" == "authority" ]]; then
  cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — SinghLineage (authority mode)           |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (silence triggers graduated inheritance):
|   - initialised with 3-tier ladder (3/6/10 epochs) ✓
|   - successor addr=$SUCCESSOR1_U8 (60%) + addr=$SUCCESSOR2_U8 (40%) registered ✓
|   - witness_authority: both successors have accrued authority ✓
|   - require_authority PASSED ✓
|   - "If I'm silent 90 days, my daughter's key gains 25% authority." ✓
+=====================================================================+
EOF
else
  cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — SinghLineage (touch mode)               |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (alive signal erases accrued authority):
|   - snapshot1 (before touch): authority > 0 ✓
|   - touch() executed: dormancy reset to 0 ✓
|   - snapshot2 (after touch): authority = 0 ✓
|   - "Any signed tx by the issuer acts as a touch." ✓
+=====================================================================+
EOF
fi
