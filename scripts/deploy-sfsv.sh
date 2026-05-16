#!/usr/bin/env bash
#
# deploy-sfsv.sh — End-to-end deployment runbook for the SFSV reference dApp.
#
# Closes gap #3 from `research/SFSV_ARCHITECTURE.md` §10.2. Drives a full
# vault lifecycle through a running EvaporChain node:
#
#   1. Deploy the `.es` contract from `contracts/evaporscript/future_self_vault.es`
#   2. Seal a vault via `set_terms(...)`                  (the lock step)
#   3. Wait for the predicate to trip                     (decay simulation)
#   4. Call `try_payout()`                                (the reclaim step)
#   5. Verify the vault transitioned to `released = true`
#
# Works against ANY running EvaporChain node — local single-node devnet,
# the 3-Mini Tailscale cluster, or a future mainnet. The node URL,
# deployer address, auth token, and predicate parameters are all
# overridable via flags or environment variables.
#
# ── Usage ──────────────────────────────────────────────────────────
#
#   ./scripts/deploy-sfsv.sh --dry-run
#       Validate the .es source + print the curl invocations that
#       WOULD be executed against $NODE_URL. Requires no running node.
#       Use this as the runbook preview before going live.
#
#   ./scripts/deploy-sfsv.sh \
#       --node http://100.119.53.101:9001 \
#       --deployer-key ~/.evaporchain/keys/deployer.json \
#       --token "$EVAPORCHAIN_TX_TOKEN" \
#       --energy 1000000 --half-life 64 --release-epoch 200
#       Execute the full deploy → seal → wait → payout flow.
#
# ── Preconditions for live mode ────────────────────────────────────
#
#   - `evaporchain-node` running and reachable at $NODE_URL.
#   - `evaporchain-cli` on $PATH (or built at $ROOT/target/release/evaporchain-cli).
#   - Deployer account funded — `evaporchain-cli faucet $DEPLOYER` against
#     the node if running in devnet/testnet mode.
#   - Auth token set: `--token` flag or `EVAPORCHAIN_TX_TOKEN` env var.
#     (Devnet usually accepts a single shared token; mainnet requires
#     per-user tokens — see `docs/runbooks/cluster-deploy.md`.)
#
# ── Exit codes ─────────────────────────────────────────────────────
#
#   0   end-to-end flow completed; vault is Released
#   2   precondition failure (missing tools, unreachable node)
#   3   deploy step failed
#   4   set_terms step failed
#   5   try_payout step failed
#   6   verification failed (vault still Locked)
#
# Co-Authored-By: Claude Opus 4.7 (1M context)

set -euo pipefail

# ── Defaults ───────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/future_self_vault.es"

NODE_URL="${NODE_URL:-http://127.0.0.1:9001}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_KEY="${DEPLOYER_KEY:-}"
DEPLOYER_ADDR="${DEPLOYER_ADDR:-}"
FUTURE_SELF_ADDR="${FUTURE_SELF_ADDR:-}"

# Predicate parameters. Sensible defaults exercise EpochReached on a
# devnet whose epoch advances every block.
PREDICATE_TYPE="${PREDICATE_TYPE:-0}"        # 0 = EpochReached, 1 = EnergyDecaysBelow
RELEASE_PARAM="${RELEASE_PARAM:-200}"         # target epoch (type 0) or threshold (type 1)
INITIAL_ENERGY="${INITIAL_ENERGY:-1000000}"
HALF_LIFE="${HALF_LIFE:-64}"
DEPOSIT_AMOUNT="${DEPOSIT_AMOUNT:-1000}"

DRY_RUN=false
VERBOSE=false
POLL_TIMEOUT_SEC=120

# ── Arg parsing ────────────────────────────────────────────────────

usage() {
    cat <<'EOF'
Usage:
    deploy-sfsv.sh [options]

Options:
    --dry-run                Validate + print what would run; no network
    --node URL               Node base URL (default: http://127.0.0.1:9001)
    --token TOKEN            Auth token (default: $EVAPORCHAIN_TX_TOKEN)
    --deployer-key PATH      ML-DSA keyfile for the deployer
    --deployer-addr HEX      Deployer's account address (32-byte hex)
    --future-self HEX        Beneficiary address (32-byte hex); defaults to deployer
    --predicate {0,1}        0 = EpochReached, 1 = EnergyDecaysBelow
    --release-param N        Release epoch (type 0) or energy threshold (type 1)
    --energy N               Initial contract energy
    --half-life N            Energy half-life in epochs
    --deposit N              Deposit amount snapshot
    --timeout SEC            Per-step poll timeout (default 120)
    --verbose                Echo every curl response
    -h, --help               This message
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)        DRY_RUN=true; shift ;;
        --node)           NODE_URL="$2"; shift 2 ;;
        --token)          TOKEN="$2"; shift 2 ;;
        --deployer-key)   DEPLOYER_KEY="$2"; shift 2 ;;
        --deployer-addr)  DEPLOYER_ADDR="$2"; shift 2 ;;
        --future-self)    FUTURE_SELF_ADDR="$2"; shift 2 ;;
        --predicate)      PREDICATE_TYPE="$2"; shift 2 ;;
        --release-param)  RELEASE_PARAM="$2"; shift 2 ;;
        --energy)         INITIAL_ENERGY="$2"; shift 2 ;;
        --half-life)      HALF_LIFE="$2"; shift 2 ;;
        --deposit)        DEPOSIT_AMOUNT="$2"; shift 2 ;;
        --timeout)        POLL_TIMEOUT_SEC="$2"; shift 2 ;;
        --verbose)        VERBOSE=true; shift ;;
        -h|--help)        usage; exit 0 ;;
        *)                echo "Unknown flag: $1" >&2; usage; exit 2 ;;
    esac
done

if [[ -z "$FUTURE_SELF_ADDR" ]]; then
    FUTURE_SELF_ADDR="$DEPLOYER_ADDR"
fi

# ── Helpers ────────────────────────────────────────────────────────

log()  { printf '\033[1;36m[deploy-sfsv]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[deploy-sfsv]\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[deploy-sfsv ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }

run_curl() {
    # run_curl <description> <curl args...>
    local desc="$1"; shift
    if $DRY_RUN; then
        echo "  [DRY-RUN] curl $*"
        return 0
    fi
    if $VERBOSE; then
        echo "  curl $*" >&2
    fi
    local resp
    resp=$(curl -sS -m 30 "$@") || die "$desc: curl failed" 2
    if $VERBOSE; then
        echo "  ← $resp" >&2
    fi
    printf '%s' "$resp"
}

# Submit a transaction body to an endpoint, return the tx hash on success.
submit_tx() {
    local endpoint="$1" body="$2" step_name="$3" fail_code="$4"
    local resp hash success
    resp=$(run_curl "$step_name" \
        -X POST \
        -H 'Content-Type: application/json' \
        ${TOKEN:+-H "Authorization: Bearer $TOKEN"} \
        -d "$body" \
        "$NODE_URL$endpoint")
    if $DRY_RUN; then
        echo "DRYRUNHASH"
        return 0
    fi
    success=$(printf '%s' "$resp" | jq -r '.success // false')
    hash=$(printf '%s' "$resp" | jq -r '.tx_hash // empty')
    if [[ "$success" != "true" || -z "$hash" ]]; then
        local msg
        msg=$(printf '%s' "$resp" | jq -r '.message // "(no message)"')
        die "$step_name failed: $msg" "$fail_code"
    fi
    printf '%s' "$hash"
}

# Poll /api/tx/:hash until included or finalised. Returns when reached;
# dies on rejected or timeout.
poll_until_included() {
    local hash="$1" step_name="$2" fail_code="$3"
    if $DRY_RUN; then return 0; fi
    local deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC ))
    while (( $(date +%s) < deadline )); do
        local resp state
        resp=$(run_curl "$step_name poll" "$NODE_URL/api/tx/$hash") || true
        state=$(printf '%s' "$resp" | jq -r '.state // "unknown"')
        case "$state" in
            included|finalised) return 0 ;;
            rejected)
                local err
                err=$(printf '%s' "$resp" | jq -r '.error // "no detail"')
                die "$step_name was rejected by chain: $err" "$fail_code"
                ;;
        esac
        sleep 2
    done
    die "$step_name did not reach 'included' within ${POLL_TIMEOUT_SEC}s" "$fail_code"
}

# ── Preflight ──────────────────────────────────────────────────────

[[ -f "$CONTRACT_PATH" ]] || die "contract not found at $CONTRACT_PATH" 2

if ! $DRY_RUN; then
    command -v curl >/dev/null || die "curl is required" 2
    command -v jq   >/dev/null || die "jq is required (https://jqlang.org)" 2
    [[ -n "$DEPLOYER_ADDR" ]] || die "--deployer-addr is required in live mode" 2
    [[ -n "$TOKEN" ]]         || warn "no auth token set — submission may be rejected"

    # Connectivity probe.
    if ! curl -sS -m 5 "$NODE_URL/api/health" >/dev/null 2>&1; then
        warn "Cannot reach $NODE_URL/api/health — is the node running?"
        warn "Falling back to /api/version probe..."
        if ! curl -sS -m 5 "$NODE_URL/api/version" >/dev/null; then
            die "Node at $NODE_URL is unreachable" 2
        fi
    fi
fi

# Validate the .es source structurally so a corrupt file fails fast.
if ! grep -q "^contract FutureSelfVault" "$CONTRACT_PATH"; then
    die ".es source missing 'contract FutureSelfVault' header" 3
fi
if ! grep -q "fn set_terms(" "$CONTRACT_PATH"; then
    die ".es source missing fn set_terms" 3
fi
if ! grep -q "fn try_payout(" "$CONTRACT_PATH"; then
    die ".es source missing fn try_payout" 3
fi

SOURCE_LEN=$(wc -c < "$CONTRACT_PATH" | tr -d ' ')
if (( SOURCE_LEN > 65536 )); then
    die ".es source is $SOURCE_LEN bytes; node caps at 64KB" 3
fi

# ── Banner ─────────────────────────────────────────────────────────

cat <<EOF
╔══════════════════════════════════════════════════════════════════╗
║              SFSV reference dApp — end-to-end deploy             ║
╠══════════════════════════════════════════════════════════════════╣
║  node:           $NODE_URL
║  deployer:       ${DEPLOYER_ADDR:-(dry-run)}
║  future_self:    ${FUTURE_SELF_ADDR:-(dry-run)}
║  predicate:      $PREDICATE_TYPE ($([[ "$PREDICATE_TYPE" == "0" ]] && echo "EpochReached" || echo "EnergyDecaysBelow"))
║  release_param:  $RELEASE_PARAM
║  energy / hl:    $INITIAL_ENERGY / $HALF_LIFE
║  deposit:        $DEPOSIT_AMOUNT
║  contract:       $(printf '%s (%d bytes)' "$CONTRACT_PATH" "$SOURCE_LEN")
║  mode:           $($DRY_RUN && echo DRY-RUN || echo LIVE)
╚══════════════════════════════════════════════════════════════════╝
EOF

# ── Step 1: deploy script ──────────────────────────────────────────

log "Step 1/4 — POST /api/tx/deploy-script"
# Read .es source and JSON-escape it via jq.
SOURCE_CODE_JSON=$(jq -Rs . < "$CONTRACT_PATH")
DEPLOY_BODY=$(jq -n \
    --argjson source "$SOURCE_CODE_JSON" \
    --arg deployer "$DEPLOYER_ADDR" \
    --argjson energy "$INITIAL_ENERGY" \
    --argjson hl "$HALF_LIFE" \
    '{deployer: $deployer, source_code: $source, energy: $energy, half_life: $hl}')

DEPLOY_HASH=$(submit_tx "/api/tx/deploy-script" "$DEPLOY_BODY" "deploy" 3)
log "Deploy tx hash: $DEPLOY_HASH"
poll_until_included "$DEPLOY_HASH" "deploy" 3
log "Deploy included."

# Resolve the contract address — the node indexes deploy-script
# transactions by hash → contract_addr.
if ! $DRY_RUN; then
    CONTRACT_ADDR=$(run_curl "resolve contract addr" \
        "$NODE_URL/api/contract/by-deploy/$DEPLOY_HASH" \
        | jq -r '.contract_addr // empty')
    if [[ -z "$CONTRACT_ADDR" ]]; then
        die "Could not resolve contract address for deploy $DEPLOY_HASH" 3
    fi
    log "Contract deployed at: $CONTRACT_ADDR"
else
    CONTRACT_ADDR="0xCONTRACT"
fi

# ── Step 2: seal vault via set_terms ───────────────────────────────

log "Step 2/4 — POST /api/tx/call-script (set_terms)"
SET_TERMS_BODY=$(jq -n \
    --arg caller "$DEPLOYER_ADDR" \
    --arg contract "$CONTRACT_ADDR" \
    --arg fs "$FUTURE_SELF_ADDR" \
    --argjson pred "$PREDICATE_TYPE" \
    --argjson param "$RELEASE_PARAM" \
    --argjson dep "$DEPOSIT_AMOUNT" \
    '{caller: $caller, contract: $contract, method: "set_terms",
      args: [$fs, $pred, $param, $dep]}')

SEAL_HASH=$(submit_tx "/api/tx/call-script" "$SET_TERMS_BODY" "set_terms" 4)
log "set_terms tx hash: $SEAL_HASH"
poll_until_included "$SEAL_HASH" "set_terms" 4
log "Vault sealed."

# ── Step 3: wait for predicate to trip ─────────────────────────────

log "Step 3/4 — wait for predicate to trip"
if $DRY_RUN; then
    log "[DRY-RUN] would poll /api/contract/$CONTRACT_ADDR/predicate_satisfied until = 1"
else
    deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC ))
    while (( $(date +%s) < deadline )); do
        resp=$(run_curl "predicate poll" \
            "$NODE_URL/api/contract/$CONTRACT_ADDR/state")
        satisfied=$(printf '%s' "$resp" | jq -r '.predicate_satisfied // 0')
        epoch=$(printf '%s' "$resp" | jq -r '.epoch_now // "?"')
        energy=$(printf '%s' "$resp" | jq -r '.energy // "?"')
        log "  epoch=$epoch energy=$energy satisfied=$satisfied"
        if [[ "$satisfied" == "1" ]]; then
            log "Predicate satisfied at epoch $epoch."
            break
        fi
        sleep 3
    done
    if [[ "$satisfied" != "1" ]]; then
        die "Predicate did not trip within ${POLL_TIMEOUT_SEC}s" 5
    fi
fi

# ── Step 4: try_payout ─────────────────────────────────────────────

log "Step 4/4 — POST /api/tx/call-script (try_payout)"
PAYOUT_BODY=$(jq -n \
    --arg caller "$DEPLOYER_ADDR" \
    --arg contract "$CONTRACT_ADDR" \
    '{caller: $caller, contract: $contract, method: "try_payout", args: []}')

PAYOUT_HASH=$(submit_tx "/api/tx/call-script" "$PAYOUT_BODY" "try_payout" 5)
log "try_payout tx hash: $PAYOUT_HASH"
poll_until_included "$PAYOUT_HASH" "try_payout" 5

# ── Verification ───────────────────────────────────────────────────

log "Verifying released state..."
if $DRY_RUN; then
    log "[DRY-RUN] would GET /api/contract/$CONTRACT_ADDR/state and assert released=1"
    log ""
    log "✓ Dry-run complete. Re-run without --dry-run against a live node."
    exit 0
fi

resp=$(run_curl "final state" "$NODE_URL/api/contract/$CONTRACT_ADDR/state")
released=$(printf '%s' "$resp" | jq -r '.released // false')
payout_at=$(printf '%s' "$resp" | jq -r '.payout_at // "?"')

if [[ "$released" != "true" && "$released" != "1" ]]; then
    die "Vault did NOT transition to released; got $released" 6
fi

cat <<EOF

╔══════════════════════════════════════════════════════════════════╗
║                       ✓ SFSV LIFECYCLE COMPLETE                  ║
╠══════════════════════════════════════════════════════════════════╣
║  contract:    $CONTRACT_ADDR
║  payout_at:   $payout_at (epoch)
║  deploy_tx:   $DEPLOY_HASH
║  seal_tx:     $SEAL_HASH
║  payout_tx:   $PAYOUT_HASH
╚══════════════════════════════════════════════════════════════════╝

End-to-end deploy / seal / decay / reclaim completed against $NODE_URL.
EOF
