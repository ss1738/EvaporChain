"use client";

import { useState } from "react";
import { motion } from "framer-motion";
import { Play, AlertTriangle, CheckCircle } from "lucide-react";

const API = "https://testnet.evaporchain.com/api";

type TxType = "transfer" | "create_object" | "refresh" | "call_contract";

interface SimResult {
  success: boolean;
  gas_used?: number;
  state_changes?: Array<{ key: string; before: string; after: string }>;
  error?: string;
  warnings?: string[];
}

export default function SimulatePage() {
  const [txType, setTxType] = useState<TxType>("transfer");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [amount, setAmount] = useState("");
  const [objectId, setObjectId] = useState("");
  const [energy, setEnergy] = useState("");
  const [halfLife, setHalfLife] = useState("");
  const [contractId, setContractId] = useState("");
  const [method, setMethod] = useState("");
  const [argsJson, setArgsJson] = useState("{}");
  const [result, setResult] = useState<SimResult | null>(null);
  const [running, setRunning] = useState(false);
  const [argsError, setArgsError] = useState<string | null>(null);

  const buildBody = (): Record<string, unknown> | null => {
    switch (txType) {
      case "transfer":
        return { tx_type: "transfer", from, to, amount: parseFloat(amount) || 0 };
      case "create_object":
        return { tx_type: "create_object", creator: from, object_id: objectId, energy: parseFloat(energy) || 0, half_life: parseInt(halfLife) || 0 };
      case "refresh":
        return { tx_type: "refresh", object_id: objectId, energy_deposit: parseFloat(energy) || 0 };
      case "call_contract": {
        try {
          const parsedArgs = JSON.parse(argsJson);
          setArgsError(null);
          return { tx_type: "call_contract", caller: from, contract_id: parseInt(contractId) || 0, method, args: parsedArgs };
        } catch {
          setArgsError("Invalid JSON in Args field");
          return null;
        }
      }
    }
  };

  const simulate = async () => {
    const body = buildBody();
    if (!body) return;
    setRunning(true);
    setResult(null);
    try {
      const res = await fetch(`${API}/tx/simulate`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      const data = await res.json();
      setResult(data);
    } catch (e) {
      setResult({ success: false, error: String(e) });
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h1 className="text-lg font-bold text-text-primary">Transaction Simulation</h1>
        <p className="text-sm text-text-muted mt-1">
          Dry-run a transaction against the current chain state without broadcasting. Gas usage and state changes are shown.
        </p>
      </div>

      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        className="bg-bg-card border border-white/5 rounded-xl p-5 space-y-4"
      >
        {/* Tx type selector */}
        <div>
          <label className="block text-xs text-text-muted mb-2">Transaction Type</label>
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-2">
            {(["transfer", "create_object", "refresh", "call_contract"] as TxType[]).map((t) => (
              <button
                key={t}
                onClick={() => { setTxType(t); setResult(null); }}
                className={`py-2 rounded-lg text-xs font-medium border transition ${
                  txType === t
                    ? "border-accent-cyan bg-accent-cyan/10 text-accent-cyan"
                    : "border-white/10 text-text-muted hover:border-white/30"
                }`}
              >
                {t.replace("_", " ")}
              </button>
            ))}
          </div>
        </div>

        {/* Common field: from/caller */}
        <div>
          <label className="block text-xs text-text-muted mb-1">
            {txType === "call_contract" ? "Caller Address" : txType === "create_object" ? "Creator Address" : "From Address"}
          </label>
          <input
            value={from}
            onChange={(e) => setFrom(e.target.value)}
            placeholder="0x…"
            className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-sm text-text-primary placeholder:text-text-muted font-mono focus:outline-none focus:border-accent-cyan/50 transition"
          />
        </div>

        {txType === "transfer" && (
          <>
            <div>
              <label className="block text-xs text-text-muted mb-1">To Address</label>
              <input
                value={to}
                onChange={(e) => setTo(e.target.value)}
                placeholder="0x…"
                className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-sm text-text-primary placeholder:text-text-muted font-mono focus:outline-none focus:border-accent-cyan/50 transition"
              />
            </div>
            <div>
              <label className="block text-xs text-text-muted mb-1">Amount (EVAP)</label>
              <input
                type="number"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                placeholder="0"
                className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-sm text-text-primary placeholder:text-text-muted focus:outline-none focus:border-accent-cyan/50 transition"
              />
            </div>
          </>
        )}

        {(txType === "create_object" || txType === "refresh") && (
          <>
            <div>
              <label className="block text-xs text-text-muted mb-1">Object ID</label>
              <input
                value={objectId}
                onChange={(e) => setObjectId(e.target.value)}
                placeholder="obj_…"
                className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-sm text-text-primary placeholder:text-text-muted font-mono focus:outline-none focus:border-accent-cyan/50 transition"
              />
            </div>
            <div>
              <label className="block text-xs text-text-muted mb-1">
                {txType === "refresh" ? "Energy Deposit (EVAP)" : "Initial Energy (EVAP)"}
              </label>
              <input
                type="number"
                value={energy}
                onChange={(e) => setEnergy(e.target.value)}
                placeholder="1000"
                className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-sm text-text-primary placeholder:text-text-muted focus:outline-none focus:border-accent-cyan/50 transition"
              />
            </div>
            {txType === "create_object" && (
              <div>
                <label className="block text-xs text-text-muted mb-1">Half-life (epochs)</label>
                <input
                  type="number"
                  value={halfLife}
                  onChange={(e) => setHalfLife(e.target.value)}
                  placeholder="1000"
                  className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-sm text-text-primary placeholder:text-text-muted focus:outline-none focus:border-accent-cyan/50 transition"
                />
              </div>
            )}
          </>
        )}

        {txType === "call_contract" && (
          <>
            <div>
              <label className="block text-xs text-text-muted mb-1">Contract ID</label>
              <input
                type="number"
                value={contractId}
                onChange={(e) => setContractId(e.target.value)}
                placeholder="1"
                className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-sm text-text-primary placeholder:text-text-muted focus:outline-none focus:border-accent-cyan/50 transition"
              />
            </div>
            <div>
              <label className="block text-xs text-text-muted mb-1">Method Name</label>
              <input
                value={method}
                onChange={(e) => setMethod(e.target.value)}
                placeholder="transfer"
                className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-sm text-text-primary placeholder:text-text-muted font-mono focus:outline-none focus:border-accent-cyan/50 transition"
              />
            </div>
            <div>
              <label className="block text-xs text-text-muted mb-1">Args (JSON)</label>
              <textarea
                value={argsJson}
                onChange={(e) => { setArgsJson(e.target.value); setArgsError(null); }}
                rows={4}
                className={`w-full px-3 py-2 rounded-lg bg-white/5 border text-sm text-text-primary placeholder:text-text-muted font-mono focus:outline-none transition resize-none ${
                  argsError ? "border-accent-red/50 focus:border-accent-red" : "border-white/10 focus:border-accent-cyan/50"
                }`}
              />
              {argsError && <p className="text-xs text-accent-red mt-1">{argsError}</p>}
            </div>
          </>
        )}

        <button
          onClick={simulate}
          disabled={running}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg bg-accent-cyan/10 border border-accent-cyan/30 text-sm text-accent-cyan hover:bg-accent-cyan/20 disabled:opacity-50 transition"
        >
          <Play size={12} />
          {running ? "Simulating…" : "Run Simulation"}
        </button>
      </motion.div>

      {result && (
        <motion.div
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          className={`rounded-xl border p-5 space-y-3 ${
            result.success
              ? "bg-accent-green/5 border-accent-green/20"
              : "bg-accent-red/5 border-accent-red/20"
          }`}
        >
          <div className="flex items-center gap-2">
            {result.success
              ? <CheckCircle size={16} className="text-accent-green" />
              : <AlertTriangle size={16} className="text-accent-red" />}
            <span className={`text-sm font-semibold ${result.success ? "text-accent-green" : "text-accent-red"}`}>
              {result.success ? "Simulation succeeded" : "Simulation failed"}
            </span>
          </div>

          {result.gas_used != null && (
            <p className="text-xs text-text-muted">Gas used: <span className="text-text-primary font-mono">{result.gas_used.toLocaleString()}</span></p>
          )}

          {result.error && (
            <p className="text-xs font-mono text-accent-red">{result.error}</p>
          )}

          {result.warnings && result.warnings.length > 0 && (
            <div>
              <p className="text-[10px] text-text-muted mb-1">Warnings:</p>
              {result.warnings.map((w, i) => (
                <p key={i} className="text-xs text-accent-amber font-mono">{w}</p>
              ))}
            </div>
          )}

          {result.state_changes && result.state_changes.length > 0 && (
            <div>
              <p className="text-[10px] text-text-muted mb-2">State changes:</p>
              <div className="space-y-2">
                {result.state_changes.map((sc, i) => (
                  <div key={i} className="text-xs font-mono bg-white/5 rounded-lg px-3 py-2">
                    <p className="text-text-muted">{sc.key}</p>
                    <p className="text-accent-red line-through opacity-60">{sc.before}</p>
                    <p className="text-accent-green">{sc.after}</p>
                  </div>
                ))}
              </div>
            </div>
          )}
        </motion.div>
      )}
    </div>
  );
}
