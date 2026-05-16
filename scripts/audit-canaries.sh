#!/usr/bin/env bash
# audit-canaries.sh — regression-gate for closed audit findings.
#
# Each canary is a targeted grep that verifies a previously-closed
# audit finding is still closed in the current tree. Intended to run
# as part of `make check` and as a pre-commit hook.
#
# Motivation: AUDIT_2026_05_15.md round-1 closed 10 audit findings
# (R3, R4, R6×3, R7×4, DRIFT-N3) that were then silently overwritten
# by a single large merge (`b8630ff4`). The fresh 2026-05-16 audit
# round caught all 10 as live regressions. This script would have
# failed at the merge-commit time.
#
# Add a new canary when:
#   1. The fix is small/localised (an admin gate, a return statement,
#      a specific assertion).
#   2. The fix has been silently regressed at least once OR is
#      structurally easy to drop (e.g. function signature change
#      across a large merge).
#
# Do NOT add canaries for:
#   - Implementation details that may legitimately refactor.
#   - Anything covered by a unit test (test failure already gates the
#     commit).
#
# Exit 0 = all canaries green. Exit nonzero = at least one canary
# fired; the regression description tells the operator which audit
# closure has been re-broken.

set -u
set -o pipefail

# Resolve repo root from script location.
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." >/dev/null 2>&1 && pwd)"
cd "$REPO_ROOT"

PASS=0
FAIL=0
RED=$'\033[0;31m'
GREEN=$'\033[0;32m'
RESET=$'\033[0m'

# canary <name> <audit-ref> <file> <expect-pattern>
# Asserts that `expect-pattern` (Perl-grep regex) matches at least
# once in `file`. Use this for "this line MUST exist".
canary() {
    local name="$1" audit_ref="$2" file="$3" pattern="$4"
    if [[ ! -f "$file" ]]; then
        printf '%sFAIL%s %s (%s): file not found: %s\n' \
            "$RED" "$RESET" "$name" "$audit_ref" "$file"
        FAIL=$((FAIL + 1))
        return
    fi
    if grep -qE "$pattern" "$file"; then
        PASS=$((PASS + 1))
        printf '%spass%s %s (%s)\n' "$GREEN" "$RESET" "$name" "$audit_ref"
    else
        printf '%sFAIL%s %s (%s)\n' "$RED" "$RESET" "$name" "$audit_ref"
        printf '       file: %s\n' "$file"
        printf '       expected pattern: %s\n' "$pattern"
        FAIL=$((FAIL + 1))
    fi
}

# canary_negative <name> <audit-ref> <file> <forbidden-pattern>
# Asserts that `forbidden-pattern` does NOT match in `file`. Use this
# for "this anti-pattern MUST NOT come back" (e.g. a re-introduced
# warn-only branch).
canary_negative() {
    local name="$1" audit_ref="$2" file="$3" pattern="$4"
    if [[ ! -f "$file" ]]; then
        printf '%sFAIL%s %s (%s): file not found: %s\n' \
            "$RED" "$RESET" "$name" "$audit_ref" "$file"
        FAIL=$((FAIL + 1))
        return
    fi
    if grep -qE "$pattern" "$file"; then
        printf '%sFAIL%s %s (%s)\n' "$RED" "$RESET" "$name" "$audit_ref"
        printf '       file: %s\n' "$file"
        printf '       forbidden pattern: %s\n' "$pattern"
        printf '       grep hit:\n'
        grep -nE "$pattern" "$file" | sed 's/^/         /'
        FAIL=$((FAIL + 1))
    else
        PASS=$((PASS + 1))
        printf '%spass%s %s (%s)\n' "$GREEN" "$RESET" "$name" "$audit_ref"
    fi
}

# canary_function_contains <name> <audit-ref> <file> <function-name> <expect-pattern>
# Asserts that the function body (from `async fn NAME(` or `fn NAME(`
# until the next top-level `^}`) contains `expect-pattern`. Used for
# admin-gate canaries that check a specific call (e.g.
# `require_admin_auth`) is still present inside a specific handler.
canary_function_contains() {
    local name="$1" audit_ref="$2" file="$3" fn_name="$4" pattern="$5"
    if [[ ! -f "$file" ]]; then
        printf '%sFAIL%s %s (%s): file not found: %s\n' \
            "$RED" "$RESET" "$name" "$audit_ref" "$file"
        FAIL=$((FAIL + 1))
        return
    fi
    # Slice from `^async fn fn_name(` (or `^fn fn_name(`) to the next `^}`.
    local body
    body="$(awk -v fn="$fn_name" '
        BEGIN { in_fn = 0 }
        /^(pub )?(async )?fn / {
            if (in_fn) { exit 0 }
            if ($0 ~ ("^(pub )?(async )?fn " fn "[ (]")) { in_fn = 1; print; next }
        }
        in_fn && /^}/ { print; exit 0 }
        in_fn { print }
    ' "$file")"
    if [[ -z "$body" ]]; then
        printf '%sFAIL%s %s (%s): function %s not found in %s\n' \
            "$RED" "$RESET" "$name" "$audit_ref" "$fn_name" "$file"
        FAIL=$((FAIL + 1))
        return
    fi
    if grep -qE "$pattern" <<<"$body"; then
        PASS=$((PASS + 1))
        printf '%spass%s %s (%s)\n' "$GREEN" "$RESET" "$name" "$audit_ref"
    else
        printf '%sFAIL%s %s (%s)\n' "$RED" "$RESET" "$name" "$audit_ref"
        printf '       function %s in %s does not contain pattern: %s\n' \
            "$fn_name" "$file" "$pattern"
        FAIL=$((FAIL + 1))
    fi
}

API="crates/evaporchain-node/src/api.rs"
TENDERMINT="crates/evaporchain-consensus/src/tendermint.rs"
GENESIS_TYPES="crates/evaporchain-types/src/genesis.rs"
GENESIS_EXEC="crates/evaporchain-execution/src/genesis.rs"
ENCRYPTED_MEMPOOL="crates/evaporchain-consensus/src/encrypted_mempool.rs"
PARSER="crates/evaporchain-script/src/parser.rs"
CRYPTO_BLS="crates/evaporchain-crypto/src/bls_key_store.rs"

# ─── Admin-gate canaries (R3 / R4 / R5 / R6 / R7 / R10) ──────────────
# All from AUDIT_2026_05_15.md. Each handler must still call
# require_admin_auth. Regression in the 2026-05-16 round dropped 9 of
# these at once; pinning each one ensures the next merge can't silently
# revert any of them.

canary_function_contains \
    "R3 post_demo_reset admin-gated" "AUDIT_2026_05_15:R3" \
    "$API" "post_demo_reset" "require_admin_auth"

canary_function_contains \
    "R4 post_llsa_apply_amendment admin-gated" "AUDIT_2026_05_15:R4" \
    "$API" "post_llsa_apply_amendment" "require_admin_auth"

canary_function_contains \
    "R5 post_mev_dispute admin-gated" "AUDIT_2026_05_15:R5" \
    "$API" "post_mev_dispute" "require_admin_auth"

canary_function_contains \
    "R6 post_patronage_pledge admin-gated" "AUDIT_2026_05_15:R6" \
    "$API" "post_patronage_pledge" "require_admin_auth"

canary_function_contains \
    "R6 post_patronage_honour admin-gated" "AUDIT_2026_05_15:R6" \
    "$API" "post_patronage_honour" "require_admin_auth"

canary_function_contains \
    "R6 post_patronage_revoke admin-gated" "AUDIT_2026_05_15:R6" \
    "$API" "post_patronage_revoke" "require_admin_auth"

canary_function_contains \
    "R7 post_epv_register admin-gated" "AUDIT_2026_05_15:R7" \
    "$API" "post_epv_register" "require_admin_auth"

canary_function_contains \
    "R7 post_epv_prune admin-gated" "AUDIT_2026_05_15:R7" \
    "$API" "post_epv_prune" "require_admin_auth"

canary_function_contains \
    "R7 post_dsn_fold_nullifier admin-gated" "AUDIT_2026_05_15:R7" \
    "$API" "post_dsn_fold_nullifier" "require_admin_auth"

canary_function_contains \
    "R7 post_dsn_advance_window admin-gated" "AUDIT_2026_05_15:R7" \
    "$API" "post_dsn_advance_window" "require_admin_auth"

canary_function_contains \
    "R8 post_sanov_slash admin-gated" "AUDIT_2026_05_15:R8" \
    "$API" "post_sanov_slash" "require_admin_auth"

canary_function_contains \
    "R8 post_pnt_insert admin-gated" "AUDIT_2026_05_15:R8" \
    "$API" "post_pnt_insert" "require_admin_auth"

canary_function_contains \
    "R8 post_pnt_advance_phase admin-gated" "AUDIT_2026_05_15:R8" \
    "$API" "post_pnt_advance_phase" "require_admin_auth"

canary_function_contains \
    "R8 post_fee_controller_step admin-gated" "AUDIT_2026_05_15:R8" \
    "$API" "post_fee_controller_step" "require_admin_auth"

canary_function_contains \
    "R9 post_sentinel_register_param admin-gated" "AUDIT_2026_05_15:R9" \
    "$API" "post_sentinel_register_param" "require_admin_auth"

canary_function_contains \
    "R9 post_sentinel_vote admin-gated" "AUDIT_2026_05_15:R9" \
    "$API" "post_sentinel_vote" "require_admin_auth"

canary_function_contains \
    "R10 post_hbct_seed_attestation admin-gated" "AUDIT_2026_05_15:R10" \
    "$API" "post_hbct_seed_attestation" "require_admin_auth"

# ─── DRIFT-N3 — Proposal BLS=None branch must reject ─────────────────
# The "Accept during migration window" warn-only branch was a gossip-
# forgery vector. It was removed (commit 392027ec) and silently
# re-introduced. The negative canary fires if that comment ever
# reappears.

canary_negative \
    "DRIFT-N3 no 'Accept during migration window'" "AUDIT_2026_05_15:DRIFT-N3" \
    "$TENDERMINT" "// Accept during migration window"

# ─── Audit primitives: DSTs and length caps ──────────────────────────
# These are the "this string is the source of a cryptographic domain
# separation; it MUST stay byte-exact" canaries.

canary \
    "GEN-N3 EVAPORCHAIN_V1_GENESIS_HASH DST" "AUDIT_2026_05_15:GEN-N3" \
    "$GENESIS_TYPES" 'EVAPORCHAIN_V1_GENESIS_HASH\\0'

canary \
    "GEN-N3 EVAPORCHAIN_V1_GENESIS_BIND DST" "AUDIT_2026_05_15:GEN-N3" \
    "$GENESIS_EXEC" 'EVAPORCHAIN_V1_GENESIS_BIND\\0'

canary \
    "PRIV-N5 EVAPORCHAIN_V1_MEV_AAD DST" "AUDIT_2026_05_15:PRIV-N5" \
    "$ENCRYPTED_MEMPOOL" 'EVAPORCHAIN_V1_MEV_AAD\\0'

canary \
    "PRIV-N5 EVAPORCHAIN_V1_MEV_ADM DST" "AUDIT_2026_05_15:PRIV-N5" \
    "$ENCRYPTED_MEMPOOL" 'EVAPORCHAIN_V1_MEV_ADM\\0'

# ─── Defensive asserts from the 2026-05-16 round ─────────────────────

canary \
    "CONS-B1 chain_id u8 assert" "AUDIT_2026_05_16:CONS-B1" \
    "$TENDERMINT" 'chain_id_bytes\.len\(\) < 256'

canary \
    "CONS-A1 phase literal debug_assert" "AUDIT_2026_05_16:CONS-A1" \
    "$TENDERMINT" 'matches!\(phase, "proposal" \| "prevote" \| "precommit"\)'

canary \
    "PARSER-1 MAX_SOURCE_BYTES cap" "AUDIT_2026_05_16:PARSER-1" \
    "$PARSER" 'MAX_SOURCE_BYTES'

# ─── Crypto parameters ───────────────────────────────────────────────

canary \
    "GEN-N5 ARGON2_T_COST = 4" "AUDIT_2026_05_15:GEN-N5" \
    "$CRYPTO_BLS" 'ARGON2_T_COST: u32 = 4'

# ─── Result ──────────────────────────────────────────────────────────

echo
if [[ "$FAIL" -eq 0 ]]; then
    printf '%s[ok]%s %d canaries pass, 0 fail\n' "$GREEN" "$RESET" "$PASS"
    exit 0
else
    printf '%s[FAIL]%s %d canaries pass, %d FAIL\n' "$RED" "$RESET" "$PASS" "$FAIL"
    printf '\nA closed audit finding has been silently re-introduced.\n'
    printf 'Inspect the failing canary, restore the fix, and re-run.\n'
    exit 1
fi
