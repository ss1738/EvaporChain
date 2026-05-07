#!/usr/bin/env python3
"""
EvaporChain 5-node cluster dashboard — self-hosted, Tailscale-only.

Polls /api/status + /api/mempool from all 5 validators every POLL_INTERVAL_S
seconds, keeps the last HISTORY_LEN samples in memory, and serves a single-page
HTML dashboard at http://localhost:9090/ that auto-refreshes via fetch().

No third-party deps (stdlib only), no external network calls, no CDN. Runs on
the operator's MacBook over Tailscale; the validators don't need to know it
exists. Stop with Ctrl-C.

Usage:
    python3 scripts/cluster-dashboard.py
    open http://localhost:9090/
"""

from __future__ import annotations

import json
import threading
import time
import urllib.error
import urllib.request
from collections import deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

NODES = [
    ("M1", "100.119.53.101", "UK Mac Mini 1"),
    ("M2", "100.113.253.72",  "UK Mac Mini 2"),
    ("M3", "100.103.216.125", "UK Mac Mini 3"),
    ("H1", "100.66.208.20",   "Helsinki CX23 #1"),
    ("H2", "100.91.235.22",   "Helsinki CX23 #2"),
]
API_PORT = 8081
POLL_INTERVAL_S = 3
HISTORY_LEN = 600   # 30 min @ 3s
LISTEN_PORT = 9090

# Per-node history: deque of (timestamp, status_dict). Shared across threads.
history: dict[str, deque[tuple[float, dict[str, Any]]]] = {
    label: deque(maxlen=HISTORY_LEN) for label, _, _ in NODES
}
history_lock = threading.Lock()


def fetch_json(url: str, timeout: float = 2.5) -> dict[str, Any] | None:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as resp:
            return json.loads(resp.read())
    except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, ValueError, OSError):
        return None


def poll_node(label: str, ip: str) -> dict[str, Any]:
    status = fetch_json(f"http://{ip}:{API_PORT}/api/status") or {}
    mempool = fetch_json(f"http://{ip}:{API_PORT}/api/mempool") or {}
    return {
        "ok": bool(status),
        "block_height": status.get("block_height"),
        "state_root": status.get("state_root", "")[:16],
        "peer_count": status.get("peer_count"),
        "uptime_seconds": status.get("uptime_seconds"),
        "epoch": status.get("epoch"),
        "mempool_pending": mempool.get("pending"),
        # New 2026-05-07 evening: data_dir_bytes is the on-disk size
        # of the validator's chain + state + DA shards. The decay-graph
        # plot shows this against block_height over time — a healthy
        # decay regime makes data_dir_bytes flatten while block_height
        # keeps growing linearly. Field is omitted by older binaries
        # so .get() returns None and the chart simply skips that node
        # until the operator deploys the new build.
        "data_dir_bytes": status.get("data_dir_bytes"),
    }


def poll_loop() -> None:
    while True:
        ts = time.time()
        for label, ip, _name in NODES:
            sample = poll_node(label, ip)
            with history_lock:
                history[label].append((ts, sample))
        time.sleep(POLL_INTERVAL_S)


def build_state() -> dict[str, Any]:
    """Snapshot for the JSON endpoint."""
    out: dict[str, Any] = {"nodes": [], "ts": time.time()}
    with history_lock:
        for label, ip, name in NODES:
            samples = list(history[label])
            latest = samples[-1][1] if samples else {"ok": False}

            heights = [s[1]["block_height"] for s in samples if s[1].get("block_height") is not None]
            block_rate_per_min: float | None = None
            if len(heights) >= 2:
                # Use last 60 samples (≈ 3 min) for short-term rate
                window = samples[-60:]
                window = [s for s in window if s[1].get("block_height") is not None]
                if len(window) >= 2:
                    dt = window[-1][0] - window[0][0]
                    dh = window[-1][1]["block_height"] - window[0][1]["block_height"]
                    if dt > 0:
                        block_rate_per_min = round(dh / dt * 60.0, 1)

            # Decay-graph derived metric: bytes-per-block ratio. Stays
            # ~flat or shrinks under a healthy decay regime; grows
            # linearly on a non-decaying chain. None when either the
            # node is too young or the chain reports no data_dir_bytes
            # (older binary).
            bytes_per_block: float | None = None
            disk = latest.get("data_dir_bytes")
            height = latest.get("block_height")
            if disk is not None and height and height > 0:
                bytes_per_block = round(disk / height, 1)

            # Decay-graph time-series: (relative_seconds, disk_bytes,
            # block_height) tuples for SVG plotting. Down-sample to at
            # most 80 points per node so the SVG stays readable; pick
            # evenly-spaced indices across the captured window.
            disk_series: list[list[float]] = []
            if samples:
                t0 = samples[0][0]
                samples_with_disk = [
                    (t - t0, s.get("data_dir_bytes"), s.get("block_height"))
                    for (t, s) in samples
                    if s.get("data_dir_bytes") is not None
                ]
                if samples_with_disk:
                    n = len(samples_with_disk)
                    step = max(1, n // 80)
                    disk_series = [
                        [round(t, 1), int(d), int(h or 0)]
                        for (t, d, h) in samples_with_disk[::step]
                    ]

            out["nodes"].append({
                "label": label,
                "ip": ip,
                "name": name,
                **latest,
                "block_rate_per_min": block_rate_per_min,
                "bytes_per_block": bytes_per_block,
                "disk_series": disk_series,
                "history_points": len(samples),
            })
    # Cluster-wide convergence score: how many distinct (height, state_root) pairs
    # are there right now. 1 = perfect lockstep; 5 = total fragmentation.
    pairs: set[tuple[int | None, str]] = set()
    for n in out["nodes"]:
        if n.get("ok") and n.get("block_height") is not None:
            pairs.add((n["block_height"], n["state_root"]))
    out["distinct_state_pairs"] = len(pairs)
    out["nodes_responding"] = sum(1 for n in out["nodes"] if n.get("ok"))
    return out


HTML_PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>EvaporChain — 5-node cluster</title>
<style>
  body { background:#0d1117; color:#c9d1d9; font:14px/1.4 -apple-system,BlinkMacSystemFont,monospace; margin:0; padding:18px; }
  h1 { font-weight:500; font-size:18px; margin:0 0 16px; }
  .badge { display:inline-block; padding:2px 8px; border-radius:10px; font-size:11px; font-weight:600; }
  .ok { background:#1a4d2e; color:#7ee2a3; }
  .warn { background:#553c1d; color:#e8c870; }
  .bad { background:#5a1a1a; color:#ff8e8e; }
  table { border-collapse:collapse; width:100%; margin-bottom:18px; font-variant-numeric:tabular-nums; }
  th, td { padding:8px 10px; text-align:left; border-bottom:1px solid #21262d; }
  th { color:#8b949e; font-weight:500; font-size:12px; text-transform:uppercase; letter-spacing:0.05em; }
  td.num { text-align:right; font-family:ui-monospace,monospace; }
  td.sr { font-family:ui-monospace,monospace; color:#58a6ff; }
  .header-row { display:flex; gap:14px; align-items:center; margin-bottom:18px; flex-wrap:wrap; }
  .stat { background:#161b22; padding:10px 14px; border-radius:6px; }
  .stat .lbl { color:#8b949e; font-size:11px; text-transform:uppercase; letter-spacing:0.05em; }
  .stat .val { font-size:18px; font-weight:600; font-family:ui-monospace,monospace; }
  .footer { color:#6e7681; font-size:11px; margin-top:18px; }
  .down { color:#ff8e8e; }
</style>
</head>
<body>
<h1>EvaporChain &mdash; 5-node UK + Helsinki cluster</h1>
<div class="header-row" id="hdr"></div>
<table id="nodes">
  <thead>
    <tr>
      <th>Node</th><th>Region</th><th>Height</th><th>State Root (16 hex)</th>
      <th>Peers</th><th>Mempool</th><th>Block&nbsp;rate /min</th>
      <th>Disk&nbsp;(MB)</th><th>Bytes&nbsp;/&nbsp;block</th>
      <th>Uptime</th><th>Status</th>
    </tr>
  </thead>
  <tbody></tbody>
</table>
<p style="color:#6e7681;font-size:11px;margin-top:6px;">
  <strong>Bytes/block</strong> is the decay-primitive indicator — flat or shrinking over time = healthy decay (state evaporates as it ages). Linearly growing = no decay (unbounded state growth like every other L1).
</p>

<h2 style="margin-top:24px;font-weight:500;font-size:14px;">Decay graph — disk bytes over time, per node</h2>
<p style="color:#6e7681;font-size:11px;margin:0 0 8px;">
  Live in-memory window (last ~30 min). Vanilla SVG, no chart library, no third-party. A flat or sub-linear curve under sustained block production = the decay primitive working as designed.
</p>
<svg id="decay-chart" width="100%" height="320" style="background:#0d1117;border:1px solid #21262d;border-radius:6px;"></svg>
<div class="footer">Polling every 3 s &middot; data is in-memory only &middot; dashboard process serves on localhost:9090.</div>
<script>
function fmtUptime(s) {
  if (s == null) return '-';
  const h = Math.floor(s/3600), m = Math.floor((s%3600)/60), sec = s%60;
  if (h > 0) return h + 'h' + (m<10?'0':'') + m + 'm';
  if (m > 0) return m + 'm' + (sec<10?'0':'') + sec + 's';
  return sec + 's';
}
function fmtMB(bytes) {
  if (bytes == null) return '-';
  return (bytes / (1024*1024)).toFixed(1);
}
function fmtBytesPerBlock(bpb) {
  if (bpb == null) return '-';
  if (bpb < 1024) return bpb.toFixed(0) + 'B';
  if (bpb < 1024*1024) return (bpb/1024).toFixed(1) + 'KB';
  return (bpb/(1024*1024)).toFixed(2) + 'MB';
}

// Per-node colour palette. Stable across re-renders so each node
// keeps its line colour. Picked for contrast on the dark background.
const NODE_COLORS = {
  M1: '#58a6ff',  // blue
  M2: '#7ee2a3',  // green
  M3: '#e8c870',  // amber
  H1: '#ff8e8e',  // red
  H2: '#c7a6ff',  // violet
};

function drawDecayChart(nodes) {
  const svg = document.getElementById('decay-chart');
  // Pull width from the rendered <svg> width (inherits from CSS 100%).
  const W = svg.clientWidth || 1000;
  const H = 320;
  const padL = 60, padR = 110, padT = 18, padB = 36;
  const plotW = W - padL - padR;
  const plotH = H - padT - padB;

  // Pull all (t, bytes) points across all nodes to find axis ranges.
  let minT = Infinity, maxT = -Infinity;
  let minB = Infinity, maxB = -Infinity;
  let havePoints = false;
  for (const n of nodes) {
    const series = n.disk_series || [];
    for (const [t, b, h] of series) {
      havePoints = true;
      if (t < minT) minT = t;
      if (t > maxT) maxT = t;
      if (b < minB) minB = b;
      if (b > maxB) maxB = b;
    }
  }
  if (!havePoints) {
    svg.innerHTML = `<text x="${W/2}" y="${H/2}" fill="#6e7681" font-family="monospace" font-size="13" text-anchor="middle">No decay data yet — older binary doesn't expose data_dir_bytes. Rebuild + restart at least one validator and wait one poll cycle.</text>`;
    return;
  }
  // Pad y-axis 5% above max for headroom.
  const yPad = (maxB - minB) * 0.05 || 1;
  minB = Math.max(0, minB - yPad);
  maxB = maxB + yPad;
  const tRange = Math.max(1, maxT - minT);
  const bRange = Math.max(1, maxB - minB);
  const xOf = t => padL + ((t - minT) / tRange) * plotW;
  const yOf = b => padT + plotH - ((b - minB) / bRange) * plotH;

  // y-axis MB labels (5 ticks).
  const yTicks = 5;
  let axes = '';
  for (let i = 0; i <= yTicks; i++) {
    const v = minB + (bRange * i / yTicks);
    const y = yOf(v);
    const mb = (v / (1024*1024)).toFixed(0);
    axes += `<line x1="${padL}" y1="${y}" x2="${W-padR}" y2="${y}" stroke="#21262d" stroke-width="1"/>`;
    axes += `<text x="${padL-6}" y="${y+4}" fill="#6e7681" font-family="monospace" font-size="10" text-anchor="end">${mb} MB</text>`;
  }
  // x-axis time labels (5 ticks, in seconds → minutes).
  const xTicks = 5;
  for (let i = 0; i <= xTicks; i++) {
    const t = minT + (tRange * i / xTicks);
    const x = xOf(t);
    const min = (t / 60).toFixed(1);
    axes += `<line x1="${x}" y1="${padT}" x2="${x}" y2="${H-padB}" stroke="#21262d" stroke-width="1"/>`;
    axes += `<text x="${x}" y="${H-padB+14}" fill="#6e7681" font-family="monospace" font-size="10" text-anchor="middle">${min}m</text>`;
  }

  // Per-node polylines.
  let lines = '';
  let legend = '';
  let lx = W - padR + 10;
  let ly = padT + 8;
  for (const n of nodes) {
    const colour = NODE_COLORS[n.label] || '#c9d1d9';
    const pts = (n.disk_series || []).map(([t, b]) => `${xOf(t).toFixed(1)},${yOf(b).toFixed(1)}`).join(' ');
    if (pts) {
      lines += `<polyline points="${pts}" fill="none" stroke="${colour}" stroke-width="1.6" stroke-linejoin="round"/>`;
    }
    legend += `<text x="${lx}" y="${ly}" fill="${colour}" font-family="monospace" font-size="11">${n.label}</text>`;
    ly += 16;
  }

  svg.innerHTML = axes + lines + legend;
}
async function tick() {
  let data;
  try { data = await (await fetch('/state.json')).json(); }
  catch (e) { document.getElementById('hdr').innerHTML = '<span class="bad">dashboard offline</span>'; return; }
  const distinct = data.distinct_state_pairs;
  const responding = data.nodes_responding;
  const lockstep = distinct === 1 ? 'ok' : (distinct <= 2 ? 'warn' : 'bad');
  document.getElementById('hdr').innerHTML = `
    <div class="stat"><div class="lbl">Nodes responding</div><div class="val">${responding} / 5</div></div>
    <div class="stat"><div class="lbl">Distinct state pairs</div><div class="val"><span class="badge ${lockstep}">${distinct}</span></div></div>
    <div class="stat"><div class="lbl">Cluster convergence</div><div class="val">${distinct === 1 ? '5/5 lockstep' : 'spread'}</div></div>
  `;
  const tbody = document.querySelector('#nodes tbody');
  drawDecayChart(data.nodes);
  tbody.innerHTML = data.nodes.map(n => {
    if (!n.ok) return `<tr><td>${n.label}</td><td>${n.name}</td><td colspan="9" class="down">unreachable (${n.ip})</td></tr>`;
    return `<tr>
      <td><strong>${n.label}</strong></td>
      <td>${n.name}</td>
      <td class="num">${n.block_height}</td>
      <td class="sr">${n.state_root}</td>
      <td class="num">${n.peer_count}</td>
      <td class="num">${n.mempool_pending ?? '-'}</td>
      <td class="num">${n.block_rate_per_min ?? '-'}</td>
      <td class="num">${fmtMB(n.data_dir_bytes)}</td>
      <td class="num">${fmtBytesPerBlock(n.bytes_per_block)}</td>
      <td class="num">${fmtUptime(n.uptime_seconds)}</td>
      <td><span class="badge ok">live</span></td>
    </tr>`;
  }).join('');
}
tick();
setInterval(tick, 3000);
</script>
</body>
</html>"""


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args, **kwargs) -> None:  # quiet access log
        pass

    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/state.json":
            body = json.dumps(build_state()).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)
        elif self.path in ("/", "/index.html"):
            body = HTML_PAGE.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()


def main() -> None:
    threading.Thread(target=poll_loop, daemon=True).start()
    print(f"EvaporChain cluster dashboard → http://localhost:{LISTEN_PORT}/")
    print(f"Polling {len(NODES)} validators every {POLL_INTERVAL_S}s, history cap {HISTORY_LEN}")
    server = ThreadingHTTPServer(("127.0.0.1", LISTEN_PORT), Handler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nshutting down")
        server.shutdown()


if __name__ == "__main__":
    main()
