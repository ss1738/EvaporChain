import { Network } from "lucide-react";

// Placeholder ε-machine renderer. The chain currently exposes
//   POST /api/cslc_reconstruct  → { state_count, alphabet_size }
// — a *summary* of an ε-machine reconstructed from a symbol-count
// distribution, not the full transition graph. There is no
//   GET /api/cslc/states  or  GET /api/cslc/transitions
// so we can't render the actual graph without inventing one.
//
// This component shows the substrate explanation alongside whatever summary
// the node returned. When/if the node exposes a transition-graph endpoint,
// the right-hand pane should render an SVG force-directed graph of states
// (nodes) and transition probabilities (edges) — see Shalizi & Crutchfield
// "Computational Mechanics: Pattern and Prediction Out of Statistical
// Physics" (2001) for the visualisation conventions.
export function EpsilonMachineGraph({
  stateCount,
  alphabetSize,
  detail,
}: {
  stateCount?: number;
  alphabetSize?: number;
  detail?: string;
}) {
  return (
    <div className="rounded-xl border border-evap-border bg-white p-6">
      <div className="flex items-start gap-4">
        <div className="rounded-lg border border-evap-border bg-zinc-50 p-3">
          <Network className="h-6 w-6 text-evap-cyan" />
        </div>
        <div className="flex-1">
          <h2 className="text-sm font-semibold text-zinc-900">
            CSLC ε-machine — Causal-State Light Client
          </h2>
          <p className="mt-1 text-xs text-zinc-500">
            EvaporChain reconstructs a Shalizi-Crutchfield ε-machine over the
            symbol stream of compact headers, surfacing the minimal sufficient
            statistic (the causal state) for predicting future block content.
            Light clients sync against the ε-machine's state-evolution
            distribution rather than full block bodies.
          </p>
        </div>
      </div>

      <div className="mt-6 grid grid-cols-1 gap-4 sm:grid-cols-3">
        <Stat
          label="ε-states (μ)"
          value={stateCount != null ? stateCount.toString() : "—"}
          tone="text-evap-cyan"
        />
        <Stat
          label="Symbol alphabet"
          value={alphabetSize != null ? alphabetSize.toString() : "—"}
          tone="text-evap-purple"
        />
        <Stat label="Reconstruction" value={detail || "ok"} tone="text-zinc-700" />
      </div>

      <div className="mt-6 rounded-lg border border-dashed border-evap-border bg-zinc-50 p-4">
        <p className="text-xs font-semibold text-zinc-700">Transition graph</p>
        <p className="mt-1 text-[11px] text-zinc-500">
          The node currently exposes only the CSSR reconstruction summary
          (<code className="font-mono">POST /api/cslc_reconstruct</code>) — the
          full state/transition graph isn&apos;t served. Once
          <code className="font-mono"> GET /api/cslc/states</code> and{" "}
          <code className="font-mono">GET /api/cslc/transitions</code> are
          exposed this pane will render an interactive force-directed graph
          (states as nodes, conditional emission probabilities as edges).
        </p>
      </div>
    </div>
  );
}

function Stat({ label, value, tone }: { label: string; value: string; tone: string }) {
  return (
    <div className="rounded-lg border border-evap-border bg-white px-3 py-3">
      <p className={`font-mono text-base font-semibold ${tone}`}>{value}</p>
      <p className="text-[10px] text-zinc-400">{label}</p>
    </div>
  );
}
