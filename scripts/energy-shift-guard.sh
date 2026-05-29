#!/usr/bin/env bash
# energy-shift-guard.sh — Layer 0 CI lint for the
# "energy routes through energy_at_epoch only" invariant.
#
# DOCTRINE (CLAUDE.md "Two unifying invariants" §1): every energy-decay
# computation must go through `evaporchain_types::energy_at_epoch` — the
# Coq-verified canonical halving formula. A raw bit-shift (`>>`) applied
# to an energy value ANYWHERE outside `evaporchain-types` silently
# diverges from that formula and breaks the conservation invariant.
#
# This guard scans the workspace and fails (`make check`) if it finds an
# energy value being right-shifted outside `evaporchain-types`. It is the
# regression guard the CLAUDE.md invariant always promised but that the
# tree never actually shipped.
#
# Detection (heuristic, comment-stripped so prose like
# "epoch >> half_life" does not false-positive):
#   A. an `energy`-named value being right-shifted   (foo.energy >> k)
#   B. a right-shift BY a half-life count            (x >> half_lives)
#
# False positives: if a `>>` genuinely is NOT an energy-decay shift,
# annotate the line with a trailing `// energy-shift-guard:allow`
# comment and the guard skips it. Use sparingly and explain why.
#
# Exit 0 = clean. Exit 1 = at least one raw energy bit-shift found.

set -u
set -o pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." >/dev/null 2>&1 && pwd)"
cd "$REPO_ROOT"

RED=$'\033[0;31m'
GREEN=$'\033[0;32m'
RESET=$'\033[0m'

# Scan every workspace .rs file except the canonical home of the decay
# formula (evaporchain-types), with // line-comments stripped before
# matching. The allow-marker escape hatch is checked against the
# original (un-stripped) line.
HITS="$(
    find crates -name '*.rs' -not -path '*/evaporchain-types/*' -print0 \
        | xargs -0 awk '
            {
                line = $0
                sub(/\/\/.*/, "", line)
                if ((line ~ /energy[a-z_]*[ \t]*>>/ || line ~ />>[ \t]*[a-z_]*half_l/) \
                    && $0 !~ /energy-shift-guard:allow/) {
                    printf "%s:%d: %s\n", FILENAME, FNR, $0
                }
            }
        '
)"

if [[ -n "$HITS" ]]; then
    printf '%s[FAIL]%s energy-shift guard: raw `>>` on an energy value outside evaporchain-types\n\n' \
        "$RED" "$RESET"
    printf '%s\n\n' "$HITS"
    printf 'Energy decay MUST call evaporchain_types::energy_at_epoch (the\n'
    printf 'Coq-verified halving formula). Reroute the bit-shift through it, or\n'
    printf 'if this `>>` is genuinely not an energy decay, annotate the line\n'
    printf 'with a trailing `// energy-shift-guard:allow` comment explaining why.\n'
    exit 1
fi

printf '%s[ok]%s energy-shift guard: no raw energy bit-shifts outside evaporchain-types\n' \
    "$GREEN" "$RESET"
exit 0
