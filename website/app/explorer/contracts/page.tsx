"use client";

import { useState, useEffect } from "react";
import { motion } from "framer-motion";
import { Code, Play, ChevronDown, ChevronRight, Zap } from "lucide-react";

const API = "https://testnet.evaporchain.com/api";

interface ContractInfo {
  id: number;
  template: string;
  owner: string;
  state: "Active" | "Grace" | "Ghost";
  energy: number;
  methods: string[];
  deployed_at: number;
}

interface AbiMethod {
  name: string;
  inputs: Array<{ name: string; type: string }>;
  outputs: Array<{ type: string }>;
  mutates_state: boolean;
}

interface CallResult {
  success: boolean;
  output?: unknown;
  error?: string;
  gas_used?: number;
}

export default function ContractsPage() {
  const [contracts, setContracts] = useState<ContractInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<ContractInfo | null>(null);
  const [abi, setAbi] = useState<AbiMethod[]>([]);
  const [loadingAbi, setLoadingAbi] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [args, setArgs] = useState<Record<string, Record<string, string>>>({});
  const [results, setResults] = useState<Record<string, CallResult>>({});
  const [calling, setCalling] = useState<string | null>(null);
  const [callerAddress, setCallerAddress] = useState("");

  useEffect(() => {
    fetch(`${API}/contracts`)
      .then((r) => r.json())
      .then((d) => setContracts(Array.isArray(d.contracts) ? d.contracts : []))
      .catch(() => setContracts([]))
      .finally(() => setLoading(false));
  }, []);

  const selectContract = async (c: ContractInfo) => {
    setSelected(c);
    setAbi([]);
    setResults({});
    setExpanded(null);
    setLoadingAbi(true);
    try {
      const res = await fetch(`${API}/contracts/${c.id}/abi`);
      const data = await res.json();
      setAbi(Array.isArray(data.methods) ? data.methods : []);
    } catch {
      setAbi([]);
    } finally {
      setLoadingAbi(false);
    }
  };

  const callMethod = async (method: AbiMethod) => {
    if (!selected) return;
    const methodArgs = args[method.name] ?? {};
    setCalling(method.name);
    try {
      const body: Record<string, unknown> = {
        contract_id: selected.id,
        method: method.name,
        args: Object.fromEntries(
          method.inputs.map((inp) => [inp.name, methodArgs[inp.name] ?? ""])
        ),
        caller: callerAddress || "0x0000000000000000",
        epoch: 0,
      };
      const res = await fetch(`${API}/contracts/simulate`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      const data = await res.json();
      setResults((prev) => ({ ...prev, [method.name]: data }));
    } catch (e) {
      setResults((prev) => ({
        ...prev,
        [method.name]: { success: false, error: String(e) },
      }));
    } finally {
      setCalling(null);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-lg font-bold text-text-primary">Contract Playground</h1>
        <p className="text-sm text-text-muted mt-1">
          Browse deployed EvaporScript contracts, inspect their ABI, and simulate method calls.
        </p>
      </div>

      <div className="grid lg:grid-cols-3 gap-6">
        {/* Contract list */}
        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          className="bg-bg-card border border-white/5 rounded-xl overflow-hidden"
        >
          <div className="px-5 py-4 border-b border-white/5">
            <h2 className="text-sm font-semibold text-text-primary">Deployed Contracts</h2>
          </div>
          {loading ? (
            <div className="px-5 py-8 text-center text-sm text-text-muted">Loading…</div>
          ) : contracts.length === 0 ? (
            <div className="px-5 py-8 text-center text-sm text-text-muted">No contracts deployed yet</div>
          ) : (
            <div className="divide-y divide-white/5">
              {contracts.map((c) => (
                <button
                  key={c.id}
                  onClick={() => selectContract(c)}
                  className={`w-full text-left px-5 py-3 transition ${
                    selected?.id === c.id ? "bg-accent-cyan/10" : "hover:bg-white/[0.02]"
                  }`}
                >
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <Code size={12} className="text-accent-cyan shrink-0" />
                      <span className="text-sm text-text-primary font-mono">#{c.id}</span>
                    </div>
                    <span className={`text-[9px] px-1.5 py-0.5 rounded-full font-medium ${
                      c.state === "Active" ? "bg-accent-green/10 text-accent-green" :
                      c.state === "Grace" ? "bg-accent-amber/10 text-accent-amber" :
                      "bg-white/5 text-text-muted"
                    }`}>
                      {c.state}
                    </span>
                  </div>
                  <p className="text-xs text-accent-cyan mt-1">{c.template}</p>
                  <p className="text-[10px] text-text-muted mt-0.5 font-mono truncate">{c.owner}</p>
                </button>
              ))}
            </div>
          )}
        </motion.div>

        {/* ABI + call panel */}
        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.1 }}
          className="lg:col-span-2 bg-bg-card border border-white/5 rounded-xl overflow-hidden"
        >
          {!selected ? (
            <div className="flex flex-col items-center justify-center h-full py-24 text-center px-8">
              <Code size={32} className="text-text-muted mb-4" />
              <p className="text-sm text-text-muted">Select a contract to inspect its ABI and call methods</p>
            </div>
          ) : (
            <>
              <div className="px-5 py-4 border-b border-white/5 flex items-center justify-between">
                <div>
                  <p className="text-sm font-semibold text-text-primary font-mono">Contract #{selected.id} — {selected.template}</p>
                  <p className="text-[10px] text-text-muted font-mono mt-0.5">{selected.owner}</p>
                </div>
                <div className="flex items-center gap-2 text-xs text-text-muted">
                  <Zap size={12} className="text-accent-green" />
                  {selected.energy.toLocaleString()} EVAP
                </div>
              </div>

              <div className="px-5 py-3 border-b border-white/5">
                <input
                  type="text"
                  value={callerAddress}
                  onChange={(e) => setCallerAddress(e.target.value)}
                  placeholder="Caller address (optional, for simulations)"
                  className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-xs text-text-primary placeholder:text-text-muted font-mono focus:outline-none focus:border-accent-cyan/50 transition"
                />
              </div>

              {loadingAbi ? (
                <div className="px-5 py-8 text-center text-sm text-text-muted">Loading ABI…</div>
              ) : abi.length === 0 ? (
                <div className="px-5 py-8 text-center text-sm text-text-muted">No methods in ABI</div>
              ) : (
                <div className="divide-y divide-white/5">
                  {abi.map((method) => (
                    <div key={method.name}>
                      <button
                        onClick={() => setExpanded(expanded === method.name ? null : method.name)}
                        className="w-full flex items-center justify-between px-5 py-3 hover:bg-white/[0.02] transition"
                      >
                        <div className="flex items-center gap-2">
                          {expanded === method.name
                            ? <ChevronDown size={12} className="text-text-muted" />
                            : <ChevronRight size={12} className="text-text-muted" />}
                          <span className="text-sm font-mono text-accent-cyan">{method.name}</span>
                          {method.mutates_state && (
                            <span className="text-[9px] px-1.5 py-0.5 rounded bg-accent-purple/10 text-accent-purple font-medium">
                              write
                            </span>
                          )}
                        </div>
                        <span className="text-[10px] text-text-muted">
                          {method.inputs.length} input{method.inputs.length !== 1 ? "s" : ""}
                        </span>
                      </button>

                      {expanded === method.name && (
                        <div className="px-5 pb-4 space-y-3">
                          {method.inputs.map((inp) => (
                            <div key={inp.name}>
                              <label className="block text-[10px] text-text-muted mb-1 font-mono">
                                {inp.name}: <span className="text-accent-amber">{inp.type}</span>
                              </label>
                              <input
                                type="text"
                                value={args[method.name]?.[inp.name] ?? ""}
                                onChange={(e) =>
                                  setArgs((prev) => ({
                                    ...prev,
                                    [method.name]: {
                                      ...prev[method.name],
                                      [inp.name]: e.target.value,
                                    },
                                  }))
                                }
                                placeholder={`Enter ${inp.type}`}
                                className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-xs text-text-primary placeholder:text-text-muted font-mono focus:outline-none focus:border-accent-cyan/50 transition"
                              />
                            </div>
                          ))}

                          <button
                            onClick={() => callMethod(method)}
                            disabled={calling === method.name}
                            className="flex items-center gap-2 px-4 py-2 rounded-lg bg-accent-cyan/10 border border-accent-cyan/30 text-xs text-accent-cyan hover:bg-accent-cyan/20 disabled:opacity-50 transition"
                          >
                            <Play size={10} />
                            {calling === method.name ? "Simulating…" : "Simulate"}
                          </button>

                          {results[method.name] && (
                            <div className={`p-3 rounded-lg text-xs font-mono ${
                              results[method.name].success
                                ? "bg-accent-green/5 border border-accent-green/20 text-accent-green"
                                : "bg-accent-red/5 border border-accent-red/20 text-accent-red"
                            }`}>
                              {results[method.name].success ? (
                                <>
                                  <p className="text-[10px] text-text-muted mb-1">Output:</p>
                                  <pre className="whitespace-pre-wrap break-all">
                                    {JSON.stringify(results[method.name].output, null, 2)}
                                  </pre>
                                  {results[method.name].gas_used != null && (
                                    <p className="text-[10px] text-text-muted mt-2">
                                      Gas used: {results[method.name].gas_used?.toLocaleString()}
                                    </p>
                                  )}
                                </>
                              ) : (
                                <p>{results[method.name].error}</p>
                              )}
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </>
          )}
        </motion.div>
      </div>
    </div>
  );
}
