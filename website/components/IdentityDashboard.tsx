"use client";

import { useEffect, useState } from "react";

type FourAct = {
  eulogy_count: number;
  eulogy_trie_root: string | null;
  tombstone_addresses: string[];
  refresh_pool_total: number;
  mortis_triggered: boolean;
  mortis_epoch_of_death: number | null;
  mortis_final_state_root: string | null;
  last_conservation_audit_ok: boolean | null;
  genesis_amendment_hash: string | null;
  light_cone_block_count: number;
};

type TurLiveness = {
  verdict: string;
  observed: string | null;
  bound: string | null;
  window_samples: number;
  window_capacity: number;
};

type LambdaFold = {
  acc_hash_hex: string;
  total_energy_remaining: string;
  step_count: number;
  latest_epoch: number;
  is_identity: boolean;
};

type LamportTime = {
  current_tick: number;
  accumulated_energy: number;
  tick_quantum: number;
};

type HbctEntry = {
  delivery_location: string;
  hour_slot: number;
  holder_hex: string;
  mwh_amount: number;
};

type HbctState = {
  entry_count: number;
  total_mwh: number;
  distinct_locations: number;
  distinct_holders: number;
  distinct_hour_slots: number;
  top_entries: HbctEntry[];
};

type Identity = {
  chain_id: string;
  four_act: FourAct;
  light_cone_block_count: number;
  tur_liveness: TurLiveness;
  lambda_fold: LambdaFold;
  lamport_time: LamportTime;
  sentinel_param_count: number;
  hbct: HbctState;
  wired_primitives: string[];
  headline_sentence: string;
};

const DEFAULT_NODE =
  process.env.NEXT_PUBLIC_EVAPORCHAIN_NODE ?? "http://localhost:8080";

export default function IdentityDashboard() {
  const [identity, setIdentity] = useState<Identity | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [endpoint, setEndpoint] = useState(DEFAULT_NODE);

  useEffect(() => {
    let cancelled = false;
    const fetchOnce = async () => {
      try {
        const res = await fetch(`${endpoint}/api/identity`, {
          cache: "no-store",
        });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const data: Identity = await res.json();
        if (!cancelled) {
          setIdentity(data);
          setError(null);
        }
      } catch (e) {
        if (!cancelled)
          setError(e instanceof Error ? e.message : "fetch failed");
      }
    };
    fetchOnce();
    const id = setInterval(fetchOnce, 5_000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [endpoint]);

  if (error && !identity) {
    return (
      <div className="rounded-xl border border-neutral-200 bg-neutral-50 p-8">
        <p className="mb-4 text-sm text-neutral-600">
          Couldn&rsquo;t reach an EvaporChain node at{" "}
          <code className="font-mono text-neutral-800">{endpoint}</code>.
        </p>
        <p className="mb-4 text-xs text-neutral-500">Reason: {error}</p>
        <p className="text-xs text-neutral-500">
          Override with{" "}
          <code className="font-mono">NEXT_PUBLIC_EVAPORCHAIN_NODE</code> at
          build time, or run a local node on :8080.
        </p>
      </div>
    );
  }

  if (!identity) {
    return (
      <div className="animate-pulse rounded-xl border border-neutral-200 bg-neutral-50 p-8 text-sm text-neutral-500">
        Loading chain identity from {endpoint}…
      </div>
    );
  }

  return (
    <div className="space-y-12">
      <p className="max-w-3xl text-base leading-relaxed text-neutral-700">
        {identity.headline_sentence}
      </p>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <Stat
          label="Light-Cone DAG"
          value={identity.light_cone_block_count.toLocaleString()}
          unit="blocks"
        />
        <Stat
          label="Lambda-Fold Steps"
          value={identity.lambda_fold.step_count.toLocaleString()}
          unit={`epoch ${identity.lambda_fold.latest_epoch}`}
        />
        <Stat
          label="Sentinel Params"
          value={identity.sentinel_param_count.toLocaleString()}
          unit="under homeostatic control"
        />
        <Stat
          label="Decay-Lamport Tick"
          value={identity.lamport_time.current_tick.toLocaleString()}
          unit={`q=${identity.lamport_time.tick_quantum}`}
        />
      </div>

      <HbctPanel hbct={identity.hbct} />
      <FourActPanel act={identity.four_act} />
      <LivenessPanel liveness={identity.tur_liveness} />
      <FoldPanel fold={identity.lambda_fold} />
      <PrimitivesPanel primitives={identity.wired_primitives} />

      <div className="border-t border-neutral-200 pt-6 text-xs text-neutral-400">
        Chain id: <code className="font-mono">{identity.chain_id}</code> ·
        Polling every 5s ·{" "}
        <button
          onClick={() => {
            const next =
              prompt("Node base URL", endpoint) ?? endpoint;
            setEndpoint(next);
          }}
          className="underline hover:text-neutral-600"
        >
          change node
        </button>
      </div>
    </div>
  );
}

function Stat({
  label,
  value,
  unit,
}: {
  label: string;
  value: string;
  unit: string;
}) {
  return (
    <div className="rounded-lg border border-neutral-200 bg-white p-5">
      <p className="mb-2 text-xs font-medium uppercase tracking-wider text-neutral-500">
        {label}
      </p>
      <p className="text-3xl font-light text-neutral-900">{value}</p>
      <p className="mt-1 text-xs text-neutral-500">{unit}</p>
    </div>
  );
}

function HbctPanel({ hbct }: { hbct: HbctState }) {
  return (
    <div className="rounded-xl border border-neutral-200 bg-white p-6">
      <div className="mb-4 flex items-baseline justify-between">
        <div>
          <h2 className="text-xl font-light text-neutral-900">
            HBCT — Hour-Block Capacity Tokens
          </h2>
          <p className="text-xs uppercase tracking-wider text-neutral-500">
            Launch wedge · grid capacity that decays at H+1
          </p>
        </div>
        <span className="rounded-full bg-emerald-50 px-3 py-1 text-xs font-medium text-emerald-700">
          launch dApp
        </span>
      </div>
      <div className="grid gap-4 sm:grid-cols-4">
        <Stat
          label="Open positions"
          value={hbct.entry_count.toLocaleString()}
          unit="entries"
        />
        <Stat
          label="Total committed"
          value={hbct.total_mwh.toLocaleString()}
          unit="MWh"
        />
        <Stat
          label="Hour slots"
          value={hbct.distinct_hour_slots.toLocaleString()}
          unit={`${hbct.distinct_locations} locations`}
        />
        <Stat
          label="Distinct holders"
          value={hbct.distinct_holders.toLocaleString()}
          unit="counterparties"
        />
      </div>
      {hbct.top_entries.length > 0 ? (
        <div className="mt-6">
          <p className="mb-3 text-xs uppercase tracking-wider text-neutral-500">
            Top positions by MWh
          </p>
          <div className="overflow-hidden rounded-lg border border-neutral-200">
            <table className="min-w-full divide-y divide-neutral-200 text-xs">
              <thead className="bg-neutral-50">
                <tr>
                  <th className="px-3 py-2 text-left font-medium text-neutral-500">
                    Location
                  </th>
                  <th className="px-3 py-2 text-left font-medium text-neutral-500">
                    Hour slot
                  </th>
                  <th className="px-3 py-2 text-left font-medium text-neutral-500">
                    Holder
                  </th>
                  <th className="px-3 py-2 text-right font-medium text-neutral-500">
                    MWh
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-neutral-100 bg-white">
                {hbct.top_entries.map((e, i) => (
                  <tr key={`${e.delivery_location}-${e.hour_slot}-${e.holder_hex}-${i}`}>
                    <td className="px-3 py-2 font-mono text-neutral-800">
                      {e.delivery_location}
                    </td>
                    <td className="px-3 py-2 font-mono text-neutral-800">
                      {e.hour_slot}
                    </td>
                    <td className="px-3 py-2 font-mono text-neutral-600">
                      {trunc(e.holder_hex)}
                    </td>
                    <td className="px-3 py-2 text-right font-mono text-neutral-900">
                      {e.mwh_amount.toLocaleString()}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      ) : (
        <p className="mt-6 text-sm text-neutral-500">
          No HBCT positions yet. Mint one via{" "}
          <code className="font-mono text-neutral-700">
            POST /api/hbct/mint
          </code>
          .
        </p>
      )}
    </div>
  );
}

function FourActPanel({ act }: { act: FourAct }) {
  return (
    <div className="rounded-xl border border-neutral-200 bg-white p-6">
      <h2 className="mb-1 text-xl font-light text-neutral-900">
        Four-Act Narrative Spine
      </h2>
      <p className="mb-6 text-xs uppercase tracking-wider text-neutral-500">
        Birth · Life · Small Deaths · Final Death
      </p>
      <div className="grid gap-6 sm:grid-cols-2 lg:grid-cols-4">
        <Act
          title="Birth"
          subtitle="Genesis (LLSA-checked)"
          rows={[
            [
              "Constitution",
              act.genesis_amendment_hash
                ? trunc(act.genesis_amendment_hash)
                : "—",
            ],
            [
              "Conservation audit",
              act.last_conservation_audit_ok === null
                ? "warming-up"
                : act.last_conservation_audit_ok
                  ? "ok"
                  : "violation",
            ],
          ]}
        />
        <Act
          title="Life"
          subtitle="Sentinel (homeostasis)"
          rows={[
            ["Refresh pool", act.refresh_pool_total.toLocaleString()],
          ]}
        />
        <Act
          title="Small Deaths"
          subtitle="Tombstone (eulogy trie)"
          rows={[
            ["Memorialised", act.eulogy_count.toLocaleString()],
            [
              "Trie root",
              act.eulogy_trie_root ? trunc(act.eulogy_trie_root) : "—",
            ],
          ]}
        />
        <Act
          title="Final Death"
          subtitle="Mortis (death certificate)"
          rows={[
            [
              "Triggered",
              act.mortis_triggered ? "yes" : "no",
            ],
            [
              "Epoch of death",
              act.mortis_epoch_of_death === null
                ? "—"
                : act.mortis_epoch_of_death.toString(),
            ],
          ]}
        />
      </div>
    </div>
  );
}

function Act({
  title,
  subtitle,
  rows,
}: {
  title: string;
  subtitle: string;
  rows: [string, string][];
}) {
  return (
    <div className="border-l border-neutral-200 pl-4">
      <p className="text-sm font-medium text-neutral-900">{title}</p>
      <p className="mb-3 text-xs text-neutral-500">{subtitle}</p>
      <dl className="space-y-1.5 text-xs">
        {rows.map(([k, v]) => (
          <div key={k} className="flex justify-between gap-3">
            <dt className="text-neutral-500">{k}</dt>
            <dd className="font-mono text-neutral-800">{v}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

function LivenessPanel({ liveness }: { liveness: TurLiveness }) {
  const color =
    liveness.verdict === "ok"
      ? "text-emerald-600"
      : liveness.verdict === "violation"
        ? "text-red-600"
        : "text-neutral-500";
  return (
    <div className="rounded-xl border border-neutral-200 bg-white p-6">
      <h2 className="mb-1 text-xl font-light text-neutral-900">
        TUR Liveness Detector
      </h2>
      <p className="mb-4 text-xs uppercase tracking-wider text-neutral-500">
        Cartel signature: J too steady for the entropy budget
      </p>
      <div className="grid gap-4 sm:grid-cols-3">
        <div>
          <p className="text-xs uppercase text-neutral-500">Verdict</p>
          <p className={`mt-1 text-2xl font-light ${color}`}>
            {liveness.verdict}
          </p>
        </div>
        <div>
          <p className="text-xs uppercase text-neutral-500">Observed</p>
          <p className="mt-1 break-all font-mono text-sm text-neutral-800">
            {liveness.observed ?? "—"}
          </p>
        </div>
        <div>
          <p className="text-xs uppercase text-neutral-500">Bound</p>
          <p className="mt-1 break-all font-mono text-sm text-neutral-800">
            {liveness.bound ?? "—"}
          </p>
        </div>
      </div>
      <p className="mt-4 text-xs text-neutral-500">
        Window: {liveness.window_samples} / {liveness.window_capacity} samples
      </p>
    </div>
  );
}

function FoldPanel({ fold }: { fold: LambdaFold }) {
  return (
    <div className="rounded-xl border border-neutral-200 bg-white p-6">
      <h2 className="mb-1 text-xl font-light text-neutral-900">
        Lambda-Fold Accumulator
      </h2>
      <p className="mb-4 text-xs uppercase tracking-wider text-neutral-500">
        O(1) light-client commitment to chain state + λ-decayed energy
      </p>
      <dl className="space-y-2 text-sm">
        <Row k="Step count" v={fold.step_count.toLocaleString()} />
        <Row k="Latest epoch" v={fold.latest_epoch.toLocaleString()} />
        <Row
          k="Total energy remaining"
          v={fold.total_energy_remaining}
        />
        <Row k="Acc hash" v={fold.acc_hash_hex || "(identity)"} mono />
      </dl>
    </div>
  );
}

function Row({ k, v, mono = false }: { k: string; v: string; mono?: boolean }) {
  return (
    <div className="flex justify-between gap-4 border-b border-neutral-100 pb-1.5">
      <dt className="text-xs text-neutral-500">{k}</dt>
      <dd
        className={`text-xs text-neutral-800 ${mono ? "break-all font-mono" : ""}`}
      >
        {v}
      </dd>
    </div>
  );
}

function PrimitivesPanel({ primitives }: { primitives: string[] }) {
  return (
    <div className="rounded-xl border border-neutral-200 bg-white p-6">
      <h2 className="mb-1 text-xl font-light text-neutral-900">
        Wired primitives ({primitives.length})
      </h2>
      <p className="mb-5 text-xs uppercase tracking-wider text-neutral-500">
        Theorem-grade substrate live in this node
      </p>
      <ul className="grid gap-1.5 text-xs text-neutral-700 sm:grid-cols-2 lg:grid-cols-3">
        {primitives.map((p) => (
          <li key={p} className="border-l-2 border-neutral-200 pl-3">
            {p}
          </li>
        ))}
      </ul>
    </div>
  );
}

function trunc(hex: string): string {
  if (hex.length <= 16) return hex;
  return `${hex.slice(0, 8)}…${hex.slice(-6)}`;
}
