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

      <HbctPanel
        hbct={identity.hbct}
        endpoint={endpoint}
        currentEpoch={identity.lambda_fold.latest_epoch}
      />
      <FourActPanel act={identity.four_act} endpoint={endpoint} />
      <LivenessPanel liveness={identity.tur_liveness} />
      <FoldPanel fold={identity.lambda_fold} endpoint={endpoint} />
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

function HbctPanel({
  hbct,
  endpoint,
  currentEpoch,
}: {
  hbct: HbctState;
  endpoint: string;
  currentEpoch: number;
}) {
  const [busy, setBusy] = useState<"seed" | "tick" | null>(null);
  const [msg, setMsg] = useState<string | null>(null);

  async function seed() {
    setBusy("seed");
    setMsg(null);
    try {
      const res = await fetch(`${endpoint}/api/hbct/seed_demo`, {
        method: "POST",
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      setMsg(
        `Minted ${data.minted_positions} demo positions — refresh in 5s.`,
      );
    } catch (e) {
      setMsg(e instanceof Error ? e.message : "seed failed");
    } finally {
      setBusy(null);
    }
  }

  async function tick() {
    setBusy("tick");
    setMsg(null);
    try {
      // Tick the chain ~1 epoch past the latest seeded slot so any H+1
      // expiries fire deterministically. Seeded slots top out at
      // 481252; chain epoch may be lower in dev. Pick the larger.
      const at = Math.max(currentEpoch, 481253);
      const res = await fetch(`${endpoint}/api/hbct/tick`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ current_epoch: at }),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      setMsg(
        `H+1 tick at epoch ${at}: ${data.entries_removed} entries closed, ${data.mwh_burnt} MWh burnt.`,
      );
    } catch (e) {
      setMsg(e instanceof Error ? e.message : "tick failed");
    } finally {
      setBusy(null);
    }
  }

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
      <div className="mt-5 flex flex-wrap gap-2">
        <button
          onClick={seed}
          disabled={busy !== null}
          className="rounded-md bg-neutral-900 px-4 py-2 text-xs font-medium text-white hover:bg-neutral-700 disabled:opacity-50"
        >
          {busy === "seed" ? "Seeding…" : "Seed demo positions"}
        </button>
        <button
          onClick={tick}
          disabled={busy !== null || hbct.entry_count === 0}
          className="rounded-md border border-neutral-300 bg-white px-4 py-2 text-xs font-medium text-neutral-900 hover:bg-neutral-50 disabled:opacity-50"
          title="Auto-burn any positions whose hour slot has closed (H+1 decay)"
        >
          {busy === "tick" ? "Ticking…" : "Tick H+1 (auto-burn closed slots)"}
        </button>
      </div>
      {msg && (
        <p className="mt-3 text-xs text-neutral-600">{msg}</p>
      )}
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
          No HBCT positions yet — click <strong>Seed demo positions</strong>{" "}
          above to mint 8 realistic positions across GB BMUs + DE-LU,
          then click <strong>Tick H+1</strong> to watch closed slots
          auto-burn. Manual mint also via{" "}
          <code className="font-mono text-neutral-700">
            POST /api/hbct/mint
          </code>
          .
        </p>
      )}
    </div>
  );
}

type MortisCertPreview = {
  status: string;
  final_state_root_hex: string;
  eulogy_trie_root_hex: string;
  epoch_of_death: number;
  final_refresh_pool: number;
  witness_hex: string;
  note: string;
};

function FourActPanel({
  act,
  endpoint,
}: {
  act: FourAct;
  endpoint: string;
}) {
  const [preview, setPreview] = useState<MortisCertPreview | null>(null);
  const [loadingPreview, setLoadingPreview] = useState(false);

  async function fetchPreview() {
    setLoadingPreview(true);
    try {
      const res = await fetch(`${endpoint}/api/mortis_cert_preview`);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data: MortisCertPreview = await res.json();
      setPreview(data);
    } catch (e) {
      setPreview({
        status: "error",
        final_state_root_hex: "",
        eulogy_trie_root_hex: "",
        epoch_of_death: 0,
        final_refresh_pool: 0,
        witness_hex: "",
        note: e instanceof Error ? e.message : "preview fetch failed",
      });
    } finally {
      setLoadingPreview(false);
    }
  }

  return (
    <div className="rounded-xl border border-neutral-200 bg-white p-6">
      <div className="mb-1 flex items-baseline justify-between">
        <h2 className="text-xl font-light text-neutral-900">
          Four-Act Narrative Spine
        </h2>
        <button
          onClick={fetchPreview}
          disabled={loadingPreview}
          className="rounded-md border border-neutral-300 bg-white px-3 py-1.5 text-xs font-medium text-neutral-900 hover:bg-neutral-50 disabled:opacity-50"
          title="Build the death certificate that would mint at current chain state — without triggering death"
        >
          {loadingPreview ? "Loading…" : "Preview Mortis cert"}
        </button>
      </div>
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
      {preview && (
        <div
          className={`mt-6 rounded-lg border p-4 text-xs ${
            preview.status === "preview"
              ? "border-neutral-200 bg-neutral-50"
              : preview.status === "already-triggered"
                ? "border-amber-200 bg-amber-50 text-amber-900"
                : "border-red-200 bg-red-50 text-red-900"
          }`}
        >
          <p className="mb-2 font-medium uppercase tracking-wider text-neutral-600">
            Mortis cert · {preview.status}
          </p>
          {preview.status === "preview" && (
            <dl className="space-y-1.5 text-xs">
              <Row k="Epoch of death" v={preview.epoch_of_death.toLocaleString()} />
              <Row
                k="Final refresh pool"
                v={preview.final_refresh_pool.toLocaleString()}
              />
              <Row
                k="Final state root"
                v={trunc(preview.final_state_root_hex)}
                mono
              />
              <Row
                k="Eulogy trie root"
                v={trunc(preview.eulogy_trie_root_hex)}
                mono
              />
              <Row k="Witness" v={trunc(preview.witness_hex)} mono />
            </dl>
          )}
          <p className="mt-2 italic opacity-80">{preview.note}</p>
        </div>
      )}
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

function FoldPanel({
  fold,
  endpoint,
}: {
  fold: LambdaFold;
  endpoint: string;
}) {
  const [verifying, setVerifying] = useState(false);
  const [verifyResult, setVerifyResult] = useState<{
    ok: boolean;
    elapsed_ms: number;
    detail: string;
  } | null>(null);

  async function verify() {
    if (!fold.acc_hash_hex) {
      setVerifyResult({
        ok: false,
        elapsed_ms: 0,
        detail: "fold accumulator is at identity — nothing to verify yet",
      });
      return;
    }
    setVerifying(true);
    setVerifyResult(null);
    const t0 = performance.now();
    try {
      const res = await fetch(`${endpoint}/api/lambda_fold/verify`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          expected_acc_hash_hex: fold.acc_hash_hex,
          expected_remaining_energy: Number(fold.total_energy_remaining),
        }),
      });
      const elapsed_ms = performance.now() - t0;
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      setVerifyResult({
        ok: data.status === "ok",
        elapsed_ms,
        detail:
          data.detail ||
          (data.status === "ok"
            ? "verifier accepted in O(1) work — independent of chain length"
            : "verifier rejected"),
      });
    } catch (e) {
      setVerifyResult({
        ok: false,
        elapsed_ms: performance.now() - t0,
        detail: e instanceof Error ? e.message : "verify failed",
      });
    } finally {
      setVerifying(false);
    }
  }

  return (
    <div className="rounded-xl border border-neutral-200 bg-white p-6">
      <div className="mb-4 flex items-baseline justify-between">
        <div>
          <h2 className="text-xl font-light text-neutral-900">
            Lambda-Fold Accumulator
          </h2>
          <p className="text-xs uppercase tracking-wider text-neutral-500">
            O(1) light-client commitment to chain state + λ-decayed energy
          </p>
        </div>
        <button
          onClick={verify}
          disabled={verifying}
          className="rounded-md border border-neutral-300 bg-white px-4 py-2 text-xs font-medium text-neutral-900 hover:bg-neutral-50 disabled:opacity-50"
          title="Submit (acc_hash, total_energy_remaining) to /api/lambda_fold/verify"
        >
          {verifying ? "Verifying…" : "Verify in O(1)"}
        </button>
      </div>
      <dl className="space-y-2 text-sm">
        <Row k="Step count" v={fold.step_count.toLocaleString()} />
        <Row k="Latest epoch" v={fold.latest_epoch.toLocaleString()} />
        <Row
          k="Total energy remaining"
          v={fold.total_energy_remaining}
        />
        <Row k="Acc hash" v={fold.acc_hash_hex || "(identity)"} mono />
      </dl>
      {verifyResult && (
        <div
          className={`mt-4 rounded-lg border p-3 text-xs ${
            verifyResult.ok
              ? "border-emerald-200 bg-emerald-50 text-emerald-800"
              : "border-red-200 bg-red-50 text-red-800"
          }`}
        >
          <p className="font-medium">
            {verifyResult.ok ? "✓ Verifier accepted" : "✗ Verifier rejected"}
            {verifyResult.elapsed_ms > 0 && (
              <span className="ml-2 font-normal opacity-70">
                ({verifyResult.elapsed_ms.toFixed(1)} ms round trip · {fold.step_count.toLocaleString()} folded blocks)
              </span>
            )}
          </p>
          <p className="mt-1 opacity-80">{verifyResult.detail}</p>
        </div>
      )}
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
