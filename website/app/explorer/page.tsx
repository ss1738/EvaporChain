"use client";

import { useState, useEffect, useCallback } from "react";
import Link from "next/link";
import { motion } from "framer-motion";
import {
  Activity,
  Layers,
  Clock,
  Users,
  Zap,
  Ghost,
  ArrowUpRight,
  ArrowDownLeft,
} from "lucide-react";

const API = "https://testnet.evaporchain.com/api";

interface ChainStatus {
  chain_name: string;
  version: string;
  block_height: number;
  epoch: number;
  active_objects: number;
  ghost_count: number;
  peer_count: number;
}

interface Transaction {
  hash: string;
  type: string;
  from: string;
  to: string;
  amount: string;
  timestamp: number;
}

interface ChainObject {
  id: string;
  name: string;
  owner: string;
  energy: number;
  max_energy: number;
  state: string;
  current_energy: number;
  decay_percentage: number;
}

function shortenAddr(addr: string): string {
  if (!addr || addr.length < 12) return addr;
  return `${addr.slice(0, 6)}...${addr.slice(-4)}`;
}

function timeAgo(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 60_000) return `${Math.max(1, Math.round(diff / 1000))}s ago`;
  if (diff < 3_600_000) return `${Math.round(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.round(diff / 3_600_000)}h ago`;
  return new Date(ts).toLocaleDateString();
}

function stateColor(state: string): string {
  switch (state) {
    case "Active": return "text-accent-green";
    case "Grace": return "text-accent-amber";
    case "Ghost": return "text-text-muted";
    case "Risen": return "text-accent-purple";
    default: return "text-text-muted";
  }
}

function stateBg(state: string): string {
  switch (state) {
    case "Active": return "bg-accent-green/10";
    case "Grace": return "bg-accent-amber/10";
    case "Ghost": return "bg-white/5";
    case "Risen": return "bg-accent-purple/10";
    default: return "bg-white/5";
  }
}

export default function ExplorerDashboard() {
  const [status, setStatus] = useState<ChainStatus | null>(null);
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [objects, setObjects] = useState<ChainObject[]>([]);
  const [loading, setLoading] = useState(true);

  const fetchData = useCallback(async () => {
    try {
      const [statusRes, txRes, objRes] = await Promise.allSettled([
        fetch(`${API}/status`).then((r) => r.json()),
        fetch(`${API}/transactions?limit=10`).then((r) => r.json()),
        fetch(`${API}/objects`).then((r) => r.json()),
      ]);
      if (statusRes.status === "fulfilled") setStatus(statusRes.value);
      if (txRes.status === "fulfilled") setTransactions(Array.isArray(txRes.value) ? txRes.value : []);
      if (objRes.status === "fulfilled") setObjects(Array.isArray(objRes.value) ? objRes.value : []);
    } catch {
      // retry next poll
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 8000);
    return () => clearInterval(interval);
  }, [fetchData]);

  const activeObjects = objects.filter((o) => o.state === "Active").length;
  const graceObjects = objects.filter((o) => o.state === "Grace").length;
  const ghostObjects = objects.filter((o) => o.state === "Ghost").length;

  const stats = [
    {
      label: "Block Height",
      value: status?.block_height?.toLocaleString() ?? "—",
      icon: Layers,
      color: "text-accent-cyan",
    },
    {
      label: "Current Epoch",
      value: status?.epoch?.toLocaleString() ?? "—",
      icon: Clock,
      color: "text-accent-purple",
    },
    {
      label: "Active Objects",
      value: status?.active_objects?.toLocaleString() ?? activeObjects.toLocaleString(),
      icon: Zap,
      color: "text-accent-green",
    },
    {
      label: "Evaporated",
      value: status?.ghost_count?.toLocaleString() ?? ghostObjects.toLocaleString(),
      icon: Ghost,
      color: "text-text-muted",
    },
    {
      label: "Peers",
      value: status?.peer_count?.toLocaleString() ?? "—",
      icon: Users,
      color: "text-accent-amber",
    },
    {
      label: "Network",
      value: status?.chain_name ?? "EvaporChain",
      icon: Activity,
      color: "text-accent-cyan",
    },
  ];

  if (loading) {
    return (
      <div className="flex items-center justify-center py-32">
        <div className="w-8 h-8 border-2 border-accent-cyan/30 border-t-accent-cyan rounded-full animate-spin" />
      </div>
    );
  }

  return (
    <div className="space-y-8">
      {/* Stats Grid */}
      <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-3">
        {stats.map((stat, i) => (
          <motion.div
            key={stat.label}
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.4, delay: i * 0.05 }}
            className="bg-bg-card border border-white/5 rounded-xl p-4"
          >
            <div className="flex items-center gap-2 mb-2">
              <stat.icon size={14} className={stat.color} />
              <span className="text-[10px] text-text-muted uppercase tracking-wider">
                {stat.label}
              </span>
            </div>
            <p className={`text-lg font-bold ${stat.color}`}>{stat.value}</p>
          </motion.div>
        ))}
      </div>

      <div className="grid lg:grid-cols-2 gap-6">
        {/* Recent Transactions */}
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.5, delay: 0.3 }}
          className="bg-bg-card border border-white/5 rounded-xl overflow-hidden"
        >
          <div className="px-5 py-4 border-b border-white/5 flex items-center justify-between">
            <h2 className="text-sm font-semibold text-text-primary">
              Recent Transactions
            </h2>
            <span className="text-[10px] text-text-muted">
              {transactions.length} shown
            </span>
          </div>

          {transactions.length === 0 ? (
            <div className="px-5 py-12 text-center">
              <p className="text-sm text-text-muted">No transactions yet</p>
            </div>
          ) : (
            <div className="divide-y divide-white/5">
              {transactions.slice(0, 8).map((tx) => (
                <Link
                  key={tx.hash}
                  href={`/explorer/tx/${tx.hash}`}
                  className="flex items-center justify-between px-5 py-3 hover:bg-white/[0.02] transition-colors"
                >
                  <div className="flex items-center gap-3 min-w-0">
                    <div className="w-8 h-8 rounded-lg bg-accent-cyan/10 flex items-center justify-center shrink-0">
                      {tx.type === "transfer" ? (
                        <ArrowUpRight size={14} className="text-accent-cyan" />
                      ) : (
                        <Zap size={14} className="text-accent-purple" />
                      )}
                    </div>
                    <div className="min-w-0">
                      <p className="text-sm text-text-primary font-mono truncate">
                        {shortenAddr(tx.hash)}
                      </p>
                      <p className="text-[10px] text-text-muted">
                        {shortenAddr(tx.from)} → {shortenAddr(tx.to)}
                      </p>
                    </div>
                  </div>
                  <div className="text-right shrink-0 ml-3">
                    <p className="text-sm font-medium text-text-primary">
                      {parseFloat(tx.amount).toLocaleString()} EVAP
                    </p>
                    <p className="text-[10px] text-text-muted">
                      {timeAgo(tx.timestamp)}
                    </p>
                  </div>
                </Link>
              ))}
            </div>
          )}
        </motion.div>

        {/* Active Objects with Decay */}
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.5, delay: 0.4 }}
          className="bg-bg-card border border-white/5 rounded-xl overflow-hidden"
        >
          <div className="px-5 py-4 border-b border-white/5 flex items-center justify-between">
            <h2 className="text-sm font-semibold text-text-primary">
              Objects — Live Decay
            </h2>
            <div className="flex items-center gap-3 text-[10px]">
              <span className="text-accent-green">{activeObjects} active</span>
              <span className="text-accent-amber">{graceObjects} grace</span>
              <span className="text-text-muted">{ghostObjects} ghost</span>
            </div>
          </div>

          {objects.length === 0 ? (
            <div className="px-5 py-12 text-center">
              <p className="text-sm text-text-muted">No objects on chain</p>
            </div>
          ) : (
            <div className="divide-y divide-white/5">
              {objects.slice(0, 8).map((obj) => {
                const pct = obj.max_energy > 0
                  ? Math.round((obj.current_energy / obj.max_energy) * 100)
                  : 0;
                const barColor =
                  obj.state === "Active"
                    ? "bg-accent-green"
                    : obj.state === "Grace"
                      ? "bg-accent-amber"
                      : "bg-text-muted";

                return (
                  <Link
                    key={obj.id}
                    href={`/explorer/object/${obj.id}`}
                    className="flex items-center justify-between px-5 py-3 hover:bg-white/[0.02] transition-colors"
                  >
                    <div className="flex items-center gap-3 min-w-0 flex-1">
                      <div
                        className={`w-8 h-8 rounded-lg ${stateBg(obj.state)} flex items-center justify-center shrink-0`}
                      >
                        <Zap size={14} className={stateColor(obj.state)} />
                      </div>
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <p className="text-sm text-text-primary truncate">
                            {obj.name}
                          </p>
                          <span
                            className={`text-[9px] px-1.5 py-0.5 rounded-full font-medium ${stateBg(obj.state)} ${stateColor(obj.state)}`}
                          >
                            {obj.state}
                          </span>
                        </div>
                        {/* Energy bar */}
                        <div className="mt-1.5 flex items-center gap-2">
                          <div className="flex-1 h-1 rounded-full bg-white/5 overflow-hidden">
                            <div
                              className={`h-full rounded-full ${barColor} transition-all duration-1000`}
                              style={{ width: `${pct}%` }}
                            />
                          </div>
                          <span className="text-[10px] text-text-muted w-8 text-right">
                            {pct}%
                          </span>
                        </div>
                      </div>
                    </div>
                    <div className="text-right shrink-0 ml-3">
                      <p className="text-xs font-mono text-text-secondary">
                        {obj.current_energy.toLocaleString()}
                      </p>
                      <p className="text-[10px] text-text-muted">
                        / {obj.max_energy.toLocaleString()}
                      </p>
                    </div>
                  </Link>
                );
              })}
            </div>
          )}
        </motion.div>
      </div>

      {/* Decay Overview — visual summary */}
      <motion.div
        initial={{ opacity: 0, y: 20 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.5, delay: 0.5 }}
        className="bg-bg-card border border-white/5 rounded-xl p-6"
      >
        <h2 className="text-sm font-semibold text-text-primary mb-4">
          Network Decay Distribution
        </h2>
        <div className="flex items-center gap-2 h-8 rounded-lg overflow-hidden">
          {activeObjects > 0 && (
            <div
              className="h-full bg-accent-green/80 rounded-l-lg transition-all duration-700 flex items-center justify-center"
              style={{ width: `${(activeObjects / Math.max(objects.length, 1)) * 100}%` }}
            >
              {activeObjects > 2 && (
                <span className="text-[10px] font-bold text-bg-primary">
                  {activeObjects} Active
                </span>
              )}
            </div>
          )}
          {graceObjects > 0 && (
            <div
              className="h-full bg-accent-amber/80 transition-all duration-700 flex items-center justify-center"
              style={{ width: `${(graceObjects / Math.max(objects.length, 1)) * 100}%` }}
            >
              {graceObjects > 2 && (
                <span className="text-[10px] font-bold text-bg-primary">
                  {graceObjects} Grace
                </span>
              )}
            </div>
          )}
          {ghostObjects > 0 && (
            <div
              className="h-full bg-white/10 rounded-r-lg transition-all duration-700 flex items-center justify-center"
              style={{ width: `${(ghostObjects / Math.max(objects.length, 1)) * 100}%` }}
            >
              {ghostObjects > 2 && (
                <span className="text-[10px] font-medium text-text-muted">
                  {ghostObjects} Ghost
                </span>
              )}
            </div>
          )}
          {objects.length === 0 && (
            <div className="h-full w-full bg-white/5 rounded-lg flex items-center justify-center">
              <span className="text-[10px] text-text-muted">No objects</span>
            </div>
          )}
        </div>
        <p className="text-[10px] text-text-muted mt-3">
          Objects lose energy over time following E(t) = E₀ × 2^(-t/halfLife). Without
          energy deposits, objects enter Grace period then evaporate to Ghost state.
        </p>
      </motion.div>
    </div>
  );
}
