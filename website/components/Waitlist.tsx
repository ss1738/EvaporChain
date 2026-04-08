"use client";

import { motion } from "framer-motion";
import { useState } from "react";

export default function Waitlist() {
  const [email, setEmail] = useState("");
  const [submitted, setSubmitted] = useState(false);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!email || !email.includes("@")) return;

    try {
      const existing = JSON.parse(localStorage.getItem("evaporchain_waitlist") || "[]");
      if (!existing.includes(email)) {
        existing.push(email);
        localStorage.setItem("evaporchain_waitlist", JSON.stringify(existing));
      }
    } catch {
      // localStorage not available
    }

    setSubmitted(true);
    setEmail("");
  };

  return (
    <section
      id="waitlist"
      className="relative py-32 px-6 overflow-hidden"
    >
      <div className="absolute inset-0 bg-gradient-to-br from-[#0f0a1a] via-[#0a0a0f] to-[#0a0f14]" />
      <div className="absolute inset-0 bg-[radial-gradient(circle_at_50%_50%,rgba(139,92,246,0.06),transparent_60%)]" />

      <div className="relative max-w-lg mx-auto text-center">
        <motion.div
          initial={{ opacity: 0, y: 30 }}
          whileInView={{ opacity: 1, y: 0 }}
          viewport={{ once: true }}
          transition={{ duration: 0.6 }}
        >
          <h2 className="text-3xl sm:text-4xl font-bold">
            Be First on the Testnet
          </h2>
          <p className="mt-4 text-text-secondary">
            Get early access when the testnet launches. No financial promises.
            Just technology.
          </p>
        </motion.div>

        <motion.form
          initial={{ opacity: 0, y: 20 }}
          whileInView={{ opacity: 1, y: 0 }}
          viewport={{ once: true }}
          transition={{ duration: 0.6, delay: 0.2 }}
          onSubmit={handleSubmit}
          className="mt-10 flex gap-0 max-w-md mx-auto"
        >
          <input
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="Enter your email"
            required
            className="flex-1 bg-white/10 border border-white/20 rounded-l-full px-6 py-3 text-sm text-text-primary placeholder:text-text-muted focus:outline-none focus:border-accent-cyan/50 transition-colors"
          />
          <button
            type="submit"
            className="gradient-bg px-6 py-3 rounded-r-full text-sm font-medium text-bg-primary hover:shadow-[0_0_20px_rgba(0,240,255,0.3)] transition-shadow"
          >
            Join
          </button>
        </motion.form>

        {submitted && (
          <motion.p
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="mt-4 text-accent-green text-sm"
          >
            &#10003; You&apos;re on the list
          </motion.p>
        )}

        <p className="mt-6 text-text-muted text-xs">
          Join developers and researchers building the future of sustainable
          blockchain.
        </p>
      </div>
    </section>
  );
}
