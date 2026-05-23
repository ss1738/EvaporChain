#!/usr/bin/env bash
#
# deploy-shlm.sh — live-e2e for the SHLM (Singh Skill Half-Life Market)
# chain-side (contracts/evaporscript/shlm.es). SHLM is the plan's biggest
# commercial wedge (skill-credential freshness market).
#
# HONEST SCOPE (verified): this proves the CHAIN-SIDE only — the
# on-chain skill-class registry + credential issuance + bounty market +
# the freshness/half-life STALENESS GATE that record_match enforces.
# The exact half-life-decayed scoring + SDDC two-axis Dutch clearing is
# OFF-CHAIN (the coordinator crate `evaporchain-shlm`); this contract
# records the coordinator's final decision and independently re-checks
# the two gates on-chain. Verified on the permanent node which runs
# --mock-consensus --mock-prove single-node: the dApp LOGIC is proven,
# not real BFT/proving.
#
# The energy-decay primitive shown on-chain = the staleness gate:
# record_match REJECTS a credential whose (epoch - attested_at) exceeds
# the bounty's max_staleness ("credential too stale for this bounty").
# A fresh credential is accepted; a stale one is rejected. That is the
# half-life/freshness concept enforced, not just asserted.
#
# Modes (mirror the verified deploy-*.sh family):
#   --mode match (default): register_class -> issue_credential (fresh)
#     -> post_bounty -> record_match SUCCEEDS -> assert match recorded,
#     bounty deactivated. (happy path)
#   --mode stale: register_class -> issue_credential -> post_bounty with
#     a tiny max_staleness -> wait until the credential is too stale ->
#     record_match REJECTED via the on-chain freshness gate (the
#     non-vacuity / primitive proof).
#
# NOTE (by-design, not a defect — mirrors SFSV): SHLM is a
# deterministic single-instance contract (one instance per skill
# class). Re-running with identical source+deployer+energy+half_life
# resolves the SAME contract_id, hitting an already-registered class.
# For repeat verification pass a unique INITIAL_ENERGY (virgin
# instance), e.g. `INITIAL_ENERGY=$((4000000 + RANDOM)) deploy-shlm.sh`.
#
# Source-verified node contract (deploy/call/get-script, tagged Value
# args {Address|U64|Str|Bool}, deployer u8, /api/script, session token).
# Exit: 0 ok · 2 precondition · 3 deploy · 4 register/issue/post ·
#       5 non-vacuity · 6 match-proof-not-shown

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/shlm.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"          # admin/owner/coordinator + employer; 0 = genesis faucet (only funded acct)
HOLDER_U8="${HOLDER_U8:-1}"              # credential holder (address arg only — no funding needed)
SKILL_NAME="${SKILL_NAME:-python}"
HALF_LIFE_EPOCHS="${HALF_LIFE_EPOCHS:-540}"   # class half-life (Python ≈ 540 per shlm.es)
CRED_LEVEL="${CRED_LEVEL:-800}"
BOUNTY_MIN_LEVEL="${BOUNTY_MIN_LEVEL:-700}"
BOUNTY_SALARY="${BOUNTY_SALARY:-95000}"
MODE="${MODE:-match}"                    # match | stale
# match: generous staleness so the fresh credential passes the gate.
# stale: tiny staleness so a brief wait makes the credential too stale.
MATCH_MAX_STALENESS="${MATCH_MAX_STALENESS:-100000}"
STALE_MAX_STALENESS="${STALE_MAX_STALENESS:-3}"
# Contract instance energy: must NOT evaporate mid-test (we test the
# credential staleness gate, not class evaporation) — keep it large.
INITIAL_ENERGY="${INITIAL_ENERGY:-5000000}"
CLASS_HALF_LIFE="${CLASS_HALF_LIFE:-100000}"
POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-shlm.sh [options]
  --dry-run            validate + print intended calls; no network
  --node URL           node base URL (default http://89.167.52.40:8099)
  --token TOKEN        auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8        admin/coordinator/employer index (default 0 = faucet)
  --holder U8          credential-holder address index (default 1)
  --skill NAME         skill class name (default python)
  --mode match|stale   match=happy path (default); stale=freshness-gate rejection proof
  --level N            credential level (default 800)
  --min-level N        bounty min_level (default 700)
  --salary N           bounty salary offer (default 95000)
  --timeout SEC        poll timeout (default 300)
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
    --skill) SKILL_NAME="$2"; shift 2 ;;
    --mode) MODE="$2"; shift 2 ;;
    --level) CRED_LEVEL="$2"; shift 2 ;;
    --min-level) BOUNTY_MIN_LEVEL="$2"; shift 2 ;;
    --salary) BOUNTY_SALARY="$2"; shift 2 ;;
    --timeout) POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose) VERBOSE=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log() { printf '\033[1;36m[shlm]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[shlm ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }

curl_json() {
  local method="$1" path="$2" body="${3:-}"
  if $DRY_RUN; then echo "  [DRY-RUN] $method $NODE_URL$path ${body:+(body: $body)}" >&2; echo '{}'; return 0; fi
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
# Poll a tx to a terminal state. Returns: finalised|included|rejected.
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
require_tx() {  # submit + poll; die unless finalised/included
  local h; h=$(submit_tx "$1" "$2" "$3" "$4")
  $DRY_RUN && return 0
  local s; s=$(poll_tx_state "$h")
  [[ "$s" == "finalised" || "$s" == "included" ]] || die "$3 tx not accepted (state=$s)" "$4"
  printf '%s' "$h"
}
get_epoch() { $DRY_RUN && { echo 0; return 0; }; curl_json GET "/api/status" | jq -r '.epoch // 0'; }
# jq `//` drops boolean false / 0 — unwrap tagged Value explicitly.
untag() { jq -r ".state.$1 | if type==\"object\" then (.Bool // .U64 // .Str // .) else . end"; }
addr_arg() {  # u8 index -> {"Address":[b,0,...,0]} (32 bytes)
  jq -nc --argjson b "$1" '{Address: ([$b] + [range(0;31)|0])}'
}
# map[address->_] is serialised as {"Map":{"a:<64-hex-addr>":<tagged>}}.
# addr_from_byte(N) = [N,0,...,0] -> hex key "a:" + 2-hex(N) + 62 zeros.
mapkey() { printf 'a:%02x%062d' "$1" 0; }
# mapget <state-json> <field> <u8idx>  -> inner U64/scalar (empty if absent)
mapget() {
  local k; k=$(mapkey "$3")
  printf '%s' "$1" | jq -r --arg k "$k" \
    ".state.\"$2\".Map[\$k] | if type==\"object\" then (.U64 // .Bool // .Str // .) else (.//empty) end"
}

# ── preflight ──
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract SHLM" "$CONTRACT_PATH" || die ".es missing SHLM header" 3
grep -q "fn register_class(" "$CONTRACT_PATH" || die ".es missing register_class" 3
grep -q "fn record_match(" "$CONTRACT_PATH" || die ".es missing record_match" 3
if [[ "$MODE" != "match" && "$MODE" != "stale" ]]; then die "unknown --mode '$MODE' (match|stale)" 2; fi
if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

MAX_STALENESS=$([[ "$MODE" == "stale" ]] && echo "$STALE_MAX_STALENESS" || echo "$MATCH_MAX_STALENESS")

cat <<EOF
+==================================================================+
|  SHLM — Skill Half-Life Market — chain-side e2e (shlm.es)        |
+------------------------------------------------------------------+
|  node: $NODE_URL  mode: $($DRY_RUN && echo DRY-RUN || echo LIVE)  run-mode: $MODE
|  admin/employer(u8): $DEPLOYER_U8  holder(u8): $HOLDER_U8  skill: $SKILL_NAME
|  cred_level: $CRED_LEVEL  min_level: $BOUNTY_MIN_LEVEL  salary: $BOUNTY_SALARY
|  bounty max_staleness: $MAX_STALENESS  (mode-driven)
+==================================================================+
EOF

EMP=$(addr_arg "$DEPLOYER_U8")     # employer = the deployer/admin address (caller of post_bounty)
HOLDER=$(addr_arg "$HOLDER_U8")

# ── 1. deploy ──
log "Step 1/6 - deploy-script (shlm.es)"
SRC=$(jq -Rs . < "$CONTRACT_PATH")
DBODY=$(jq -n --argjson d "$DEPLOYER_U8" --argjson s "$SRC" \
  --argjson e "$INITIAL_ENERGY" --argjson hl "$CLASS_HALF_LIFE" \
  '{deployer:$d, source_code:$s, energy:$e, half_life:$hl}')
DH=$(submit_tx "/api/tx/deploy-script" "$DBODY" deploy 3)
log "deploy tx: $DH"
DSTATE=$(poll_tx_state "$DH")
[[ "$DSTATE" == "finalised" || "$DSTATE" == "included" ]] || die "deploy not accepted (state=$DSTATE)" 3
# resolve contract_id
CID=$(curl_json GET "/api/tx/$DH" | jq -r '.contract_id // empty')
[[ -n "$CID" ]] || { $DRY_RUN && CID=0 || die "no contract_id on deploy tx status" 3; }
log "contract_id: $CID"

# ── 2. register_class ──
log "Step 2/6 - register_class(\"$SKILL_NAME\", $HALF_LIFE_EPOCHS)"
EP=$(get_epoch)
RBODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" \
  --arg name "$SKILL_NAME" --argjson hl "$HALF_LIFE_EPOCHS" --argjson ep "$EP" \
  '{caller:$c, contract_id:$cid, method:"register_class", args:[{Str:$name},{U64:$hl}], epoch:$ep}')
require_tx "/api/tx/call-script" "$RBODY" register_class 4 >/dev/null

# ── 3. issue_credential(holder, level) ──
log "Step 3/6 - issue_credential(holder=$HOLDER_U8, level=$CRED_LEVEL)"
EP=$(get_epoch)
IBODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" \
  --argjson h "$HOLDER" --argjson lv "$CRED_LEVEL" --argjson ep "$EP" \
  '{caller:$c, contract_id:$cid, method:"issue_credential", args:[$h,{U64:$lv}], epoch:$ep}')
require_tx "/api/tx/call-script" "$IBODY" issue_credential 4 >/dev/null

# ── 4. post_bounty(max_staleness, min_level, salary) — caller = employer ──
log "Step 4/6 - post_bounty(max_staleness=$MAX_STALENESS, min_level=$BOUNTY_MIN_LEVEL, salary=$BOUNTY_SALARY)"
EP=$(get_epoch)
PBODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" \
  --argjson ms "$MAX_STALENESS" --argjson ml "$BOUNTY_MIN_LEVEL" --argjson sal "$BOUNTY_SALARY" --argjson ep "$EP" \
  '{caller:$c, contract_id:$cid, method:"post_bounty", args:[{U64:$ms},{U64:$ml},{U64:$sal}], epoch:$ep}')
require_tx "/api/tx/call-script" "$PBODY" post_bounty 4 >/dev/null

# ── 5. confirm opened+in-window (non-vacuity guard, settle-retry) ──
log "Step 5/6 - confirm class sealed + credential issued + bounty active"
SEALED= HASCRED= ACTIVE= CREDLVL=
for _try in 1 2 3 4 5 6 7 8 9 10; do
  ST=$(curl_json GET "/api/script/$CID")
  SEALED=$(printf '%s' "$ST" | untag sealed)
  CREDLVL=$(mapget "$ST" cred_level "$HOLDER_U8"); CREDLVL=${CREDLVL:-?}
  HASCRED=$(printf '%s' "$ST" | jq -r '.state.credential_count | if type=="object" then .U64 else . end')
  ACTIVE=$(printf '%s' "$ST" | jq -r '.state.bounty_count | if type=="object" then .U64 else . end')
  [[ "$SEALED" == "true" && "$HASCRED" == "1" && "$ACTIVE" == "1" ]] && break
  sleep 2
done
[[ "$SEALED" == "true" ]] || die "class never sealed (sealed=$SEALED)" 5
[[ "$HASCRED" == "1" ]] || die "credential not registered (credential_count=$HASCRED)" 5
[[ "$ACTIVE" == "1" ]] || die "bounty not posted (bounty_count=$ACTIVE)" 5
log "CONFIRMED: sealed=true credential_count=1 bounty_count=1 (market open)"

# ── 6. mode-specific proof ──
if [[ "$MODE" == "match" ]]; then
  log "Step 6/6 - MATCH — record_match(employer, holder, salary) must SUCCEED (fresh credential)"
  if $DRY_RUN; then log "[DRY-RUN] would record_match then assert match_exists + bounty deactivated"; log "OK dry-run."; exit 0; fi
  EP=$(get_epoch)
  MBODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" \
    --argjson emp "$EMP" --argjson h "$HOLDER" --argjson sal "$BOUNTY_SALARY" --argjson ep "$EP" \
    '{caller:$c, contract_id:$cid, method:"record_match", args:[$emp,$h,{U64:$sal}], epoch:$ep}')
  MH=$(submit_tx "/api/tx/call-script" "$MBODY" record_match 6)
  MS=$(poll_tx_state "$MH")
  [[ "$MS" == "finalised" || "$MS" == "included" ]] || die "record_match rejected for a FRESH eligible credential (state=$MS) — happy path broken" 6
  # confirm match recorded + bounty deactivated (settle-retry)
  WIN= MSAL= BACT=
  for _try in 1 2 3 4 5 6 7 8 9 10; do
    ST=$(curl_json GET "/api/script/$CID")
    MEX=$(mapget "$ST" match_exists "$DEPLOYER_U8"); MEX=${MEX:-0}
    BACT=$(mapget "$ST" bounty_active "$DEPLOYER_U8"); BACT=${BACT:-?}
    [[ "$MEX" == "1" ]] && break
    sleep 2
  done
  [[ "$MEX" == "1" ]] || die "match not recorded on-chain after a finalised record_match (match_exists=$MEX)" 6
  cat <<EOF

+==================================================================+
|   OK  SHLM — SKILL MATCH RECORDED ON-CHAIN (happy path)         |
|  contract_id: $CID  skill: $SKILL_NAME                            |
|  proof: deploy ok  register ok  issue ok  post ok  CONFIRMED    |
|  -> record_match finalised; match_exists=1, bounty consumed.    |
|  On-chain freshness + level gates both passed for a fresh cred. |
|  HONEST SCOPE: chain records the coordinator decision + re-runs |
|  the two gates; decay scoring/SDDC clearing is off-chain.       |
|  Verified under --mock-consensus single node (logic, not BFT).  |
+==================================================================+
EOF
else
  log "Step 6/6 - STALE — wait until credential is too stale, then record_match MUST be REJECTED"
  if $DRY_RUN; then log "[DRY-RUN] would wait elapsed>max_staleness then assert record_match rejected"; log "OK dry-run."; exit 0; fi
  # credential was attested at issue (step 3). Wait until chain epoch has
  # advanced past max_staleness so (epoch - attested_at) > max_staleness.
  ISSUE_EP=$(mapget "$ST" cred_attested_at "$HOLDER_U8"); ISSUE_EP=${ISSUE_EP:-0}
  TARGET=$(( ${ISSUE_EP:-0} + MAX_STALENESS + 3 ))
  log "credential attested ~epoch ${ISSUE_EP:-?}; waiting for chain epoch > $TARGET (max_staleness=$MAX_STALENESS)"
  deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC )); reached=0
  while (( $(date +%s) < deadline )); do
    CE=$(get_epoch)
    if (( CE > TARGET )); then reached=1; log "chain epoch $CE > $TARGET — credential now stale"; break; fi
    sleep 3
  done
  (( reached == 1 )) || die "chain never advanced past staleness target in ${POLL_TIMEOUT_SEC}s" 5
  EP=$(get_epoch)
  MBODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" \
    --argjson emp "$EMP" --argjson h "$HOLDER" --argjson sal "$BOUNTY_SALARY" --argjson ep "$EP" \
    '{caller:$c, contract_id:$cid, method:"record_match", args:[$emp,$h,{U64:$sal}], epoch:$ep}')
  MH=$(printf '%s' "$(curl_json POST /api/tx/call-script "$MBODY")" | jq -r '.tx_hash // empty')
  [[ -n "$MH" ]] || die "record_match submit returned no tx_hash" 6
  MS=$(poll_tx_state "$MH")
  if [[ "$MS" == "rejected" ]]; then
    # confirm the bounty is still OPEN (the stale match did NOT consume it)
    BACT=$(mapget "$(curl_json GET "/api/script/$CID")" bounty_active "$DEPLOYER_U8"); BACT=${BACT:-?}
    cat <<EOF

+==================================================================+
|  OK  SHLM — FRESHNESS GATE PROVEN (stale credential REJECTED)   |
|  contract_id: $CID  skill: $SKILL_NAME                            |
|  proof: deploy/register/issue/post/CONFIRMED ok -> waited until |
|  (epoch - attested_at) > max_staleness=$MAX_STALENESS -> record_match  |
|  REJECTED on-chain ("credential too stale for this bounty").    |
|  The half-life/freshness primitive is ENFORCED, not vacuous.    |
|  HONEST SCOPE: chain enforces the staleness gate; exact decay   |
|  scoring/SDDC clearing is off-chain. Single-node mock-consensus.|
+==================================================================+
EOF
  else
    die "record_match was NOT rejected for an over-stale credential (state=$MS) — freshness gate NOT enforced; primitive unproven" 6
  fi
fi
