"use client";

import { motion } from "framer-motion";

const layers = [
  {
    name: "CONSENSUS",
    description: "Tendermint BFT \u00B7 Stake-weighted leader rotation \u00B7 2/3 finality threshold",
    color: "#00f0ff",
    direction: -1,
  },
  {
    name: "EXECUTION",
    description: "EvaporScript VM \u00B7 Template contracts \u00B7 Decay-native lifecycle hooks",
    color: "#8b5cf6",
    direction: 1,
  },
  {
    name: "STATE",
    description: "Verkle trie (active) \u00B7 MMR accumulator (evaporated) \u00B7 Dual commitment",
    color: "#22c55e",
    direction: -1,
  },
  {
    name: "PROOF",
    description: "Nova IVC recursive folding \u00B7 Thermodynamic proof circuit \u00B7 Constant-size output",
    color: "#f59e0b",
    direction: 1,
  },
];

export default function Architecture() {
  return (
    <section id="technology" className="py-32 px-6 bg-[#0c0c14]">
      <div className="max-w-4xl mx-auto">
        <motion.div
          initial={{ opacity: 0, y: 30 }}
          whileInView={{ opacity: 1, y: 0 }}
          viewport={{ once: true }}
          transition={{ duration: 0.6 }}
          className="text-center mb-16"
        >
          <h2 className="text-3xl sm:text-4xl font-bold">The Architecture</h2>
          <p className="mt-4 text-text-secondary text-lg">
            Proven technology. Novel combination.
          </p>
        </motion.div>

        <div className="space-y-4">
          {layers.map((layer, i) => (
            <motion.div
              key={layer.name}
              initial={{ opacity: 0, x: layer.direction * 60 }}
              whileInView={{ opacity: 1, x: 0 }}
              viewport={{ once: true }}
              transition={{ duration: 0.6, delay: i * 0.15 }}
              className="relative bg-bg-card rounded-xl p-6 border border-white/5 overflow-hidden"
            >
              <div
                className="absolute left-0 top-0 bottom-0 w-1 rounded-l-xl"
                style={{ background: layer.color }}
              />
              <div className="ml-4">
                <div
                  className="text-xs font-bold tracking-[0.15em] mb-1"
                  style={{ color: layer.color }}
                >
                  {layer.name}
                </div>
                <div className="text-sm text-text-secondary">
                  {layer.description}
                </div>
              </div>
            </motion.div>
          ))}
        </div>

        <motion.div
          initial={{ opacity: 0 }}
          whileInView={{ opacity: 1 }}
          viewport={{ once: true }}
          transition={{ duration: 0.6, delay: 0.8 }}
          className="mt-12 text-center"
        >
          <p className="text-text-secondary mb-4">
            Every component is tested and benchmarked. The innovation is the
            combination.
          </p>
          <a
            href="/whitepaper"
            className="inline-flex items-center gap-2 text-sm font-medium text-text-secondary hover:text-accent-cyan transition-colors"
          >
            Read the Technical Whitepaper
            <span className="gradient-text">&rarr;</span>
          </a>
        </motion.div>
      </div>
    </section>
  );
}
