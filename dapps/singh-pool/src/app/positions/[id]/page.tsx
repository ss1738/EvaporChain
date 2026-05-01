"use client";

import { useEffect, useState } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import { singhApi, type SinghPoolPosition } from "@/lib/api";
import { EnergyBar } from "@/components/EnergyBar";
import { DecayForecast } from "@/components/DecayForecast";
import { RefreshButton } from "@/components/RefreshButton";
import { PatronageDialog } from "@/components/PatronageDialog";

export default function PositionDetailPage() {
  const params = useParams<{ id: string }>();
  const id = decodeURIComponent(params.id);
  const [position, setPosition] = useState<SinghPoolPosition | null>(null);
  const [currentEpoch, setCurrentEpoch] = useState<number>(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showPatronage, setShowPatronage] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const fetchAll = async () => {
      try {
        const [pos, status] = await Promise.all([
          singhApi.getPosition(id),
          singhApi.chainStatus().catch(() => null),
        ]);
        if (cancelled) return;
        if (!pos) {
          setError("Position not found");
        } else {
          setPosition(pos);
          setError(null);
        }
        if (status) setCurrentEpoch(status.epoch);
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : "Failed to load");
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    fetchAll();
    const t = setInterval(fetchAll, 8_000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, [id]);

  if (loading) {
    return (
      <div className="flex items-center justify-center py-20">
        <div className="h-8 w-8 animate-spin rounded-full border-2 border-evap-cyan/30 border-t-evap-cyan" />
      </div>
    );
  }

  if (error || !position) {
    return (
      <div className="space-y-4">
        <Link href="/" className="text-sm text-evap-cyan hover:underline">
          ← Back
        </Link>
        <div className="rounded-lg border border-evap-red/30 bg-red-50 p-3 text-sm text-evap-red">
          {error ?? "Position not found"}
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <Link href="/" className="text-sm text-evap-cyan hover:underline">
        ← Back to Pools
      </Link>

      <div className="rounded-2xl border border-evap-border bg-white p-6 shadow-sm">
        <div className="flex items-start justify-between">
          <div>
            <h1 className="text-xl font-bold text-zinc-900">
              {position.tokenA} / {position.tokenB}
            </h1>
            <p className="text-[10px] font-mono text-zinc-400">{position.id}</p>
          </div>
          <span
            className={`rounded-full border px-2 py-0.5 text-xs font-medium ${
              position.state === "Active"
                ? "border-evap-green/30 bg-emerald-50 text-evap-green"
                : position.state === "Grace"
                  ? "border-evap-amber/30 bg-amber-50 text-evap-amber"
                  : "border-zinc-200 bg-zinc-100 text-zinc-500"
            }`}
          >
            {position.state}
          </span>
        </div>

        <div className="mt-4 grid grid-cols-2 gap-3 sm:grid-cols-4">
          <Stat label="Liquidity" value={position.liquidity.toLocaleString()} />
          <Stat label="Lower tick" value={position.lowerTick.toString()} />
          <Stat label="Upper tick" value={position.upperTick.toString()} />
          <Stat label="Fees earned" value={position.feesEarned.toLocaleString()} tone="text-evap-cyan" />
        </div>

        <div className="mt-4">
          <EnergyBar
            energy={position.energy}
            maxEnergy={position.maxEnergy}
            state={position.state}
          />
        </div>

        <div className="mt-4">
          <DecayForecast position={position} currentEpoch={currentEpoch} />
        </div>

        <div className="mt-4 flex flex-wrap gap-2">
          <RefreshButton position={position} />
          <button
            onClick={() => setShowPatronage(true)}
            className="rounded-lg border border-evap-purple/30 bg-purple-50 px-3 py-1.5 text-xs font-semibold text-evap-purple hover:bg-purple-100"
          >
            ✦ Add Patronage
          </button>
        </div>

        {position.state !== "Active" && (
          <div className="mt-4 rounded-lg border border-evap-amber/30 bg-amber-50 p-3 text-xs text-evap-amber">
            This position has decayed and is no longer earning fees. Refresh its energy
            reserve to bring it back into the active liquidity range.
          </div>
        )}
      </div>

      {showPatronage && (
        <PatronageDialog
          position={position}
          currentEpoch={currentEpoch}
          onClose={() => setShowPatronage(false)}
        />
      )}
    </div>
  );
}

function Stat({ label, value, tone }: { label: string; value: string; tone?: string }) {
  return (
    <div className="rounded-md bg-zinc-50 px-3 py-2">
      <p className={`text-sm font-bold ${tone ?? "text-zinc-700"}`}>{value}</p>
      <p className="text-[10px] text-zinc-400">{label}</p>
    </div>
  );
}
