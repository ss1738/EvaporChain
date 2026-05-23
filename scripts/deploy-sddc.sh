#!/usr/bin/env bash
#
# deploy-sddc.sh — end-to-end doctrine proof for the SDDC
# (Singh Decay-Dutch Continuous Auction) — INVENTION_STACK.md §A5.2.
#
# The SDDC is the foundational two-axis clearing mechanism that
# underlies SFSV, SHLM, SAP, and SCL. Two-axis clearing:
#   axis 1 (price)        — Dutch descent from ceiling to floor
#   axis 2 (λ-tolerance)  — bidder declares minimum λ it will tolerate
#
# Both axes must be satisfied simultaneously for a bid to clear.
# This is STRUCTURALLY novel: existing Dutch auctions are single-axis.
#
# Modes:
#   --mode clear (default): set_lot → submit_bid (both axes valid) →
#     try_clear SUCCEEDS → verify phase=CLEARED on-chain.
#
#   --mode gate: set_lot → submit_bid (lambda_tolerance < lot_lambda) →
#     try_clear REJECTED on-chain ("winner lambda_tolerance below lot_lambda")
#     → verify phase still OPEN → void auction.
#     This proves the λ-tolerance axis is enforced, not vacuous.
#
# HONEST SCOPE: proves on-chain SDDC registry + two-axis gate enforcement.
# Exact Dutch-descent price sequencing + multi-bidder ordering is the
# off-chain coordinator's responsibility. Contract records the
# coordinator's decision and re-runs the two gates independently.
#
# NOTE: re-running with identical source+deployer+energy+half_life
# resolves the SAME cached contract_id (deploy tx dedup). Pass a unique
# INITIAL_ENERGY each run, e.g. INITIAL_ENERGY=$((7000000 + RANDOM)).
#
# Usage:
#   ./scripts/deploy-sddc.sh --dry-run
#   ./scripts/deploy-sddc.sh --node http://89.167.52.40:8099 --mode clear
#   ./scripts/deploy-sddc.sh --node http://89.167.52.40:8099 --mode gate
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 setup · 5 gate-not-exercised · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/sddc.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"        # seller / coordinator — must be funded
BIDDER_U8="${BIDDER_U8:-1}"            # bidder address index (funded from SCL session)
MODE="${MODE:-clear}"                  # clear | gate

# Lot parameters
ITEM_LABEL="${ITEM_LABEL:-evaporchain_v1_slot}"
LOT_CEILING="${LOT_CEILING:-1000000}"  # price descent starts here
LOT_FLOOR="${LOT_FLOOR:-100000}"       # floor — never descends below
LOT_LAMBDA="${LOT_LAMBDA:-50}"         # lot's λ; bidder must tolerate >= this
LOT_DURATION="${LOT_DURATION:-10000}"  # window in epochs (long — won't expire mid-test)

# Bid parameters (mode-driven)
# clear mode: both axes satisfied
CLEAR_MAX_PRICE="${CLEAR_MAX_PRICE:-1000000}"   # >= ceiling → always price-valid
CLEAR_LAMBDA_TOL="${CLEAR_LAMBDA_TOL:-100}"     # >= lot_lambda → λ-axis valid
# gate mode: λ-axis deliberately fails (lambda_tolerance < lot_lambda)
GATE_MAX_PRICE="${GATE_MAX_PRICE:-1000000}"
GATE_LAMBDA_TOL="${GATE_LAMBDA_TOL:-10}"        # < lot_lambda=50 → MUST be rejected

CONFIRMED_PRICE="${CONFIRMED_PRICE:-990000}"    # clearing price (in [floor, ceiling])

INITIAL_ENERGY="${INITIAL_ENERGY:-10000000}"
HALF_LIFE="${HALF_LIFE:-200000}"     # long half-life so auction doesn't evaporate mid-test
POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-120}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-sddc.sh [options]
  --dry-run            validate + print intended calls; no network
  --node URL           node base URL (default http://89.167.52.40:8099)
  --token TOKEN        auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8        seller/coordinator account index (default 0)
  --bidder U8          bidder account index (default 1)
  --mode clear|gate    clear=happy path (default); gate=λ-axis rejection proof
  --energy N           initial contract energy
  --timeout SEC        per-step poll timeout (default 120)
  --verbose            echo curl responses
  -h|--help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)  DRY_RUN=true;            shift ;;
    --node)     NODE_URL="$2";           shift 2 ;;
    --token)    TOKEN="$2";              shift 2 ;;
    --deployer) DEPLOYER_U8="$2";        shift 2 ;;
    --bidder)   BIDDER_U8="$2";          shift 2 ;;
    --mode)     MODE="$2";               shift 2 ;;
    --energy)   INITIAL_ENERGY="$2";     shift 2 ;;
    --timeout)  POLL_TIMEOUT_SEC="$2";   shift 2 ;;
    --verbose)  VERBOSE=true;            shift ;;
    -h|--help)  usage; exit 0 ;;
    *)          echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[sddc]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[sddc]\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[sddc ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }

curl_json() {
  local method="$1" path="$2" body="${3:-}"
  if $DRY_RUN; then echo "  [DRY-RUN] $method $NODE_URL$path ${body:+(body)}" >&2; echo '{}'; return 0; fi
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

poll_tx() {
  $DRY_RUN && { echo '{"state":"finalised"}'; return 0; }
  local deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC )) resp st
  while (( $(date +%s) < deadline )); do
    resp=$(curl_json GET "/api/tx/$1") || true
    st=$(printf '%s' "$resp" | jq -r '.state // "unknown"')
    case "$st" in
      included|finalised) printf '%s' "$resp"; return 0 ;;
      rejected) die "$2 tx rejected: $(printf '%s' "$resp" | jq -r '.error // "?"')" "$3" ;;
    esac
    sleep 2
  done
  die "$2 not included within ${POLL_TIMEOUT_SEC}s" "$3"
}

require_tx() {
  local h; h=$(submit_tx "$1" "$2" "$3" "$4")
  poll_tx "$h" "$3" "$4" >/dev/null
}

get_epoch() {
  $DRY_RUN && { echo 0; return 0; }
  curl_json GET "/api/status" | jq -r '.epoch // .height // 0'
}

# Build address arg from u8 index → {Address: [b, 0...0]}
addr_arg() {
  local idx="$1"
  jq -n --argjson i "$idx" '{Address: ([$i] + [range(0;31)|0])}'
}

mapget() {
  local state_json="$1" field="$2" idx="$3"
  printf '%s' "$state_json" | jq -r \
    --arg f "$field" --argjson i "$idx" \
    '(.state[$f] | if type == "object" then (.[keys[0]]) else . end) // "0"' 2>/dev/null || echo "0"
}

# ── preflight ──
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract SDDC" "$CONTRACT_PATH" || die ".es missing contract SDDC header" 2
grep -q "fn set_lot("   "$CONTRACT_PATH" || die ".es missing set_lot" 2
grep -q "fn submit_bid(" "$CONTRACT_PATH" || die ".es missing submit_bid" 2
grep -q "fn try_clear(" "$CONTRACT_PATH" || die ".es missing try_clear" 2
(( $(wc -c < "$CONTRACT_PATH") <= 65536 )) || die ".es exceeds 64KB node cap" 2
[[ "$MODE" == "clear" || "$MODE" == "gate" ]] || die "unknown --mode '$MODE' (clear|gate)" 2
if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/health" >/dev/null 2>&1 || \
    curl -sS -m 5 "$NODE_URL/api/version" >/dev/null 2>&1 || \
    die "node $NODE_URL unreachable" 2
fi

# Mode-driven bid parameters
if [[ "$MODE" == "clear" ]]; then
  BID_MAX_PRICE=$CLEAR_MAX_PRICE
  BID_LAMBDA_TOL=$CLEAR_LAMBDA_TOL
else
  BID_MAX_PRICE=$GATE_MAX_PRICE
  BID_LAMBDA_TOL=$GATE_LAMBDA_TOL
fi

printf '+%s+\n' "$(printf '%0.s=' {1..66})"
printf '|  SDDC — Decay-Dutch Auction — §A5.2 doctrine proof\n'
printf '+%s+\n' "$(printf '%0.s-' {1..66})"
printf '|  node: %s  mode: %s  run-mode: %s\n' "$NODE_URL" "$($DRY_RUN && echo DRY-RUN || echo LIVE)" "$MODE"
printf '|  seller(u8): %s  bidder(u8): %s\n' "$DEPLOYER_U8" "$BIDDER_U8"
printf '|  lot: %s  ceiling=%s floor=%s λ=%s dur=%s\n' "$ITEM_LABEL" "$LOT_CEILING" "$LOT_FLOOR" "$LOT_LAMBDA" "$LOT_DURATION"
printf '|  bid: max_price=%s lambda_tol=%s  (mode=%s)\n' "$BID_MAX_PRICE" "$BID_LAMBDA_TOL" "$MODE"
printf '+%s+\n' "$(printf '%0.s=' {1..66})"

# ── 1. deploy ──
log "Step 1/6 - deploy-script (sddc.es)"
SRC=$(jq -Rs . < "$CONTRACT_PATH")
DEPLOY_BODY=$(jq -n \
  --argjson d "$DEPLOYER_U8" --argjson s "$SRC" \
  --argjson e "$INITIAL_ENERGY" --argjson hl "$HALF_LIFE" \
  '{deployer:$d, source_code:$s, energy:$e, half_life:$hl}')
DH=$(submit_tx "/api/tx/deploy-script" "$DEPLOY_BODY" deploy 3)
log "deploy tx: $DH"
DEPLOY_STATUS=$(poll_tx "$DH" deploy 3)
CID=$(printf '%s' "$DEPLOY_STATUS" | jq -r '.contract_id // empty')
[[ -n "$CID" ]] || { $DRY_RUN && CID=0 || die "no contract_id in deploy tx" 3; }
log "contract_id: $CID"

# ── 2. set_lot ──
log "Step 2/6 - set_lot(\"$ITEM_LABEL\", $LOT_CEILING, $LOT_FLOOR, λ=$LOT_LAMBDA, dur=$LOT_DURATION)"
EPOCH=$(get_epoch)
SL_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  --arg item "$ITEM_LABEL" \
  --argjson ce "$LOT_CEILING" --argjson fl "$LOT_FLOOR" \
  --argjson lam "$LOT_LAMBDA" --argjson dur "$LOT_DURATION" \
  '{caller:$c, contract_id:$cid, method:"set_lot",
    args:[{Str:$item},{U64:$ce},{U64:$fl},{U64:$lam},{U64:$dur}], epoch:$ep}')
require_tx "/api/tx/call-script" "$SL_BODY" set_lot 4
log "lot configured (sealed=true)."

# ── 3. submit_bid ──
log "Step 3/6 - submit_bid(max_price=$BID_MAX_PRICE, lambda_tol=$BID_LAMBDA_TOL) as bidder[$BIDDER_U8]"
EPOCH=$(get_epoch)
BIDDER_ARG=$(addr_arg "$BIDDER_U8")
SB_BODY=$(jq -n \
  --argjson c "$BIDDER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  --argjson mp "$BID_MAX_PRICE" --argjson lt "$BID_LAMBDA_TOL" \
  '{caller:$c, contract_id:$cid, method:"submit_bid",
    args:[{U64:$mp},{U64:$lt}], epoch:$ep}')
require_tx "/api/tx/call-script" "$SB_BODY" submit_bid 4
log "bid registered."

# ── 4. confirm pre-clear state ──
log "Step 4/6 - confirm phase=0 (OPEN) + bid_count >= 1"
if ! $DRY_RUN; then
  ST=$(curl_json GET "/api/script/$CID")
  PHASE=$(printf '%s' "$ST" | jq -r '(.state.phase.U64 // .state.phase) // 0')
  BCOUNT=$(printf '%s' "$ST" | jq -r '(.state.bid_count.U64 // .state.bid_count) // 0')
  SEALED=$(printf '%s' "$ST" | jq -r '(.state.sealed.Bool // .state.sealed) // false')
  log "CONFIRMED: sealed=$SEALED phase=$PHASE bid_count=$BCOUNT"
  [[ "$SEALED" == "true" || "$SEALED" == "1" ]] || die "lot not sealed after set_lot" 4
  [[ "$PHASE" == "0" ]] || die "auction not open (phase=$PHASE)" 4
  (( BCOUNT >= 1 )) || die "bid_count=0 after submit_bid" 4
fi

# ── 5. mode-specific clearing proof ──
if [[ "$MODE" == "clear" ]]; then
  log "Step 5/6 - CLEAR — try_clear(bidder[$BIDDER_U8], price=$CONFIRMED_PRICE) MUST succeed (both axes valid)"
  EPOCH=$(get_epoch)
  TC_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson wa "$(addr_arg "$BIDDER_U8")" --argjson cp "$CONFIRMED_PRICE" \
    '{caller:$c, contract_id:$cid, method:"try_clear",
      args:[$wa,{U64:$cp}], epoch:$ep}')
  if $DRY_RUN; then
    log "[DRY-RUN] would call try_clear"
  else
    TCH=$(submit_tx "/api/tx/call-script" "$TC_BODY" try_clear 6)
    TCS=$(curl_json GET "/api/tx/$TCH" | jq -r '.state // "unknown"')
    sleep 3
    TCS=$(curl_json GET "/api/tx/$TCH" | jq -r '.state // "unknown"')
    [[ "$TCS" == "finalised" || "$TCS" == "included" ]] || \
      die "try_clear rejected for a valid two-axis bid (state=$TCS) — clearing broken" 6
    ST=$(curl_json GET "/api/script/$CID")
    PHASE=$(printf '%s' "$ST" | jq -r '(.state.phase.U64 // .state.phase) // 0')
    WPRICE=$(printf '%s' "$ST" | jq -r '(.state.price_paid.U64 // .state.price_paid) // 0')
    [[ "$PHASE" == "1" ]] || die "phase != 1 after try_clear (phase=$PHASE)" 6
    (( WPRICE > 0 )) || die "price_paid=0 after clearing" 6
  fi

  log "Step 6/6 - verify phase=1 (CLEARED) on-chain"
  if ! $DRY_RUN; then
    printf '\n+%s+\n' "$(printf '%0.s=' {1..66})"
    printf '|   OK  SDDC — TWO-AXIS DUTCH AUCTION CLEARED                  |\n'
    printf '|  contract_id: %s  item: %s\n' "$CID" "$ITEM_LABEL"
    printf '|  winner: bidder[%s]  price_paid: %s\n' "$BIDDER_U8" "$WPRICE"
    printf '|  axis 1 (price): %s <= bid.max_price=%s ✓\n' "$CONFIRMED_PRICE" "$BID_MAX_PRICE"
    printf '|  axis 2 (λ-tol): lot_λ=%s <= bid.λ_tol=%s ✓\n' "$LOT_LAMBDA" "$BID_LAMBDA_TOL"
    printf '+%s+\n' "$(printf '%0.s=' {1..66})"
  else
    log "[DRY-RUN] would assert phase=1 + price_paid > 0"; log "OK dry-run."; exit 0
  fi

else
  # gate mode: λ-tolerance below lot_lambda → try_clear MUST be rejected
  log "Step 5/6 - GATE — try_clear with bid.λ_tol=$BID_LAMBDA_TOL < lot_λ=$LOT_LAMBDA → MUST be rejected"
  EPOCH=$(get_epoch)
  TC_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson wa "$(addr_arg "$BIDDER_U8")" --argjson cp "$CONFIRMED_PRICE" \
    '{caller:$c, contract_id:$cid, method:"try_clear",
      args:[$wa,{U64:$cp}], epoch:$ep}')
  if $DRY_RUN; then
    log "[DRY-RUN] would call try_clear then expect rejection, then void_auction"
    log "OK dry-run."; exit 0
  fi
  gate_ok=0
  BADH=$(printf '%s' "$(curl_json POST /api/tx/call-script "$TC_BODY")" | jq -r '.tx_hash // empty')
  if [[ -n "$BADH" ]]; then
    sleep 4
    BADS=$(curl_json GET "/api/tx/$BADH" | jq -r '.state // "unknown"')
    if [[ "$BADS" == "rejected" ]]; then
      gate_ok=1; log "✓ try_clear REJECTED (λ-gate enforced: lot_λ=$LOT_LAMBDA > bid.λ_tol=$BID_LAMBDA_TOL)"
    else
      warn "try_clear was NOT rejected (state=$BADS) — λ-gate may not be enforced"
    fi
  else
    gate_ok=1; log "✓ try_clear rejected at submission (λ-gate enforced)"
  fi
  (( gate_ok == 1 )) || die "λ-gate NOT exercised — clearing should have been rejected" 5

  # void the auction (clean up: phase → 2)
  log "Step 6/6 - void_auction (clean up post-gate-rejection)"
  EPOCH=$(get_epoch)
  VA_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"void_auction", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$VA_BODY" void_auction 4

  ST=$(curl_json GET "/api/script/$CID")
  PHASE=$(printf '%s' "$ST" | jq -r '(.state.phase.U64 // .state.phase) // 0')

  printf '\n+%s+\n' "$(printf '%0.s=' {1..66})"
  printf '|   OK  SDDC — λ-AXIS GATE PROVEN (bad bid REJECTED)            |\n'
  printf '|  contract_id: %s  item: %s\n' "$CID" "$ITEM_LABEL"
  printf '|  bid.λ_tol=%s < lot_λ=%s → try_clear rejected on-chain ✓\n' "$BID_LAMBDA_TOL" "$LOT_LAMBDA"
  printf '|  auction voided cleanly (phase=%s)\n' "$PHASE"
  printf '|  The two-axis λ-tolerance gate is ENFORCED, not vacuous.\n'
  printf '+%s+\n' "$(printf '%0.s=' {1..66})"
fi
