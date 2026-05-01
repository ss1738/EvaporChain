"use client";

import { useEffect, useState } from "react";
import { singhApi, type SinghPoolPosition } from "@/lib/api";
import { PositionCard } from "@/components/PositionCard";

export default function PoolListPage() {
  const [positions, setPositions] = useState<SinghPoolPosition[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [refreshPool, setRefreshPool] = useState<{ totalAccrued: number } | null>(null);
  const [createOpen, setCreateOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const fetchData = async () => {
      try {
        const [pos, pool] = await Promise.all([
          singhApi.listPositions(),
          singhApi.refreshPool().catch(() => null),
        ]);
        if (cancelled) return;
        setPositions(pos);
        if (pool) setRefreshPool({ totalAccrued: pool.totalAccrued });
        setError(null);
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : "Failed to load positions");
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    fetchData();
    const id = setInterval(fetchData, 8_000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  const active = positions.filter((p) => p.state === "Active");
  const grace = positions.filter((p) => p.state === "Grace");
  const ghost = positions.filter((p) => p.state === "Ghost");
  const totalLiquidity = active.reduce((s, p) => s + p.liquidity, 0);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-bold text-zinc-900">Liquidity Positions</h1>
          <p className="mt-1 text-xs text-zinc-500">
            Concentrated-liquidity AMM with energy-decaying positions. Decayed
            positions stop earning fees and eventually evaporate unless refreshed.
          </p>
        </div>
        <button
          onClick={() => setCreateOpen(true)}
          className="rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple px-4 py-2 text-xs font-semibold text-white hover:opacity-90"
        >
          + Add Position
        </button>
      </div>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Stat label="Active" value={active.length} tone="text-evap-green" />
        <Stat label="Decaying" value={grace.length} tone="text-evap-amber" />
        <Stat label="Ghosted" value={ghost.length} tone="text-zinc-500" />
        <Stat
          label="Active Liquidity"
          value={totalLiquidity.toLocaleString()}
          tone="text-evap-cyan"
        />
      </div>

      {refreshPool && (
        <div className="rounded-xl border border-evap-border bg-white p-4">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-xs font-semibold text-zinc-900">Protocol Refresh Pool</p>
              <p className="text-[10px] text-zinc-500">
                EVAP held by the chain to fund object refreshes. LPs that pay
                demurrage on their positions help feed this pool.
              </p>
            </div>
            <p className="text-lg font-bold text-evap-cyan">
              {refreshPool.totalAccrued.toLocaleString()} EVAP
            </p>
          </div>
        </div>
      )}

      {error && (
        <div className="rounded-lg border border-evap-red/30 bg-red-50 p-3 text-sm text-evap-red">
          {error}
        </div>
      )}

      {loading ? (
        <div className="flex items-center justify-center py-20">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-evap-cyan/30 border-t-evap-cyan" />
        </div>
      ) : positions.length === 0 ? (
        <div className="rounded-xl border border-evap-border bg-white py-20 text-center">
          <p className="text-sm text-zinc-500">No positions yet.</p>
          <p className="mt-1 text-xs text-zinc-400">
            Add a position above to provide concentrated liquidity.
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {positions.map((p) => (
            <PositionCard key={p.id} position={p} />
          ))}
        </div>
      )}

      {createOpen && <CreatePositionDialog onClose={() => setCreateOpen(false)} />}
    </div>
  );
}

function Stat({ label, value, tone }: { label: string; value: number | string; tone: string }) {
  return (
    <div className="rounded-xl border border-evap-border bg-white px-4 py-3">
      <p className={`text-xl font-bold ${tone}`}>{value}</p>
      <p className="text-[10px] text-zinc-400">{label}</p>
    </div>
  );
}

function CreatePositionDialog({ onClose }: { onClose: () => void }) {
  const [tokenA, setTokenA] = useState("EVAP");
  const [tokenB, setTokenB] = useState("USDC");
  const [liquidity, setLiquidity] = useState(1_000);
  const [lowerTick, setLowerTick] = useState(-1_000);
  const [upperTick, setUpperTick] = useState(1_000);
  const [energy, setEnergy] = useState(500);
  const [halfLife, setHalfLife] = useState(120);
  const [error, setError] = useState<string | null>(null);

  const handleCreate = async () => {
    if (lowerTick >= upperTick) {
      setError("Lower tick must be < upper tick");
      return;
    }
    // Position creation lands on-chain via /api/tx/create-object once the
    // wallet provider supplies a connected signer. Until that wiring is in
    // place we surface a non-destructive notice rather than fabricating a tx.
    setError(
      "Connect EvaporChain Wallet to broadcast: tokens, ticks, and energy are validated locally.",
    );
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
      <div className="w-full max-w-md rounded-2xl border border-evap-border bg-white p-6 shadow-xl">
        <h3 className="text-lg font-semibold text-zinc-900">New LP Position</h3>
        <p className="mt-1 text-xs text-zinc-500">
          Concentrated-liquidity range with decaying energy reserve. Refresh to keep earning fees.
        </p>

        <div className="mt-4 grid grid-cols-2 gap-3">
          <Field label="Token A" value={tokenA} onChange={setTokenA} />
          <Field label="Token B" value={tokenB} onChange={setTokenB} />
          <NumField label="Liquidity" value={liquidity} onChange={setLiquidity} />
          <NumField label="Energy" value={energy} onChange={setEnergy} />
          <NumField label="Lower tick" value={lowerTick} onChange={setLowerTick} />
          <NumField label="Upper tick" value={upperTick} onChange={setUpperTick} />
          <NumField label="Half-life (epochs)" value={halfLife} onChange={setHalfLife} />
        </div>

        {error && <p className="mt-3 text-xs text-evap-red">{error}</p>}

        <div className="mt-6 flex gap-3">
          <button
            onClick={onClose}
            className="flex-1 rounded-lg border border-evap-border px-4 py-2 text-sm font-medium text-zinc-600 hover:bg-zinc-50"
          >
            Cancel
          </button>
          <button
            onClick={handleCreate}
            className="flex-1 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple px-4 py-2 text-sm font-medium text-white hover:opacity-90"
          >
            Create Position
          </button>
        </div>
      </div>
    </div>
  );
}

function Field({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <div>
      <label className="block text-xs font-medium text-zinc-700">{label}</label>
      <input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="mt-1 w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-sm focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
      />
    </div>
  );
}

function NumField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <div>
      <label className="block text-xs font-medium text-zinc-700">{label}</label>
      <input
        type="number"
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="mt-1 w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-sm focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
      />
    </div>
  );
}
