"use client";

import { useState, useEffect, useCallback } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import { motion } from "framer-motion";
import {
  Copy,
  Check,
  Wallet,
  ArrowUpRight,
  ArrowDownLeft,
  Zap,
  Image,
} from "lucide-react";

const API = "https://testnet.evaporchain.com/api";

interface Balance {
  address: string;
  balance: number;
  nonce: number;
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
  energy: number;
  max_energy: number;
  current_energy: number;
  state: string;
  decay_percentage: number;
}

interface NFT {
  id: string;
  name: string;
  collection_name: string;
  energy: number;
  max_energy: number;
  current_energy: number;
  state: string;
  image_uri?: string;
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

type Tab = "transactions" | "objects" | "nfts";

export default function AddressDetailPage() {
  const params = useParams();
  const address = params.address as string;
  const [balance, setBalance] = useState<Balance | null>(null);
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [objects, setObjects] = useState<ChainObject[]>([]);
  const [nfts, setNfts] = useState<NFT[]>([]);
  const [tab, setTab] = useState<Tab>("transactions");
  const [loading, setLoading] = useState(true);

  const fetchData = useCallback(async () => {
    try {
      const [balRes, txRes, objRes, nftRes] = await Promise.allSettled([
        fetch(`${API}/address/${address}`).then((r) => r.json()),
        fetch(`${API}/transactions?address=${address}&limit=20`).then((r) => r.json()),
        fetch(`${API}/objects?owner=${address}`).then((r) => r.json()),
        fetch(`${API}/nfts?owner=${address}`).then((r) => r.json()),
      ]);
      if (balRes.status === "fulfilled") setBalance(balRes.value);
      if (txRes.status === "fulfilled") setTransactions(Array.isArray(txRes.value) ? txRes.value : []);
      if (objRes.status === "fulfilled") setObjects(Array.isArray(objRes.value) ? objRes.value : []);
      if (nftRes.status === "fulfilled") setNfts(Array.isArray(nftRes.value) ? nftRes.value : []);
    } catch {
      // retry
    } finally {
      setLoading(false);
    }
  }, [address]);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  if (loading) {
    return (
      <div className="flex items-center justify-center py-32">
        <div className="w-8 h-8 border-2 border-accent-cyan/30 border-t-accent-cyan rounded-full animate-spin" />
      </div>
    );
  }

  const tabs: { key: Tab; label: string; count: number }[] = [
    { key: "transactions", label: "Transactions", count: transactions.length },
    { key: "objects", label: "Objects", count: objects.length },
    { key: "nfts", label: "NFTs", count: nfts.length },
  ];

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.4 }}
      className="space-y-6"
    >
      {/* Address Header */}
      <div className="bg-bg-card border border-white/5 rounded-xl p-6">
        <div className="flex items-start justify-between flex-wrap gap-4">
          <div>
            <div className="flex items-center gap-2 mb-1">
              <Wallet size={14} className="text-accent-cyan" />
              <span className="text-[10px] text-text-muted uppercase tracking-wider">Address</span>
            </div>
            <div className="flex items-center gap-2">
              <p className="text-sm font-mono text-text-primary break-all">{address}</p>
              <CopyButton text={address} />
            </div>
          </div>
          <div className="text-right">
            <p className="text-[10px] text-text-muted uppercase tracking-wider mb-1">Balance</p>
            <p className="text-2xl font-bold gradient-text">
              {balance ? balance.balance.toLocaleString() : "0"} EVAP
            </p>
            {balance && (
              <p className="text-xs text-text-muted mt-0.5">Nonce: {balance.nonce}</p>
            )}
          </div>
        </div>

        {/* Quick stats */}
        <div className="mt-4 flex gap-4">
          <div className="px-3 py-2 rounded-lg bg-white/5">
            <p className="text-lg font-bold text-accent-cyan">{transactions.length}</p>
            <p className="text-[10px] text-text-muted">Transactions</p>
          </div>
          <div className="px-3 py-2 rounded-lg bg-white/5">
            <p className="text-lg font-bold text-accent-green">{objects.length}</p>
            <p className="text-[10px] text-text-muted">Objects</p>
          </div>
          <div className="px-3 py-2 rounded-lg bg-white/5">
            <p className="text-lg font-bold text-accent-purple">{nfts.length}</p>
            <p className="text-[10px] text-text-muted">NFTs</p>
          </div>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex gap-1 bg-bg-card border border-white/5 rounded-xl p-1">
        {tabs.map((t) => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            className={`flex-1 px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
              tab === t.key
                ? "bg-accent-cyan/10 text-accent-cyan"
                : "text-text-muted hover:text-text-secondary"
            }`}
          >
            {t.label}
            <span className="ml-1.5 text-[10px] opacity-60">({t.count})</span>
          </button>
        ))}
      </div>

      {/* Tab Content */}
      <div className="bg-bg-card border border-white/5 rounded-xl overflow-hidden">
        {tab === "transactions" && (
          transactions.length === 0 ? (
            <div className="px-5 py-12 text-center">
              <p className="text-sm text-text-muted">No transactions found</p>
            </div>
          ) : (
            <div className="divide-y divide-white/5">
              {transactions.map((tx) => {
                const isSent = tx.from.toLowerCase() === address.toLowerCase();
                return (
                  <Link
                    key={tx.hash}
                    href={`/explorer/tx/${tx.hash}`}
                    className="flex items-center justify-between px-5 py-3 hover:bg-white/[0.02] transition-colors"
                  >
                    <div className="flex items-center gap-3 min-w-0">
                      <div className={`w-8 h-8 rounded-lg flex items-center justify-center ${
                        isSent ? "bg-accent-red/10" : "bg-accent-green/10"
                      }`}>
                        {isSent
                          ? <ArrowUpRight size={14} className="text-accent-red" />
                          : <ArrowDownLeft size={14} className="text-accent-green" />
                        }
                      </div>
                      <div className="min-w-0">
                        <p className="text-sm font-mono text-text-primary truncate">
                          {shortenAddr(tx.hash)}
                        </p>
                        <p className="text-[10px] text-text-muted">
                          {isSent ? "To " : "From "}
                          {shortenAddr(isSent ? tx.to : tx.from)}
                        </p>
                      </div>
                    </div>
                    <div className="text-right shrink-0 ml-3">
                      <p className={`text-sm font-medium ${isSent ? "text-accent-red" : "text-accent-green"}`}>
                        {isSent ? "-" : "+"}{parseFloat(tx.amount).toLocaleString()} EVAP
                      </p>
                      <p className="text-[10px] text-text-muted">{timeAgo(tx.timestamp)}</p>
                    </div>
                  </Link>
                );
              })}
            </div>
          )
        )}

        {tab === "objects" && (
          objects.length === 0 ? (
            <div className="px-5 py-12 text-center">
              <p className="text-sm text-text-muted">No objects owned</p>
            </div>
          ) : (
            <div className="divide-y divide-white/5">
              {objects.map((obj) => {
                const pct = obj.max_energy > 0
                  ? Math.round((obj.current_energy / obj.max_energy) * 100)
                  : 0;
                const barColor = obj.state === "Active" ? "bg-accent-green"
                  : obj.state === "Grace" ? "bg-accent-amber" : "bg-text-muted";
                return (
                  <Link
                    key={obj.id}
                    href={`/explorer/object/${obj.id}`}
                    className="flex items-center justify-between px-5 py-3 hover:bg-white/[0.02] transition-colors"
                  >
                    <div className="flex items-center gap-3 min-w-0 flex-1">
                      <div className={`w-8 h-8 rounded-lg ${stateBg(obj.state)} flex items-center justify-center shrink-0`}>
                        <Zap size={14} className={stateColor(obj.state)} />
                      </div>
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <p className="text-sm text-text-primary truncate">{obj.name}</p>
                          <span className={`text-[9px] px-1.5 py-0.5 rounded-full font-medium ${stateBg(obj.state)} ${stateColor(obj.state)}`}>
                            {obj.state}
                          </span>
                        </div>
                        <div className="mt-1 flex items-center gap-2">
                          <div className="flex-1 h-1 rounded-full bg-white/5 overflow-hidden">
                            <div className={`h-full rounded-full ${barColor}`} style={{ width: `${pct}%` }} />
                          </div>
                          <span className="text-[10px] text-text-muted w-8 text-right">{pct}%</span>
                        </div>
                      </div>
                    </div>
                    <p className="text-xs font-mono text-text-secondary ml-3">
                      {obj.current_energy.toLocaleString()} E
                    </p>
                  </Link>
                );
              })}
            </div>
          )
        )}

        {tab === "nfts" && (
          nfts.length === 0 ? (
            <div className="px-5 py-12 text-center">
              <p className="text-sm text-text-muted">No NFTs owned</p>
            </div>
          ) : (
            <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-3 p-4">
              {nfts.map((nft) => {
                const pct = nft.max_energy > 0
                  ? Math.round((nft.current_energy / nft.max_energy) * 100)
                  : 0;
                return (
                  <div key={nft.id} className="bg-white/5 rounded-xl overflow-hidden">
                    <div className="aspect-square bg-bg-card-hover flex items-center justify-center">
                      {nft.image_uri ? (
                        <img src={nft.image_uri} alt={nft.name} className="w-full h-full object-cover" />
                      ) : (
                        <Image size={24} className="text-text-muted" />
                      )}
                    </div>
                    <div className="p-3">
                      <p className="text-xs font-medium text-text-primary truncate">{nft.name}</p>
                      <p className="text-[10px] text-text-muted truncate">{nft.collection_name}</p>
                      <div className="mt-2 flex items-center gap-2">
                        <div className="flex-1 h-1 rounded-full bg-white/5 overflow-hidden">
                          <div
                            className={`h-full rounded-full ${
                              nft.state === "Active" ? "bg-accent-green"
                              : nft.state === "Grace" ? "bg-accent-amber"
                              : "bg-text-muted"
                            }`}
                            style={{ width: `${pct}%` }}
                          />
                        </div>
                        <span className="text-[9px] text-text-muted">{pct}%</span>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )
        )}
      </div>
    </motion.div>
  );
}
