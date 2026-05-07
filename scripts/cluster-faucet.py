#!/usr/bin/env python3
"""
EvaporChain internal soak-test faucet — Tailscale-only.

Submits a real transfer transaction every INTERVAL_S seconds, rotating
destination among the validator accounts other than the sender. Designed
to keep the cluster doing real work during the 14-day private decay-proven
run, NOT to bootstrap external accounts.

Self-hosted, no third-party deps, no external network. Auth token is read
from /tmp/tx_user.env (created earlier in the session) — rotate the token
or re-register a user if you want a fresh one.

Logs every attempt to FAUCET_LOG (one line per tx: timestamp,nonce,to,amount,hash,result).

Usage:
    python3 scripts/cluster-faucet.py
    tail -f /tmp/cluster-faucet.log

Stop with Ctrl-C or `pkill -f cluster-faucet.py`.
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.request

# -- Config ---------------------------------------------------------------

# Submit to ALL 5 validator APIs in parallel. The chain has a real
# tx-inclusion race on the proposer side: when validator V's mempool
# is the only one with the tx, the tx only lands when V happens to be
# the round-0 proposer for the next height — otherwise round
# advancement finalises an empty block from another validator and the
# tx is lost. Submitting to all 5 ensures whoever proposes next has
# the tx in their local mempool. The deeper proposer-side fix
# (re-submit drained txs on round advance) is a separate change in
# crates/evaporchain-consensus/src/tendermint.rs.
SUBMIT_URLS = [
    "http://100.119.53.101:8081/api/tx/transfer",  # M1
    "http://100.113.253.72:8081/api/tx/transfer",  # M2
    "http://100.103.216.125:8081/api/tx/transfer", # M3
    "http://100.66.208.20:8081/api/tx/transfer",   # H1
    "http://100.91.235.22:8081/api/tx/transfer",   # H2
]
NONCE_QUERY_URL = "http://100.119.53.101:8081/api/accounts"

# Sender = val-3 (highest balance after the cluster's been running a while —
# val-1 ran out of balance for per-tx gas fees, ~21k each, so it now
# rejects every tx with "insufficient balance for gas". val-3's never
# proposed enough blocks to drain itself and stays well-funded.)
SENDER = "0300000000000000000000000000000000000000000000000000000000000000"

# Rotate destination among the other validator accounts (skip sender).
DESTINATIONS = [
    "0100000000000000000000000000000000000000000000000000000000000000",
    "0200000000000000000000000000000000000000000000000000000000000000",
    "0400000000000000000000000000000000000000000000000000000000000000",
    "0500000000000000000000000000000000000000000000000000000000000000",
]

INTERVAL_S = 30          # one tx every 30s — ~2,880 txs over 24h
AMOUNT = 1               # tiny transfer; doesn't drain val-1
ENV_FILE = "/tmp/tx_user.env"
FAUCET_LOG = "/tmp/cluster-faucet.log"


def load_token() -> str:
    if not os.path.exists(ENV_FILE):
        sys.exit(f"FATAL: {ENV_FILE} not found — run register/login flow first.")
    with open(ENV_FILE) as f:
        for line in f:
            line = line.strip()
            if line.startswith("TOKEN=") and len(line) > 70:
                return line[len("TOKEN="):]
    sys.exit("FATAL: TOKEN not found in env file.")


def starting_nonce() -> int:
    """Read val-1's current nonce from the API and use that as the next nonce."""
    try:
        with urllib.request.urlopen(NONCE_QUERY_URL, timeout=5) as resp:
            accounts = json.loads(resp.read())
        for a in accounts:
            if a.get("address", "").lower() == "0x" + SENDER:
                return int(a.get("nonce", 0))
    except Exception as e:
        print(f"WARN: couldn't fetch starting nonce ({e}); defaulting to 0")
    return 0


def submit_to_one(url: str, token: str, body: bytes) -> tuple[bool, str]:
    req = urllib.request.Request(
        url,
        data=body,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {token}",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=6) as resp:
            payload = json.loads(resp.read())
        if payload.get("success"):
            return True, payload.get("tx_hash", "")
        return False, payload.get("message", "no message")
    except urllib.error.HTTPError as e:
        try:
            return False, f"HTTP {e.code}: {e.read().decode()[:120]}"
        except Exception:
            return False, f"HTTP {e.code}"
    except Exception as e:
        return False, f"exception: {type(e).__name__}: {e}"


def submit_transfer(token: str, nonce: int, dest: str) -> tuple[bool, str]:
    """Submit to ALL 5 validator APIs in parallel; treat as success if any
    returns success. Logs the first success's tx_hash; on total failure,
    logs the first error message."""
    body = json.dumps({
        "from": SENDER,
        "to": dest,
        "amount": AMOUNT,
        "nonce": nonce,
    }).encode()
    import concurrent.futures as cf
    results: list[tuple[str, bool, str]] = []
    with cf.ThreadPoolExecutor(max_workers=len(SUBMIT_URLS)) as pool:
        futures = {pool.submit(submit_to_one, url, token, body): url for url in SUBMIT_URLS}
        for fut in cf.as_completed(futures, timeout=10):
            url = futures[fut]
            try:
                ok, info = fut.result()
            except Exception as e:
                ok, info = False, f"future-exc: {e}"
            results.append((url, ok, info))
    # Prefer the first success
    for url, ok, info in results:
        if ok:
            return True, info
    # No success — return the first non-empty error
    for url, ok, info in results:
        if info:
            return False, info
    return False, "all 5 endpoints failed silently"


def log_event(line: str) -> None:
    with open(FAUCET_LOG, "a") as f:
        f.write(line + "\n")


def main() -> None:
    token = load_token()
    nonce = starting_nonce()
    print(f"EvaporChain faucet starting — fan-out submit to {len(SUBMIT_URLS)} validator APIs")
    print(f"Sender val-1 ({SENDER[:16]}…), starting nonce {nonce}, rate 1/{INTERVAL_S}s, log {FAUCET_LOG}")
    log_event(f"# faucet started ts={int(time.time())} nonce={nonce}")

    n = 0
    while True:
        dest = DESTINATIONS[n % len(DESTINATIONS)]
        ok, info = submit_transfer(token, nonce, dest)
        ts = int(time.time())
        if ok:
            log_event(f"{ts},{nonce},{dest[:16]},{AMOUNT},{info},ok")
            print(f"[{ts}] tx {n+1:5d} nonce={nonce} → {dest[:16]}…  hash={info[:16]}…")
            nonce += 1
        else:
            log_event(f"{ts},{nonce},{dest[:16]},{AMOUNT},,err:{info[:80]}")
            print(f"[{ts}] tx {n+1:5d} nonce={nonce} FAILED: {info[:120]}")
            # On nonce mismatch, re-fetch from chain.
            if "nonce" in info.lower():
                fresh = starting_nonce()
                if fresh != nonce:
                    print(f"  refreshed nonce {nonce} → {fresh}")
                    nonce = fresh
        n += 1
        time.sleep(INTERVAL_S)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\nshutting down faucet")
