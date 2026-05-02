import { useState } from "react";
import { useWallet } from "@/hooks/useWallet";
import { daVerify, type DaVerifyOutcome } from "@/utils/da-verify";
import { Header } from "./Header";

/**
 * Wallet-side DA light-client verification UI.
 *
 * One-click "is this node honest?" button. Pulls the active node URL from
 * wallet state, fetches the latest block from /api/blocks (so the user
 * doesn't have to type a number), runs the full chain-attestation +
 * cell-sampling pipeline, and renders the outcome — verified, faulty
 * peers, confidence, etc.
 *
 * Mirrors `evaporchain da verify`. Bug-for-bug parity with the Rust
 * verifier is the contract; the UI is just a thin shell around the same
 * verifyCellProof math the chain uses internally.
 */
export function DaVerifyScreen() {
  const { setView, nodeUrl } = useWallet();
  const [blockInput, setBlockInput] = useState<string>("");
  const [samples, setSamples] = useState<number>(16);
  const [skipChain, setSkipChain] = useState(false);
  const [running, setRunning] = useState(false);
  const [outcome, setOutcome] = useState<DaVerifyOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Extra peer URLs. Active node is always first; the textarea lets the
  // operator paste additional URLs (one per line) for multi-peer round-
  // robin sampling. Faulty-peer reports name the specific URL that
  // served bad cells, so two-or-more peers makes the signal sharper.
  const [extraPeersText, setExtraPeersText] = useState<string>("");

  // Parse + dedupe + drop the active node if the operator pasted it again.
  const extraPeers = extraPeersText
    .split(/\r?\n/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
    .filter((s, i, arr) => arr.indexOf(s) === i)
    .filter((s) => s !== nodeUrl);
  const nodes = [nodeUrl, ...extraPeers];

  const resolveBlockNumber = async (): Promise<number> => {
    if (blockInput.trim()) {
      const n = parseInt(blockInput.trim(), 10);
      if (Number.isFinite(n) && n >= 0) return n;
      throw new Error(`block must be a non-negative integer; got "${blockInput}"`);
    }
    // Fall back to the latest block on the active node so the user can
    // just hit "Verify" without thinking about block numbers.
    const res = await fetch(`${nodeUrl.replace(/\/$/, "")}/api/blocks?limit=1`);
    if (!res.ok) {
      throw new Error(`could not fetch latest block: ${res.status}`);
    }
    const list = (await res.json()) as Array<{ number: number }>;
    if (!Array.isArray(list) || list.length === 0) {
      throw new Error("node has no blocks yet");
    }
    return list[0].number;
  };

  const handleVerify = async () => {
    setRunning(true);
    setError(null);
    setOutcome(null);
    try {
      const block = await resolveBlockNumber();
      const result = await daVerify(nodes, block, {
        samples,
        threshold: 0.999,
        skipChainAttestation: skipChain,
      });
      setOutcome(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="flex flex-col h-full">
      <Header />
      <div className="px-4 pt-4 pb-2">
        <button
          onClick={() => setView("settings")}
          className="text-xs text-zinc-500 hover:text-zinc-300 mb-3"
        >
          ← Back
        </button>
        <h2 className="text-lg font-semibold text-zinc-200">Verify node honesty</h2>
        <p className="text-[11px] text-zinc-500 mt-1 leading-snug">
          Run a Celestia-style data-availability sampling round against the{" "}
          {nodes.length === 1 ? "active node" : `${nodes.length} configured nodes`}.
          Cross-checks the on-chain <code>data_root</code> against the served 2D
          header, then samples random cells and verifies each Merkle path
          locally. With multiple peers the sampler round-robins per cell —
          faulty-peer reports name the specific URL that served bad data.
        </p>
      </div>

      <div className="flex-1 px-4 space-y-3 overflow-y-auto">
        {/* Active node + extra peers */}
        <div>
          <label className="block text-xs font-medium text-zinc-400 mb-1">
            Active node <span className="text-zinc-600">(always sampled)</span>
          </label>
          <div className="rounded-lg bg-evap-surface border border-evap-border px-3 py-2 text-[11px] font-mono text-zinc-400 break-all">
            {nodeUrl}
          </div>
        </div>
        <div>
          <label className="block text-xs font-medium text-zinc-400 mb-1">
            Additional peers{" "}
            <span className="text-zinc-600">(one URL per line, optional)</span>
          </label>
          <textarea
            value={extraPeersText}
            onChange={(e) => setExtraPeersText(e.target.value)}
            placeholder={"http://node2:8080\nhttp://node3:8080"}
            rows={3}
            className="w-full rounded-lg border border-evap-border bg-evap-surface px-3 py-2 text-[11px] font-mono text-zinc-200 placeholder:text-zinc-600 focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan resize-y"
          />
          {extraPeers.length > 0 && (
            <p className="mt-1 text-[10px] text-zinc-500">
              Round-robining across {nodes.length} peers per cell.
            </p>
          )}
        </div>

        {/* Block selector */}
        <div>
          <label className="block text-xs font-medium text-zinc-400 mb-1">
            Block number
          </label>
          <input
            type="text"
            inputMode="numeric"
            placeholder="Leave empty for latest"
            value={blockInput}
            onChange={(e) => setBlockInput(e.target.value)}
            className="w-full rounded-lg border border-evap-border bg-evap-surface px-3 py-2 text-sm text-zinc-200 placeholder:text-zinc-500 focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
          />
        </div>

        {/* Samples slider */}
        <div>
          <label className="block text-xs font-medium text-zinc-400 mb-1">
            Cells to sample: <span className="text-zinc-200">{samples}</span>{" "}
            <span className="text-zinc-600">
              (confidence {(1 - Math.pow(2, -samples)).toFixed(6)})
            </span>
          </label>
          <input
            type="range"
            min={4}
            max={64}
            step={2}
            value={samples}
            onChange={(e) => setSamples(parseInt(e.target.value, 10))}
            className="w-full"
          />
        </div>

        {/* Skip-attestation toggle (advanced) */}
        <label className="flex items-center gap-2 text-[11px] text-zinc-500">
          <input
            type="checkbox"
            checked={skipChain}
            onChange={(e) => setSkipChain(e.target.checked)}
          />
          Skip on-chain <code>data_root</code> cross-check
          <span className="text-zinc-600">(advanced)</span>
        </label>

        {/* Run button */}
        <button
          onClick={handleVerify}
          disabled={running}
          className="w-full py-2.5 rounded-lg bg-evap-cyan/20 border border-evap-cyan/40 text-sm text-evap-cyan font-semibold hover:bg-evap-cyan/30 transition disabled:opacity-50"
        >
          {running ? "Verifying…" : "Verify node now"}
        </button>

        {/* Error surface */}
        {error && (
          <div className="rounded-lg border border-evap-red/30 bg-evap-red/10 p-3 text-xs text-evap-red whitespace-pre-wrap">
            {error}
          </div>
        )}

        {/* Outcome */}
        {outcome && (
          <div
            className={`rounded-lg border p-3 space-y-2 ${
              outcome.passes
                ? "border-evap-green/30 bg-evap-green/5"
                : "border-evap-red/30 bg-evap-red/5"
            }`}
          >
            <div className="flex items-center justify-between">
              <span
                className={`text-sm font-semibold ${
                  outcome.passes ? "text-evap-green" : "text-evap-red"
                }`}
              >
                {outcome.passes ? "✓ Honest" : "✗ Not honest"}
              </span>
              <span className="text-[10px] text-zinc-500">
                chain attest: {outcome.attestation.kind}
              </span>
            </div>

            <div className="grid grid-cols-2 gap-1 text-[11px] text-zinc-400">
              <span>Samples valid:</span>
              <span className="text-zinc-200 text-right">
                {outcome.samples_valid} / {outcome.samples_requested}
              </span>
              <span>Rows hit:</span>
              <span className="text-zinc-200 text-right">
                {outcome.unique_rows_hit}
              </span>
              <span>Cols hit:</span>
              <span className="text-zinc-200 text-right">
                {outcome.unique_cols_hit}
              </span>
              <span>Confidence:</span>
              <span className="text-zinc-200 text-right">
                {outcome.confidence.toFixed(6)}
              </span>
            </div>

            {outcome.attestation.kind === "mismatch" && (
              <div className="rounded border border-evap-red/40 bg-evap-red/10 p-2 text-[10px] text-evap-red">
                <div className="font-semibold mb-0.5">data_root mismatch</div>
                <div className="font-mono break-all">
                  on-chain: {outcome.attestation.on_chain.slice(0, 32)}…
                </div>
                <div className="font-mono break-all">
                  served: {outcome.attestation.served.slice(0, 32)}…
                </div>
              </div>
            )}

            {outcome.faulty_peers.length > 0 && (
              <div className="rounded border border-evap-red/40 bg-evap-red/10 p-2 text-[10px] text-evap-red space-y-0.5">
                <div className="font-semibold">
                  ⚠ {outcome.faulty_peers.length} peer(s) served bad cells:
                </div>
                {outcome.faulty_peers.map((p) => (
                  <div key={p.peer} className="font-mono break-all">
                    {p.peer}
                    <span className="text-evap-red/70"> — {p.reason}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        <p className="text-[10px] text-zinc-600 leading-snug pt-2">
          Verification math is byte-identical to the Rust chain code in{" "}
          <code>evaporchain-da</code>. The wallet rejects a block iff a Rust
          node would. See <code>extension/src/utils/da-verify.ts</code> for
          the audit trail.
        </p>
      </div>
    </div>
  );
}
