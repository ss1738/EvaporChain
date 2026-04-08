"use client";

import { motion } from "framer-motion";
import AnimatedCounter from "./AnimatedCounter";
import { Lock } from "lucide-react";

const stats = [
  { value: "18ms", label: "Per-block proof time" },
  { value: "<1s", label: "Transaction finality", special: "<1s" },
  { value: "11.3KB", label: "Chain proof size", special: "11.3KB" },
  { value: "PQ", label: "Quantum-resistant", showLock: true },
];

export default function Numbers() {
  return (
    <section className="relative py-32 px-6 overflow-hidden">
      <div className="absolute inset-0 bg-[radial-gradient(circle_at_50%_50%,rgba(0,240,255,0.06),transparent_60%)]" />

      <div className="relative max-w-5xl mx-auto">
        <motion.h2
          initial={{ opacity: 0, y: 30 }}
          whileInView={{ opacity: 1, y: 0 }}
          viewport={{ once: true }}
          transition={{ duration: 0.6 }}
          className="text-3xl sm:text-4xl font-bold text-center mb-16"
        >
          Proven. Not Promised.
        </motion.h2>

        <div className="grid grid-cols-2 md:grid-cols-4 gap-8">
          {stats.map((stat, i) => (
            <motion.div
              key={stat.label}
              initial={{ opacity: 0, y: 20 }}
              whileInView={{ opacity: 1, y: 0 }}
              viewport={{ once: true }}
              transition={{ duration: 0.5, delay: i * 0.1 }}
              className="text-center"
            >
              <div className="text-4xl sm:text-5xl font-bold gradient-text mb-2 flex items-center justify-center gap-2">
                {stat.showLock ? (
                  <div className="flex items-center gap-2">
                    <Lock size={32} className="text-accent-cyan" />
                    <span>PQ</span>
                  </div>
                ) : stat.special ? (
                  <span>{stat.special}</span>
                ) : (
                  <AnimatedCounter value={stat.value} />
                )}
              </div>
              <div className="text-sm text-text-secondary">{stat.label}</div>
            </motion.div>
          ))}
        </div>
      </div>
    </section>
  );
}
