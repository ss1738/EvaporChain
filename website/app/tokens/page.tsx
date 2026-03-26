"use client";

import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import Navbar from "@/components/Navbar";
import Footer from "@/components/Footer";

const API = "https://testnet.evaporchain.com/api";

interface Token {
  id: number;
  name: string;
  symbol: string;
  total_supply: number;
  decimals: number;
  deployer: string;
}

export default function TokensPage() {
  const [tokens, setTokens] = useState<Token[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetch(`${API}/tokens`)
      .then((r) => r.json())
      .then((d) => setTokens(Array.isArray(d) ? d : []))
      .catch(() => setTokens([]))
      .finally(() => setLoading(false));
  }, []);

  return (
    <>
      <Navbar />
      <main className="pt-24 pb-20">
        <section className="px-6 pb-20">
          <div className="max-w-5xl mx-auto text-center">
            <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.6 }}>
              <p className="text-accent-purple text-sm font-medium tracking-widest uppercase mb-4">EVR-20 Standard</p>
              <h1 className="text-4xl md:text-6xl font-bold mb-6">
                <span className="gradient-text">Decaying Tokens</span>
              </h1>
              <p className="text-lg md:text-xl text-text-secondary max-w-2xl mx-auto mb-4">
                Currency that circulates. EVR-20 tokens lose value over time unless they&apos;re actively used,
                creating natural economic velocity.
              </p>
              <p className="text-text-muted max-w-xl mx-auto mb-10">
                No more dead tokens sitting in forgotten wallets. Decaying tokens solve token hoarding, reduce
                speculative holding, and create genuinely useful digital currencies.
              </p>
            </motion.div>
            <motion.div className="flex flex-wrap justify-center gap-4" initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.3 }}>
              <a
                href="https://testnet.evaporchain.com/tokens"
                target="_blank"
                rel="noopener noreferrer"
                className="gradient-bg text-bg-primary font-medium px-8 py-3 rounded-full hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-shadow"
              >
                Deploy a Token
              </a>
            </motion.div>
          </div>
        </section>

        {/* Features */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <h2 className="text-2xl font-bold text-center mb-12">Why Decaying Tokens?</h2>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-8">
              {[
                {
                  title: "Natural Velocity",
                  desc: "Tokens that decay create incentive to spend and circulate, producing healthier economies than deflationary or static-supply models.",
                  icon: "&#x26A1;",
                },
                {
                  title: "Anti-Hoarding",
                  desc: "Holding tokens without using them costs energy. This discourages whale accumulation and promotes active participation.",
                  icon: "&#x1F6E1;",
                },
                {
                  title: "Self-Cleaning Supply",
                  desc: "Abandoned tokens eventually evaporate, keeping total supply honest. No phantom supply from lost wallets.",
                  icon: "&#x2728;",
                },
              ].map((item, i) => (
                <motion.div
                  key={item.title}
                  className="bg-bg-card border border-white/5 rounded-xl p-6"
                  initial={{ opacity: 0, y: 20 }}
                  whileInView={{ opacity: 1, y: 0 }}
                  viewport={{ once: true }}
                  transition={{ delay: i * 0.15 }}
                >
                  <span className="text-3xl" dangerouslySetInnerHTML={{ __html: item.icon }} />
                  <h3 className="text-lg font-semibold mt-4 mb-2">{item.title}</h3>
                  <p className="text-sm text-text-muted leading-relaxed">{item.desc}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Live tokens */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <h2 className="text-2xl font-bold mb-8">Live Tokens on Testnet</h2>
            {loading ? (
              <div className="text-center py-16 text-text-muted">Loading from testnet...</div>
            ) : tokens.length === 0 ? (
              <div className="text-center py-16 border border-white/5 rounded-xl bg-bg-card">
                <p className="text-text-muted mb-4">No tokens deployed yet.</p>
                <a href="https://testnet.evaporchain.com/tokens" target="_blank" rel="noopener noreferrer" className="text-accent-cyan hover:underline">
                  Deploy the first EVR-20 token &rarr;
                </a>
              </div>
            ) : (
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                {tokens.map((t, i) => (
                  <motion.div
                    key={t.id}
                    className="bg-bg-card border border-white/5 rounded-xl p-5 flex items-center justify-between hover:border-accent-purple/20 transition-colors"
                    initial={{ opacity: 0, x: -20 }}
                    whileInView={{ opacity: 1, x: 0 }}
                    viewport={{ once: true }}
                    transition={{ delay: i * 0.05 }}
                  >
                    <div>
                      <h3 className="font-semibold">{t.name}</h3>
                      <p className="text-xs text-text-muted">{t.symbol}</p>
                    </div>
                    <div className="text-right">
                      <p className="text-sm font-mono text-accent-cyan">{t.total_supply.toLocaleString()}</p>
                      <p className="text-xs text-text-muted">supply</p>
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
