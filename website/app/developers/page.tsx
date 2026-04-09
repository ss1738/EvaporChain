"use client";

import { motion } from "framer-motion";
import Link from "next/link";
import Navbar from "@/components/Navbar";
import Footer from "@/components/Footer";
import {
  BookOpen,
  Code,
  Server,
  Zap,
  Layers,
  ArrowRight,
  Terminal,
  GitBranch,
  Box,
  Ghost,
  Flame,
  RotateCcw,
  MessageSquare,
  Image,
  Vote,
  Droplets,
  CheckCircle,
  Copy,
} from "lucide-react";
import { useState } from "react";

const fade = (delay: number) => ({
  initial: { opacity: 0, y: 20 },
  whileInView: { opacity: 1, y: 0 },
  viewport: { once: true },
  transition: { duration: 0.5, delay },
});

const QUICK_LINKS = [
  {
    icon: Server,
    title: "API Reference",
    description: "All REST endpoints — chain status, objects, NFTs, staking, swap, messages, and more.",
    href: "/developers/api",
    color: "accent-cyan",
  },
  {
    icon: Code,
    title: "SDK Reference",
    description: "TypeScript SDK with wallet provider, API client, and 10 React hooks.",
    href: "/developers/sdk",
    color: "accent-purple",
  },
  {
    icon: BookOpen,
    title: "Whitepaper",
    description: "Full technical paper covering thermodynamic state decay, consensus, and ZK proofs.",
    href: "/whitepaper",
    color: "accent-green",
  },
  {
    icon: Layers,
    title: "Explorer",
    description: "Live testnet explorer with block data, objects, validators, and decay visualization.",
    href: "/explorer",
    color: "accent-amber",
  },
];

const LIFECYCLE_STEPS = [
  {
    icon: Flame,
    state: "Active",
    energy: "100% → 50%",
    color: "accent-cyan",
    bgColor: "bg-accent-cyan/10",
    description: "Object is fully alive. Energy decays exponentially: E(t) = E₀ × 2^(-t/halfLife). Readable and writable by its owner.",
  },
  {
    icon: Zap,
    state: "Grace",
    energy: "50% → 10%",
    color: "accent-amber",
    bgColor: "bg-accent-amber/10",
    description: "Energy below threshold. Object is read-only. Owner can refresh with an energy deposit to return to Active.",
  },
  {
    icon: Ghost,
    state: "Ghost",
    energy: "< 10%",
    color: "accent-red",
    bgColor: "bg-accent-red/10",
    description: "Nearly evaporated. Data is archived to a ZK-compressed proof. Can be resurrected (Risen) with sufficient energy.",
  },
  {
    icon: RotateCcw,
    state: "Risen",
    energy: "Restored",
    color: "accent-purple",
    bgColor: "bg-accent-purple/10",
    description: "Resurrected from Ghost state. Data restored from ZK proof. A fresh energy deposit starts a new decay cycle.",
  },
];

const DAPPS = [
  {
    icon: Droplets,
    name: "Energy Pools",
    description: "Community-funded energy pools that collectively keep important objects alive. Stake EVAP into pools, earn rewards, and vote on which objects to protect.",
    features: ["Pool creation & staking", "Contributor leaderboard", "Activity feed"],
    port: 5174,
    path: "dapps/energy-pool",
    color: "accent-cyan",
  },
  {
    icon: MessageSquare,
    name: "Mortal Messages",
    description: "Ephemeral messaging where messages decay over time. Boost energy to keep important conversations alive, or let them evaporate naturally.",
    features: ["Send & receive messages", "Energy boost", "Decay monitoring"],
    port: 5175,
    path: "dapps/mortal-messages",
    color: "accent-purple",
  },
  {
    icon: Image,
    name: "NFT Marketplace",
    description: "Mint, trade, and manage mortal NFTs. Every NFT has energy that decays — collectors must actively curate their collections or lose them.",
    features: ["Mint & transfer NFTs", "Collection browser", "Energy refresh"],
    port: 5176,
    path: "dapps/nft-marketplace",
    color: "accent-green",
  },
  {
    icon: Vote,
    name: "Governance",
    description: "DAO governance where proposals decay. Community must actively boost proposals they care about — naturally cleaning up governance debt.",
    features: ["Create proposals", "Vote & boost energy", "Delegation"],
    port: 5177,
    path: "dapps/governance",
    color: "accent-amber",
  },
];

const GETTING_STARTED_CODE = `# 1. Clone the repo
git clone https://github.com/ss1738/EvaporChain.git
cd EvaporChain

# 2. Install the SDK
cd wallet-sdk && npm install && npm run build && cd ..

# 3. Run a reference dApp
cd dapps/energy-pool && npm install && npm run dev

# Open http://localhost:5174`;

const CONNECT_CODE = `import { useEvaporChain } from "@evaporchain/wallet-sdk/react";

function MyDApp() {
  const { address, balance, connected, connect, disconnect } = useEvaporChain();

  if (!connected) {
    return <button onClick={connect}>Connect Wallet</button>;
  }

  return (
    <div>
      <p>Address: {address}</p>
      <p>Balance: {balance} EVAP</p>
      <button onClick={disconnect}>Disconnect</button>
    </div>
  );
}`;

const OBJECT_CODE = `import { EvaporChainAPI } from "@evaporchain/wallet-sdk/api";

const api = new EvaporChainAPI({ network: "testnet" });

// Fetch all objects owned by an address
const objects = await api.getObjects("0x1234...");

// Check energy levels
for (const obj of objects) {
  const pct = (obj.currentEnergy / obj.maxEnergy) * 100;
  console.log(\`\${obj.name}: \${pct.toFixed(1)}% energy (\${obj.state})\`);

  // Refresh objects below 30% energy
  if (pct < 30) {
    await api.refreshObject(obj.id, 1000);
  }
}`;

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <button
      onClick={handleCopy}
      className="text-text-muted hover:text-accent-cyan transition-colors"
      title="Copy to clipboard"
    >
      {copied ? <CheckCircle size={14} /> : <Copy size={14} />}
    </button>
  );
}

function CodeBlock({ code, filename, language }: { code: string; filename: string; language: string }) {
  return (
    <div className="bg-bg-card border border-white/5 rounded-xl overflow-hidden">
      <div className="flex items-center justify-between px-4 py-2 border-b border-white/5">
        <div className="flex items-center gap-2">
          <div className="w-2.5 h-2.5 rounded-full bg-accent-red/60" />
          <div className="w-2.5 h-2.5 rounded-full bg-accent-amber/60" />
          <div className="w-2.5 h-2.5 rounded-full bg-accent-green/60" />
        </div>
        <div className="flex items-center gap-3">
          <span className="text-[10px] text-text-muted font-mono">{filename}</span>
          <CopyButton text={code} />
        </div>
      </div>
      <pre className="p-5 text-sm text-text-secondary overflow-x-auto leading-relaxed">
        <code className={`language-${language}`}>{code}</code>
      </pre>
    </div>
  );
}

export default function DevelopersPage() {
  return (
    <>
      <Navbar />
      <main className="pt-24 pb-0">
        {/* Hero */}
        <section className="px-6 py-20">
          <div className="max-w-4xl mx-auto text-center">
            <motion.div {...fade(0)}>
              <p className="text-accent-cyan text-sm font-medium tracking-widest uppercase mb-4">
                Developer Portal
              </p>
              <h1 className="text-4xl md:text-6xl font-bold mb-6 leading-[1.1]">
                <span className="gradient-text">Build on the Chain</span>
                <br />
                <span className="text-text-primary">That Evaporates.</span>
              </h1>
              <p className="text-lg text-text-secondary max-w-2xl mx-auto mb-4">
                Everything you need to build dApps on EvaporChain — SDK, API docs,
                reference apps, and core concepts for energy-based state decay.
              </p>
              <p className="text-sm text-text-muted max-w-xl mx-auto">
                Objects decay. State compresses. Governance evaporates. Build applications
                that embrace thermodynamic blockchain architecture.
              </p>
            </motion.div>

            <motion.div {...fade(0.3)} className="flex flex-wrap items-center justify-center gap-4 mt-10">
              <a
                href="#getting-started"
                className="gradient-bg text-bg-primary font-semibold px-8 py-3 rounded-full hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-shadow"
              >
                Get Started &rarr;
              </a>
              <Link
                href="/developers/api"
                className="border border-white/20 text-text-primary px-8 py-3 rounded-full hover:border-accent-cyan/40 transition-colors"
              >
                API Reference
              </Link>
            </motion.div>
          </div>
        </section>

        {/* Quick Links */}
        <section className="px-6 py-16 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <div className="grid md:grid-cols-2 lg:grid-cols-4 gap-4">
              {QUICK_LINKS.map((link, i) => (
                <motion.div key={link.title} {...fade(i * 0.08)}>
                  <Link
                    href={link.href}
                    className="block bg-bg-card border border-white/5 rounded-xl p-5 hover:border-white/15 transition-all group h-full"
                  >
                    <div className={`w-10 h-10 rounded-lg bg-${link.color}/10 flex items-center justify-center mb-3`}>
                      <link.icon size={18} className={`text-${link.color}`} />
                    </div>
                    <h3 className="text-sm font-semibold text-text-primary mb-1 group-hover:text-accent-cyan transition-colors">
                      {link.title} <ArrowRight size={12} className="inline ml-1 opacity-0 group-hover:opacity-100 transition-opacity" />
                    </h3>
                    <p className="text-xs text-text-muted leading-relaxed">{link.description}</p>
                  </Link>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Core Concepts: Energy Decay */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <motion.div {...fade(0)} className="text-center mb-12">
              <h2 className="text-2xl md:text-3xl font-bold">Core Concept: Energy Decay</h2>
              <p className="text-sm text-text-muted mt-2 max-w-lg mx-auto">
                Every object on EvaporChain has energy that decays exponentially.
                Understanding this model is key to building on the chain.
              </p>
            </motion.div>

            {/* Decay Formula */}
            <motion.div {...fade(0.1)} className="mb-10">
              <div className="bg-bg-card border border-white/5 rounded-2xl p-8 text-center">
                <p className="text-text-muted text-xs uppercase tracking-widest mb-4">Decay Formula</p>
                <p className="text-3xl md:text-4xl font-mono font-bold gradient-text mb-4">
                  E(t) = E₀ × 2<sup>−t/halfLife</sup>
                </p>
                <div className="grid md:grid-cols-3 gap-6 mt-8 text-left max-w-2xl mx-auto">
                  <div>
                    <p className="text-xs text-accent-cyan font-mono font-semibold mb-1">E(t)</p>
                    <p className="text-xs text-text-muted">Energy at epoch t. When it hits zero, the object evaporates.</p>
                  </div>
                  <div>
                    <p className="text-xs text-accent-purple font-mono font-semibold mb-1">E₀</p>
                    <p className="text-xs text-text-muted">Initial energy deposited at creation. More energy = longer life.</p>
                  </div>
                  <div>
                    <p className="text-xs text-accent-amber font-mono font-semibold mb-1">halfLife</p>
                    <p className="text-xs text-text-muted">Epochs for energy to halve. Configurable per object (50–500 epochs).</p>
                  </div>
                </div>
              </div>
            </motion.div>

            {/* Object Lifecycle */}
            <motion.div {...fade(0.2)} className="mb-6">
              <h3 className="text-lg font-semibold text-text-primary text-center mb-6">Object Lifecycle</h3>
            </motion.div>

            <div className="grid md:grid-cols-4 gap-4">
              {LIFECYCLE_STEPS.map((step, i) => (
                <motion.div
                  key={step.state}
                  {...fade(0.2 + i * 0.08)}
                  className="bg-bg-card border border-white/5 rounded-xl p-5 relative"
                >
                  {i < LIFECYCLE_STEPS.length - 1 && (
                    <div className="hidden md:block absolute right-0 top-1/2 translate-x-1/2 -translate-y-1/2 z-10">
                      <ArrowRight size={16} className="text-text-muted" />
                    </div>
                  )}
                  <div className={`w-9 h-9 rounded-lg ${step.bgColor} flex items-center justify-center mb-3`}>
                    <step.icon size={16} className={`text-${step.color}`} />
                  </div>
                  <h4 className={`text-sm font-semibold text-${step.color} mb-0.5`}>{step.state}</h4>
                  <p className="text-[10px] text-text-muted font-mono mb-2">{step.energy}</p>
                  <p className="text-xs text-text-muted leading-relaxed">{step.description}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Getting Started */}
        <section id="getting-started" className="px-6 py-20 border-t border-white/5">
          <div className="max-w-4xl mx-auto">
            <motion.div {...fade(0)} className="text-center mb-12">
              <h2 className="text-2xl md:text-3xl font-bold">Getting Started</h2>
              <p className="text-sm text-text-muted mt-2 max-w-lg mx-auto">
                Clone, install, and run a reference dApp in under 2 minutes.
              </p>
            </motion.div>

            <div className="space-y-6">
              {/* Step 1: Setup */}
              <motion.div {...fade(0.1)}>
                <div className="flex items-center gap-3 mb-3">
                  <div className="w-8 h-8 rounded-lg gradient-bg flex items-center justify-center shrink-0">
                    <span className="text-xs font-bold text-bg-primary">1</span>
                  </div>
                  <div>
                    <h3 className="text-sm font-semibold text-text-primary">Clone & Run</h3>
                    <p className="text-xs text-text-muted">Get a reference dApp running locally</p>
                  </div>
                </div>
                <CodeBlock code={GETTING_STARTED_CODE} filename="terminal" language="bash" />
              </motion.div>

              {/* Step 2: Connect Wallet */}
              <motion.div {...fade(0.2)}>
                <div className="flex items-center gap-3 mb-3">
                  <div className="w-8 h-8 rounded-lg gradient-bg flex items-center justify-center shrink-0">
                    <span className="text-xs font-bold text-bg-primary">2</span>
                  </div>
                  <div>
                    <h3 className="text-sm font-semibold text-text-primary">Connect a Wallet</h3>
                    <p className="text-xs text-text-muted">Use the SDK&apos;s React hook for wallet connection</p>
                  </div>
                </div>
                <CodeBlock code={CONNECT_CODE} filename="App.tsx" language="tsx" />
              </motion.div>

              {/* Step 3: Read Chain Data */}
              <motion.div {...fade(0.3)}>
                <div className="flex items-center gap-3 mb-3">
                  <div className="w-8 h-8 rounded-lg gradient-bg flex items-center justify-center shrink-0">
                    <span className="text-xs font-bold text-bg-primary">3</span>
                  </div>
                  <div>
                    <h3 className="text-sm font-semibold text-text-primary">Read & Manage Objects</h3>
                    <p className="text-xs text-text-muted">Use the API client for chain data — no wallet needed for reads</p>
                  </div>
                </div>
                <CodeBlock code={OBJECT_CODE} filename="monitor.ts" language="typescript" />
              </motion.div>
            </div>
          </div>
        </section>

        {/* Reference dApps */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <motion.div {...fade(0)} className="text-center mb-12">
              <h2 className="text-2xl md:text-3xl font-bold">Reference dApps</h2>
              <p className="text-sm text-text-muted mt-2 max-w-lg mx-auto">
                Four production-grade dApps demonstrating every major feature of EvaporChain.
                Fork them as a starting point for your own project.
              </p>
            </motion.div>

            <div className="grid md:grid-cols-2 gap-6">
              {DAPPS.map((dapp, i) => (
                <motion.div
                  key={dapp.name}
                  {...fade(i * 0.1)}
                  className="bg-bg-card border border-white/5 rounded-2xl p-6 hover:border-white/10 transition-colors"
                >
                  <div className="flex items-start justify-between mb-4">
                    <div className={`w-11 h-11 rounded-xl bg-${dapp.color}/10 flex items-center justify-center`}>
                      <dapp.icon size={20} className={`text-${dapp.color}`} />
                    </div>
                    <span className="text-[10px] text-text-muted font-mono bg-white/5 px-2 py-1 rounded">
                      localhost:{dapp.port}
                    </span>
                  </div>

                  <h3 className="text-base font-semibold text-text-primary mb-1">{dapp.name}</h3>
                  <p className="text-xs text-text-muted leading-relaxed mb-4">{dapp.description}</p>

                  <ul className="space-y-1.5 mb-5">
                    {dapp.features.map((f) => (
                      <li key={f} className="flex items-center gap-2">
                        <CheckCircle size={10} className={`text-${dapp.color} shrink-0`} />
                        <span className="text-xs text-text-secondary">{f}</span>
                      </li>
                    ))}
                  </ul>

                  <div className="flex items-center gap-3">
                    <a
                      href={`https://github.com/ss1738/EvaporChain/tree/main/${dapp.path}`}
                      target="_blank"
                      rel="noopener noreferrer"
                      className={`text-xs font-medium text-${dapp.color} hover:underline`}
                    >
                      View Source <ArrowRight size={12} className="inline ml-0.5" />
                    </a>
                    <span className="text-text-muted text-[10px]">|</span>
                    <span className="text-xs text-text-muted font-mono">
                      cd {dapp.path} && npm run dev
                    </span>
                  </div>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Architecture Overview */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-4xl mx-auto">
            <motion.div {...fade(0)} className="text-center mb-12">
              <h2 className="text-2xl md:text-3xl font-bold">Architecture</h2>
              <p className="text-sm text-text-muted mt-2 max-w-lg mx-auto">
                How the pieces fit together — from your dApp to the chain.
              </p>
            </motion.div>

            <motion.div {...fade(0.1)}>
              <div className="bg-bg-card border border-white/5 rounded-2xl p-8">
                <div className="space-y-3">
                  {[
                    { label: "Your dApp", detail: "React / Next.js / any framework", color: "accent-cyan", sub: "Imports SDK hooks and API client" },
                    { label: "Wallet SDK", detail: "@evaporchain/wallet-sdk", color: "accent-purple", sub: "Provider wrapper + REST API client + React hooks" },
                    { label: "Browser Extension", detail: "window.evaporchain provider", color: "accent-green", sub: "ML-DSA signing, key storage, transaction approval" },
                    { label: "REST API", detail: "testnet.evaporchain.com/api/*", color: "accent-amber", sub: "Chain data, transactions, objects, NFTs, staking" },
                    { label: "EvaporChain Node", detail: "Rust consensus + state engine", color: "accent-red", sub: "Thermodynamic decay, ZK compression, ML-DSA verification" },
                  ].map((layer, i) => (
                    <div key={layer.label} className="flex items-center gap-4">
                      <div className={`w-10 h-10 rounded-lg bg-${layer.color}/10 flex items-center justify-center shrink-0`}>
                        <span className={`text-xs font-bold text-${layer.color}`}>{i + 1}</span>
                      </div>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-baseline gap-2">
                          <p className="text-sm font-semibold text-text-primary">{layer.label}</p>
                          <p className="text-[10px] text-text-muted font-mono truncate">{layer.detail}</p>
                        </div>
                        <p className="text-xs text-text-muted">{layer.sub}</p>
                      </div>
                    </div>
                  ))}
                </div>

                <div className="mt-6 pt-6 border-t border-white/5">
                  <div className="grid grid-cols-3 gap-4 text-center">
                    <div>
                      <p className="text-lg font-bold text-accent-cyan">ML-DSA</p>
                      <p className="text-[10px] text-text-muted">Post-quantum signatures</p>
                    </div>
                    <div>
                      <p className="text-lg font-bold text-accent-purple">BLAKE3</p>
                      <p className="text-[10px] text-text-muted">Address derivation</p>
                    </div>
                    <div>
                      <p className="text-lg font-bold text-accent-green">ZK Proofs</p>
                      <p className="text-[10px] text-text-muted">State compression</p>
                    </div>
                  </div>
                </div>
              </div>
            </motion.div>
          </div>
        </section>

        {/* Key Differences */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-4xl mx-auto">
            <motion.div {...fade(0)} className="text-center mb-12">
              <h2 className="text-2xl md:text-3xl font-bold">What Makes It Different</h2>
              <p className="text-sm text-text-muted mt-2 max-w-lg mx-auto">
                If you&apos;ve built on Ethereum or Solana, here&apos;s what changes on EvaporChain.
              </p>
            </motion.div>

            <motion.div {...fade(0.1)}>
              <div className="space-y-4">
                {[
                  {
                    traditional: "State persists forever",
                    evaporchain: "State decays — objects have energy that depletes over time",
                    icon: Flame,
                  },
                  {
                    traditional: "One-time storage fee",
                    evaporchain: "Continuous energy cost — refreshing objects extends their life",
                    icon: Zap,
                  },
                  {
                    traditional: "ECDSA / Ed25519 signatures",
                    evaporchain: "ML-DSA (Dilithium3) — quantum-resistant from day one",
                    icon: GitBranch,
                  },
                  {
                    traditional: "State bloat grows forever",
                    evaporchain: "Evaporated state compresses into ZK proofs — chain gets lighter",
                    icon: Box,
                  },
                  {
                    traditional: "Smart contracts are eternal",
                    evaporchain: "Governance proposals decay — only active participation survives",
                    icon: Terminal,
                  },
                ].map((diff, i) => (
                  <motion.div
                    key={i}
                    initial={{ opacity: 0, x: -20 }}
                    whileInView={{ opacity: 1, x: 0 }}
                    viewport={{ once: true }}
                    transition={{ duration: 0.4, delay: i * 0.08 }}
                    className="bg-bg-card border border-white/5 rounded-xl p-4 flex items-start gap-4"
                  >
                    <div className="w-9 h-9 rounded-lg bg-accent-cyan/10 flex items-center justify-center shrink-0 mt-0.5">
                      <diff.icon size={16} className="text-accent-cyan" />
                    </div>
                    <div className="flex-1 min-w-0">
                      <p className="text-xs text-text-muted line-through mb-1">{diff.traditional}</p>
                      <p className="text-sm text-text-primary">{diff.evaporchain}</p>
                    </div>
                  </motion.div>
                ))}
              </div>
            </motion.div>
          </div>
        </section>

        {/* CTA */}
        <section className="px-6 py-20 border-t border-white/5">
          <div className="max-w-3xl mx-auto text-center">
            <motion.div {...fade(0)}>
              <h2 className="text-2xl md:text-3xl font-bold mb-4">
                Start Building
              </h2>
              <p className="text-sm text-text-muted mb-8">
                The testnet is live. Install the SDK, fork a reference dApp,
                and ship your first mortal application.
              </p>
              <div className="flex flex-col sm:flex-row items-center justify-center gap-4">
                <code className="text-sm text-accent-cyan font-mono bg-white/5 px-5 py-2.5 rounded-lg">
                  npm install @evaporchain/wallet-sdk
                </code>
              </div>
              <div className="flex flex-wrap items-center justify-center gap-4 mt-6">
                <Link
                  href="/developers/api"
                  className="gradient-bg text-bg-primary font-semibold px-8 py-3 rounded-full hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-shadow"
                >
                  API Reference &rarr;
                </Link>
                <Link
                  href="/developers/sdk"
                  className="border border-white/20 text-text-primary px-8 py-3 rounded-full hover:border-accent-cyan/40 transition-colors"
                >
                  SDK Reference
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
