#!/usr/bin/env python3
"""Scrape Ethereum block headers for the Causal-CHSH gate.

Fetches one row per block:
    height, timestamp_secs, size, gas_used, tx_count

Writes CSV in the BlockSummary shape consumed by
`evaporchain-causal-chsh::trace::extract_chsh_samples` (via the Rust
gate runner at `bin/run_real_gate.rs`).

Same RPC pattern as `/tmp/scrape_eth.py` (the MERA scraper):
publicnode + blastapi round-robin, browser User-Agent. Header-only
requests (`eth_getBlockByNumber(.., false)`) so no per-tx payload
fetch — much faster + lighter than MERA's per-tx scrape.

Usage:
    scrape_eth.py START END OUTPUT.csv [--sleep N]

Example:
    python3 scrape_eth.py 19900000 19901000 honest.csv --sleep 0.20
"""

import argparse
import csv
import json
import sys
import time
import urllib.error
import urllib.request

RPCS = [
    "https://ethereum.publicnode.com",
    "https://eth-mainnet.public.blastapi.io",
]

UA = (
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
    "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36"
)


def post_json(url, payload, timeout=15.0):
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=data,
        headers={"Content-Type": "application/json", "User-Agent": UA},
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read())


def fetch_header(block, rpc_idx):
    """Fetch one block header (no full tx). Returns dict or None."""
    payload = {
        "jsonrpc": "2.0",
        "method": "eth_getBlockByNumber",
        "params": [hex(block), False],
        "id": block,
    }
    for offset in range(len(RPCS)):
        url = RPCS[(rpc_idx + offset) % len(RPCS)]
        try:
            resp = post_json(url, payload)
            if "result" in resp and resp["result"]:
                return resp["result"]
        except urllib.error.HTTPError as e:
            if e.code in (429, 403):
                time.sleep(2.0)
        except (urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError):
            pass
    return None


def to_int(hex_str):
    return int(hex_str, 16) if hex_str else 0


def main():
    p = argparse.ArgumentParser()
    p.add_argument("start", type=int)
    p.add_argument("end", type=int)
    p.add_argument("output", type=str)
    p.add_argument("--sleep", type=float, default=0.25)
    args = p.parse_args()

    n_total = args.end - args.start
    print(f"Scraping {n_total} block headers via {RPCS}", flush=True)
    print(f"Output: {args.output}", flush=True)

    rows = 0
    failures = 0
    t0 = time.time()
    with open(args.output, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["height", "timestamp_secs", "energy", "gas", "tx_count"])
        for i, block in enumerate(range(args.start, args.end)):
            header = fetch_header(block, i % len(RPCS))
            if header is None:
                failures += 1
                if failures > 50:
                    print(f"too many failures at block {block}, aborting", flush=True)
                    sys.exit(1)
                time.sleep(1.0)
                continue
            # energy proxy = block.size (rough "weight" measure)
            # gas = gasUsed
            # tx_count = len(header.transactions)
            w.writerow([
                block,
                to_int(header.get("timestamp")),
                to_int(header.get("size")),
                to_int(header.get("gasUsed")),
                len(header.get("transactions", [])),
            ])
            rows += 1
            if rows % 100 == 0:
                elapsed = time.time() - t0
                rate = rows / elapsed if elapsed > 0 else 0
                eta = (n_total - rows) / rate if rate > 0 else 0
                print(f"  {rows}/{n_total}  ({rate:.1f} blk/s, ETA {eta:.0f}s)", flush=True)
            time.sleep(args.sleep)

    print(f"DONE: {rows} rows written, {failures} failures, {time.time()-t0:.1f}s", flush=True)


if __name__ == "__main__":
    main()
