"use client";

import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import Navbar from "@/components/Navbar";
import Footer from "@/components/Footer";

const API = "https://testnet.evaporchain.com/api";

interface Proposal {
  id: number;
  title: string;
  description: string;
  proposer: string;
  votes_for: number;
  votes_against: number;
  status: string;
  created_at: number;
}

export default function DaoPage() {
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetch(`${API}/dao/proposals`)
      .then((r) => r.json())
      .then((d) => setProposals(Array.isArray(d) ? d : []))
      .catch(() => setProposals([]))
      .finally(() => setLoading(false));
  }, []);

  return (
    <>
      <Navbar />
      <main className="pt-24 pb-20">
        <section className="px-6 pb-20">
          <div className="max-w-5xl mx-auto text-center">
            <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.6 }}>
              <p className="text-accent-amber text-sm font-medium tracking-widest uppercase mb-4">On-Chain Governance</p>
              <h1 className="text-4xl md:text-6xl font-bold mb-6">
                <span className="gradient-text">Governance That Cleans Up After Itself</span>
              </h1>
              <p className="text-lg md:text-xl text-text-secondary max-w-2xl mx-auto mb-4">
                Proposals that expire. Votes that decay. No more governance graveyard of
                abandoned proposals clogging the chain.
              </p>
              <p className="text-text-muted max-w-xl mx-auto mb-10">
                On EvaporChain, proposals have energy too. If a proposal doesn&apos;t reach
                quorum before its energy runs out, it evaporates. Only active governance survives.
              </p>
            </motion.div>
            <motion.div className="flex flex-wrap justify-center gap-4" initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.3 }}>
              <a
                href="https://testnet.evaporchain.com/dao"
                target="_blank"
                rel="noopener noreferrer"
                className="gradient-bg text-bg-primary font-medium px-8 py-3 rounded-full hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-shadow"
              >
                Vote Now
              </a>
              <a
                href="https://testnet.evaporchain.com/dao"
                target="_blank"
                rel="noopener noreferrer"
                className="border border-white/10 text-text-primary font-medium px-8 py-3 rounded-full hover:border-accent-cyan/40 transition-colors"
              >
                Create Proposal
              </a>
            </motion.div>
          </div>
        </section>

        {/* Features */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <h2 className="text-2xl font-bold text-center mb-12">Decaying Governance</h2>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-8">
              {[
                { title: "Time-Limited Proposals", desc: "Every proposal has energy. If it doesn't pass before energy depletes, it evaporates — keeping governance focused and urgent." },
                { title: "Active Voting", desc: "Voting power decays too. Delegates who stop participating gradually lose influence, preventing voter apathy." },
                { title: "Clean State", desc: "Failed proposals don't linger forever. The governance module self-cleans, keeping only passed and active proposals on-chain." },
              ].map((item, i) => (
                <motion.div
                  key={item.title}
                  className="bg-bg-card border border-white/5 rounded-xl p-6"
                  initial={{ opacity: 0, y: 20 }}
                  whileInView={{ opacity: 1, y: 0 }}
                  viewport={{ once: true }}
                  transition={{ delay: i * 0.15 }}
                >
                  <h3 className="text-lg font-semibold mb-2">{item.title}</h3>
                  <p className="text-sm text-text-muted leading-relaxed">{item.desc}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Live proposals */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <h2 className="text-2xl font-bold mb-8">Active Proposals</h2>
            {loading ? (
              <div className="text-center py-16 text-text-muted">Loading from testnet...</div>
            ) : proposals.length === 0 ? (
              <div className="text-center py-16 border border-white/5 rounded-xl bg-bg-card">
                <p className="text-text-muted mb-4">No active proposals right now.</p>
                <a href="https://testnet.evaporchain.com/dao" target="_blank" rel="noopener noreferrer" className="text-accent-cyan hover:underline">
                  Create the first proposal &rarr;
                </a>
              </div>
            ) : (
              <div className="space-y-4">
                {proposals.map((p, i) => {
                  const total = p.votes_for + p.votes_against;
                  const forPct = total > 0 ? (p.votes_for / total) * 100 : 50;
                  return (
                    <motion.div
                      key={p.id}
                      className="bg-bg-card border border-white/5 rounded-xl p-5 hover:border-accent-amber/20 transition-colors"
                      initial={{ opacity: 0, y: 10 }}
                      whileInView={{ opacity: 1, y: 0 }}
                      viewport={{ once: true }}
                      transition={{ delay: i * 0.05 }}
                    >
                      <div className="flex items-start justify-between mb-3">
                        <div>
                          <h3 className="font-semibold">{p.title}</h3>
                          <p className="text-xs text-text-muted mt-1 line-clamp-2">{p.description}</p>
                        </div>
                        <span className={`text-xs px-2 py-1 rounded-full ${
                          p.status === "active" ? "bg-accent-green/10 text-accent-green" :
                          p.status === "passed" ? "bg-accent-cyan/10 text-accent-cyan" :
                          "bg-white/5 text-text-muted"
                        }`}>{p.status}</span>
                      </div>
                      <div className="w-full h-2 bg-white/5 rounded-full overflow-hidden">
                        <div className="h-full bg-accent-green rounded-full" style={{ width: `${forPct}%` }} />
                      </div>
                      <div className="flex justify-between mt-2 text-xs text-text-muted">
                        <span>For: {p.votes_for}</span>
                        <span>Against: {p.votes_against}</span>
                      </div>
                    </motion.div>
                  );
                })}
              </div>
            )}
          </div>
        </section>
      </main>
      <Footer />
    </>
  );
}
