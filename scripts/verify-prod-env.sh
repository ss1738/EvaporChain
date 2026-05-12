#!/usr/bin/env bash
#
# verify-prod-env.sh — pre-flight check for production env-var configuration.
#
# Run on each node BEFORE `systemctl start evaporchain-validator` (or
# wire as `ExecStartPre=` in the systemd unit) so a missing required
# env var is a loud startup failure rather than a silent
# unauthenticated production cluster.
#
# Pairs with `docs/runbooks/production-env-checklist.md`.
#
# Exit codes:
#   0 = all required env vars set
#   1 = at least one required env var missing/empty
#
# Usage:
#   bash scripts/verify-prod-env.sh
#
#   # Or, to also accept `--allow-missing-mcp` for non-MCP deployments:
#   bash scripts/verify-prod-env.sh --allow-missing-mcp
#
#   # As systemd ExecStartPre:
#   ExecStartPre=/bin/bash /home/<user>/EvaporChain/scripts/verify-prod-env.sh
#
# Optional env vars (`EVAPORCHAIN_TLS_CERT`, `EVAPORCHAIN_TLS_KEY`,
# `EVAPORCHAIN_MCP_REQUIRE_AUTH`) are NOT checked — they're informational
# and the node prints its own startup warning when missing.

set -euo pipefail

ALLOW_MISSING_MCP="false"
while [ $# -gt 0 ]; do
  case "$1" in
    --allow-missing-mcp) ALLOW_MISSING_MCP="true"; shift ;;
    -h|--help)
      sed -n '2,/^$/p' "$0" | sed 's/^# \?//'
      exit 0
      ;;
    *) echo "Unknown arg: $1" >&2; exit 2 ;;
  esac
done

red()   { printf '\033[0;31m%s\033[0m\n' "$*"; }
green() { printf '\033[0;32m%s\033[0m\n' "$*"; }
yel()   { printf '\033[0;33m%s\033[0m\n' "$*"; }

REQUIRED=(
  EVAPORCHAIN_ADMIN_KEY
  EVAPORCHAIN_ORACLE_KEY
  EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE
)
# MCP token is required unless --allow-missing-mcp is passed (dev-mode
# or non-MCP-exposed validators can skip it; the runbook is explicit
# that pass-through is the default).
if [ "$ALLOW_MISSING_MCP" = "false" ]; then
  REQUIRED+=(EVAPORCHAIN_MCP_API_TOKEN)
fi

missing=0
for v in "${REQUIRED[@]}"; do
  if [ -z "${!v:-}" ]; then
    red "  ✗ MISSING: $v"
    missing=$((missing + 1))
  else
    green "  ✓ set: $v"
  fi
done

# Extra check: VALIDATOR_KEY_PASS_FILE should point to an existing readable
# file (not just any non-empty string). Catches "I set the path but forgot
# to create the file" mistakes.
if [ -n "${EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE:-}" ]; then
  if [ ! -f "$EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE" ]; then
    red "  ✗ EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE points at \"$EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE\" which doesn't exist or isn't a file"
    missing=$((missing + 1))
  elif [ ! -r "$EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE" ]; then
    red "  ✗ EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE points at unreadable file (check ownership/perms)"
    missing=$((missing + 1))
  fi
fi

# Informational reminder for the optional MCP-require-auth gate.
if [ "$ALLOW_MISSING_MCP" = "false" ] && [ -z "${EVAPORCHAIN_MCP_REQUIRE_AUTH:-}" ]; then
  yel "  (informational) EVAPORCHAIN_MCP_REQUIRE_AUTH not set — the MCP server will start"
  yel "                  in advisory mode if its own EVAPORCHAIN_MCP_API_TOKEN is unset."
  yel "                  Set EVAPORCHAIN_MCP_REQUIRE_AUTH=true on the MCP process for"
  yel "                  fail-closed startup."
fi

if [ "$missing" -gt 0 ]; then
  echo
  red "$missing required env var(s) missing — refusing to start."
  red "See docs/runbooks/production-env-checklist.md for what each one gates."
  exit 1
fi

echo
green "All required env vars present."
