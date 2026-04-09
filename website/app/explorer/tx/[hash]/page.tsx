"use client";

import { useState, useEffect } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import { motion } from "framer-motion";
import { ArrowRight, Copy, Check, Clock, Zap } from "lucide-react";

const API = "https://testnet.evaporchain.com/api";

interface Transaction {
  hash: string;
  type: string;
  detail: string;
  from: string;
  to: string;
  amount: string;
  timestamp: number;
}

function shortenAddr(addr: string): string {
  if (!addr || addr.length < 12) return addr;
  return `${addr.slice(0, 8)}...${addr.slice(-6)}`;
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const handleCopy = () => {
    navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };
  return (
    <button
      onClick={handleCopy}
      className="text-text-muted hover:text-accent-cyan transition-colors"
    >
      {copied ? <Check size={12} /> : <Copy size={12} />}
    </button>
  );
}

export default function TransactionDetailPage() {
  const params = useParams();
  const hash = params.hash as string;
  const [tx, setTx] = useState<Transaction | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!hash) return;
    fetch(`${API}/transactions?limit=50`)
      .then((r) => r.json())
      .then((txns: Transaction[]) => {
        const found = txns.find((t) => t.hash === hash);
        if (found) {
          setTx(found);
        } else {
          setError("Transaction not found");
        }
      })
      .catch(() => setError("Failed to fetch transaction"))
      .finally(() => setLoading(false));
  }, [hash]);

  if (loading) {
    return (
      <div className="flex items-center justify-center py-32">
        <div className="w-8 h-8 border-2 border-accent-cyan/30 border-t-accent-cyan rounded-full animate-spin" />
      </div>
    );
  }

  if (error || !tx) {
    return (
      <div className="flex flex-col items-center justify-center py-32">
        <p className="text-sm text-text-muted">{error ?? "Transaction not found"}</p>
        <Link href="/explorer" className="text-sm text-accent-cyan mt-4 hover:underline">
          Back to Explorer
        </Link>
      </div>
    );
  }

  const date = new Date(tx.timestamp);
  const fields = [
    { label: "Transaction Hash", value: tx.hash, mono: true, copyable: true },
    { label: "Type", value: tx.type.charAt(0).toUpperCase() + tx.type.slice(1) },
    { label: "Status", value: "Confirmed", badge: true },
    { label: "Timestamp", value: `${date.toLocaleDateString()} ${date.toLocaleTimeString()}` },
    { label: "Amount", value: `${parseFloat(tx.amount).toLocaleString()} EVAP`, highlight: true },
    ...(tx.detail ? [{ label: "Detail", value: tx.detail }] : []),
  ];

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.4 }}
      className="space-y-6"
    >
      {/* Title */}
      <div>
        <h1 className="text-lg font-semibold text-text-primary">Transaction Details</h1>
        <p className="text-xs text-text-muted mt-1 font-mono break-all">{tx.hash}</p>
      </div>

      {/* From → To */}
      <div className="bg-bg-card border border-white/5 rounded-xl p-6">
        <div className="flex items-center justify-center gap-6 flex-wrap">
          <div className="text-center">
            <p className="text-[10px] text-text-muted uppercase tracking-wider mb-1">From</p>
            <Link
              href={`/explorer/address/${tx.from}`}
              className="text-sm font-mono text-accent-cyan hover:underline"
            >
              {shortenAddr(tx.from)}
            </Link>
          </div>
          <div className="w-10 h-10 rounded-full gradient-bg flex items-center justify-center">
            <ArrowRight size={16} className="text-bg-primary" />
          </div>
          <div className="text-center">
            <p className="text-[10px] text-text-muted uppercase tracking-wider mb-1">To</p>
            <Link
              href={`/explorer/address/${tx.to}`}
              className="text-sm font-mono text-accent-cyan hover:underline"
            >
              {shortenAddr(tx.to)}
            </Link>
          </div>
        </div>

        <div className="mt-4 text-center">
          <span className="text-2xl font-bold gradient-text">
            {parseFloat(tx.amount).toLocaleString()} EVAP
          </span>
        </div>
      </div>

      {/* Details Table */}
      <div className="bg-bg-card border border-white/5 rounded-xl overflow-hidden">
        <div className="divide-y divide-white/5">
          {fields.map((field) => (
            <div key={field.label} className="flex items-start px-5 py-3.5">
              <p className="text-xs text-text-muted w-40 shrink-0 pt-0.5">{field.label}</p>
              <div className="flex items-center gap-2 min-w-0 flex-1">
                {"badge" in field && field.badge ? (
                  <span className="text-xs px-2 py-0.5 rounded-full bg-accent-green/10 text-accent-green font-medium">
                    {field.value}
                  </span>
                ) : "highlight" in field && field.highlight ? (
                  <span className="text-sm font-semibold text-accent-cyan">{field.value}</span>
                ) : (
                  <span
                    className={`text-sm text-text-primary break-all ${
                      "mono" in field && field.mono ? "font-mono" : ""
                    }`}
                  >
                    {field.value}
                  </span>
                )}
                {"copyable" in field && field.copyable && <CopyButton text={field.value} />}
              </div>
            </div>
          ))}
        </div>
      </div>
    </motion.div>
  );
}
