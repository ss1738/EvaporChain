import { useState } from "react";

interface QuantumBadgeProps {
  variant?: "full" | "inline";
}

const competitors = [
  { name: "EvaporChain", algo: "ML-DSA (FIPS 204)", safe: true },
  { name: "MetaMask", algo: "ECDSA (secp256k1)", safe: false },
  { name: "Phantom", algo: "ECDSA (Ed25519)", safe: false },
  { name: "Trust Wallet", algo: "ECDSA (secp256k1)", safe: false },
];

export function QuantumBadge({ variant = "full" }: QuantumBadgeProps) {
  const [expanded, setExpanded] = useState(false);

  if (variant === "inline") {
    return (
      <button
        onClick={() => setExpanded(!expanded)}
        className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium text-evap-cyan bg-evap-cyan/10 border border-evap-cyan/20 hover:border-evap-cyan/40 transition"
      >
        <span>🛡️</span>
        <span>Quantum-Safe</span>
      </button>
    );
  }

  return (
    <div className="mx-4 mb-3">
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full rounded-xl p-[1px] transition-all duration-300"
        style={{
          background: "linear-gradient(135deg, #00f0ff, #8b5cf6)",
        }}
      >
        <div className="w-full rounded-[11px] bg-evap-surface px-4 py-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <span className="text-lg">🛡️</span>
              <div className="text-left">
                <p className="text-xs font-semibold text-zinc-200">
                  Post-Quantum Secured
                </p>
                <p className="text-[10px] text-zinc-500">
                  ML-DSA (FIPS 204)
                </p>
              </div>
            </div>
            <span
              className={`text-zinc-500 text-xs transition-transform duration-200 ${
                expanded ? "rotate-180" : ""
              }`}
            >
              ▼
            </span>
          </div>
        </div>
      </button>

      {expanded && (
        <div
          className="mt-1 rounded-xl p-[1px]"
          style={{
            background: "linear-gradient(135deg, #00f0ff33, #8b5cf633)",
          }}
        >
          <div className="rounded-[11px] bg-evap-surface px-4 py-4 space-y-4">
            {/* Explanation bullets */}
            <div className="space-y-2">
              <ExplainRow
                icon="✓"
                iconColor="text-evap-green"
                text="Your keys use ML-DSA (FIPS 204) — safe against quantum computers"
              />
              <ExplainRow
                icon="✗"
                iconColor="text-evap-red"
                text="MetaMask uses ECDSA — vulnerable to quantum attacks"
              />
              <ExplainRow
                icon="✗"
                iconColor="text-evap-red"
                text="Trust Wallet uses ECDSA — vulnerable to quantum attacks"
              />
              <ExplainRow
                icon="★"
                iconColor="text-evap-cyan"
                text="EvaporChain is the only L1 with built-in quantum resistance"
              />
            </div>

            {/* Comparison table */}
            <div className="rounded-lg border border-evap-border overflow-hidden">
              <div className="grid grid-cols-3 bg-evap-bg px-3 py-2 border-b border-evap-border">
                <span className="text-[10px] font-semibold text-zinc-400">
                  Wallet
                </span>
                <span className="text-[10px] font-semibold text-zinc-400">
                  Algorithm
                </span>
                <span className="text-[10px] font-semibold text-zinc-400 text-right">
                  Quantum-Safe
                </span>
              </div>
              {competitors.map((c) => (
                <div
                  key={c.name}
                  className={`grid grid-cols-3 px-3 py-2 border-b border-evap-border last:border-b-0 ${
                    c.safe ? "bg-evap-green/5" : ""
                  }`}
                >
                  <span
                    className={`text-[10px] font-medium ${
                      c.safe ? "text-evap-cyan" : "text-zinc-400"
                    }`}
                  >
                    {c.name}
                  </span>
                  <span className="text-[10px] text-zinc-500">{c.algo}</span>
                  <span
                    className={`text-[10px] text-right font-semibold ${
                      c.safe ? "text-evap-green" : "text-evap-red"
                    }`}
                  >
                    {c.safe ? "✓ Safe" : "✗ Vulnerable"}
                  </span>
                </div>
              ))}
            </div>

            {/* Learn more */}
            <button className="w-full text-center text-[10px] text-evap-purple hover:text-evap-cyan transition py-1">
              Learn More →
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function ExplainRow({
  icon,
  iconColor,
  text,
}: {
  icon: string;
  iconColor: string;
  text: string;
}) {
  return (
    <div className="flex items-start gap-2">
      <span className={`text-xs font-bold ${iconColor} mt-0.5 shrink-0`}>
        {icon}
      </span>
      <p className="text-[11px] text-zinc-300 leading-tight">{text}</p>
    </div>
  );
}
