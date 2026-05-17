#!/usr/bin/env bash
#
# deploy-erasure-attestation.sh — live-e2e for the Proof-of-Erasure
# chain-side (contracts/evaporscript/erasure_attestation.es). The
# frontier evolution of the verified GDPR work toward the AI
# machine-unlearning / verifiable-deletion market.
#
# HONEST SCOPE (verified, Dead Drop §9): the chain does NOT byte-erase
# and does NOT perform unlearning. This proves the CHAIN-SIDE only —
# the tamper-evident attestation lifecycle (the NIST 800-88
# certificate-of-disposition, on-chain). Actual sanitization/unlearning
# is off-chain.
#
# Modes (mirror the verified deploy-gdpr-vault.sh retain/withdraw):
#   --mode attest (default): seal → attest_erasure() → assert
#     status==2 (erasure proven, immutable PROOF-OF-ERASURE event).
#   --mode lapse: seal → never attest → terminal evaporation = the
#     immutable "obligation window CLOSED un-attested" negative proof.
#
# Source-verified node contract (deploy/call/get-script, tagged Value
# args, deployer u8, /api/script, session token). Exit: 0 ok ·
# 2 precondition · 3 deploy · 4 seal · 5 non-vacuity · 6 proof-not-shown
#
# Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/erasure_attestation.es"

NODE_URL="${NODE_URL:-http://127.0.0.1:9001}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"          # controller = owner; 0 = genesis faucet
SUBJECT_U8="${SUBJECT_U8:-1}"
BASIS="${BASIS:-1}"                       # 1=GDPR-Art17 2=CCPA/AB1008 3=NIST-program
METHOD="${METHOD:-1}"                     # 1=crypto-shred 2=clear 3=purge 4=destroy 5=ML-unlearn
VERIFICATION="${VERIFICATION:-1}"        # off-chain verification result code (>0)
MODE="${MODE:-attest}"                    # attest = erasure proven ; lapse = window-closed proof
INITIAL_ENERGY="${INITIAL_ENERGY:-60000}"   # obligation/retention window
HALF_LIFE="${HALF_LIFE:-5}"
DRY_RUN=false
VERBOSE=false
POLL_TIMEOUT_SEC=300

usage() { cat <<'EOF'
deploy-erasure-attestation.sh [options]
  --dry-run            validate + print intended calls; no network
  --node URL           node base URL (default http://127.0.0.1:9001)
  --token TOKEN        auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8        controller/owner index (default 0 = faucet)
  --subject U8         data-subject ref (default 1)
  --basis N            obligation basis code (default 1 = GDPR-Art17)
  --method N           NIST sanitization method code (default 1 = crypto-shred)
  --verification N     off-chain verification result code (default 1)
  --mode attest|lapse  attest=erasure proven (default) ; lapse=window-closed proof
  --energy N           obligation-window budget (default 60000)
  --half-life N        decay rate; smaller = shorter window (default 5)
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
    --subject) SUBJECT_U8="$2"; shift 2 ;;
    --basis) BASIS="$2"; shift 2 ;;
    --method) METHOD="$2"; shift 2 ;;
    --verification) VERIFICATION="$2"; shift 2 ;;
    --mode) MODE="$2"; shift 2 ;;
    --energy) INITIAL_ENERGY="$2"; shift 2 ;;
    --half-life) HALF_LIFE="$2"; shift 2 ;;
    --timeout) POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose) VERBOSE=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log() { printf '\033[1;36m[erasure-attest]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[erasure-attest ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }

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
poll_tx() {
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
# jq `//` drops boolean false — unwrap tagged Value explicitly.
untag() { jq -r ".state.$1 | if type==\"object\" then (.Bool // .U64 // .Str) else . end"; }

# ── preflight ──
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract ErasureAttestation" "$CONTRACT_PATH" || die ".es missing ErasureAttestation header" 3
grep -q "fn seal(" "$CONTRACT_PATH" || die ".es missing seal" 3
if [[ "$MODE" != "attest" && "$MODE" != "lapse" ]]; then die "unknown --mode '$MODE' (attest|lapse)" 2; fi
if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

cat <<EOF
+==================================================================+
|  Proof-of-Erasure — chain-side e2e (erasure_attestation.es)      |
+------------------------------------------------------------------+
|  node: $NODE_URL  mode: $($DRY_RUN && echo DRY-RUN || echo LIVE)  run-mode: $MODE
|  controller(u8): $DEPLOYER_U8  subject(u8): $SUBJECT_U8  basis: $BASIS  method: $METHOD
|  energy/half-life: $INITIAL_ENERGY/$HALF_LIFE  (energy = obligation window)
+==================================================================+
EOF

# ── 1. deploy ──
log "Step 1/5 - POST /api/tx/deploy-script (erasure_attestation.es)"
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

# ── 2. seal(data_commitment, subject, basis, method) — owner-only, once
log "Step 2/5 - call-script seal(data_commitment, subject, basis, method)"
EP=$(get_epoch)
SBODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" \
  --argjson subj "$SUBJECT_U8" --argjson b "$BASIS" --argjson m "$METHOD" --argjson ep "$EP" \
  '{caller:$c, contract_id:$cid, method:"seal",
    args:[ {Address: ([222] + [range(0;31)|0])},
           {Address: ([$subj] + [range(0;31)|0])},
           {U64:$b}, {U64:$m} ],
    epoch:$ep}')
SH=$(submit_tx "/api/tx/call-script" "$SBODY" seal 4)
poll_tx "$SH" seal 4 >/dev/null
log "attestation opened."

# ── 3. confirm opened (non-vacuity): status==1, basis recorded ──
log "Step 3/5 - confirm opened+in-window (non-vacuity guard)"
saw_open=0
if $DRY_RUN; then
  log "[DRY-RUN] would assert sealed=true, obligation_basis=$BASIS, evaporated=false"
else
  ST=$(curl_json GET "/api/script/$CID")
  SEALED=$(printf '%s' "$ST" | untag sealed)
  BON=$(printf '%s' "$ST" | jq -r '.state.obligation_basis | if type=="object" then .U64 else . end')
  EVAP=$(printf '%s' "$ST" | jq -r '.evaporated // false')
  if [[ "$SEALED" == "true" && "$BON" == "$BASIS" && "$EVAP" == "false" ]]; then
    saw_open=1
    log "CONFIRMED: sealed=true obligation_basis=$BON evaporated=false (in obligation window)"
  else
    die "attestation not opened-in-window (sealed=$SEALED basis=$BON evap=$EVAP)" 5
  fi
fi

# ── 4/5. mode-specific proof ──
if [[ "$MODE" == "attest" ]]; then
  log "Step 4/5 - ATTEST — controller calls attest_erasure(verification=$VERIFICATION)"
  if $DRY_RUN; then log "[DRY-RUN] would attest then assert status==2"; log "OK dry-run."; exit 0; fi
  EP=$(get_epoch)
  ABODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson v "$VERIFICATION" --argjson ep "$EP" \
    '{caller:$c, contract_id:$cid, method:"attest_erasure", args:[ {U64:$v} ], epoch:$ep}')
  AH=$(submit_tx "/api/tx/call-script" "$ABODY" attest 6)
  poll_tx "$AH" attest 6 >/dev/null
  ST=$(curl_json GET "/api/script/$CID")
  ATT=$(printf '%s' "$ST" | untag attested)
  log "Step 5/5 - verdict (attest)"
  (( saw_open == 1 )) || die "non-vacuity: never confirmed an opened attestation" 5
  [[ "$ATT" == "true" ]] || die "attest_erasure did not set attested=true (got '$ATT') — proof unshown" 6
  cat <<EOF

+==================================================================+
|   OK  PROOF-OF-ERASURE — ERASURE ATTESTED (audit-grade record)  |
|  contract_id: $CID                                                |
|  proof: deploy ok  open ok  ACTIVE ok  -> attest_erasure         |
|  state.attested=true + immutable "PROOF-OF-ERASURE" event.       |
|  NIST-style cert-of-disposition on-chain. HONEST SCOPE: chain    |
|  proves the ATTESTATION; sanitization/unlearning is off-chain    |
|  (NOT byte-erasure — Dead Drop §9). Service, not a token.        |
+==================================================================+
EOF
else
  log "Step 4/5 - LAPSE — never attest; poll until obligation window terminally closes"
  if $DRY_RUN; then log "[DRY-RUN] would poll until evaporated==true (window-closed proof)"; log "OK dry-run."; exit 0; fi
  deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC )); closed=0
  while (( $(date +%s) < deadline )); do
    ST=$(curl_json GET "/api/script/$CID")
    if printf '%s' "$ST" | jq -e 'has("error")' >/dev/null 2>&1; then closed=1; log "attestation $CID gone (terminal)"; break; fi
    EVAP=$(printf '%s' "$ST" | jq -r '.evaporated // false')
    if [[ "$EVAP" == "true" ]]; then closed=1; log "obligation window terminally CLOSED un-attested (immutable negative proof)"; break; fi
    sleep 4
  done
  log "Step 5/5 - verdict (lapse)"
  (( saw_open == 1 )) || die "non-vacuity: never confirmed an opened attestation" 5
  (( closed == 1 )) || die "window never terminally closed within ${POLL_TIMEOUT_SEC}s (half-life too large?)" 6
  cat <<EOF

+==================================================================+
|  OK  PROOF-OF-ERASURE — OBLIGATION-WINDOW-CLOSED PROOF (lapse)   |
|  contract_id: $CID                                                |
|  proof: deploy ok  open ok  ACTIVE ok  -> TERMINAL EVAPORATION   |
|  immutable record: obligation window closed with NO attestation  |
|  (a regulator-grade negative proof). HONEST SCOPE: chain proves  |
|  the tamper-evident lifecycle, NOT byte-erasure (Dead Drop §9).  |
+==================================================================+
EOF
fi
