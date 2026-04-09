"use client";

import { useState, useEffect, useRef } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import { motion } from "framer-motion";
import { Copy, Check, Zap, Clock, Activity, User } from "lucide-react";

const API = "https://testnet.evaporchain.com/api";

interface ChainObject {
  id: string;
  name: string;
  owner: string;
  energy: number;
  max_energy: number;
  half_life: number;
  state: string;
  current_energy: number;
  decay_percentage: number;
  estimated_ghost_time: number;
  created_epoch: number;
  last_refreshed: number;
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
    case "Active": return "bg-accent-green/10 border-accent-green/20";
    case "Grace": return "bg-accent-amber/10 border-accent-amber/20";
    case "Ghost": return "bg-white/5 border-white/10";
    case "Risen": return "bg-accent-purple/10 border-accent-purple/20";
    default: return "bg-white/5 border-white/10";
  }
}

function stateGlow(state: string): string {
  switch (state) {
    case "Active": return "shadow-[0_0_40px_rgba(34,197,94,0.15)]";
    case "Grace": return "shadow-[0_0_40px_rgba(245,158,11,0.15)]";
    case "Risen": return "shadow-[0_0_40px_rgba(139,92,246,0.15)]";
    default: return "";
  }
}

function barGradient(state: string): string {
  switch (state) {
    case "Active": return "from-accent-green to-accent-cyan";
    case "Grace": return "from-accent-amber to-accent-red";
    case "Risen": return "from-accent-purple to-accent-cyan";
    default: return "from-text-muted to-text-muted";
  }
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      onClick={() => { navigator.clipboard.writeText(text); setCopied(true); setTimeout(() => setCopied(false), 2000); }}
      className="text-text-muted hover:text-accent-cyan transition-colors"
    >
      {copied ? <Check size={12} /> : <Copy size={12} />}
    </button>
  );
}

function formatCountdown(seconds: number): string {
  if (seconds <= 0) return "Evaporated";
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  if (d > 0) return `${d}d ${h}h ${m}m`;
  if (h > 0) return `${h}h ${m}m ${s}s`;
  return `${m}m ${s}s`;
}

export default function ObjectDetailPage() {
  const params = useParams();
  const objectId = params.id as string;
  const [obj, setObj] = useState<ChainObject | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [liveEnergy, setLiveEnergy] = useState<number>(0);
  const [countdown, setCountdown] = useState<number>(0);
  const animFrameRef = useRef<number>(0);

  // Fetch object data
  useEffect(() => {
    if (!objectId) return;
    fetch(`${API}/objects`)
      .then((r) => r.json())
      .then((objects: ChainObject[]) => {
        const found = objects.find((o) => o.id === objectId);
        if (found) {
          setObj(found);
          setLiveEnergy(found.current_energy);
          setCountdown(Math.max(0, (found.estimated_ghost_time - Date.now()) / 1000));
        } else {
          setError("Object not found");
        }
      })
      .catch(() => setError("Failed to fetch object"))
      .finally(() => setLoading(false));
  }, [objectId]);

  // Live decay simulation
  useEffect(() => {
    if (!obj || obj.state === "Ghost") return;

    const startEnergy = obj.current_energy;
    const startTime = Date.now();
    const ghostTime = obj.estimated_ghost_time;
    const totalDecayTime = ghostTime - startTime;

    const tick = () => {
      const elapsed = Date.now() - startTime;
      const remaining = Math.max(0, ghostTime - Date.now());

      // Simulate exponential decay: E(t) = E0 * 2^(-t/halfLife)
      // Approximate with linear interpolation for visual smoothness
      const progress = totalDecayTime > 0 ? Math.min(1, elapsed / totalDecayTime) : 1;
      const decayed = Math.max(0, Math.round(startEnergy * (1 - progress * 0.01)));

      setLiveEnergy(decayed);
      setCountdown(remaining / 1000);

      animFrameRef.current = requestAnimationFrame(tick);
    };

    animFrameRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(animFrameRef.current);
  }, [obj]);

  if (loading) {
    return (
      <div className="flex items-center justify-center py-32">
        <div className="w-8 h-8 border-2 border-accent-cyan/30 border-t-accent-cyan rounded-full animate-spin" />
      </div>
    );
  }

  if (error || !obj) {
    return (
      <div className="flex flex-col items-center justify-center py-32">
        <p className="text-sm text-text-muted">{error ?? "Object not found"}</p>
        <Link href="/explorer" className="text-sm text-accent-cyan mt-4 hover:underline">
          Back to Explorer
        </Link>
      </div>
    );
  }

  const pct = obj.max_energy > 0 ? (liveEnergy / obj.max_energy) * 100 : 0;

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.4 }}
      className="space-y-6"
    >
      {/* Title */}
      <div className="flex items-center gap-3">
        <div className={`w-10 h-10 rounded-xl ${stateBg(obj.state)} border flex items-center justify-center ${stateGlow(obj.state)}`}>
          <Zap size={18} className={stateColor(obj.state)} />
        </div>
        <div>
          <h1 className="text-lg font-semibold text-text-primary">{obj.name}</h1>
          <div className="flex items-center gap-2">
            <span className={`text-[10px] px-2 py-0.5 rounded-full font-medium ${stateBg(obj.state)} ${stateColor(obj.state)}`}>
              {obj.state}
            </span>
            <span className="text-xs text-text-muted font-mono">{obj.id.slice(0, 12)}...</span>
            <CopyButton text={obj.id} />
          </div>
        </div>
      </div>

      {/* Live Energy Ring */}
      <div className={`bg-bg-card border border-white/5 rounded-xl p-8 ${stateGlow(obj.state)}`}>
        <div className="flex flex-col items-center">
          {/* Circular energy indicator */}
          <div className="relative w-48 h-48 mb-6">
            <svg viewBox="0 0 200 200" className="w-full h-full -rotate-90">
              {/* Background track */}
              <circle
                cx="100" cy="100" r="85"
                fill="none"
                stroke="rgba(255,255,255,0.05)"
                strokeWidth="12"
              />
              {/* Energy arc */}
              <circle
                cx="100" cy="100" r="85"
                fill="none"
                stroke="url(#energyGrad)"
                strokeWidth="12"
                strokeLinecap="round"
                strokeDasharray={`${pct * 5.34} 534`}
                className="transition-all duration-1000"
              />
              <defs>
                <linearGradient id="energyGrad" x1="0%" y1="0%" x2="100%" y2="100%">
                  <stop offset="0%" stopColor={obj.state === "Active" ? "#22c55e" : obj.state === "Grace" ? "#f59e0b" : "#64748b"} />
                  <stop offset="100%" stopColor="#00f0ff" />
                </linearGradient>
              </defs>
            </svg>
            {/* Center text */}
            <div className="absolute inset-0 flex flex-col items-center justify-center">
              <span className="text-3xl font-bold text-text-primary">
                {Math.round(pct)}%
              </span>
              <span className="text-[10px] text-text-muted mt-0.5">Energy</span>
            </div>
          </div>

          {/* Energy numbers */}
          <div className="text-center mb-4">
            <p className="text-2xl font-bold gradient-text">
              {liveEnergy.toLocaleString()}
            </p>
            <p className="text-sm text-text-muted">
              of {obj.max_energy.toLocaleString()} max energy
            </p>
          </div>

          {/* Countdown */}
          {obj.state !== "Ghost" && (
            <div className="flex items-center gap-2 px-4 py-2 rounded-lg bg-white/5">
              <Clock size={14} className={stateColor(obj.state)} />
              <span className="text-sm text-text-primary font-mono">
                {formatCountdown(countdown)}
              </span>
              <span className="text-[10px] text-text-muted">until evaporation</span>
            </div>
          )}
          {obj.state === "Ghost" && (
            <div className="flex items-center gap-2 px-4 py-2 rounded-lg bg-white/5">
              <span className="text-sm text-text-muted">Evaporated — energy depleted</span>
            </div>
          )}
        </div>

        {/* Linear energy bar */}
        <div className="mt-6">
          <div className="h-2 rounded-full bg-white/5 overflow-hidden">
            <motion.div
              className={`h-full rounded-full bg-gradient-to-r ${barGradient(obj.state)}`}
              initial={{ width: 0 }}
              animate={{ width: `${pct}%` }}
              transition={{ duration: 1.5, ease: "easeOut" }}
            />
          </div>
        </div>
      </div>

      {/* Stats Grid */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        {[
          { label: "Half-Life", value: `${obj.half_life} epochs`, icon: Activity, color: "text-accent-cyan" },
          { label: "Decay Rate", value: `${obj.decay_percentage.toFixed(1)}%`, icon: Zap, color: "text-accent-amber" },
          { label: "Created", value: `Epoch ${obj.created_epoch}`, icon: Clock, color: "text-accent-purple" },
          { label: "Last Refresh", value: `Epoch ${obj.last_refreshed}`, icon: Zap, color: "text-accent-green" },
        ].map((stat, i) => (
          <motion.div
            key={stat.label}
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.3, delay: 0.5 + i * 0.05 }}
            className="bg-bg-card border border-white/5 rounded-xl p-4"
          >
            <div className="flex items-center gap-2 mb-2">
              <stat.icon size={12} className={stat.color} />
              <span className="text-[10px] text-text-muted uppercase tracking-wider">{stat.label}</span>
            </div>
            <p className="text-sm font-semibold text-text-primary">{stat.value}</p>
          </motion.div>
        ))}
      </div>

      {/* Owner */}
      <div className="bg-bg-card border border-white/5 rounded-xl p-5">
        <div className="flex items-center gap-2 mb-2">
          <User size={12} className="text-text-muted" />
          <span className="text-[10px] text-text-muted uppercase tracking-wider">Owner</span>
        </div>
        <Link
          href={`/explorer/address/${obj.owner}`}
          className="text-sm font-mono text-accent-cyan hover:underline break-all"
        >
          {obj.owner}
        </Link>
      </div>

      {/* Decay Formula */}
      <div className="bg-bg-card border border-white/5 rounded-xl p-5">
        <p className="text-[10px] text-text-muted uppercase tracking-wider mb-2">Decay Formula</p>
        <p className="text-sm text-text-secondary font-mono">
          E(t) = {obj.max_energy.toLocaleString()} × 2<sup>−t/{obj.half_life}</sup>
        </p>
        <p className="text-[10px] text-text-muted mt-2">
          Energy halves every {obj.half_life} epochs. Without refresh transactions,
          this object will enter Grace period and eventually evaporate to Ghost state.
        </p>
      </div>
    </motion.div>
  );
}
