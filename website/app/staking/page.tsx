"use client";

import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import Navbar from "@/components/Navbar";
import Footer from "@/components/Footer";

const API = "https://testnet.evaporchain.com/api";

interface Pool {
  id: number;
  name: string;
  total_staked: number;
  apy: number;
  stakers: number;
  min_stake: number;
}

export default function StakingPage() {
  const [pools, setPools] = useState<Pool[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetch(`${API}/staking/pools`)
      .then((r) => r.json())
      .then((d) => setPools(Array.isArray(d) ? d : []))
      .catch(() => setPools([]))
      .finally(() => setLoading(false));
  }, []);

  return (
    <>
      <Navbar />
      <main className="pt-24 pb-20">
        <section className="px-6 pb-20">
          <div className="max-w-5xl mx-auto text-center">
            <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.6 }}>
              <p className="text-accent-green text-sm font-medium tracking-widest uppercase mb-4">Proof of Stake</p>
              <h1 className="text-4xl md:text-6xl font-bold mb-6">
                <span className="gradient-text">Use-It-Or-Lose-It Staking</span>
              </h1>
              <p className="text-lg md:text-xl text-text-secondary max-w-2xl mx-auto mb-4">
                Staking rewards that decay if you don&apos;t claim them. Validators who go
                inactive lose their stake over time. The chain rewards active participation.
              </p>
              <p className="text-text-muted max-w-xl mx-auto mb-10">
                Traditional staking lets validators lock tokens and forget. EvaporChain staking
                requires continuous engagement — your stake decays if you stop participating.
              </p>
            </motion.div>
            <motion.div className="flex flex-wrap justify-center gap-4" initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.3 }}>
              <a
                href="https://testnet.evaporchain.com/staking"
                target="_blank"
                rel="noopener noreferrer"
                className="gradient-bg text-bg-primary font-medium px-8 py-3 rounded-full hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-shadow"
              >
                Start Staking
              </a>
            </motion.div>
          </div>
        </section>

        {/* How it differs */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <h2 className="text-2xl font-bold text-center mb-12">How Decaying Staking Works</h2>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-8">
              {[
                { title: "Stake with Energy", desc: "Lock tokens into a staking pool. Your stake has energy that decays with a half-life, just like any EvaporChain object." },
                { title: "Earn Rewards", desc: "Active validators earn rewards proportional to their remaining stake energy, not just their initial deposit." },
                { title: "Claim or Decay", desc: "Rewards accumulate but also decay. Claim regularly to maximize returns. Abandoned rewards evaporate back into the protocol." },
                { title: "Slash Protection", desc: "Inactive validators naturally lose stake over time — no harsh slashing needed. The decay mechanism is the slashing mechanism." },
              ].map((item, i) => (
                <motion.div
                  key={item.title}
                  className="bg-bg-card border border-white/5 rounded-xl p-6"
                  initial={{ opacity: 0, y: 20 }}
                  whileInView={{ opacity: 1, y: 0 }}
                  viewport={{ once: true }}
                  transition={{ delay: i * 0.1 }}
                >
                  <h3 className="text-lg font-semibold mb-2">{item.title}</h3>
                  <p className="text-sm text-text-muted leading-relaxed">{item.desc}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Live pools */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <h2 className="text-2xl font-bold mb-8">Staking Pools</h2>
            {loading ? (
              <div className="text-center py-16 text-text-muted">Loading from testnet...</div>
            ) : pools.length === 0 ? (
              <div className="text-center py-16 border border-white/5 rounded-xl bg-bg-card">
                <p className="text-text-muted mb-4">No staking pools available yet.</p>
                <a href="https://testnet.evaporchain.com/staking" target="_blank" rel="noopener noreferrer" className="text-accent-cyan hover:underline">
                  View staking on testnet &rarr;
                </a>
              </div>
            ) : (
              <div className="space-y-4">
                {pools.map((p, i) => (
                  <motion.div
                    key={p.id}
                    className="bg-bg-card border border-white/5 rounded-xl p-5 flex flex-col md:flex-row md:items-center justify-between gap-4 hover:border-accent-green/20 transition-colors"
                    initial={{ opacity: 0, y: 10 }}
                    whileInView={{ opacity: 1, y: 0 }}
                    viewport={{ once: true }}
                    transition={{ delay: i * 0.05 }}
                  >
                    <div>
                      <h3 className="font-semibold">{p.name}</h3>
                      <p className="text-xs text-text-muted">{p.stakers} stakers</p>
                    </div>
                    <div className="flex gap-8 text-sm">
                      <div className="text-center">
                        <p className="font-mono text-accent-green">{p.apy}%</p>
                        <p className="text-xs text-text-muted">APY</p>
                      </div>
                      <div className="text-center">
                        <p className="font-mono text-accent-cyan">{p.total_staked.toLocaleString()}</p>
                        <p className="text-xs text-text-muted">Total Staked</p>
                      </div>
                      <div className="text-center">
                        <p className="font-mono text-text-secondary">{p.min_stake.toLocaleString()}</p>
                        <p className="text-xs text-text-muted">Min Stake</p>
                      </div>
                    </div>
                  </motion.div>
                ))}
              </div>
            )}
          </div>
        </section>
      </main>
      <Footer />
    </>
  );
}
