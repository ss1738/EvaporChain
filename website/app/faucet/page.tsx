"use client";

import { useState } from "react";
import { motion } from "framer-motion";
import { Droplets, CheckCircle, AlertCircle, Clock, Zap } from "lucide-react";
import Navbar from "@/components/Navbar";
import Footer from "@/components/Footer";

const API = "https://testnet.evaporchain.com/api";
const FAUCET_AMOUNT = 1000;
const COOLDOWN_HOURS = 24;

type FaucetStatus = "idle" | "pending" | "success" | "error";

interface FaucetResult {
  tx_hash?: string;
  amount?: number;
  message?: string;
}

export default function FaucetPage() {
  const [address, setAddress] = useState("");
  const [status, setStatus] = useState<FaucetStatus>("idle");
  const [result, setResult] = useState<FaucetResult | null>(null);

  async function handleRequest(e: React.FormEvent) {
    e.preventDefault();
    if (!address.trim()) return;

    setStatus("pending");
    setResult(null);

    try {
      const res = await fetch(`${API}/faucet`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ address: address.trim() }),
      });
      const data = await res.json();
      if (res.ok) {
        setStatus("success");
        setResult(data);
      } else {
        setStatus("error");
        setResult({ message: data.error ?? data.message ?? "Request failed" });
      }
    } catch {
      setStatus("error");
      setResult({ message: "Could not reach testnet. Check your connection." });
    }
  }

  return (
    <>
      <Navbar />
      <main className="pt-24 pb-20">
        {/* Hero */}
        <section className="px-6 pb-16">
          <div className="max-w-2xl mx-auto text-center">
            <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.6 }}>
              <div className="inline-flex items-center gap-2 bg-accent-cyan/10 border border-accent-cyan/20 rounded-full px-4 py-2 mb-6">
                <Droplets className="w-4 h-4 text-accent-cyan" />
                <span className="text-accent-cyan text-sm font-medium">Testnet Faucet</span>
              </div>
              <h1 className="text-4xl md:text-5xl font-bold mb-4">
                <span className="gradient-text">Free Testnet Tokens</span>
              </h1>
              <p className="text-text-secondary text-lg mb-2">
                Get {FAUCET_AMOUNT} EVAP tokens every {COOLDOWN_HOURS} hours.
              </p>
              <p className="text-text-muted text-sm">
                Tokens have real energy and decay — experiment with the chain before mainnet.
              </p>
            </motion.div>
          </div>
        </section>

        {/* Faucet form */}
        <section className="px-6 pb-20">
          <motion.div
            className="max-w-xl mx-auto bg-bg-card border border-white/5 rounded-2xl p-8"
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.2 }}
          >
            <form onSubmit={handleRequest} className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-text-secondary mb-2">
                  Your Testnet Address
                </label>
                <input
                  type="text"
                  value={address}
                  onChange={(e) => setAddress(e.target.value)}
                  placeholder="0x7f3a8b2ce419d605a1c74e823fb960d4159ae378"
                  className="w-full bg-bg-primary border border-white/10 rounded-lg px-4 py-3 text-sm font-mono text-text-primary placeholder:text-text-muted focus:outline-none focus:border-accent-cyan/50 transition-colors"
                  disabled={status === "pending"}
                />
              </div>

              <button
                type="submit"
                disabled={status === "pending" || !address.trim()}
                className="w-full gradient-bg text-bg-primary font-semibold py-3 rounded-lg disabled:opacity-50 disabled:cursor-not-allowed hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-all flex items-center justify-center gap-2"
              >
                {status === "pending" ? (
                  <>
                    <span className="animate-spin h-4 w-4 border-2 border-bg-primary border-t-transparent rounded-full" />
                    Sending tokens...
                  </>
                ) : (
                  <>
                    <Droplets className="w-4 h-4" />
                    Request {FAUCET_AMOUNT} EVAP
                  </>
                )}
              </button>
            </form>

            {/* Result */}
            {status === "success" && result && (
              <motion.div
                className="mt-6 p-4 bg-accent-green/10 border border-accent-green/20 rounded-lg"
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
              >
                <div className="flex items-start gap-3">
                  <CheckCircle className="w-5 h-5 text-accent-green mt-0.5 shrink-0" />
                  <div>
                    <p className="text-sm font-semibold text-accent-green mb-1">
                      {result.amount ?? FAUCET_AMOUNT} EVAP sent!
                    </p>
                    {result.tx_hash && (
                      <p className="text-xs text-text-muted font-mono break-all">
                        Tx: {result.tx_hash}
                      </p>
                    )}
                    <p className="text-xs text-text-muted mt-2">
                      Tokens arrive in ~1 block (≈12 s). Check your balance in the{" "}
                      <a href="/explorer" className="text-accent-cyan hover:underline">
                        explorer
                      </a>
                      .
                    </p>
                  </div>
                </div>
              </motion.div>
            )}

            {status === "error" && result && (
              <motion.div
                className="mt-6 p-4 bg-red-500/10 border border-red-500/20 rounded-lg"
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
              >
                <div className="flex items-start gap-3">
                  <AlertCircle className="w-5 h-5 text-red-400 mt-0.5 shrink-0" />
                  <p className="text-sm text-red-300">{result.message}</p>
                </div>
              </motion.div>
            )}
          </motion.div>
        </section>

        {/* Info cards */}
        <section className="px-6 pb-20 border-t border-white/5 pt-16">
          <div className="max-w-4xl mx-auto">
            <h2 className="text-xl font-bold text-center mb-10">How the Faucet Works</h2>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
              {[
                {
                  icon: Droplets,
                  title: `${FAUCET_AMOUNT} EVAP per request`,
                  desc: `Each request drips ${FAUCET_AMOUNT} EVAP to your address. Tokens have initial energy of 10,000 units with a 100-epoch half-life.`,
                },
                {
                  icon: Clock,
                  title: `${COOLDOWN_HOURS}h cooldown`,
                  desc: "One request per address per 24 hours. Cooldown is enforced on-chain — the faucet object tracks timestamps in state.",
                },
                {
                  icon: Zap,
                  title: "Energy included",
                  desc: "Faucet tokens come pre-loaded with energy. Watch them decay in real-time on the explorer. Refresh to keep them alive.",
                },
              ].map((item) => (
                <motion.div
                  key={item.title}
                  className="bg-bg-card border border-white/5 rounded-xl p-6 text-center"
                  initial={{ opacity: 0, y: 20 }}
                  whileInView={{ opacity: 1, y: 0 }}
                  viewport={{ once: true }}
                >
                  <item.icon className="w-8 h-8 text-accent-cyan mx-auto mb-4" />
                  <h3 className="font-semibold mb-2">{item.title}</h3>
                  <p className="text-sm text-text-muted leading-relaxed">{item.desc}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Quick links */}
        <section className="px-6 pb-8">
          <div className="max-w-xl mx-auto text-center">
            <p className="text-text-muted text-sm mb-4">What to do next</p>
            <div className="flex flex-wrap justify-center gap-3">
              <a href="/explorer" className="text-sm text-accent-cyan hover:underline">
                View in Explorer →
              </a>
              <a href="/developers" className="text-sm text-accent-cyan hover:underline">
                Read the Docs →
              </a>
              <a href="/wallet" className="text-sm text-accent-cyan hover:underline">
                Get the Wallet →
              </a>
            </div>
          </div>
        </section>
      </main>
      <Footer />
    </>
  );
}
