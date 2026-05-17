#!/usr/bin/env bash
#
# deploy-sfsv.sh — End-to-end deployment runbook for the SFSV reference dApp.
#
# REWRITTEN 2026-05-16 to the source-verified node API (see
# VERIFICATION_2026_05_16.md "DEPLOY-RUNBOOK API SPEC"). The prior #357
# version was mis-shaped vs crates/evaporchain-node/src/api.rs:
#   - deployer/caller are u8 account indices, NOT hex addresses
#   - call-script requires contract_id:u64 + epoch:u64
#   - no /api/contract/by-deploy/:hash  (contract_id is in /api/tx/:hash)
#   - EvaporScript state is at GET /api/script/:id (.state tagged map),
#     NOT /api/contract/:id (that is the unrelated template store)
#   - no predicate_satisfied field — try_payout self-guards; retry it.
#
# Flow (all source-verified except the epoch-field note in get_epoch):
#   1. POST /api/tx/deploy-script   → deploy future_self_vault.es
#   2. poll GET /api/tx/<hash>      → state included; read .contract_id
#   3. call-script set_terms(...)   → seal the vault
#   4. retry call-script try_payout → reverts until predicate trips
#   5. GET /api/script/<id>         → assert .state.released is truthy
#
#   ./scripts/deploy-sfsv.sh --dry-run
#   ./scripts/deploy-sfsv.sh --node http://127.0.0.1:9001 \
#       --deployer 1 --future-self 2 --predicate 0 --release-param 200 \
#       --energy 1000000 --half-life 64 --deposit 1000
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 set_terms · 5 try_payout · 6 verify
#
# Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/future_self_vault.es"

NODE_URL="${NODE_URL:-http://127.0.0.1:9001}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-1}"          # u8 devnet account index (tx deployer/caller)
FUTURE_SELF="${FUTURE_SELF:-2}"          # set_terms `future_self` arg (.es address)
PREDICATE_TYPE="${PREDICATE_TYPE:-0}"    # 0=EpochReached 1=EnergyDecaysBelow
RELEASE_PARAM="${RELEASE_PARAM:-200}"    # epoch (type0) or threshold (type1)
INITIAL_ENERGY="${INITIAL_ENERGY:-1000000}"
HALF_LIFE="${HALF_LIFE:-64}"
DEPOSIT_AMOUNT="${DEPOSIT_AMOUNT:-1000}"
DRY_RUN=false
VERBOSE=false
POLL_TIMEOUT_SEC=180

usage() { cat <<'EOF'
deploy-sfsv.sh [options]
  --dry-run               validate + print intended calls; no network
  --node URL              node base URL (default http://127.0.0.1:9001)
  --token TOKEN           auth token ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8           deployer/caller account index (u8)
  --future-self V         set_terms future_self arg
  --predicate {0,1}       0=EpochReached 1=EnergyDecaysBelow
  --release-param N       epoch (type0) or energy threshold (type1)
  --energy N              initial contract energy
  --half-life N           energy half-life (epochs)
  --deposit N             deposit_amount snapshot
  --timeout SEC           per-step poll timeout (default 180)
  --verbose               echo curl responses
  -h|--help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=true; shift ;;
    --node) NODE_URL="$2"; shift 2 ;;
    --token) TOKEN="$2"; shift 2 ;;
    --deployer) DEPLOYER_U8="$2"; shift 2 ;;
    --future-self) FUTURE_SELF="$2"; shift 2 ;;
    --predicate) PREDICATE_TYPE="$2"; shift 2 ;;
    --release-param) RELEASE_PARAM="$2"; shift 2 ;;
    --energy) INITIAL_ENERGY="$2"; shift 2 ;;
    --half-life) HALF_LIFE="$2"; shift 2 ;;
    --deposit) DEPOSIT_AMOUNT="$2"; shift 2 ;;
    --timeout) POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose) VERBOSE=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[deploy-sfsv]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[deploy-sfsv]\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[deploy-sfsv ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }

curl_json() {  # curl_json <METHOD> <path> [json-body]
  local method="$1" path="$2" body="${3:-}"
  if $DRY_RUN; then echo "  [DRY-RUN] $method $NODE_URL$path ${body:+(body: $body)}" >&2; echo '{}'; return 0; fi
  local args=(-sS -m 30 -X "$method" -H 'Content-Type: application/json')
  [[ -n "$TOKEN" ]] && args+=(-H "Authorization: Bearer $TOKEN")
  [[ -n "$body" ]] && args+=(-d "$body")
  local resp; resp=$(curl "${args[@]}" "$NODE_URL$path") || die "curl $method $path failed" 2
  $VERBOSE && echo "  ← $resp" >&2
  printf '%s' "$resp"
}

submit_tx() {  # submit_tx <path> <body> <name> <failcode> → tx hash
  local resp; resp=$(curl_json POST "$1" "$2")
  $DRY_RUN && { echo "DRYHASH"; return 0; }
  local hash; hash=$(printf '%s' "$resp" | jq -r '.tx_hash // empty')
  [[ -n "$hash" ]] || die "$3 failed: $(printf '%s' "$resp" | jq -r '.message // .error // "(no msg)"')" "$4"
  printf '%s' "$hash"
}

# Poll /api/tx/:hash. Echoes the final status JSON. Dies on rejected/timeout.
poll_tx() {  # poll_tx <hash> <name> <failcode>
  $DRY_RUN && { echo '{"state":"included","contract_id":0,"epoch":0}'; return 0; }
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

# Current chain epoch. NOTE: confirm get_status's epoch field name on first
# live run; the tx-status response also carries `.epoch` (api.rs:8695).
get_epoch() {
  $DRY_RUN && { echo 0; return 0; }
  curl_json GET "/api/status" | jq -r '.epoch // .height // 0'
}

# ── preflight ──
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract FutureSelfVault" "$CONTRACT_PATH" || die ".es missing contract header" 3
grep -q "fn set_terms(" "$CONTRACT_PATH" || die ".es missing set_terms" 3
grep -q "fn try_payout(" "$CONTRACT_PATH" || die ".es missing try_payout" 3
(( $(wc -c < "$CONTRACT_PATH") <= 65536 )) || die ".es exceeds 64KB node cap" 3
if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/health" >/dev/null 2>&1 || \
    curl -sS -m 5 "$NODE_URL/api/version" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

cat <<EOF
╔══════════════════════════════════════════════════════════════════╗
║  SFSV reference dApp — end-to-end deploy (source-verified spec)  ║
╠══════════════════════════════════════════════════════════════════╣
║  node: $NODE_URL   mode: $($DRY_RUN && echo DRY-RUN || echo LIVE)
║  deployer(u8): $DEPLOYER_U8   future_self: $FUTURE_SELF
║  predicate: $PREDICATE_TYPE   release_param: $RELEASE_PARAM
║  energy/hl: $INITIAL_ENERGY/$HALF_LIFE   deposit: $DEPOSIT_AMOUNT
╚══════════════════════════════════════════════════════════════════╝
EOF

# ── 1. deploy ──
log "Step 1/4 — POST /api/tx/deploy-script"
SRC=$(jq -Rs . < "$CONTRACT_PATH")
DEPLOY_BODY=$(jq -n --argjson d "$DEPLOYER_U8" --argjson s "$SRC" \
  --argjson e "$INITIAL_ENERGY" --argjson hl "$HALF_LIFE" \
  '{deployer:$d, source_code:$s, energy:$e, half_life:$hl}')
DH=$(submit_tx "/api/tx/deploy-script" "$DEPLOY_BODY" deploy 3)
log "deploy tx: $DH"
DEPLOY_STATUS=$(poll_tx "$DH" deploy 3)
CID=$(printf '%s' "$DEPLOY_STATUS" | jq -r '.contract_id // empty')
[[ -n "$CID" ]] || { $DRY_RUN && CID=0 || die "no contract_id on deploy tx status" 3; }
log "contract_id: $CID"

# ── 2. set_terms ──
log "Step 2/4 — call-script set_terms"
EPOCH=$(get_epoch)
# call-script args decode as Vec<evaporchain_script::Value> — an
# EXTERNALLY-TAGGED serde enum (verified: evaporchain-script/src/lib.rs
# `pub enum Value`, no #[serde(untagged)]; canonical form proven by
# evaporchain-execution tests, e.g. args: r#"[{"U64": 42}]"#).
#   u64     → {"U64": n}
#   address → {"Address": [b0,…,b31]}  (32 bytes; node maps a u8 index
#             to addr_from_byte(b) = [b,0,…,0], so future_self index N
#             ⇒ first byte N, rest zero — mirrors --deployer/--caller).
# `.es` set_terms(future_self_addr: address, predicate: u64,
#                 release_param: u64, deposit_amount: u64).
ST_BODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" \
  --argjson fsb "$FUTURE_SELF" --argjson p "$PREDICATE_TYPE" \
  --argjson rp "$RELEASE_PARAM" --argjson dp "$DEPOSIT_AMOUNT" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"set_terms",
    args:[ {Address: ([$fsb] + [range(0;31)|0])},
           {U64:$p}, {U64:$rp}, {U64:$dp} ],
    epoch:$ep}')
SH=$(submit_tx "/api/tx/call-script" "$ST_BODY" set_terms 4)
poll_tx "$SH" set_terms 4 >/dev/null
log "vault sealed."

# ── 3. retry try_payout until predicate trips (.es self-guards) ──
log "Step 3/4 — retry call-script try_payout until included"
if $DRY_RUN; then
  log "[DRY-RUN] would loop call-script try_payout until tx state=included"
else
  deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC )); ok=0; saw_gate=0
  while (( $(date +%s) < deadline )); do
    EPOCH=$(get_epoch)
    TP_BODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
      '{caller:$c, contract_id:$cid, method:"try_payout", args:[], epoch:$ep}')
    TH=$(printf '%s' "$(curl_json POST /api/tx/call-script "$TP_BODY")" | jq -r '.tx_hash // empty')
    [[ -z "$TH" ]] && { sleep 4; continue; }
    rs=$(curl_json GET "/api/tx/$TH" | jq -r '.state // "unknown"')
    if [[ "$rs" == "included" || "$rs" == "finalised" ]]; then ok=1; log "try_payout finalised at epoch $EPOCH"; break; fi
    [[ "$rs" == "rejected" ]] && saw_gate=1   # predicate `require` reverted — gate is real
    sleep 4   # rejected/pending ⇒ predicate not satisfied yet; retry
  done
  (( ok == 1 )) || die "try_payout never succeeded within ${POLL_TIMEOUT_SEC}s (predicate never tripped)" 5
  # Non-vacuity: an EpochReached vault sealed before release_epoch MUST
  # have been predicate-rejected at least once before it could finalise.
  # If it finalised on the first shot the gate was never exercised
  # (release_param too low / set after the chain already passed it) —
  # that would make a "green" run meaningless.
  (( saw_gate == 1 )) || die "try_payout finalised WITHOUT a prior predicate-gated rejection — predicate gate not exercised (raise release_param margin)" 5
  log "predicate gate proven: try_payout was rejected pre-release, finalised post-release."
fi

# ── 4. verify .state.released (DIRECT observation) ──
# EvaporScript contracts live in the node's SCRIPT engine, queried at
# GET /api/script/:id (api.rs get_script, route api.rs:18971) — NOT
# /api/contract/:id, which is the unrelated TEMPLATE/CallContract store
# (an earlier version queried that, always 404'd, and was wrongly
# explained as "retired on payout / devnet limitation"). Script
# contracts persist and are fully observable; get_script returns the
# tagged `.state` map. This is a STRICT, directly-observed assertion —
# a 404 or released!=true here is a real failure. Empirically PASSED:
# contract_id 9, released={Bool:true}, payout_at={U64:5803} (2026-05-16).
log "Step 4/4 — verify GET /api/script/$CID .state.released"
if $DRY_RUN; then
  log "[DRY-RUN] would assert .state.released == {\"Bool\":true}"
  log "✓ dry-run complete."; exit 0
fi
STATE=$(curl_json GET "/api/script/$CID")
if printf '%s' "$STATE" | jq -e 'has("error")' >/dev/null 2>&1; then
  die "GET /api/script/$CID returned an error AFTER a finalised \
predicate-gated try_payout — script contracts persist on this node, so \
this is a real failure: $(printf '%s' "$STATE" | jq -c .)" 6
fi
# State fields are externally-tagged evaporchain_script::Value:
# released ⇒ {"Bool":true}, payout_at ⇒ {"U64":n}.
REL=$(printf '%s' "$STATE" | jq -r '
  (.state.released.Bool) // (.state.released) //
  (.state.Released.Bool) // (.state.Released) // "0"')
case "$REL" in
  1|true|True) log "✓ released==true DIRECTLY observed on live script state" ;;
  *) die "vault NOT released; .state.released=$REL ; full state: $(printf '%s' "$STATE" | jq -c '.state')" 6 ;;
esac
PAYOUT_AT=$(printf '%s' "$STATE" | jq -r '.state.payout_at.U64 // .state.payout_at // "?"')
cat <<EOF

╔══════════════════════════════════════════════════════════════════╗
║                  ✓ SFSV LIFECYCLE COMPLETE                       ║
║  contract_id: $CID   payout_at: $PAYOUT_AT                         ║
║  deploy: $DH                                                      ║
╚══════════════════════════════════════════════════════════════════╝
EOF
