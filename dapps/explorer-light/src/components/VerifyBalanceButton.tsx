"use client";

import { useState } from "react";
import { ShieldCheck, ShieldAlert, Loader2, Info } from "lucide-react";
import { explorerApi } from "@/lib/api";
import { verifyVerkleStructural, type VerkleVerificationResult } from "@/lib/verifyVerkle";

// Calls /api/light/state-proof/account/:addr and runs a structural verifier
// in-browser. Full cryptographic Pedersen-commitment opening is NOT done —
// the result is clearly labelled. Surfacing this honestly is more useful to
// auditors than fabricating a "verified" badge.
export function VerifyBalanceButton({ address }: { address: string }) {
  const [result, setResult] = useState<VerkleVerificationResult | null>(null);
  const [stateRoot, setStateRoot] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setLoading(true);
    setError(null);
    setResult(null);
    try {
      const proof = await explorerApi.accountProof(address);
      const v = verifyVerkleStructural(proof);
      setResult(v);
      setStateRoot(proof.state_root);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="rounded-lg border border-evap-border bg-white p-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <p className="text-xs font-semibold text-zinc-900">Verkle state proof</p>
          <p className="text-[10px] text-zinc-500">
            Pulls the inclusion proof for this address against the latest state
            root. Structural verification runs in your browser; full Pedersen
            opening lives in the prover crate.
          </p>
        </div>
        <button
          onClick={run}
          disabled={loading}
          className="rounded-md border border-evap-purple/40 bg-evap-purple/5 px-3 py-1.5 text-[11px] font-semibold text-evap-purple hover:bg-evap-purple/10 disabled:opacity-50"
        >
          {loading ? (
            <span className="inline-flex items-center gap-1">
              <Loader2 className="h-3 w-3 animate-spin" /> verifying
            </span>
          ) : (
            "Verify balance"
          )}
        </button>
      </div>

      {error && <p className="mt-2 text-[11px] text-evap-red">{error}</p>}
      {result && (
        <div
          className={`mt-3 flex items-start gap-2 rounded-md border p-2 text-[11px] ${
            result.level === "structurally_valid"
              ? "border-emerald-200 bg-emerald-50 text-evap-green"
              : result.level === "unverifiable_compressed"
                ? "border-amber-200 bg-amber-50 text-evap-amber"
                : "border-red-200 bg-red-50 text-evap-red"
          }`}
        >
          {result.level === "structurally_invalid" ? (
            <ShieldAlert className="mt-0.5 h-3.5 w-3.5 flex-shrink-0" />
          ) : result.level === "unverifiable_compressed" ? (
            <Info className="mt-0.5 h-3.5 w-3.5 flex-shrink-0" />
          ) : (
            <ShieldCheck className="mt-0.5 h-3.5 w-3.5 flex-shrink-0" />
          )}
          <div className="flex-1">
            <p className="font-semibold">{result.label}</p>
            <p className="font-mono text-[10px] opacity-70">
              state_root {result.state_root.slice(0, 24)}… · depth {result.depth}
              {result.exists ? " · key present" : " · key absent"}
            </p>
            {result.reason && <p className="text-[10px]">reason: {result.reason}</p>}
            {stateRoot && (
              <p className="text-[10px]">
                full crypto verification requires <code>evaporchain-proving</code> + the
                Pedersen opening circuit; this proof only attests structural integrity.
              </p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
