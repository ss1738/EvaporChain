#!/bin/bash
# EvaporChain governance flag-flip wrapper.
#
# Combines the readiness check + curl POST + propagation watch +
# rollback hint into a single safe-by-default sequence. Refuses to
# proceed if the relevant readiness script returns non-zero.
#
# Usage:
#   ./scripts/governance-flip.sh <flag_name> <flag_value> [<validator_ip>]
#
# Example:
#   export EVAPORCHAIN_ADMIN_KEY=...
#   ./scripts/governance-flip.sh parent_acceptance_mode mcc_full
#
# Exit codes:
#   0 — flip succeeded, propagation observed
#   1 — readiness check rejected the flip
#   2 — operator cancelled at the confirmation prompt
#   3 — curl POST failed
#   4 — propagation didn't observe the new value within timeout
#
# The script ALWAYS prints the rollback command (the inverse flag-flip)
# before executing the forward one, so the operator can copy-paste it
# if something goes wrong post-flip.

set -euo pipefail

# ─────────────────────── Args ───────────────────────────────────────────
FLAG_NAME="${1:-}"
FLAG_VALUE="${2:-}"
VALIDATOR_IP="${3:-100.119.53.101}"   # default: M1 (mini-1)
API_PORT=8081
WATCH_SECS=30
WATCH_INTERVAL=2

if [[ -z "$FLAG_NAME" || -z "$FLAG_VALUE" ]]; then
    cat <<EOF
usage: $0 <flag_name> <flag_value> [<validator_ip>]

Known governance flags + safe values:
  parent_acceptance_mode      linear | mcc | mcc_full
  block_source_mode           fifo | antichain
  lambda_fold_mode            hash_chain | nova
  conservation_enforcement    observe | enforce
  crooks_mev_settlement_mode  observe | enforce

Required env: EVAPORCHAIN_ADMIN_KEY (Bearer token for the
governance-param POST).
EOF
    exit 1
fi

if [[ -z "${EVAPORCHAIN_ADMIN_KEY:-}" ]]; then
    echo "✗ EVAPORCHAIN_ADMIN_KEY is not set; the governance-param POST" >&2
    echo "  endpoint requires Bearer auth. Set it and retry." >&2
    exit 1
fi

# ─────────────────────── ANSI ───────────────────────────────────────────
GREEN=$'\033[92m'; AMBER=$'\033[93m'; RED=$'\033[91m'
DIM=$'\033[2m'; BOLD=$'\033[1m'; RESET=$'\033[0m'

# ─────────────────────── Pick readiness script ──────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
case "$FLAG_NAME" in
    parent_acceptance_mode | block_source_mode | lambda_fold_mode | conservation_enforcement)
        READINESS="$SCRIPT_DIR/mcc-readiness.py"
        ;;
    crooks_mev_settlement_mode)
        READINESS="$SCRIPT_DIR/crooks-mev-readiness.py"
        ;;
    *)
        echo "✗ unknown flag '$FLAG_NAME'; refusing to flip something this" >&2
        echo "  script doesn't know how to validate. Add a readiness check" >&2
        echo "  case before flipping new flags via this wrapper." >&2
        exit 1
        ;;
esac

# ─────────────────────── Capture current value (for rollback) ──────────
echo "${BOLD}Step 1/4 — capture current value for rollback hint${RESET}"
CURRENT_JSON="$(curl -s --max-time 5 "http://${VALIDATOR_IP}:${API_PORT}/api/governance/flags" \
    || { echo "${RED}✗${RESET} failed to fetch /api/governance/flags from $VALIDATOR_IP" >&2; exit 1; })"
CURRENT_VALUE="$(echo "$CURRENT_JSON" \
    | python3 -c "import sys,json; d=json.load(sys.stdin); f=d.get('flags',d); print(f.get('$FLAG_NAME','?'))" \
    2>/dev/null || echo "?")"
echo "  current $FLAG_NAME = ${BOLD}$CURRENT_VALUE${RESET}"
echo "  proposed         = ${BOLD}$FLAG_VALUE${RESET}"
echo

if [[ "$CURRENT_VALUE" == "$FLAG_VALUE" ]]; then
    echo "${AMBER}~${RESET} flag is already at the proposed value; nothing to do."
    exit 0
fi

# ─────────────────────── Run readiness check ───────────────────────────
echo "${BOLD}Step 2/4 — readiness check ($(basename "$READINESS"))${RESET}"
echo
if python3 "$READINESS"; then
    echo
    echo "${GREEN}✓${RESET} readiness script returned 0 — safe to proceed"
else
    echo
    echo "${RED}✗${RESET} readiness script rejected the flip — refusing to proceed."
    echo "  Re-run the script with --watch and re-attempt this command once it's green."
    exit 1
fi

# ─────────────────────── Print rollback hint + confirm ─────────────────
echo
echo "${BOLD}Step 3/4 — rollback hint + confirmation${RESET}"
ROLLBACK_CMD="curl -X POST http://${VALIDATOR_IP}:${API_PORT}/api/governance/param \\
    -H 'Authorization: Bearer \$EVAPORCHAIN_ADMIN_KEY' \\
    -H 'Content-Type: application/json' \\
    -d '{\"key\":\"$FLAG_NAME\",\"value\":\"$CURRENT_VALUE\"}'"
echo "  ${DIM}If anything goes wrong post-flip, the rollback is:${RESET}"
echo "  ${DIM}---${RESET}"
echo "  $ROLLBACK_CMD"
echo "  ${DIM}---${RESET}"
echo
read -r -p "Proceed with $FLAG_NAME = $FLAG_VALUE? [yes/NO] " confirm
if [[ "$confirm" != "yes" ]]; then
    echo "${AMBER}~${RESET} cancelled by operator."
    exit 2
fi

# ─────────────────────── Execute the flip ──────────────────────────────
echo
echo "${BOLD}Step 4/4 — execute flip + watch propagation${RESET}"
RESPONSE="$(curl -s --max-time 5 -X POST \
    "http://${VALIDATOR_IP}:${API_PORT}/api/governance/param" \
    -H "Authorization: Bearer $EVAPORCHAIN_ADMIN_KEY" \
    -H "Content-Type: application/json" \
    -d "{\"key\":\"$FLAG_NAME\",\"value\":\"$FLAG_VALUE\"}" \
    || true)"

if [[ -z "$RESPONSE" ]]; then
    echo "${RED}✗${RESET} POST returned empty response — network or auth failure"
    exit 3
fi

# Server returns {"success":true,...} on success; check for that.
if echo "$RESPONSE" | grep -q '"success":\s*true'; then
    echo "${GREEN}✓${RESET} flip POST accepted: $RESPONSE"
else
    echo "${RED}✗${RESET} flip POST returned non-success: $RESPONSE"
    exit 3
fi

# Watch /api/governance/flags until the new value is observed.
echo
echo "  Watching /api/governance/flags for '$FLAG_NAME = $FLAG_VALUE' "
echo "  (timeout ${WATCH_SECS}s, polling every ${WATCH_INTERVAL}s)..."
elapsed=0
while (( elapsed < WATCH_SECS )); do
    obs="$(curl -s --max-time 3 "http://${VALIDATOR_IP}:${API_PORT}/api/governance/flags" \
        | python3 -c "import sys,json; d=json.load(sys.stdin); f=d.get('flags',d); print(f.get('$FLAG_NAME','?'))" \
        2>/dev/null || echo "?")"
    if [[ "$obs" == "$FLAG_VALUE" ]]; then
        echo "  ${GREEN}✓${RESET} observed at t=${elapsed}s: $FLAG_NAME = $FLAG_VALUE"
        echo
        echo "${GREEN}${BOLD}DONE${RESET} — flag flip complete + propagated."
        echo "${DIM}  Re-run the readiness script to confirm the cluster stays steady,${RESET}"
        echo "${DIM}  and watch validator logs for ConservationViolation / vote-stall signals.${RESET}"
        exit 0
    fi
    sleep "$WATCH_INTERVAL"
    elapsed=$(( elapsed + WATCH_INTERVAL ))
done

echo "${RED}✗${RESET} did NOT observe '$FLAG_NAME = $FLAG_VALUE' within ${WATCH_SECS}s"
echo "  Server accepted the POST but the API-side flag readback hasn't reflected it."
echo "  This may be a propagation lag or a server-side persistence bug."
echo
echo "  ${BOLD}Investigate before flipping anything else.${RESET} Rollback cmd:"
echo "  $ROLLBACK_CMD"
exit 4
