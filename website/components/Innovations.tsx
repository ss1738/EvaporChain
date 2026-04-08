"use client";

import { motion } from "framer-motion";
import { Flame, Shield, FileCode, Lock } from "lucide-react";

const innovations = [
  {
    icon: Flame,
    color: "#00f0ff",
    title: "Thermodynamic State Decay",
    description:
      "Every state object has energy that depletes. Unused state evaporates automatically. The chain\u2019s state database can actually shrink \u2014 a first in blockchain history.",
  },
  {
    icon: Shield,
    color: "#8b5cf6",
    title: "Constant-Size Chain Proofs",
    description:
      "Every block folds into a recursive proof. A chain running 10 years produces the same verification cost as one running 10 minutes. One proof. Milliseconds to verify.",
  },
  {
    icon: FileCode,
    color: "#22c55e",
    title: "Smart Contracts That Live and Die",
    description:
      "Six contract templates with decay built in. Tokens that expire. NFTs with lifespans. Escrows that evaporate. Every contract is thermodynamically aware.",
  },
  {
    icon: Lock,
    color: "#f59e0b",
    title: "Post-Quantum From Day One",
    description:
      "ML-DSA (NIST-standardized) signatures protect every transaction. When quantum computers arrive, EvaporChain is already safe.",
  },
];

export default function Innovations() {
  return (
    <section className="py-32 px-6">
      <div className="max-w-5xl mx-auto">
        <motion.h2
          initial={{ opacity: 0, y: 30 }}
          whileInView={{ opacity: 1, y: 0 }}
          viewport={{ once: true }}
          transition={{ duration: 0.6 }}
          className="text-3xl sm:text-4xl font-bold text-center"
        >
          Built Different
        </motion.h2>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-6 mt-16">
          {innovations.map((item, i) => (
            <motion.div
              key={item.title}
              initial={{ opacity: 0, y: 30 }}
              whileInView={{ opacity: 1, y: 0 }}
              viewport={{ once: true }}
              transition={{ duration: 0.5, delay: i * 0.1 }}
              className="bg-bg-card rounded-2xl p-8 border border-white/5 hover:border-white/10 hover:-translate-y-1 transition-all duration-300 group"
              style={{
                ["--hover-color" as string]: item.color,
              }}
            >
              <div
                className="w-12 h-12 rounded-xl flex items-center justify-center mb-5"
                style={{ background: `${item.color}15` }}
              >
                <item.icon size={22} style={{ color: item.color }} />
              </div>
              <h3 className="text-lg font-semibold mb-3">{item.title}</h3>
              <p className="text-text-secondary text-sm leading-relaxed">
                {item.description}
              </p>
            </motion.div>
          ))}
        </div>
      </div>
    </section>
  );
}
