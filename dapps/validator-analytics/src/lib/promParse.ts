/**
 * Minimal Prometheus text-exposition parser.
 *
 * Only extracts what this dashboard needs from the EvaporChain node's
 * `/metrics` endpoint:
 *  - simple gauges/counters: `name value`
 *  - labelled samples:       `name{a="x",b="y"} value`
 *  - histogram buckets:      `name_bucket{le="0.5", ...} value`
 *
 * Lines starting with `#` (HELP / TYPE) are ignored. We don't validate types;
 * the consumer asks for a specific metric and we return matching samples.
 */
export interface PromSample {
  name: string;
  labels: Record<string, string>;
  value: number;
}

const LINE_RE = /^([a-zA-Z_:][a-zA-Z0-9_:]*)(\{([^}]*)\})?\s+([0-9eE+\-.NaInf]+)\s*$/;

export function parsePrometheus(text: string): PromSample[] {
  const out: PromSample[] = [];
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const m = LINE_RE.exec(line);
    if (!m) continue;
    const [, name, , labelStr, valueStr] = m;
    const value = parseFloat(valueStr);
    if (!Number.isFinite(value) && valueStr !== "+Inf") continue;
    out.push({ name, labels: parseLabels(labelStr), value });
  }
  return out;
}

function parseLabels(s: string | undefined): Record<string, string> {
  if (!s) return {};
  const labels: Record<string, string> = {};
  // Handles a="x",b="y,with,commas" — we split by `",` then trim trailing `"`.
  for (const part of s.split(/",/)) {
    const eq = part.indexOf("=");
    if (eq < 0) continue;
    const k = part.slice(0, eq).trim();
    let v = part.slice(eq + 1).trim();
    if (v.startsWith("\"")) v = v.slice(1);
    if (v.endsWith("\"")) v = v.slice(0, -1);
    labels[k] = v;
  }
  return labels;
}

/** Filter samples by metric name. */
export function samplesByName(samples: PromSample[], name: string): PromSample[] {
  return samples.filter((s) => s.name === name);
}

/** Per-validator histogram view derived from `evap_block_production_seconds_*`. */
export interface ValidatorBlockHist {
  producer: string;
  count: number;
  sum: number;
  /** Mean execution time (seconds) for the producer's blocks. */
  meanSecs: number;
  /** Cumulative bucket counts, keyed by upper bound. */
  buckets: Array<{ le: string; cum: number }>;
}

export function blockProductionByValidator(samples: PromSample[]): ValidatorBlockHist[] {
  const counts = new Map<string, number>();
  const sums = new Map<string, number>();
  const buckets = new Map<string, Array<{ le: string; cum: number }>>();
  for (const s of samples) {
    const p = s.labels.producer;
    if (!p) continue;
    if (s.name === "evap_block_production_seconds_count") counts.set(p, s.value);
    else if (s.name === "evap_block_production_seconds_sum") sums.set(p, s.value);
    else if (s.name === "evap_block_production_seconds_bucket") {
      const arr = buckets.get(p) ?? [];
      arr.push({ le: s.labels.le ?? "+Inf", cum: s.value });
      buckets.set(p, arr);
    }
  }
  const out: ValidatorBlockHist[] = [];
  for (const [p, count] of counts) {
    const sum = sums.get(p) ?? 0;
    out.push({
      producer: p,
      count,
      sum,
      meanSecs: count > 0 ? sum / count : 0,
      buckets: (buckets.get(p) ?? []).sort((a, b) => +a.le - +b.le),
    });
  }
  return out;
}

/** Peer score samples → array of {peer_id, score}. */
export function peerScores(samples: PromSample[]): Array<{ peerId: string; score: number }> {
  return samples
    .filter((s) => s.name === "evap_peer_score")
    .map((s) => ({ peerId: s.labels.peer_id ?? "unknown", score: s.value }));
}

/** Inbound rejection counters by reason. */
export function rejectionCounters(samples: PromSample[]): Record<string, number> {
  const r: Record<string, number> = {};
  for (const s of samples) {
    if (s.name !== "evap_inbound_rejections_total") continue;
    r[s.labels.reason ?? "unknown"] = s.value;
  }
  return r;
}

/** Look up a single scalar gauge by name (returns 0 if absent). */
export function gauge(samples: PromSample[], name: string): number {
  const m = samples.find((s) => s.name === name);
  return m ? m.value : 0;
}
