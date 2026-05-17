#!/usr/bin/env bash
#
# deploy-evaporcash.sh — live-e2e runbook for the EvaporCash flagship
# demo (model (A) bearer-note — contracts/evaporscript/evaporcash_note.es).
#
# Proves "your money rots if you hoard it" on a live chain: deploy a
# note, issue it, confirm it has live value, watch that value DECAY
# across epochs (the demurrage), then — deliberately NOT spending it
# (the hoarder) — watch the chain's evaporation engine take the value
# to zero and retire the note unspent. That terminal
# `evaporated == true  AND  spent == false` is the punchline, directly
# observed.
#
# Authored fresh against the SOURCE-VERIFIED node contract with the
# CORRECTED endpoint (GET /api/script/:id — NOT /api/contract/:id; the
# on-branch deploy-sfsv.sh is still the old wrong-endpoint version,
# forking it would re-introduce that bug). Same auth/arg shape proven
# by the SFSV + Dead Drop live e2es:
#   - deployer/caller u8 indices; addr_from_byte(0) = genesis faucet
#   - call-script args externally-tagged: address {"Address":[b0..b31]},
#     u64 {"U64":n}; epoch REQUIRED
#
# Flow:
#   1. deploy evaporcash_note.es  (energy=value, half_life=demurrage)
#   2. poll /api/tx/<hash> -> .contract_id
#   3. issue(to, face)            (owner-only, once)
#   4. CONFIRM issued + has value (non-vacuity): GET /api/script/:id
#      shows sealed=true, spent=false, energy>0  -> saw_value=1
#   5. (no gradual-decay assertion — /api/script .energy is the static
#      deploy value, verified 2026-05-17; live decay is NOT
#      API-surfaced. Demurrage is proven by the TERMINAL loss.)
#   5/6. --mode hoard (default): never spend; poll until
#      evaporated==true with spent==false — "money rots if you hoard".
#   5/6. --mode spend: call spend(to) as the holder; assert
#      state.spent==true AND holder moved to `to` — circulation
#      preserves value (the OTHER half of the Wörgl thesis; the claim
#      is retired-by-spend, NOT lost to evaporation).
#
# Exit: 0 hoarded-value-lost-proven · 2 precondition · 3 deploy ·
#       4 issue · 5 non-vacuity/decay · 6 never-evaporated
#
# Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/evaporcash_note.es"

NODE_URL="${NODE_URL:-http://127.0.0.1:9001}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"          # 0 = genesis-funded faucet account
HOLDER_U8="${HOLDER_U8:-0}"              # issue() bearer (read auth not needed here)
SPEND_TO_U8="${SPEND_TO_U8:-1}"          # --mode spend: recipient of spend(to)
MODE="${MODE:-hoard}"                    # hoard = lose-by-decay ; spend = circulate-to-preserve
FACE="${FACE:-1000}"                     # accounting snapshot only
INITIAL_ENERGY="${INITIAL_ENERGY:-120000}"  # the note's value at issue
HALF_LIFE="${HALF_LIFE:-6}"              # demurrage rate; smaller = rots faster
DECAY_WAIT_EPOCHS="${DECAY_WAIT_EPOCHS:-8}"  # gap for the decay assertion
DRY_RUN=false
VERBOSE=false
POLL_TIMEOUT_SEC=300

usage() { cat <<'EOF'
deploy-evaporcash.sh [options]
  --dry-run            validate + print intended calls; no network
  --node URL           node base URL (default http://127.0.0.1:9001)
  --token TOKEN        auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8        deployer/caller index (default 0 = faucet)
  --holder U8          issue() bearer index (default 0)
  --mode hoard|spend   hoard=lose-by-decay (default) ; spend=circulate-to-preserve
  --spend-to U8        --mode spend recipient index (default 1)
  --face N             face_value accounting snapshot (default 1000)
  --energy N           note value at issue (default 120000)
  --half-life N        demurrage rate; smaller = rots sooner (default 6)
  --decay-wait N       epochs to wait before the decay assertion (default 8)
  --timeout SEC        evaporation poll timeout (default 300)
  --verbose            echo node responses
  -h|--help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=true; shift ;;
    --node) NODE_URL="$2"; shift 2 ;;
    --token) TOKEN="$2"; shift 2 ;;
    --deployer) DEPLOYER_U8="$2"; shift 2 ;;
    --holder) HOLDER_U8="$2"; shift 2 ;;
    --mode) MODE="$2"; shift 2 ;;
    --spend-to) SPEND_TO_U8="$2"; shift 2 ;;
    --face) FACE="$2"; shift 2 ;;
    --energy) INITIAL_ENERGY="$2"; shift 2 ;;
    --half-life) HALF_LIFE="$2"; shift 2 ;;
    --decay-wait) DECAY_WAIT_EPOCHS="$2"; shift 2 ;;
    --timeout) POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose) VERBOSE=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[evaporcash]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[evaporcash ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }

curl_json() {  # <METHOD> <path> [body]
  local method="$1" path="$2" body="${3:-}"
  if $DRY_RUN; then echo "  [DRY-RUN] $method $NODE_URL$path ${body:+(body: $body)}" >&2; echo '{}'; return 0; fi
  local args=(-sS -m 30 -X "$method" -H 'Content-Type: application/json')
  [[ -n "$TOKEN" ]] && args+=(-H "Authorization: Bearer $TOKEN")
  [[ -n "$body" ]] && args+=(-d "$body")
  local resp; resp=$(curl "${args[@]}" "$NODE_URL$path") || die "curl $method $path failed" 2
  $VERBOSE && echo "  <- $resp" >&2
  printf '%s' "$resp"
}

submit_tx() {  # <path> <body> <name> <failcode> -> tx hash
  local resp; resp=$(curl_json POST "$1" "$2")
  $DRY_RUN && { echo "DRYHASH"; return 0; }
  local hash; hash=$(printf '%s' "$resp" | jq -r '.tx_hash // empty')
  [[ -n "$hash" ]] || die "$3 failed: $(printf '%s' "$resp" | jq -r '.message // .error // "(no msg)"')" "$4"
  printf '%s' "$hash"
}

poll_tx() {  # <hash> <name> <failcode>
  $DRY_RUN && { echo '{"state":"finalised","contract_id":0}'; return 0; }
  local deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC )) resp st
  while (( $(date +%s) < deadline )); do
    resp=$(curl_json GET "/api/tx/$1") || true
    st=$(printf '%s' "$resp" | jq -r '.state // "unknown"')
    case "$st" in
      included|finalised) printf '%s' "$resp"; return 0 ;;
      rejected) die "$2 rejected: $(printf '%s' "$resp" | jq -r '.error // "?"')" "$3" ;;
    esac
    sleep 2
  done
  die "$2 not included within ${POLL_TIMEOUT_SEC}s" "$3"
}

get_epoch() { $DRY_RUN && { echo 0; return 0; }; curl_json GET "/api/status" | jq -r '.epoch // 0'; }
note_energy() { printf '%s' "$(curl_json GET "/api/script/$1")" | jq -r '.energy // -1'; }

# ── preflight ──
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract EvaporCashNote" "$CONTRACT_PATH" || die ".es missing EvaporCashNote header" 3
grep -q "fn issue(" "$CONTRACT_PATH" || die ".es missing issue" 3
if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

cat <<EOF
+==================================================================+
|  EvaporCash — hoarding-demurrage e2e (evaporcash_note.es, model A)|
+------------------------------------------------------------------+
|  node: $NODE_URL   mode: $($DRY_RUN && echo DRY-RUN || echo LIVE)
|  deployer(u8): $DEPLOYER_U8   holder(u8): $HOLDER_U8   face: $FACE
|  energy/half-life: $INITIAL_ENERGY/$HALF_LIFE   (NOT spending = hoarding)
+==================================================================+
EOF

# ── 1. deploy ──
log "Step 1/6 - POST /api/tx/deploy-script (evaporcash_note.es)"
SRC=$(jq -Rs . < "$CONTRACT_PATH")
DBODY=$(jq -n --argjson d "$DEPLOYER_U8" --argjson s "$SRC" \
  --argjson e "$INITIAL_ENERGY" --argjson hl "$HALF_LIFE" \
  '{deployer:$d, source_code:$s, energy:$e, half_life:$hl}')
DH=$(submit_tx "/api/tx/deploy-script" "$DBODY" deploy 3)
log "deploy tx: $DH"
DSTAT=$(poll_tx "$DH" deploy 3)
CID=$(printf '%s' "$DSTAT" | jq -r '.contract_id // empty')
[[ -n "$CID" ]] || { $DRY_RUN && CID=0 || die "no contract_id on deploy tx status" 3; }
log "contract_id: $CID"

# ── 2. issue (bind the bearer, once) ──
log "Step 2/6 - call-script issue(to, face)"
EP=$(get_epoch)
IBODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" \
  --argjson h "$HOLDER_U8" --argjson f "$FACE" --argjson ep "$EP" \
  '{caller:$c, contract_id:$cid, method:"issue",
    args:[ {Address: ([$h] + [range(0;31)|0])}, {U64:$f} ],
    epoch:$ep}')
IH=$(submit_tx "/api/tx/call-script" "$IBODY" issue 4)
poll_tx "$IH" issue 4 >/dev/null
log "note issued."

# ── 3. CONFIRM issued + has value (non-vacuity) ──
log "Step 3/6 - confirm issued with value (non-vacuity guard)"
saw_value=0; E1=-1
if $DRY_RUN; then
  log "[DRY-RUN] would assert sealed=true, spent=false, energy>0"
else
  ST=$(curl_json GET "/api/script/$CID")
  # jq `//` treats boolean false as empty and falls through — so
  # `.x.Bool // .x // false` mis-extracts a legit `false` as the raw
  # {"Bool":false} object. Unwrap the tagged value explicitly instead.
  SEALED=$(printf '%s' "$ST" | jq -r '.state.sealed | if type=="object" then .Bool else . end')
  SPENT=$(printf '%s' "$ST" | jq -r '.state.spent | if type=="object" then .Bool else . end')
  E1=$(printf '%s' "$ST" | jq -r '.energy // -1')
  if [[ "$SEALED" == "true" && "$SPENT" == "false" && "$E1" -gt 0 ]]; then
    saw_value=1
    log "CONFIRMED: sealed=true spent=false live energy=$E1 (the note has value)"
  else
    die "note not in issued-with-value state (sealed=$SEALED spent=$SPENT energy=$E1) — cannot prove demurrage from a never-valued note" 5
  fi
fi

# ── 4. (observability note — NOT an assertion) ──
# DIRECTLY VERIFIED 2026-05-17 (probe on node 8099, and the Dead Drop
# transcript): GET /api/script/:id `.energy` is the STATIC deploy
# value — it reads 60000 at issue and 60000 ~10 epochs later, then the
# instance flips straight to `evaporated:true`. The node does NOT
# surface the live-decaying energy. Demurrage is real (it is what
# drives the eventual evaporation) but the gradual "watch it tick
# down" is NOT API-observable; only the TERMINAL loss is. So this
# runbook does not (and honestly cannot) assert a gradual decrease —
# the provable claim is the same observable shape as Dead Drop's
# forgetting: confirmed-issued-with-value -> hoarded -> evaporated
# unspent = value lost. Over-asserting gradual decay here was a real
# bug (spurious exit 5); removed, not papered over.
log "Step 4/6 - (skipped) live energy is not API-surfaced; demurrage"
log "  is proven by the TERMINAL loss in step 5, not a gradual read."

# ── 5. mode-specific behaviour ──
if [[ "$MODE" != "hoard" && "$MODE" != "spend" ]]; then
  die "unknown --mode '$MODE' (expected: hoard | spend)" 2
fi

if [[ "$MODE" == "hoard" ]]; then
  # HOARD: never spend; wait for the value to evaporate (the Wörgl
  # "money rots if you hoard it" half).
  log "Step 5/6 - HOARD (never spend) — poll until the note evaporates"
  if $DRY_RUN; then
    log "[DRY-RUN] would poll until evaporated==true while spent==false"
    log "OK dry-run complete."; exit 0
  fi
  deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC )); lost=0
  while (( $(date +%s) < deadline )); do
    ST=$(curl_json GET "/api/script/$CID")
    if printf '%s' "$ST" | jq -e 'has("error")' >/dev/null 2>&1; then
      lost=1; log "note $CID gone from the script store (evaporated, unspent)"; break
    fi
    EVAP=$(printf '%s' "$ST" | jq -r '.evaporated // false')
    SPENT=$(printf '%s' "$ST" | jq -r '.state.spent | if type=="object" then .Bool else . end')
    if [[ "$EVAP" == "true" ]]; then
      [[ "$SPENT" == "true" ]] && die "note was SPENT — hoarding scenario invalidated (someone spent it)" 6
      lost=1; log "note evaporated with spent=false — value lost to hoarding"; break
    fi
    sleep 4
  done
  log "Step 6/6 - verdict (hoard)"
  (( saw_value == 1 )) || die "non-vacuity: never confirmed an issued note with value" 5
  (( lost == 1 )) || die "note never evaporated within ${POLL_TIMEOUT_SEC}s — demurrage-to-zero not reached (half-life too large?)" 6
  cat <<EOF

+==================================================================+
|        OK  EVAPORCASH — HOARDING PENALTY PROVEN BY PHYSICS        |
|  contract_id: $CID                                                |
|  proof: deploy ok  issue ok  HAD-VALUE ok  ->  LOST-UNSPENT       |
|  an unspent note (issued value $E1) was retired by the engine     |
|  unspent — "money rots if you hoard it", native, no keeper.       |
|  (gradual decay is real but not API-surfaced; terminal loss is.)  |
+==================================================================+
EOF
else
  # SPEND: circulate before decay — the OTHER half of the thesis
  # (circulation preserves value; the claim moves to a new holder and
  # the note is retired-by-spend, NOT lost to evaporation).
  log "Step 5/6 - SPEND (circulate) — call spend(to=$SPEND_TO_U8) as the holder"
  if $DRY_RUN; then
    log "[DRY-RUN] would spend(to) then assert spent=true & holder moved"
    log "OK dry-run complete."; exit 0
  fi
  EP=$(get_epoch)
  SPBODY=$(jq -n --argjson c "$HOLDER_U8" --argjson cid "$CID" \
    --argjson t "$SPEND_TO_U8" --argjson ep "$EP" \
    '{caller:$c, contract_id:$cid, method:"spend",
      args:[ {Address: ([$t] + [range(0;31)|0])} ], epoch:$ep}')
  SPH=$(submit_tx "/api/tx/call-script" "$SPBODY" spend 6)
  poll_tx "$SPH" spend 6 >/dev/null
  log "spend tx finalised."
  ST=$(curl_json GET "/api/script/$CID")
  if printf '%s' "$ST" | jq -e 'has("error")' >/dev/null 2>&1; then
    die "note $CID gone right after spend — expected it to persist as spent" 6
  fi
  SPENT=$(printf '%s' "$ST" | jq -r '.state.spent | if type=="object" then .Bool else . end')
  HOLDER0=$(printf '%s' "$ST" | jq -r '.state.holder.Address[0] // (.state.holder|.[0]) // -1')
  log "Step 6/6 - verdict (spend)"
  (( saw_value == 1 )) || die "non-vacuity: never confirmed an issued note with value" 5
  [[ "$SPENT" == "true" ]] || die "spend did not set spent=true (got '$SPENT') — circulation unproven" 6
  [[ "$HOLDER0" == "$SPEND_TO_U8" ]] || die "holder did not transfer to $SPEND_TO_U8 (holder[0]=$HOLDER0) — claim did not move" 6
  cat <<EOF

+==================================================================+
|     OK  EVAPORCASH — CIRCULATION PRESERVES VALUE (the other half) |
|  contract_id: $CID                                                |
|  proof: deploy ok  issue ok  HAD-VALUE ok  ->  SPENT & MOVED      |
|  spend(to=$SPEND_TO_U8): state.spent=true, holder[0]=$HOLDER0 —    |
|  the claim circulated to a new holder (retired-by-spend, NOT      |
|  lost to evaporation). Hoarding loses; circulating preserves.     |
+==================================================================+
EOF
fi
