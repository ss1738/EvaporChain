"use client";

import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import Navbar from "@/components/Navbar";
import Footer from "@/components/Footer";

const API = "https://testnet.evaporchain.com/api";

interface Nft {
  id: number;
  name: string;
  collection: string;
  energy: number;
  half_life: number;
  owner: string;
  created_at: number;
  image_url?: string;
}

export default function NftPage() {
  const [nfts, setNfts] = useState<Nft[]>([]);
  const [ghosts, setGhosts] = useState<number>(0);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    Promise.all([
      fetch(`${API}/nfts`).then((r) => r.json()).catch(() => []),
      fetch(`${API}/status`).then((r) => r.json()).catch(() => ({ ghost_count: 0 })),
    ]).then(([nftData, status]) => {
      setNfts(Array.isArray(nftData) ? nftData : []);
      setGhosts(status.ghost_count || 0);
      setLoading(false);
    });
  }, []);

  return (
    <>
      <Navbar />
      <main className="pt-24 pb-20">
        {/* Hero */}
        <section className="px-6 pb-20">
          <div className="max-w-5xl mx-auto text-center">
            <motion.div
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.6 }}
            >
              <p className="text-accent-cyan text-sm font-medium tracking-widest uppercase mb-4">
                EVR-721 Standard
              </p>
              <h1 className="text-4xl md:text-6xl font-bold mb-6">
                <span className="gradient-text">Mortal NFTs</span>
              </h1>
              <p className="text-lg md:text-xl text-text-secondary max-w-2xl mx-auto mb-4">
                Digital assets with lifespans. Every NFT on EvaporChain has energy that decays
                over time. Refresh it to keep it alive, or let it evaporate into a permanent
                ghost record.
              </p>
              <p className="text-text-muted max-w-xl mx-auto mb-10">
                No more dead NFTs clogging the chain. No more forgotten collections wasting storage.
                If nobody cares about it, it evaporates — but the proof it existed lives forever.
              </p>
            </motion.div>

            <motion.div
              className="flex flex-wrap justify-center gap-4"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ delay: 0.3 }}
            >
              <a
                href="https://testnet.evaporchain.com/nft"
                target="_blank"
                rel="noopener noreferrer"
                className="gradient-bg text-bg-primary font-medium px-8 py-3 rounded-full hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-shadow"
              >
                Mint Your First NFT
              </a>
              <a
                href="https://testnet.evaporchain.com/explorer"
                target="_blank"
                rel="noopener noreferrer"
                className="border border-white/10 text-text-primary font-medium px-8 py-3 rounded-full hover:border-accent-cyan/40 transition-colors"
              >
                View Ghost Gallery
              </a>
            </motion.div>
          </div>
        </section>

        {/* How it works */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <h2 className="text-2xl font-bold text-center mb-12">How Mortal NFTs Work</h2>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-8">
              {[
                {
                  step: "01",
                  title: "Mint with Energy",
                  desc: "Each NFT is created with an energy level and a half-life. The energy determines how long it stays alive on-chain.",
                  color: "text-accent-cyan",
                },
                {
                  step: "02",
                  title: "Decay Over Time",
                  desc: "Energy decays exponentially. When energy hits zero, the NFT enters a grace period. Refresh it with energy to save it.",
                  color: "text-accent-amber",
                },
                {
                  step: "03",
                  title: "Evaporate or Resurrect",
                  desc: "If nobody refreshes it during grace, the NFT evaporates. A ghost record with a nullifier proof remains forever in the MMR accumulator.",
                  color: "text-accent-purple",
                },
              ].map((item, i) => (
                <motion.div
                  key={item.step}
                  className="bg-bg-card border border-white/5 rounded-xl p-6"
                  initial={{ opacity: 0, y: 20 }}
                  whileInView={{ opacity: 1, y: 0 }}
                  viewport={{ once: true }}
                  transition={{ delay: i * 0.15 }}
                >
                  <span className={`text-3xl font-bold ${item.color} opacity-40`}>{item.step}</span>
                  <h3 className="text-lg font-semibold mt-3 mb-2">{item.title}</h3>
                  <p className="text-sm text-text-muted leading-relaxed">{item.desc}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Live NFT Gallery */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <div className="flex items-center justify-between mb-8">
              <h2 className="text-2xl font-bold">Live NFT Gallery</h2>
              <div className="flex items-center gap-6 text-sm text-text-muted">
                <span>{nfts.length} active</span>
                <span>{ghosts} evaporated</span>
              </div>
            </div>

            {loading ? (
              <div className="text-center py-20 text-text-muted">Loading from testnet...</div>
            ) : nfts.length === 0 ? (
              <div className="text-center py-20 border border-white/5 rounded-xl bg-bg-card">
                <p className="text-text-muted mb-4">No active NFTs on the testnet right now.</p>
                <a
                  href="https://testnet.evaporchain.com/nft"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-accent-cyan hover:underline"
                >
                  Be the first to mint one &rarr;
                </a>
              </div>
            ) : (
              <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
                {nfts.slice(0, 9).map((nft, i) => {
                  const energyPct = Math.min(100, Math.max(0, (nft.energy / 10000) * 100));
                  return (
                    <motion.div
                      key={nft.id}
                      className="bg-bg-card border border-white/5 rounded-xl overflow-hidden hover:border-accent-cyan/20 transition-colors"
                      initial={{ opacity: 0, y: 20 }}
                      whileInView={{ opacity: 1, y: 0 }}
                      viewport={{ once: true }}
                      transition={{ delay: i * 0.05 }}
                    >
                      <div className="h-40 bg-gradient-to-br from-accent-cyan/10 to-accent-purple/10 flex items-center justify-center">
                        <span className="text-5xl opacity-60">
                          {nft.collection === "Genesis" ? "&#x1F48E;" : "&#x1F525;"}
                        </span>
                      </div>
                      <div className="p-4">
                        <div className="flex items-center justify-between mb-2">
                          <h3 className="font-semibold text-sm">{nft.name}</h3>
                          <span className="text-xs text-text-muted">#{nft.id}</span>
                        </div>
                        <p className="text-xs text-text-muted mb-3">{nft.collection}</p>
                        <div className="mb-1 flex justify-between text-xs text-text-muted">
                          <span>Energy</span>
                          <span>{nft.energy.toLocaleString()}</span>
                        </div>
                        <div className="w-full h-1.5 bg-white/5 rounded-full overflow-hidden">
                          <div
                            className="h-full rounded-full bg-gradient-to-r from-accent-cyan to-accent-purple transition-all"
                            style={{ width: `${energyPct}%` }}
                          />
                        </div>
                      </div>
                    </motion.div>
                  );
                })}
              </div>
            )}
          </div>
        </section>

        {/* EVR-721 Standard */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-3xl mx-auto">
            <h2 className="text-2xl font-bold text-center mb-8">EVR-721 Standard</h2>
            <div className="bg-bg-card border border-white/5 rounded-xl p-6 md:p-8">
              <p className="text-text-secondary leading-relaxed mb-6">
                EVR-721 extends the familiar NFT standard with decay semantics. Every token has an
                <code className="text-accent-cyan mx-1">energy</code> field and a
                <code className="text-accent-cyan mx-1">half_life</code> parameter that governs exponential decay.
              </p>
              <div className="bg-bg-primary rounded-lg p-4 font-mono text-sm text-text-muted overflow-x-auto">
                <pre>{`contract MortalNFT {
  state {
    owners: map(u64 -> address),
    energies: map(u64 -> u64),
  }

  fn mint(to: address, energy: u64) -> u64 { ... }
  fn transfer(from: address, to: address, id: u64) { ... }
  fn refresh(id: u64, energy: u64) { ... }

  OnEvaporate { /* ghost record created */ }
  OnGrace { /* last chance to refresh */ }
}`}</pre>
              </div>
            </div>
          </div>
        </section>
      </main>
      <Footer />
    </>
  );
}
