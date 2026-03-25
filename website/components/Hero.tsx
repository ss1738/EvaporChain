"use client";

import { motion } from "framer-motion";
import { ChevronDown } from "lucide-react";
import dynamic from "next/dynamic";

const ParticleBackground = dynamic(() => import("./ParticleBackground"), {
  ssr: false,
});

export default function Hero() {
  return (
    <section
      id="home"
      className="relative min-h-screen flex items-center justify-center overflow-hidden"
    >
      <ParticleBackground />

      <div className="absolute inset-0 bg-[radial-gradient(circle_at_50%_50%,rgba(0,240,255,0.04),transparent_70%)]" />

      <div className="relative z-10 max-w-4xl mx-auto px-6 text-center">
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.6 }}
          className="mb-6"
        >
          <span className="inline-block text-xs font-medium tracking-[0.2em] uppercase gradient-text">
            Next-Generation L1 Blockchain
          </span>
        </motion.div>

        <motion.h1
          initial={{ opacity: 0, y: 30 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.8 }}
          className="text-5xl sm:text-6xl md:text-7xl font-bold leading-[1.1] tracking-tight"
        >
          The Blockchain That Gets{" "}
          <br className="hidden sm:block" />
          <span className="gradient-text">Lighter</span> Over Time
        </motion.h1>

        <motion.p
          initial={{ opacity: 0, y: 30 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.8, delay: 0.2 }}
          className="mt-6 text-lg md:text-xl text-text-secondary max-w-2xl mx-auto leading-relaxed"
        >
          State decays. Objects evaporate. The chain compresses to a single
          proof. Welcome to thermodynamic blockchain architecture.
        </motion.p>

        <motion.div
          initial={{ opacity: 0, y: 30 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.8, delay: 0.4 }}
          className="mt-10 flex flex-col sm:flex-row gap-4 justify-center"
        >
          <a
            href="/whitepaper"
            className="px-8 py-3 rounded-full border border-white/20 text-text-primary hover:border-accent-cyan hover:text-accent-cyan transition-all duration-300 text-sm font-medium"
          >
            Read Whitepaper
          </a>
          <a
            href="https://testnet.evaporchain.com"
            className="gradient-bg px-8 py-3 rounded-full text-bg-primary text-sm font-medium hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-shadow duration-300"
          >
            Try the Testnet
          </a>
        </motion.div>
      </div>

      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ delay: 1.2, duration: 0.8 }}
        className="absolute bottom-8 left-1/2 -translate-x-1/2"
      >
        <a href="#problem">
          <ChevronDown
            size={28}
            className="text-text-muted animate-bounce-slow"
          />
        </a>
      </motion.div>
    </section>
  );
}
