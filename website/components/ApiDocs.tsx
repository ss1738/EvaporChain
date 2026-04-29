"use client";

import { useEffect, useState } from "react";

type ApiDocEntry = {
  method: string;
  path: string;
  category: string;
  description: string;
  example: string | null;
};

type ApiDocsResp = {
  chain: string;
  launch_sprint_endpoints: number;
  endpoints: ApiDocEntry[];
};

const DEFAULT_NODE =
  process.env.NEXT_PUBLIC_EVAPORCHAIN_NODE ?? "http://localhost:8080";

const CATEGORY_ORDER = [
  "identity",
  "substrate",
  "hbct",
  "sentinel",
  "demo",
] as const;

const CATEGORY_BLURB: Record<string, string> = {
  identity:
    "Single-call dashboard summaries + per-primitive observability.",
  substrate:
    "Theorem-graded primitives callable as pure compute by light clients and dApps.",
  hbct:
    "Hour-Block Capacity Tokens — the launch wedge. Mint, transfer, burn, settle.",
  sentinel:
    "Autonomic governance — homeostasis, not legislators.",
  demo:
    "Reset endpoints for the dashboard demo loop.",
};

export default function ApiDocs() {
  const [docs, setDocs] = useState<ApiDocsResp | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [endpoint] = useState(DEFAULT_NODE);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const res = await fetch(`${endpoint}/api/docs`, { cache: "no-store" });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const data: ApiDocsResp = await res.json();
        if (!cancelled) {
          setDocs(data);
          setError(null);
        }
      } catch (e) {
        if (!cancelled)
          setError(e instanceof Error ? e.message : "fetch failed");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [endpoint]);

  if (error && !docs) {
    return (
      <div className="rounded-xl border border-neutral-200 bg-neutral-50 p-8 text-sm text-neutral-600">
        Couldn&rsquo;t reach a node at{" "}
        <code className="font-mono">{endpoint}</code>: {error}
      </div>
    );
  }

  if (!docs) {
    return (
      <div className="animate-pulse rounded-xl border border-neutral-200 bg-neutral-50 p-8 text-sm text-neutral-500">
        Loading endpoint catalog…
      </div>
    );
  }

  const grouped = new Map<string, ApiDocEntry[]>();
  for (const e of docs.endpoints) {
    grouped.set(e.category, [...(grouped.get(e.category) ?? []), e]);
  }
  const ordered = CATEGORY_ORDER.filter((c) => grouped.has(c)).concat(
    [...grouped.keys()].filter(
      (c) => !CATEGORY_ORDER.includes(c as (typeof CATEGORY_ORDER)[number]),
    ),
  );

  return (
    <div className="space-y-10">
      <div className="rounded-lg border border-neutral-200 bg-neutral-50 px-4 py-3 text-xs text-neutral-600">
        {docs.launch_sprint_endpoints} endpoints registered ·{" "}
        chain id <code className="font-mono">{docs.chain}</code> ·{" "}
        served from <code className="font-mono">{endpoint}/api/docs</code>
      </div>

      {ordered.map((cat) => (
        <section key={cat}>
          <h2 className="mb-1 text-xl font-light text-neutral-900">
            {cat[0].toUpperCase() + cat.slice(1)}
          </h2>
          <p className="mb-4 text-sm text-neutral-500">
            {CATEGORY_BLURB[cat] ?? ""}
          </p>
          <div className="overflow-hidden rounded-lg border border-neutral-200">
            <table className="min-w-full divide-y divide-neutral-200 text-xs">
              <thead className="bg-neutral-50">
                <tr>
                  <th className="w-16 px-3 py-2 text-left font-medium text-neutral-500">
                    Method
                  </th>
                  <th className="w-1/3 px-3 py-2 text-left font-medium text-neutral-500">
                    Path
                  </th>
                  <th className="px-3 py-2 text-left font-medium text-neutral-500">
                    Description / example
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-neutral-100 bg-white">
                {(grouped.get(cat) ?? []).map((e) => (
                  <tr key={`${e.method}-${e.path}`}>
                    <td className="px-3 py-2 align-top font-mono text-neutral-700">
                      <span
                        className={`inline-block rounded px-1.5 py-0.5 ${
                          e.method === "GET"
                            ? "bg-emerald-50 text-emerald-700"
                            : "bg-blue-50 text-blue-700"
                        }`}
                      >
                        {e.method}
                      </span>
                    </td>
                    <td className="px-3 py-2 align-top font-mono text-neutral-900">
                      {e.path}
                    </td>
                    <td className="px-3 py-2 align-top text-neutral-700">
                      <p>{e.description}</p>
                      {e.example && (
                        <pre className="mt-2 overflow-x-auto rounded bg-neutral-50 px-2 py-1.5 text-[11px] text-neutral-700">
                          {e.example}
                        </pre>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      ))}
    </div>
  );
}
