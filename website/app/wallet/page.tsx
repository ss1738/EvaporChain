"use client";

import { motion } from "framer-motion";
import Navbar from "@/components/Navbar";
import Footer from "@/components/Footer";

export default function WalletPage() {
  return (
    <>
      <Navbar />
      <main className="pt-24 pb-20">
        <section className="px-6 py-20">
          <div className="max-w-4xl mx-auto text-center">
            <motion.div initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.6 }}>
              <p className="text-accent-cyan text-sm font-medium tracking-widest uppercase mb-4">Account Management</p>
              <h1 className="text-4xl md:text-6xl font-bold mb-6">
                <span className="gradient-text">EvaporChain Wallet</span>
              </h1>
              <p className="text-lg md:text-xl text-text-secondary max-w-2xl mx-auto mb-4">
                Create accounts, manage assets, send transactions, and monitor your energy levels
                — all in one place.
              </p>
              <p className="text-text-muted max-w-xl mx-auto mb-12">
                The EvaporChain wallet gives you full control over your decaying assets.
                Track energy levels, set refresh reminders, and manage your portfolio of mortal tokens and NFTs.
              </p>
            </motion.div>

            <motion.div
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.3 }}
              className="flex flex-col items-center gap-6"
            >
              <a
                href="https://testnet.evaporchain.com/wallet"
                target="_blank"
                rel="noopener noreferrer"
                className="gradient-bg text-bg-primary font-semibold text-lg px-10 py-4 rounded-full hover:shadow-[0_0_30px_rgba(0,240,255,0.3)] transition-shadow"
              >
                Open Wallet &rarr;
              </a>
              <p className="text-xs text-text-muted">Opens the testnet wallet application</p>
            </motion.div>
          </div>
        </section>

        {/* Features */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <h2 className="text-2xl font-bold text-center mb-12">Wallet Features</h2>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-8">
              {[
                {
                  title: "Energy Dashboard",
                  desc: "Monitor energy levels across all your assets. See which NFTs and tokens are approaching grace period and need refreshing.",
                },
                {
                  title: "Post-Quantum Secure",
                  desc: "Every transaction is signed with ML-DSA (FIPS 204) lattice-based signatures. Your assets are quantum-resistant from day one.",
                },
                {
                  title: "Faucet Access",
                  desc: "Get free testnet tokens instantly. Test transfers, mint NFTs, deploy tokens, and experiment with staking — no real money needed.",
                },
                {
                  title: "Transaction History",
                  desc: "Full history of transfers, mints, refreshes, and evaporations. See exactly when and why your assets changed state.",
                },
                {
                  title: "Multi-Asset View",
                  desc: "See your EVR balance, NFTs, deployed tokens, staking positions, and DAO votes all in one unified interface.",
                },
                {
                  title: "Ghost Records",
                  desc: "View your evaporated assets and their nullifier proofs. Ghost records prove your assets once existed even after evaporation.",
                },
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
      </main>
      <Footer />
    </>
  );
}
