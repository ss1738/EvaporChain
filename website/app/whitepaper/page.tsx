import type { Metadata } from "next";
import Navbar from "@/components/Navbar";
import Footer from "@/components/Footer";

export const metadata: Metadata = {
  title: "Whitepaper — EvaporChain",
  description:
    "EvaporChain technical whitepaper: Thermodynamic State Decay for Sustainable Blockchain Architecture.",
};

const tocItems = [
  "Abstract",
  "1. Introduction",
  "2. The State Growth Problem",
  "3. Thermodynamic State Model",
  "4. Energy Decay Mechanics",
  "5. Evaporation and Ghost Records",
  "6. Dual State Commitment",
  "7. Recursive Proof Architecture",
  "8. Consensus: Mysticeti DAG-BFT",
  "9. Decay-Native Smart Contracts",
  "10. Post-Quantum Cryptography",
  "11. Economic Model",
  "12. Benchmarks and Analysis",
  "13. Related Work",
  "14. Conclusion",
  "References",
];

export default function WhitepaperPage() {
  return (
    <>
      <Navbar />
      <main className="pt-24 pb-32 px-6">
        <div className="max-w-3xl mx-auto">
          <div className="mb-12">
            <p className="text-xs font-medium tracking-[0.2em] uppercase text-text-muted mb-4">
              Technical Whitepaper
            </p>
            <h1 className="text-3xl sm:text-4xl font-bold leading-tight mb-4">
              EvaporChain: Thermodynamic State Decay for{" "}
              <span className="gradient-text">
                Sustainable Blockchain Architecture
              </span>
            </h1>
            <p className="text-text-muted text-sm">
              Built by cryptographers and systems engineers
            </p>
          </div>

          <div className="bg-bg-card rounded-2xl p-8 border border-white/5 mb-12">
            <h2 className="text-lg font-semibold mb-4">Abstract</h2>
            <p className="text-text-secondary text-sm leading-relaxed">
              We present EvaporChain, a Layer 1 blockchain architecture that
              addresses the fundamental sustainability problem of perpetual
              state growth. Every state object in EvaporChain carries an energy
              parameter that depletes over time according to a configurable
              half-life decay function. Objects whose energy reaches zero enter
              a grace period and, if not refreshed, evaporate from active state
              — leaving behind a compact ghost record in a Merkle Mountain
              Range accumulator. This dual-commitment structure (Verkle trie for
              active state, MMR for evaporated state) enables the chain to
              maintain a complete audit trail while allowing active state to
              shrink. Every block transition is folded into a Nova IVC
              recursive proof, producing a constant-size proof regardless of
              chain history. All signatures use ML-DSA (NIST FIPS 204),
              providing post-quantum security from genesis. The result is a
              blockchain that can, for the first time in the history of
              distributed ledger technology, become lighter over time.
            </p>
          </div>

          <div className="bg-bg-card rounded-2xl p-8 border border-white/5 mb-12">
            <h2 className="text-lg font-semibold mb-4">Table of Contents</h2>
            <ol className="space-y-2">
              {tocItems.map((item, i) => (
                <li
                  key={i}
                  className="text-sm text-text-secondary hover:text-accent-cyan transition-colors"
                >
                  {item}
                </li>
              ))}
            </ol>
          </div>

          <div className="flex flex-col sm:flex-row gap-4 items-center justify-center">
            <a
              href="/whitepaper.pdf"
              className="gradient-bg px-8 py-3 rounded-full text-bg-primary text-sm font-medium hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-shadow"
            >
              Download PDF
            </a>
            <a
              href="/"
              className="px-8 py-3 rounded-full border border-white/20 text-text-primary hover:border-accent-cyan text-sm font-medium transition-colors"
            >
              Back to Home
            </a>
          </div>

          <p className="mt-8 text-center text-text-muted text-xs">
            This is a living document. Last updated: March 2026.
          </p>
        </div>
      </main>
      <Footer />
    </>
  );
}
