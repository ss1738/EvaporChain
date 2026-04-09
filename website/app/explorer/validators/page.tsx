"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { motion } from "framer-motion";
import { Shield, TrendingUp, AlertTriangle } from "lucide-react";

const API = "https://testnet.evaporchain.com/api";

interface Validator {
  address: string;
  name: string;
  stake: number;
  commission: number;
  uptime: number;
  status: "active" | "jailed" | "inactive";
}

function statusConfig(status: string) {
  switch (status) {
    case "active":
      return { label: "Active", color: "text-accent-green", bg: "bg-accent-green/10", dot: "bg-accent-green" };
    case "jailed":
      return { label: "Jailed", color: "text-accent-red", bg: "bg-accent-red/10", dot: "bg-accent-red" };
    default:
      return { label: "Inactive", color: "text-text-muted", bg: "bg-white/5", dot: "bg-text-muted" };
  }
}

export default function ValidatorsPage() {
  const [validators, setValidators] = useState<Validator[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetch(`${API}/validators`)
      .then((r) => r.json())
      .then((data) => setValidators(Array.isArray(data) ? data : []))
      .catch(() => setValidators([]))
      .finally(() => setLoading(false));
  }, []);

  const activeCount = validators.filter((v) => v.status === "active").length;
  const totalStake = validators.reduce((s, v) => s + v.stake, 0);
  const avgUptime = validators.length > 0
    ? validators.reduce((s, v) => s + v.uptime, 0) / validators.length
    : 0;

  if (loading) {
    return (
      <div className="flex items-center justify-center py-32">
        <div className="w-8 h-8 border-2 border-accent-cyan/30 border-t-accent-cyan rounded-full animate-spin" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Stats */}
      <div className="grid grid-cols-3 gap-3">
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.4 }}
          className="bg-bg-card border border-white/5 rounded-xl p-4"
        >
          <div className="flex items-center gap-2 mb-2">
            <Shield size={14} className="text-accent-cyan" />
            <span className="text-[10px] text-text-muted uppercase tracking-wider">Active Validators</span>
          </div>
          <p className="text-2xl font-bold text-accent-cyan">{activeCount}</p>
          <p className="text-[10px] text-text-muted">of {validators.length} total</p>
        </motion.div>

        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.4, delay: 0.05 }}
          className="bg-bg-card border border-white/5 rounded-xl p-4"
        >
          <div className="flex items-center gap-2 mb-2">
            <TrendingUp size={14} className="text-accent-green" />
            <span className="text-[10px] text-text-muted uppercase tracking-wider">Total Stake</span>
          </div>
          <p className="text-2xl font-bold text-accent-green">
            {totalStake.toLocaleString()}
          </p>
          <p className="text-[10px] text-text-muted">EVAP staked</p>
        </motion.div>

        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.4, delay: 0.1 }}
          className="bg-bg-card border border-white/5 rounded-xl p-4"
        >
          <div className="flex items-center gap-2 mb-2">
            <Activity size={14} className="text-accent-purple" />
            <span className="text-[10px] text-text-muted uppercase tracking-wider">Avg Uptime</span>
          </div>
          <p className="text-2xl font-bold text-accent-purple">
            {avgUptime.toFixed(1)}%
          </p>
          <p className="text-[10px] text-text-muted">network reliability</p>
        </motion.div>
      </div>

      {/* Validator Table */}
      <motion.div
        initial={{ opacity: 0, y: 20 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.5, delay: 0.15 }}
        className="bg-bg-card border border-white/5 rounded-xl overflow-hidden"
      >
        {/* Header */}
        <div className="hidden sm:grid grid-cols-[40px_1fr_120px_100px_100px_90px] gap-3 px-5 py-3 border-b border-white/5 text-[10px] text-text-muted uppercase tracking-wider">
          <span>#</span>
          <span>Validator</span>
          <span className="text-right">Stake</span>
          <span className="text-right">Commission</span>
          <span className="text-right">Uptime</span>
          <span className="text-right">Status</span>
        </div>

        {validators.length === 0 ? (
          <div className="px-5 py-12 text-center">
            <p className="text-sm text-text-muted">No validators found</p>
          </div>
        ) : (
          <div className="divide-y divide-white/5">
            {validators.map((v, i) => {
              const cfg = statusConfig(v.status);
              return (
                <Link
                  key={v.address}
                  href={`/explorer/address/${v.address}`}
                  className="grid grid-cols-1 sm:grid-cols-[40px_1fr_120px_100px_100px_90px] gap-1 sm:gap-3 px-5 py-3 hover:bg-white/[0.02] transition-colors items-center"
                >
                  {/* Rank */}
                  <span className="text-sm font-medium text-text-muted hidden sm:block">
                    {i + 1}
                  </span>

                  {/* Name + Address */}
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <div className="w-6 h-6 rounded-lg gradient-bg flex items-center justify-center shrink-0">
                        <span className="text-[10px] font-bold text-bg-primary">
                          {v.name.charAt(0).toUpperCase()}
                        </span>
                      </div>
                      <div className="min-w-0">
                        <p className="text-sm font-medium text-text-primary truncate">
                          {v.name}
                        </p>
                        <p className="text-[10px] font-mono text-text-muted truncate">
                          {v.address.slice(0, 8)}...{v.address.slice(-6)}
                        </p>
                      </div>
                    </div>
                  </div>

                  {/* Stake */}
                  <p className="text-sm text-text-primary text-right hidden sm:block">
                    {v.stake.toLocaleString()}
                  </p>

                  {/* Commission */}
                  <p className="text-sm text-text-secondary text-right hidden sm:block">
                    {v.commission}%
                  </p>

                  {/* Uptime */}
                  <div className="hidden sm:flex items-center justify-end gap-2">
                    <div className="w-16 h-1.5 rounded-full bg-white/5 overflow-hidden">
                      <div
                        className="h-full rounded-full bg-accent-green"
                        style={{ width: `${v.uptime}%` }}
                      />
                    </div>
                    <span className="text-xs text-text-secondary w-10 text-right">
                      {v.uptime}%
                    </span>
                  </div>

                  {/* Status */}
                  <div className="hidden sm:flex justify-end">
                    <span className={`text-[10px] px-2 py-0.5 rounded-full font-medium ${cfg.bg} ${cfg.color} flex items-center gap-1`}>
                      <span className={`w-1.5 h-1.5 rounded-full ${cfg.dot}`} />
                      {cfg.label}
                    </span>
                  </div>

                  {/* Mobile summary */}
                  <div className="sm:hidden flex items-center gap-3 text-[10px] text-text-muted mt-1">
                    <span>{v.stake.toLocaleString()} EVAP</span>
                    <span>{v.commission}% fee</span>
                    <span>{v.uptime}% uptime</span>
                    <span className={`${cfg.color}`}>{cfg.label}</span>
                  </div>
                </Link>
              );
            })}
          </div>
        )}
      </motion.div>
    </div>
  );
}

function Activity({ size, className }: { size: number; className?: string }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      <polyline points="22 12 18 12 15 21 9 3 6 12 2 12" />
    </svg>
  );
}
