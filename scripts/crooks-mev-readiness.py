#!/usr/bin/env python3
"""
EvaporChain Crooks-MEV settlement activation readiness checker.

Sibling to `mcc-readiness.py`. Probes all 5 cluster validators for
cross-validator agreement on the MEV-observation state and renders a
verdict gating the eventual `crooks_mev_settlement_mode → enforce`
governance flag flip — the third governance flag in the activation
ladder (alongside `parent_acceptance_mode → mcc_full` and
`conservation_enforcement → enforce`).

Usage:
    python3 scripts/crooks-mev-readiness.py
    python3 scripts/crooks-mev-readiness.py --watch 5

The endpoints checked:
- /api/mev/state_digest    — BLAKE3 digest of (observations,
                             attacker_stats); MUST match cross-cluster
- /api/mev/observations    — full ring buffer; for divergence diagnosis
- /api/governance/flags    — current crooks_mev_settlement_mode value
- /api/four_act            — chain health context

Exits 0 if ready to flip enforce, non-zero on any blocker.

Threshold for "safe to flip":
- digest agreement across all 5 nodes
- ≥1 observation in the buffer (proves detection has fired in observe
  mode without anyone being slashed yet — empirical confidence)
- current mode is "observe" (otherwise this script is moot)
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

MIN_OBSERVATION_COUNT = 1   # ≥ this many observations in the ring buffer
                            # to consider detection "exercised"

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
    out: dict[str, Any] = {"reachable": False}
    identity = fetch(node_ip, "/api/identity")
    if identity is None:
        return out
    out["reachable"] = True
    out["chain_id"] = identity.get("chain_id")

    digest_resp = fetch(node_ip, "/api/mev/state_digest")
    if digest_resp:
        out["mev_digest"] = digest_resp.get("digest")
        out["mev_observation_count"] = digest_resp.get("observation_count")
        out["has_state_digest_endpoint"] = True
    else:
        out["has_state_digest_endpoint"] = False

    obs_resp = fetch(node_ip, "/api/mev/observations")
    if obs_resp:
        out["observations"] = obs_resp.get("observations", [])

    flags = fetch(node_ip, "/api/governance/flags")
    if flags:
        flag_map = flags.get("flags", flags)
        out["settlement_mode"] = flag_map.get("crooks_mev_settlement_mode")

    return out


def render(probes: list[tuple[tuple[str, str, str], dict[str, Any]]]) -> int:
    print(f"\n{BOLD}EvaporChain Crooks-MEV settlement readiness — "
          f"{time.strftime('%Y-%m-%d %H:%M:%S')}{RESET}\n")

    fmt = "  {tag:<3}  {ip:<16}  {ok:<10}  {mode:>10}  {n:>5}  {dgst:>16}  {hint}"
    print(BOLD + fmt.format(
        tag="ID", ip="IP", ok="reachable",
        mode="mode", n="obs_n",
        dgst="mev_dig[:14]", hint="status",
    ) + RESET)

    digests: list[str] = []
    obs_counts: list[int] = []
    modes: list[str] = []
    chain_ids: list[str] = []
    has_endpoint: list[bool] = []

    for (tag, ip, _name), p in probes:
        if not p["reachable"]:
            print(f"  {tag:<3}  {ip:<16}  {RED}UNREACHABLE{RESET}")
            continue
        chain_ids.append(p.get("chain_id", "?"))
        digest = p.get("mev_digest") or "—"
        if digest != "—":
            digests.append(digest)
        n = p.get("mev_observation_count")
        if n is not None:
            obs_counts.append(n)
        mode = p.get("settlement_mode") or "?"
        modes.append(mode)
        has_endpoint.append(p.get("has_state_digest_endpoint", False))

        hints = []
        if not p.get("has_state_digest_endpoint"):
            hints.append(f"{AMBER}stale-binary{RESET}")
        if mode == "enforce":
            hints.append(f"{GREEN}already-enforce{RESET}")
        elif mode == "observe":
            hints.append(f"{DIM}observe{RESET}")
        elif mode != "?":
            hints.append(f"{RED}mode={mode}{RESET}")

        print(fmt.format(
            tag=tag, ip=ip,
            ok=GREEN + "ok" + RESET,
            mode=mode if mode != "?" else "—",
            n=str(n) if n is not None else "—",
            dgst=digest[:14],
            hint=" ".join(hints) if hints else f"{GREEN}clean{RESET}",
        ))

    print()

    # ── Cross-validator checks ──
    print(f"{BOLD}Cross-validator agreement{RESET}")

    if len(set(chain_ids)) == 1 and chain_ids:
        print(f"  {GREEN}✓{RESET} chain_id unanimous: {chain_ids[0]}")
    else:
        print(f"  {RED}✗{RESET} chain_id split: {set(chain_ids)}")

    # MEV digest agreement
    if digests:
        unique = set(digests)
        if len(unique) == 1:
            print(f"  {GREEN}✓{RESET} mev_state_digest unanimous across reporting "
                  f"nodes ({len(digests)}/5)")
        else:
            print(f"  {RED}✗{RESET} mev_state_digest split: {len(unique)} distinct values "
                  f"across {len(digests)} reporting nodes — observation-state disagreement")
    else:
        print(f"  {AMBER}—{RESET} mev_state_digest unavailable (all nodes pre-state_digest "
              "endpoint binary)")

    # Mode parity
    nontrivial_modes = [m for m in modes if m != "?"]
    if nontrivial_modes:
        unique_modes = set(nontrivial_modes)
        if len(unique_modes) == 1:
            mode = nontrivial_modes[0]
            print(f"  {GREEN}✓{RESET} settlement_mode unanimous: '{mode}'")
        else:
            print(f"  {RED}✗{RESET} settlement_mode split across cluster: {unique_modes} "
                  "— DO NOT flip until unified")

    # Observation buffer non-empty (proves detection fired at least once)
    if obs_counts:
        if min(obs_counts) >= MIN_OBSERVATION_COUNT:
            print(f"  {GREEN}✓{RESET} mev observation_count min = {min(obs_counts)} ≥ "
                  f"{MIN_OBSERVATION_COUNT} (detection has fired in observe mode)")
        else:
            print(f"  {AMBER}~{RESET} mev observation_count min = {min(obs_counts)} < "
                  f"{MIN_OBSERVATION_COUNT}; consider keeping observe mode until detection "
                  "has empirically fired")
    else:
        print(f"  {AMBER}—{RESET} mev observation_count unavailable (no nodes report)")

    if not all(has_endpoint):
        n_stale = sum(1 for x in has_endpoint if not x)
        print(f"  {AMBER}~{RESET} {n_stale}/5 nodes don't expose /api/mev/state_digest "
              "(binary pre-32b359b sibling commit) — deploy first")

    print()

    # ── Verdict ──
    print(f"{BOLD}Activation verdict — flip crooks_mev_settlement_mode → enforce{RESET}")

    chain_id_ok = len(set(chain_ids)) == 1 and bool(chain_ids)
    digest_unanimous = digests and len(set(digests)) == 1 and len(digests) == len(
        [p for _, p in probes if p["reachable"]]
    )
    in_observe = nontrivial_modes and all(m == "observe" for m in nontrivial_modes)
    detection_fired = obs_counts and min(obs_counts) >= MIN_OBSERVATION_COUNT
    all_have_endpoint = all(has_endpoint) and bool(has_endpoint)

    if not all_have_endpoint:
        print(f"  {AMBER}DEPLOY-FIRST{RESET}: nodes need the /api/mev/state_digest endpoint "
              "(commit 32b359b sibling)")
    elif not chain_id_ok:
        print(f"  {RED}NOT READY{RESET}: chain_id split — cluster fork")
    elif not in_observe:
        if nontrivial_modes and all(m == "enforce" for m in nontrivial_modes):
            print(f"  {GREEN}ALREADY ENFORCED{RESET}: nothing to flip; the chain is already "
                  "punishing detected attackers economically")
            return 0
        print(f"  {RED}NOT READY{RESET}: settlement_mode disagreement or unexpected value")
    elif not digest_unanimous:
        print(f"  {RED}NOT READY{RESET}: mev_state_digest disagreement — observations diverge")
    elif not detection_fired:
        print(f"  {AMBER}WAIT{RESET}: 0 observations across the cluster — keep observing until "
              "real sandwiches surface so the flip is proven on real data")
    else:
        print(f"  {GREEN}READY{RESET}: all 5 nodes agree on observation state, detection has "
              "fired in observe mode, safe to flip:")
        print(f"    {DIM}curl -X POST http://node:8081/api/governance/param \\\n"
              f"      -d '{{\"key\":\"crooks_mev_settlement_mode\",\"value\":\"enforce\"}}'{RESET}")
        return 0

    return 1 if not all_have_endpoint else 2


def main() -> None:
    ap = argparse.ArgumentParser(description="Crooks-MEV settlement readiness checker")
    ap.add_argument("--watch", type=int, default=0,
                    help="Re-run every N seconds (0 = one-shot)")
    args = ap.parse_args()
    while True:
        probes = [(n, probe_node(n[1])) for n in NODES]
        rc = render(probes)
        if args.watch <= 0:
            sys.exit(rc)
        time.sleep(args.watch)


if __name__ == "__main__":
    main()
