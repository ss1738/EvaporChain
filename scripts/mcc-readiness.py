#!/usr/bin/env python3
"""
EvaporChain MCC activation readiness checker.

One-shot pre-flight + dashboard for the eventual `parent_acceptance_mode`
flag flip ladder: `linear → mcc → mcc_full`. Probes all 5 cluster
validators for cross-node agreement on the DAG state and the
conservation audit, then renders a 3-step readiness verdict.

The endpoints checked already exist on every node running a binary at
or after MCC plan Phase E.1+E.2 (commit ba5d591, 2026-05-05) AND the
`consecutive_clean_audits` counter (commit 616bf28, 2026-05-08). Older
binaries report partial data; the script flags those nodes amber.

Usage:
    python3 scripts/mcc-readiness.py

Or with a continuous watch:
    python3 scripts/mcc-readiness.py --watch 5

No third-party deps (stdlib only). Tailscale reachability assumed.
Exits 0 if all three ladder steps are green; non-zero on amber/red.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.error
import urllib.request
from typing import Any

NODES = [
    ("M1", "100.119.53.101", "UK Mac Mini 1"),
    ("M2", "100.113.253.72",  "UK Mac Mini 2"),
    ("M3", "100.103.216.125", "UK Mac Mini 3"),
    ("H1", "100.66.208.20",   "Helsinki CX23 #1"),
    ("H2", "100.91.235.22",   "Helsinki CX23 #2"),
]
API_PORT = 8081
HTTP_TIMEOUT_S = 5

# Operator-tunable thresholds.  These are the "is it safe to flip?"
# gating values — defensible defaults, override on the CLI when a
# specific cluster needs different bars.
MIN_CONSECUTIVE_CLEAN_AUDITS = 500   # ≥ this on every node before flipping
                                     # conservation_enforcement to "enforce"
MAX_HEIGHT_SPREAD = 5                # ≥ this many blocks of skew = network not
                                     # in lockstep, abort flag flip


# ─── ANSI ───────────────────────────────────────────────────────────────
GREEN = "\033[92m"
AMBER = "\033[93m"
RED = "\033[91m"
DIM = "\033[2m"
BOLD = "\033[1m"
RESET = "\033[0m"


def fetch(node_ip: str, path: str) -> dict[str, Any] | None:
    url = f"http://{node_ip}:{API_PORT}{path}"
    try:
        with urllib.request.urlopen(url, timeout=HTTP_TIMEOUT_S) as resp:
            return json.loads(resp.read())
    except (urllib.error.URLError, ValueError, TimeoutError):
        return None


def probe_node(node_ip: str) -> dict[str, Any]:
    """Pull every field the readiness check needs from one node."""
    out: dict[str, Any] = {"reachable": False}
    identity = fetch(node_ip, "/api/identity")
    if identity is None:
        return out
    out["reachable"] = True
    out["chain_id"] = identity.get("chain_id")

    # Canonical block height — light_cone_block_count is sliding-window
    # pruned, NOT a height. Use /api/blocks?limit=1 instead.
    blocks = fetch(node_ip, "/api/blocks?limit=1")
    if isinstance(blocks, list) and blocks:
        out["block_height"] = blocks[0].get("number")
    elif isinstance(blocks, dict) and "blocks" in blocks and blocks["blocks"]:
        out["block_height"] = blocks["blocks"][0].get("number")
    else:
        out["block_height"] = None

    four_act = fetch(node_ip, "/api/four_act")
    if four_act:
        # /api/four_act returns either the snapshot directly or wrapped
        # in {"four_act": {...}}.  Handle both.
        fa = four_act.get("four_act", four_act)
        out["last_conservation_audit_ok"] = fa.get("last_conservation_audit_ok")
        out["last_conservation_violation_type"] = fa.get(
            "last_conservation_violation_type"
        )
        out["consecutive_clean_audits"] = fa.get("consecutive_clean_audits")
        out["eulogy_count"] = fa.get("eulogy_count")
        out["refresh_pool_total"] = fa.get("refresh_pool_total")

    flags = fetch(node_ip, "/api/governance/flags")
    if flags:
        out["governance_flags"] = flags.get("flags", flags)

    cand = fetch(node_ip, "/api/light_cone/candidate_heads")
    if cand:
        out["candidate_heads"] = cand
        # Sort by block_id so cross-node comparison is order-independent
        # even when the API result varies.
        if isinstance(cand.get("heads"), list):
            heads_set = sorted(h.get("block_id", "") for h in cand["heads"])
            out["candidate_heads_set"] = tuple(heads_set)

    auth = fetch(node_ip, "/api/light_cone/authoritative_head")
    if auth:
        out["authoritative_head"] = auth.get("head")
        out["authoritative_head_caliber"] = auth.get("caliber")

    digest = fetch(node_ip, "/api/light_cone/antichain_digest")
    if digest:
        out["antichain_digest"] = digest.get("digest")
        out["closing_antichain_size"] = digest.get("closing_antichain_size")

    return out


def render_readiness(probes: list[tuple[tuple[str, str, str], dict[str, Any]]]) -> int:
    """Render the 3-step readiness verdict.  Returns shell exit code:
    0 = all green, 1 = any amber, 2 = any red.
    """
    print(f"\n{BOLD}EvaporChain MCC activation readiness — {time.strftime('%Y-%m-%d %H:%M:%S')}{RESET}\n")

    # ── Per-node summary table ──
    fmt = "  {tag:<3}  {ip:<16}  {ok:<10}  {h:>10}  {ccas:>6}  {ah:>16}  {dgst:>16}  {hint}"
    print(BOLD + fmt.format(
        tag="ID", ip="IP", ok="reachable",
        h="height", ccas="ccas",
        ah="auth_head[:14]", dgst="antichain_dig[:14]", hint="status",
    ) + RESET)

    heights: list[int] = []
    digests: list[str] = []
    auth_heads: list[str] = []
    cand_sets: list[tuple] = []
    chain_ids: list[str] = []
    ccas: list[int] = []
    has_violation_type_field: list[bool] = []
    has_ccas_field: list[bool] = []

    for (tag, ip, _name), p in probes:
        if not p["reachable"]:
            print(f"  {tag:<3}  {ip:<16}  {RED}UNREACHABLE{RESET}")
            return_code_marker = "red"
            heights.append(-1)
            continue

        h = p.get("block_height")
        if h is not None:
            heights.append(h)

        chid = p.get("chain_id", "?")
        chain_ids.append(chid)

        digest = p.get("antichain_digest") or "—"
        digests.append(digest)
        ah = p.get("authoritative_head") or "—"
        auth_heads.append(ah)
        cset = p.get("candidate_heads_set", ())
        cand_sets.append(cset)

        cca = p.get("consecutive_clean_audits")
        has_ccas_field.append(cca is not None)
        if cca is not None:
            ccas.append(cca)

        has_violation_type_field.append(
            "last_conservation_violation_type" in p
        )

        # Per-node hint
        hints = []
        if p.get("last_conservation_audit_ok") is False:
            v = p.get("last_conservation_violation_type") or "kind=?"
            hints.append(f"{RED}audit-fail({v}){RESET}")
        elif p.get("last_conservation_audit_ok") is None:
            hints.append(f"{DIM}audit=?{RESET}")
        if cca is None:
            hints.append(f"{AMBER}stale-binary{RESET}")
        if not hints:
            hints.append(f"{GREEN}clean{RESET}")

        print(fmt.format(
            tag=tag, ip=ip,
            ok=GREEN + "ok" + RESET,
            h=str(h) if h is not None else "?",
            ccas=str(cca) if cca is not None else "—",
            ah=ah[:14] if isinstance(ah, str) else "—",
            dgst=digest[:14] if isinstance(digest, str) else "—",
            hint=" ".join(hints),
        ))

    print()

    # ── Cross-validator checks ──
    print(f"{BOLD}Cross-validator agreement{RESET}")

    # 1. Chain ID parity
    chain_ids_unique = set(chain_ids)
    if len(chain_ids_unique) == 1:
        print(f"  {GREEN}✓{RESET} chain_id unanimous: {chain_ids_unique.pop()}")
    else:
        print(f"  {RED}✗{RESET} chain_id split: {chain_ids_unique}")

    # 2. Height spread
    if heights and -1 not in heights:
        spread = max(heights) - min(heights)
        if spread <= MAX_HEIGHT_SPREAD:
            print(f"  {GREEN}✓{RESET} height spread = {spread} blocks (max allowed {MAX_HEIGHT_SPREAD})")
        else:
            print(f"  {RED}✗{RESET} height spread = {spread} blocks (> {MAX_HEIGHT_SPREAD}); cluster not in lockstep")
    else:
        print(f"  {AMBER}—{RESET} height spread N/A (some nodes unreachable)")

    # 3. Antichain digest agreement (skip nodes with stale binary that
    # don't expose the field)
    nontrivial_digests = [d for d in digests if d not in ("—", None)]
    if nontrivial_digests:
        digest_set = set(nontrivial_digests)
        if len(digest_set) == 1:
            print(f"  {GREEN}✓{RESET} antichain_digest unanimous across reporting nodes ({len(nontrivial_digests)}/5)")
        else:
            print(f"  {RED}✗{RESET} antichain_digest split: {len(digest_set)} distinct values across {len(nontrivial_digests)} nodes")
    else:
        print(f"  {AMBER}—{RESET} antichain_digest unavailable (all nodes pre-Phase-E.1 binaries)")

    # 4. Authoritative head agreement (only meaningful when multi-fork
    # candidate set has >1 head; under linear default everyone agrees
    # trivially).
    nontrivial_auth = [a for a in auth_heads if a not in ("—", None)]
    if nontrivial_auth:
        auth_set = set(nontrivial_auth)
        if len(auth_set) == 1:
            print(f"  {GREEN}✓{RESET} authoritative_head unanimous across reporting nodes ({len(nontrivial_auth)}/5)")
        else:
            print(f"  {AMBER}~{RESET} authoritative_head transient split: {len(auth_set)} distinct values — expected briefly under multi-fork; persists = bad")

    # 5. Candidate-head sets — ALL nodes that report should agree on
    # the set of leaf candidates.  Order is irrelevant; we sorted.
    nontrivial_cands = [c for c in cand_sets if c]
    if nontrivial_cands:
        cand_set_unique = set(nontrivial_cands)
        if len(cand_set_unique) == 1:
            print(f"  {GREEN}✓{RESET} candidate_heads sets unanimous across reporting nodes ({len(nontrivial_cands)}/5)")
        else:
            print(f"  {RED}✗{RESET} candidate_heads sets split: {len(cand_set_unique)} distinct sets — DAG topology disagreement")

    # 6. consecutive_clean_audits — minimum across reporting nodes vs
    # the readiness threshold for the conservation_enforcement flip.
    if ccas:
        min_cca = min(ccas)
        if min_cca >= MIN_CONSECUTIVE_CLEAN_AUDITS:
            print(f"  {GREEN}✓{RESET} consecutive_clean_audits min = {min_cca} ≥ {MIN_CONSECUTIVE_CLEAN_AUDITS} (enforce-flip safe)")
        elif min_cca > 0:
            print(f"  {AMBER}~{RESET} consecutive_clean_audits min = {min_cca} < {MIN_CONSECUTIVE_CLEAN_AUDITS} (keep observing)")
        else:
            print(f"  {RED}✗{RESET} consecutive_clean_audits min = 0 — at least one node has a recent violation; do NOT flip enforce")
        if not all(has_ccas_field):
            print(f"  {AMBER}~{RESET} {sum(1 for x in has_ccas_field if not x)}/5 nodes don't expose consecutive_clean_audits (binary < commit 616bf28)")
    else:
        print(f"  {AMBER}—{RESET} consecutive_clean_audits unavailable (all nodes pre-616bf28 binaries)")

    print()

    # ── Three-step ladder verdict ──
    print(f"{BOLD}Activation ladder verdict{RESET}")

    # Step 1: linear → mcc.  Requires only chain_id agreement + height
    # lockstep + every node reachable.  No DAG state required (we run on
    # the linear flag default).
    step1_ok = (
        len(chain_ids_unique) == 1
        and -1 not in heights
        and (max(heights) - min(heights)) <= MAX_HEIGHT_SPREAD
        if heights else False
    )
    print(f"  Step 1 — flip {BOLD}block_source_mode → antichain{RESET} + {BOLD}lambda_fold_mode → nova{RESET}: "
          + (f"{GREEN}READY{RESET}" if step1_ok else f"{RED}NOT READY{RESET} (cluster not in lockstep)"))

    # Step 2: mcc → mcc_full.  Requires every node to expose the
    # antichain_digest endpoint AND digests agree.  Older binaries
    # without the endpoint = NOT READY.
    step2_ok = (
        step1_ok
        and len(nontrivial_digests) == len([p for _, p in probes if p["reachable"]])
        and len(set(nontrivial_digests)) == 1
        and len(set(nontrivial_cands)) == 1
        if nontrivial_digests else False
    )
    if step2_ok:
        print(f"  Step 2 — flip {BOLD}parent_acceptance_mode → mcc{RESET} → soak → {BOLD}mcc_full{RESET}: {GREEN}READY{RESET}")
    elif not nontrivial_digests:
        print(f"  Step 2 — flip parent_acceptance_mode: {AMBER}DEPLOY-FIRST{RESET} (nodes need MCC Phase E.1 binary)")
    else:
        print(f"  Step 2 — flip parent_acceptance_mode: {RED}NOT READY{RESET} (DAG state not converged across cluster)")

    # Step 3: observe → enforce on conservation_enforcement.  Requires
    # consecutive_clean_audits ≥ threshold on every reporting node AND
    # every reporting node passes its current audit.
    step3_ok = (
        step1_ok
        and ccas
        and min(ccas) >= MIN_CONSECUTIVE_CLEAN_AUDITS
        and all(p.get("last_conservation_audit_ok") is True
                for _, p in probes if p["reachable"] and p.get("last_conservation_audit_ok") is not None)
    )
    if step3_ok:
        print(f"  Step 3 — flip {BOLD}conservation_enforcement → enforce{RESET}: {GREEN}READY{RESET}")
    elif not ccas:
        print(f"  Step 3 — flip conservation_enforcement: {AMBER}DEPLOY-FIRST{RESET} (nodes need consecutive_clean_audits binary, commit 616bf28)")
    else:
        cca_min = min(ccas) if ccas else 0
        print(f"  Step 3 — flip conservation_enforcement: {RED}NOT READY{RESET} (min consecutive_clean_audits = {cca_min}; need ≥ {MIN_CONSECUTIVE_CLEAN_AUDITS})")

    print()

    # Exit code: 0 if all three steps green, 1 if amber-anywhere, 2 if red-anywhere
    if step1_ok and step2_ok and step3_ok:
        print(f"{GREEN}{BOLD}All three ladder steps READY.{RESET}  Operator can proceed with the governance flag flips.")
        return 0
    elif any(p["reachable"] is False for _, p in probes):
        return 2
    else:
        return 1


def main() -> None:
    ap = argparse.ArgumentParser(description="MCC activation readiness checker")
    ap.add_argument("--watch", type=int, default=0,
                    help="Re-run every N seconds (0 = one-shot)")
    args = ap.parse_args()

    while True:
        probes = [(node, probe_node(node[1])) for node in NODES]
        rc = render_readiness(probes)
        if args.watch <= 0:
            sys.exit(rc)
        time.sleep(args.watch)


if __name__ == "__main__":
    main()
