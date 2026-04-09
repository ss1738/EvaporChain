"use client";

import { motion } from "framer-motion";
import Link from "next/link";
import Navbar from "@/components/Navbar";
import Footer from "@/components/Footer";
import {
  Shield,
  Zap,
  Smartphone,
  Globe,
  Code,
  Lock,
  Eye,
  RefreshCw,
  Bell,
  Layers,
  ArrowRight,
  CheckCircle,
} from "lucide-react";

const fade = (delay: number) => ({
  initial: { opacity: 0, y: 20 },
  whileInView: { opacity: 1, y: 0 },
  viewport: { once: true },
  transition: { duration: 0.5, delay },
});

const PLATFORMS = [
  {
    icon: Globe,
    title: "Browser Extension",
    subtitle: "Chrome / Brave / Edge",
    description:
      "Full-featured wallet with ML-DSA signing via WASM, dApp provider injection, and real-time decay forecasting. 36 components including NFT gallery, bridge, and Ledger support.",
    features: [
      "ML-DSA post-quantum signatures (WASM)",
      "dApp provider (window.evaporchain)",
      "Decay forecasting & alerts",
      "Ghost recovery tools",
      "Plugin store for extensions",
    ],
    cta: "Install Extension",
    ctaHref: "https://testnet.evaporchain.com/extension",
    color: "accent-cyan",
    gradient: "from-accent-cyan to-accent-purple",
  },
  {
    icon: Smartphone,
    title: "Mobile Wallet",
    subtitle: "iOS / Android (Expo)",
    description:
      "17-screen mobile wallet with biometric auth, QR code scanning, energy dashboard, staking, token swaps, and real-time decay monitoring. Built with React Native.",
    features: [
      "Biometric auth + PIN",
      "QR code send & receive",
      "Energy dashboard with batch refresh",
      "Staking & validator selection",
      "Token swap with slippage control",
    ],
    cta: "View on GitHub",
    ctaHref: "https://github.com/ss1738/EvaporChain/tree/main/mobile-wallet",
    color: "accent-purple",
    gradient: "from-accent-purple to-accent-cyan",
  },
  {
    icon: Code,
    title: "Wallet SDK",
    subtitle: "npm — @evaporchain/wallet-sdk",
    description:
      "Zero-dependency TypeScript SDK for building dApps. Includes wallet provider, REST API client, and 10 React hooks. Used by all 3 reference dApps.",
    features: [
      "Wallet connection + signing",
      "Full REST API client",
      "10 React hooks (staking, swap, objects...)",
      "TypeScript-first with strict types",
      "Zero external dependencies",
    ],
    cta: "npm install",
    ctaHref: "https://github.com/ss1738/EvaporChain/tree/main/wallet-sdk",
    color: "accent-green",
    gradient: "from-accent-green to-accent-cyan",
  },
];

const FEATURES = [
  {
    icon: Shield,
    title: "Post-Quantum Secure",
    description:
      "Every transaction signed with ML-DSA (FIPS 204 / Dilithium3) lattice-based signatures. Your assets are quantum-resistant today, not after a migration.",
    color: "accent-cyan",
  },
  {
    icon: Zap,
    title: "Energy Dashboard",
    description:
      "Monitor energy levels across all assets in real-time. Visual health ring, urgency-sorted lists, and batch refresh for critical objects with one tap.",
    color: "accent-green",
  },
  {
    icon: Eye,
    title: "Decay Forecasting",
    description:
      "See exactly when each asset will enter Grace period or evaporate. Countdown timers and decay curves let you plan refreshes ahead of time.",
    color: "accent-amber",
  },
  {
    icon: RefreshCw,
    title: "Batch Refresh",
    description:
      "Refresh multiple objects and NFTs in a single action. The energy dashboard identifies your most critical assets and refreshes them all at once.",
    color: "accent-purple",
  },
  {
    icon: Bell,
    title: "Decay Alerts",
    description:
      "Push notifications when assets approach Grace period. Never lose an NFT or object to unexpected evaporation again.",
    color: "accent-red",
  },
  {
    icon: Lock,
    title: "Auto-Lock",
    description:
      "Configurable auto-lock timer with biometric or PIN unlock. AppState monitoring ensures the wallet locks when backgrounded.",
    color: "accent-cyan",
  },
];

const SECURITY_LAYERS = [
  { label: "Application", detail: "PIN / Biometric auth, auto-lock timer, encrypted keystore" },
  { label: "Signing", detail: "ML-DSA (FIPS 204) — 128-bit post-quantum security level" },
  { label: "Key Storage", detail: "Secure Enclave (mobile) / encrypted IndexedDB (extension)" },
  { label: "Transport", detail: "TLS 1.3 to RPC nodes, BLAKE3 address derivation" },
];

const SDK_CODE = `import { useEvaporChain, useObjects } from "@evaporchain/wallet-sdk/react";

function App() {
  const { address, balance, connected, connect } = useEvaporChain();
  const { objects } = useObjects(address);

  if (!connected) return <button onClick={connect}>Connect</button>;

  return (
    <div>
      <p>{balance} EVAP</p>
      {objects.map(obj => (
        <div key={obj.id}>
          {obj.name}: {obj.currentEnergy}/{obj.maxEnergy}
        </div>
      ))}
    </div>
  );
}`;

export default function WalletPage() {
  return (
    <>
      <Navbar />
      <main className="pt-24 pb-0">
        {/* Hero */}
        <section className="px-6 py-20">
          <div className="max-w-4xl mx-auto text-center">
            <motion.div {...fade(0)}>
              <p className="text-accent-cyan text-sm font-medium tracking-widest uppercase mb-4">
                Wallet Ecosystem
              </p>
              <h1 className="text-4xl md:text-6xl font-bold mb-6 leading-[1.1]">
                <span className="gradient-text">One Chain.</span>
                <br />
                <span className="text-text-primary">Three Wallets.</span>
              </h1>
              <p className="text-lg text-text-secondary max-w-2xl mx-auto mb-4">
                Browser extension, mobile app, and developer SDK — all built for
                EvaporChain&apos;s energy-based state model. Post-quantum secure from day one.
              </p>
              <p className="text-sm text-text-muted max-w-xl mx-auto">
                Track decay, refresh objects, stake EVAP, swap tokens, and monitor your entire
                portfolio of mortal assets across any device.
              </p>
            </motion.div>

            <motion.div {...fade(0.3)} className="flex flex-wrap items-center justify-center gap-4 mt-10">
              <a
                href="https://testnet.evaporchain.com"
                target="_blank"
                rel="noopener noreferrer"
                className="gradient-bg text-bg-primary font-semibold px-8 py-3 rounded-full hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-shadow"
              >
                Try the Testnet &rarr;
              </a>
              <Link
                href="/explorer"
                className="border border-white/20 text-text-primary px-8 py-3 rounded-full hover:border-accent-cyan/40 transition-colors"
              >
                View Explorer
              </Link>
            </motion.div>
          </div>
        </section>

        {/* Platform Cards */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-6xl mx-auto">
            <motion.div {...fade(0)} className="text-center mb-12">
              <h2 className="text-2xl md:text-3xl font-bold">Three Surfaces, One SDK</h2>
              <p className="text-sm text-text-muted mt-2 max-w-lg mx-auto">
                Every wallet surface connects to the same chain, uses the same signing
                algorithm, and speaks the same API.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-3 gap-6">
              {PLATFORMS.map((platform, i) => (
                <motion.div
                  key={platform.title}
                  {...fade(i * 0.1)}
                  className="bg-bg-card border border-white/5 rounded-2xl p-6 flex flex-col hover:border-white/10 transition-colors"
                >
                  <div className={`w-12 h-12 rounded-xl bg-${platform.color}/10 flex items-center justify-center mb-4`}>
                    <platform.icon size={22} className={`text-${platform.color}`} />
                  </div>

                  <h3 className="text-lg font-semibold text-text-primary">{platform.title}</h3>
                  <p className="text-xs text-text-muted mb-3">{platform.subtitle}</p>
                  <p className="text-sm text-text-secondary leading-relaxed mb-4 flex-1">
                    {platform.description}
                  </p>

                  <ul className="space-y-2 mb-6">
                    {platform.features.map((f) => (
                      <li key={f} className="flex items-start gap-2">
                        <CheckCircle size={12} className={`text-${platform.color} mt-0.5 shrink-0`} />
                        <span className="text-xs text-text-secondary">{f}</span>
                      </li>
                    ))}
                  </ul>

                  <a
                    href={platform.ctaHref}
                    target="_blank"
                    rel="noopener noreferrer"
                    className={`text-center text-sm font-medium py-2.5 rounded-xl border border-${platform.color}/30 text-${platform.color} hover:bg-${platform.color}/5 transition-colors`}
                  >
                    {platform.cta} <ArrowRight size={14} className="inline ml-1" />
                  </a>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Features Grid */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <motion.div {...fade(0)} className="text-center mb-12">
              <h2 className="text-2xl md:text-3xl font-bold">Built for Decay</h2>
              <p className="text-sm text-text-muted mt-2 max-w-lg mx-auto">
                Features you won&apos;t find in any other blockchain wallet — because no other
                chain has energy-based state decay.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-6">
              {FEATURES.map((feature, i) => (
                <motion.div
                  key={feature.title}
                  {...fade(i * 0.08)}
                  className="bg-bg-card border border-white/5 rounded-xl p-5 hover:border-white/10 transition-colors"
                >
                  <div className={`w-9 h-9 rounded-lg bg-${feature.color}/10 flex items-center justify-center mb-3`}>
                    <feature.icon size={16} className={`text-${feature.color}`} />
                  </div>
                  <h3 className="text-sm font-semibold text-text-primary mb-1">{feature.title}</h3>
                  <p className="text-xs text-text-muted leading-relaxed">{feature.description}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Security Architecture */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-4xl mx-auto">
            <motion.div {...fade(0)} className="text-center mb-12">
              <h2 className="text-2xl md:text-3xl font-bold">Security Architecture</h2>
              <p className="text-sm text-text-muted mt-2 max-w-lg mx-auto">
                Four layers of protection. Post-quantum signatures at the core.
                Private keys never leave the device.
              </p>
            </motion.div>

            <div className="space-y-3">
              {SECURITY_LAYERS.map((layer, i) => (
                <motion.div
                  key={layer.label}
                  initial={{ opacity: 0, x: -30 }}
                  whileInView={{ opacity: 1, x: 0 }}
                  viewport={{ once: true }}
                  transition={{ duration: 0.4, delay: i * 0.1 }}
                  className="flex items-center gap-4 bg-bg-card border border-white/5 rounded-xl p-4"
                >
                  <div className="w-10 h-10 rounded-lg gradient-bg flex items-center justify-center shrink-0">
                    <span className="text-xs font-bold text-bg-primary">{i + 1}</span>
                  </div>
                  <div>
                    <p className="text-sm font-semibold text-text-primary">{layer.label}</p>
                    <p className="text-xs text-text-muted">{layer.detail}</p>
                  </div>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* SDK Quick Start */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-4xl mx-auto">
            <motion.div {...fade(0)} className="text-center mb-8">
              <h2 className="text-2xl md:text-3xl font-bold">Build with the SDK</h2>
              <p className="text-sm text-text-muted mt-2 max-w-lg mx-auto">
                Connect to EvaporChain in under 10 lines. One package for wallet connection,
                chain data, and React hooks.
              </p>
            </motion.div>

            <motion.div {...fade(0.2)} className="mb-6">
              <div className="bg-bg-card border border-white/5 rounded-xl overflow-hidden">
                <div className="flex items-center justify-between px-4 py-2 border-b border-white/5">
                  <div className="flex items-center gap-2">
                    <div className="w-2.5 h-2.5 rounded-full bg-accent-red/60" />
                    <div className="w-2.5 h-2.5 rounded-full bg-accent-amber/60" />
                    <div className="w-2.5 h-2.5 rounded-full bg-accent-green/60" />
                  </div>
                  <span className="text-[10px] text-text-muted font-mono">App.tsx</span>
                </div>
                <pre className="p-5 text-sm text-text-secondary overflow-x-auto leading-relaxed">
                  <code>{SDK_CODE}</code>
                </pre>
              </div>
            </motion.div>

            <motion.div {...fade(0.3)} className="flex items-center justify-center gap-4">
              <code className="text-sm text-accent-cyan font-mono bg-white/5 px-4 py-2 rounded-lg">
                npm install @evaporchain/wallet-sdk
              </code>
              <a
                href="https://github.com/ss1738/EvaporChain/tree/main/wallet-sdk"
                target="_blank"
                rel="noopener noreferrer"
                className="text-sm text-text-muted hover:text-accent-cyan transition-colors"
              >
                View Docs &rarr;
              </a>
            </motion.div>
          </div>
        </section>

        {/* CTA */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-3xl mx-auto text-center">
            <motion.div {...fade(0)}>
              <h2 className="text-2xl md:text-3xl font-bold mb-4">
                Ready to explore?
              </h2>
              <p className="text-sm text-text-muted mb-8">
                The testnet is live. Create a wallet, claim tokens from the faucet, and start
                building dApps on the first blockchain where everything eventually evaporates.
              </p>
              <div className="flex flex-wrap items-center justify-center gap-4">
                <a
                  href="https://testnet.evaporchain.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="gradient-bg text-bg-primary font-semibold px-8 py-3 rounded-full hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-shadow"
                >
                  Launch Testnet &rarr;
                </a>
                <Link
                  href="/explorer"
                  className="border border-white/20 text-text-primary px-8 py-3 rounded-full hover:border-accent-cyan/40 transition-colors"
                >
                  Open Explorer
                </Link>
                <a
                  href="https://github.com/ss1738/EvaporChain"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-sm text-text-muted hover:text-accent-cyan transition-colors"
                >
                  GitHub &rarr;
                </a>
              </div>
            </motion.div>
          </div>
        </section>
      </main>
      <Footer />
    </>
  );
}
