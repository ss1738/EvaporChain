"use client";

import { motion } from "framer-motion";

const contracts = [
  {
    emoji: "\uD83D\uDCB0",
    title: "Expiring Tokens",
    template: "DecayingToken",
    description:
      "Fungible tokens where balances decay. Loyalty points, time-limited credits, circulating currencies that punish hoarding.",
  },
  {
    emoji: "\uD83C\uDFA8",
    title: "Mortal NFTs",
    template: "MortalNFT",
    description:
      "Digital assets with lifespans. Event tickets, seasonal collectibles, temporary access passes that die gracefully.",
  },
  {
    emoji: "\uD83D\uDD12",
    title: "Evaporating Escrow",
    template: "ThermodynamicEscrow",
    description:
      "Conditional payments that evaporate if unclaimed. No dead capital. No forgotten funds. Inaction has consequences.",
  },
  {
    emoji: "\u26A1",
    title: "Self-Cleaning Auctions",
    template: "DecayingAuction",
    description:
      "Auctions that auto-finalize and evaporate. Zero cleanup. Zero gas wasted on stale bids.",
  },
  {
    emoji: "\uD83C\uDFE6",
    title: "Use-It-Or-Lose-It Staking",
    template: "StakingPool",
    description:
      "Stake and earn. But claim your rewards \u2014 unclaimed rewards decay back to the pool. Active participants win.",
  },
  {
    emoji: "\uD83D\uDDF3\uFE0F",
    title: "Ephemeral Governance",
    template: "DAOVote",
    description:
      "Proposals that auto-finalize and evaporate. No permanent governance spam. Clean decision-making.",
  },
];

export default function Contracts() {
  return (
    <section id="contracts" className="py-32 px-6">
      <div className="max-w-6xl mx-auto">
        <motion.div
          initial={{ opacity: 0, y: 30 }}
          whileInView={{ opacity: 1, y: 0 }}
          viewport={{ once: true }}
          transition={{ duration: 0.6 }}
          className="text-center mb-16"
        >
          <h2 className="text-3xl sm:text-4xl font-bold">
            <span className="gradient-text">Contracts That Live and Die</span>
          </h2>
          <p className="mt-4 text-text-secondary text-lg">
            Six templates. Infinite possibilities. All thermodynamically aware.
          </p>
        </motion.div>

        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
          {contracts.map((c, i) => (
            <motion.div
              key={c.template}
              initial={{ opacity: 0, y: 20 }}
              whileInView={{ opacity: 1, y: 0 }}
              viewport={{ once: true }}
              transition={{ duration: 0.4, delay: i * 0.08 }}
              className="bg-bg-card rounded-2xl p-6 border border-white/5 hover:border-accent-cyan/20 transition-all duration-300"
            >
              <div className="text-3xl mb-4">{c.emoji}</div>
              <h3 className="text-base font-semibold mb-2">{c.title}</h3>
              <p className="text-sm text-text-secondary leading-relaxed">
                {c.description}
              </p>
            </motion.div>
          ))}
        </div>

        <motion.p
          initial={{ opacity: 0 }}
          whileInView={{ opacity: 1 }}
          viewport={{ once: true }}
          transition={{ duration: 0.6, delay: 0.5 }}
          className="mt-10 text-center text-text-muted text-sm"
        >
          Plus a rule engine for custom contract behavior. Full Move VM
          integration coming in Phase 3.
        </motion.p>
      </div>
    </section>
  );
}
