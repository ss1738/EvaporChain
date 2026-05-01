import { useEffect } from "react";
import { useWallet } from "@/hooks/useWallet";
import { EnergyBar } from "./EnergyBar";
import { energyStatus } from "@/utils/format";
import { Header } from "./Header";
import { QuickRefresh } from "./QuickRefresh";

export function ObjectsScreen() {
  const { objects, refreshObjects, setView } = useWallet();

  useEffect(() => {
    refreshObjects();
    const interval = setInterval(refreshObjects, 5000);
    return () => clearInterval(interval);
  }, [refreshObjects]);

  return (
    <div className="flex flex-col h-full relative">
      <Header />
      <QuickRefresh />
      <div className="px-4 pt-4 pb-2 flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold text-zinc-100">Objects</h2>
          <p className="text-xs text-zinc-500">{objects.length} owned objects</p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => setView("patronage")}
            className="text-[10px] px-2 py-1 rounded border border-evap-cyan/30 text-evap-cyan hover:border-evap-cyan/60 transition"
            title="Patronage Covenants — pre-fund eviction immunity"
          >
            Patronage
          </button>
          <button
            onClick={() => setView("home")}
            className="text-xs text-zinc-500 hover:text-zinc-300"
          >
            ← Back
          </button>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto px-4 pb-4 space-y-2">
        {objects.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-12">
            <span className="text-3xl mb-3">◈</span>
            <p className="text-sm text-zinc-500">No objects yet</p>
            <p className="text-xs text-zinc-600 mt-1">Create one via the testnet explorer</p>
          </div>
        ) : (
          objects
            .sort((a, b) => a.current_energy - b.current_energy) // most urgent first
            .map(obj => (
              <div
                key={obj.id}
                className="px-3 py-3 rounded-lg bg-evap-surface border border-evap-border"
              >
                <div className="flex items-center justify-between mb-2">
                  <div>
                    <p className="text-xs font-semibold text-zinc-200">{obj.name || "Object"}</p>
                    <p className="text-[10px] text-zinc-500 font-mono">
                      {obj.id.slice(0, 8)}...{obj.id.slice(-6)}
                    </p>
                  </div>
                  <div className="flex items-center gap-1">
                    {obj.is_lad_typed === true && (
                      <span
                        className="text-[9px] font-semibold px-1.5 py-0.5 rounded-full bg-evap-purple/15 text-evap-purple border border-evap-purple/40"
                        title="LAD-VM substructural-resource type"
                      >
                        LAD
                      </span>
                    )}
                    <span className={`text-[10px] px-2 py-0.5 rounded-full ${
                      obj.state === "Active" ? "bg-evap-green/10 text-evap-green" :
                      obj.state === "Grace" ? "bg-evap-amber/10 text-evap-amber" :
                      obj.state === "Ghost" ? "bg-evap-ghost/10 text-evap-ghost" :
                      "bg-evap-purple/10 text-evap-purple"
                    }`}>
                      {obj.state}
                    </span>
                  </div>
                </div>

                <EnergyBar current={obj.current_energy} max={obj.max_energy} />

                <div className="flex items-center justify-between mt-2">
                  <span className="text-[10px] text-zinc-500">
                    Half-life: {obj.half_life} epochs
                  </span>
                  <span className="text-[10px] text-zinc-500">
                    {energyStatus(Math.round((obj.current_energy / obj.max_energy) * 100))}
                  </span>
                </div>
              </div>
            ))
        )}
      </div>
    </div>
  );
}
