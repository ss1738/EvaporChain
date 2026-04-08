"use client";

import { motion } from "framer-motion";
import { Check } from "lucide-react";

const phases = [
  {
    title: "Research & Prototype",
    status: "complete",
    items: "Whitepaper \u00B7 Benchmark prototype \u00B7 Core architecture",
  },
  {
    title: "Testnet",
    status: "active",
    items: "Smart contracts \u00B7 Multi-node network \u00B7 Developer tools",
  },
  {
    title: "Audit & Harden",
    status: "upcoming",
    items: "Security audits \u00B7 Move VM integration \u00B7 Public testnet",
  },
  {
    title: "Mainnet Genesis",
    status: "upcoming",
    items: "Mainnet launch \u00B7 Ecosystem growth \u00B7 Governance activation",
  },
];

function StatusDot({ status }: { status: string }) {
  if (status === "complete") {
    return (
      <div className="w-8 h-8 rounded-full bg-accent-green/20 border-2 border-accent-green flex items-center justify-center">
        <Check size={14} className="text-accent-green" />
      </div>
    );
  }
  if (status === "active") {
    return (
      <div className="relative w-8 h-8 rounded-full bg-accent-cyan/20 border-2 border-accent-cyan flex items-center justify-center">
        <div className="w-2.5 h-2.5 rounded-full bg-accent-cyan animate-pulse-glow" />
      </div>
    );
  }
  return (
    <div className="w-8 h-8 rounded-full bg-white/5 border-2 border-white/20" />
  );
}

export default function Roadmap() {
  return (
    <section id="roadmap" className="py-32 px-6 bg-[#0c0c14]">
      <div className="max-w-5xl mx-auto">
        <motion.h2
          initial={{ opacity: 0, y: 30 }}
          whileInView={{ opacity: 1, y: 0 }}
          viewport={{ once: true }}
          transition={{ duration: 0.6 }}
          className="text-3xl sm:text-4xl font-bold text-center mb-16"
        >
          The Path Forward
        </motion.h2>

        {/* Desktop horizontal */}
        <div className="hidden md:block">
          <div className="relative">
            {/* Timeline line */}
            <div className="absolute top-4 left-0 right-0 h-px">
              <div
                className="h-full"
                style={{
                  background: "linear-gradient(90deg, #22c55e, #00f0ff, #ffffff20, #ffffff10)",
                }}
              />
            </div>

            <div className="grid grid-cols-4 gap-6">
              {phases.map((phase, i) => (
                <motion.div
                  key={phase.title}
                  initial={{ opacity: 0, y: 20 }}
                  whileInView={{ opacity: 1, y: 0 }}
                  viewport={{ once: true }}
                  transition={{ duration: 0.5, delay: i * 0.15 }}
                >
                  <StatusDot status={phase.status} />
                  <div className="mt-6">
                    <div className="text-xs font-semibold tracking-wider text-text-muted uppercase mb-1">
                      Phase {i + 1}
                    </div>
                    <h3 className="text-base font-semibold mb-2">{phase.title}</h3>
                    <p className="text-sm text-text-secondary leading-relaxed">
                      {phase.items}
                    </p>
                  </div>
                </motion.div>
              ))}
            </div>
          </div>
        </div>

        {/* Mobile vertical */}
        <div className="md:hidden space-y-8">
          {phases.map((phase, i) => (
            <motion.div
              key={phase.title}
              initial={{ opacity: 0, x: -20 }}
              whileInView={{ opacity: 1, x: 0 }}
              viewport={{ once: true }}
              transition={{ duration: 0.5, delay: i * 0.1 }}
              className="flex gap-4"
            >
              <div className="flex flex-col items-center">
                <StatusDot status={phase.status} />
                {i < phases.length - 1 && (
                  <div className="w-px flex-1 mt-2 bg-white/10" />
                )}
              </div>
              <div className="pb-8">
                <div className="text-xs font-semibold tracking-wider text-text-muted uppercase mb-1">
                  Phase {i + 1}
                </div>
                <h3 className="text-base font-semibold mb-1">{phase.title}</h3>
                <p className="text-sm text-text-secondary">{phase.items}</p>
              </div>
            </motion.div>
          ))}
        </div>
      </div>
    </section>
  );
}
