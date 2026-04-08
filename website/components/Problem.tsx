"use client";

import { motion } from "framer-motion";
import { Database, Cpu, TrendingUp } from "lucide-react";
import AnimatedCounter from "./AnimatedCounter";

const cards = [
  {
    icon: Database,
    number: "300+",
    unit: " GB",
    label: "Ethereum State Size",
    sub: "And growing every block. Forever.",
  },
  {
    icon: Cpu,
    number: "256",
    unit: " GB RAM",
    label: "Solana Validator Requirement",
    sub: "Enterprise hardware to participate.",
  },
  {
    icon: TrendingUp,
    number: "",
    unit: "",
    label: "State Growth",
    sub: "No blockchain has ever gotten lighter. Until now.",
    special: true,
  },
];

export default function Problem() {
  return (
    <section id="problem" className="py-32 px-6">
      <div className="max-w-6xl mx-auto">
        <motion.div
          initial={{ opacity: 0, y: 30 }}
          whileInView={{ opacity: 1, y: 0 }}
          viewport={{ once: true }}
          transition={{ duration: 0.6 }}
          className="text-center"
        >
          <h2 className="text-3xl sm:text-4xl font-bold">
            Every Blockchain Has a Fatal Flaw
          </h2>
          <p className="mt-4 text-text-secondary text-lg max-w-xl mx-auto">
            The more successful a chain becomes, the harder it is to run.
          </p>
        </motion.div>

        <div className="grid grid-cols-1 md:grid-cols-3 gap-8 mt-16">
          {cards.map((card, i) => (
            <motion.div
              key={card.label}
              initial={{ opacity: 0, y: 30 }}
              whileInView={{ opacity: 1, y: 0 }}
              viewport={{ once: true }}
              transition={{ duration: 0.5, delay: i * 0.15 }}
              className="bg-bg-card rounded-2xl p-8 border border-white/5 hover:border-accent-red/30 transition-all duration-300 group"
            >
              <div className="w-12 h-12 rounded-xl bg-accent-red/10 flex items-center justify-center mb-6 group-hover:bg-accent-red/20 transition-colors">
                <card.icon size={24} className="text-accent-red" />
              </div>
              <div className="text-4xl font-bold text-text-primary mb-2">
                {card.special ? (
                  <span className="text-accent-red">&infin;</span>
                ) : (
                  <AnimatedCounter
                    value={card.number + card.unit}
                    className="text-text-primary"
                  />
                )}
              </div>
              <div className="text-sm font-medium text-text-secondary mb-1">
                {card.label}
              </div>
              <div className="text-sm text-text-muted">{card.sub}</div>
            </motion.div>
          ))}
        </div>
      </div>
    </section>
  );
}
