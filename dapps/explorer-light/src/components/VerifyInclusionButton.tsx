"use client";

import { useState } from "react";
import { ShieldCheck, ShieldAlert, Loader2 } from "lucide-react";
import { explorerApi } from "@/lib/api";
import { verifyTxInclusion, type MerkleVerificationResult } from "@/lib/verifyMerkle";

// Pulls /api/light/tx-proof/:block/:tx_index and recomputes the Merkle root
// in-browser via blake3. Anchors the result against the *trusted compact
// header*'s tx_merkle_root if we already have it cached — so the UI surfaces
// "verified against header X" rather than "verified the indexer's word".
export function VerifyInclusionButton({
  blockNumber,
  txIndex,
  headerTxMerkleRoot,
}: {
  blockNumber: number;
  txIndex: number;
  headerTxMerkleRoot?: string;
}) {
  const [result, setResult] = useState<MerkleVerificationResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setLoading(true);
    setError(null);
    setResult(null);
    try {
      const r = await explorerApi.txProof(blockNumber, txIndex);
      const v = verifyTxInclusion(r.proof, headerTxMerkleRoot);
      setResult(v);
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
          <p className="text-xs font-semibold text-zinc-900">Inclusion proof</p>
          <p className="text-[10px] text-zinc-500">
            Pulls the Merkle path from the node and recomputes the root in your
            browser using blake3. The indexer is never trusted.
          </p>
        </div>
        <button
          onClick={run}
          disabled={loading || txIndex < 0}
          className="rounded-md border border-evap-cyan/40 bg-evap-cyan/5 px-3 py-1.5 text-[11px] font-semibold text-evap-cyan hover:bg-evap-cyan/10 disabled:opacity-50"
        >
          {loading ? (
            <span className="inline-flex items-center gap-1">
              <Loader2 className="h-3 w-3 animate-spin" /> verifying
            </span>
          ) : (
            "Verify inclusion"
          )}
        </button>
      </div>

      {error && <p className="mt-2 text-[11px] text-evap-red">{error}</p>}
      {result && (
        <div
          className={`mt-3 flex items-start gap-2 rounded-md border p-2 text-[11px] ${
            result.valid
              ? "border-emerald-200 bg-emerald-50 text-evap-green"
              : "border-red-200 bg-red-50 text-evap-red"
          }`}
        >
          {result.valid ? (
            <ShieldCheck className="mt-0.5 h-3.5 w-3.5 flex-shrink-0" />
          ) : (
            <ShieldAlert className="mt-0.5 h-3.5 w-3.5 flex-shrink-0" />
          )}
          <div className="flex-1">
            <p className="font-semibold">
              {result.valid ? "Path verified locally" : "Path FAILED verification"}
            </p>
            <p className="font-mono text-[10px] opacity-70">
              root {result.computed_root.slice(0, 24)}…
            </p>
            {result.matches_header_root === true && (
              <p className="text-[10px]">anchored to compact header tx_merkle_root</p>
            )}
            {result.matches_header_root === false && (
              <p className="text-[10px] text-evap-red">
                proof root != trusted header root — DO NOT trust this proof
              </p>
            )}
            {result.reason && <p className="text-[10px]">reason: {result.reason}</p>}
          </div>
        </div>
      )}
    </div>
  );
}
