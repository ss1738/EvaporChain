"use client";

import { motion } from "framer-motion";
import Link from "next/link";
import Navbar from "@/components/Navbar";
import Footer from "@/components/Footer";
import {
  ArrowLeft,
  Code,
  Package,
  Plug,
  Server,
  ArrowRight,
  Copy,
  CheckCircle,
} from "lucide-react";
import { useState } from "react";

const fade = (delay: number) => ({
  initial: { opacity: 0, y: 20 },
  whileInView: { opacity: 1, y: 0 },
  viewport: { once: true },
  transition: { duration: 0.5, delay },
});

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const handleCopy = () => {
    navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };
  return (
    <button onClick={handleCopy} className="text-text-muted hover:text-accent-cyan transition-colors" title="Copy">
      {copied ? <CheckCircle size={12} /> : <Copy size={12} />}
    </button>
  );
}

function CodeBlock({ code, filename }: { code: string; filename: string }) {
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
        <code>{code}</code>
      </pre>
    </div>
  );
}

interface HookDoc {
  name: string;
  description: string;
  import: string;
  params?: string;
  returns: Array<{ name: string; type: string; description: string }>;
  example: string;
  requiresWallet: boolean;
}

const HOOKS: HookDoc[] = [
  {
    name: "useEvaporChain",
    description: "Primary wallet connection hook. Manages connect/disconnect lifecycle, provides address, balance, nonce, and API client instance.",
    import: `import { useEvaporChain } from "@evaporchain/wallet-sdk/react";`,
    returns: [
      { name: "connected", type: "boolean", description: "Whether wallet is connected" },
      { name: "connecting", type: "boolean", description: "Connection in progress" },
      { name: "address", type: "string | null", description: "Connected wallet address" },
      { name: "balance", type: "number", description: "EVAP balance" },
      { name: "nonce", type: "number", description: "Account nonce for transactions" },
      { name: "connect", type: "() => Promise<void>", description: "Trigger wallet connection" },
      { name: "disconnect", type: "() => void", description: "Disconnect wallet" },
      { name: "error", type: "string | null", description: "Last error message" },
      { name: "api", type: "EvaporChainAPI", description: "API client instance" },
    ],
    example: `const { address, balance, connected, connect } = useEvaporChain();

if (!connected) return <button onClick={connect}>Connect</button>;
return <p>{balance} EVAP</p>;`,
    requiresWallet: true,
  },
  {
    name: "useObjects",
    description: "Fetches and monitors decaying objects for an address. Polls automatically. No wallet required — reads from API.",
    import: `import { useObjects } from "@evaporchain/wallet-sdk/react";`,
    params: "address?: string",
    returns: [
      { name: "objects", type: "EvaporObject[]", description: "All objects owned by the address" },
      { name: "loading", type: "boolean", description: "Initial fetch in progress" },
      { name: "error", type: "string | null", description: "Error message if fetch failed" },
      { name: "refresh", type: "() => void", description: "Trigger manual re-fetch" },
    ],
    example: `const { objects, loading } = useObjects(address);

// Filter by state
const ghosts = objects.filter(o => o.state === "Ghost");
const critical = objects.filter(o => o.decayPercentage > 70);`,
    requiresWallet: false,
  },
  {
    name: "useTransactions",
    description: "Fetches transaction history for an address. Supports optional limit parameter.",
    import: `import { useTransactions } from "@evaporchain/wallet-sdk/react";`,
    params: "address?: string, limit?: number",
    returns: [
      { name: "transactions", type: "Transaction[]", description: "Transaction history" },
      { name: "loading", type: "boolean", description: "Fetching in progress" },
      { name: "error", type: "string | null", description: "Error message" },
      { name: "refresh", type: "() => void", description: "Manual re-fetch" },
    ],
    example: `const { transactions } = useTransactions(address, 20);

return transactions.map(tx => (
  <div key={tx.hash}>{tx.type}: {tx.amount} EVAP</div>
));`,
    requiresWallet: false,
  },
  {
    name: "useStaking",
    description: "Staking management hook — read staking info and perform stake/unstake/claim actions. Requires wallet for write operations.",
    import: `import { useStaking } from "@evaporchain/wallet-sdk/react";`,
    params: "address?: string",
    returns: [
      { name: "info", type: "StakingInfo | null", description: "Current staking status" },
      { name: "validators", type: "Validator[]", description: "Available validators" },
      { name: "loading", type: "boolean", description: "Fetching in progress" },
      { name: "stake", type: "(amount: number) => Promise<TxResult>", description: "Stake EVAP" },
      { name: "unstake", type: "(amount: number) => Promise<TxResult>", description: "Begin unstaking" },
      { name: "claimRewards", type: "() => Promise<TxResult>", description: "Claim rewards" },
    ],
    example: `const { info, validators, stake, claimRewards } = useStaking(address);

// Stake 5000 EVAP
await stake(5000);

// Claim pending rewards
if (info?.rewards > 0) await claimRewards();`,
    requiresWallet: true,
  },
  {
    name: "useSwap",
    description: "Token swap with quote preview and execution.",
    import: `import { useSwap } from "@evaporchain/wallet-sdk/react";`,
    returns: [
      { name: "quote", type: "SwapQuote | null", description: "Current swap quote" },
      { name: "loading", type: "boolean", description: "Quote fetching in progress" },
      { name: "getQuote", type: "(from, to, amount) => Promise<void>", description: "Fetch a swap quote" },
      { name: "execute", type: "(slippage: number) => Promise<TxResult>", description: "Execute the quoted swap" },
    ],
    example: `const { quote, getQuote, execute } = useSwap();

// Get a quote
await getQuote("EVAP", "GHOST", 1000);
console.log(quote?.amountOut); // e.g. 4850

// Execute with 1% slippage tolerance
await execute(0.01);`,
    requiresWallet: true,
  },
  {
    name: "usePools",
    description: "Energy pool management — list pools, stake, and unstake energy.",
    import: `import { usePools } from "@evaporchain/wallet-sdk/react";`,
    returns: [
      { name: "pools", type: "EnergyPool[]", description: "All energy pools" },
      { name: "loading", type: "boolean", description: "Fetching in progress" },
      { name: "stakeToPool", type: "(poolId, amount) => Promise<TxResult>", description: "Stake into a pool" },
      { name: "unstakeFromPool", type: "(poolId, amount) => Promise<TxResult>", description: "Unstake from a pool" },
      { name: "refresh", type: "() => void", description: "Manual re-fetch" },
    ],
    example: `const { pools, stakeToPool } = usePools();

// Find a pool and stake
const pool = pools.find(p => p.name === "Genesis Protection");
if (pool) await stakeToPool(pool.id, 1000);`,
    requiresWallet: true,
  },
  {
    name: "useMessages",
    description: "Mortal messaging — send messages, read inbox, boost energy.",
    import: `import { useMessages } from "@evaporchain/wallet-sdk/react";`,
    params: "address?: string",
    returns: [
      { name: "inbox", type: "MortalMessage[]", description: "Received messages" },
      { name: "sent", type: "MortalMessage[]", description: "Sent messages" },
      { name: "loading", type: "boolean", description: "Fetching in progress" },
      { name: "send", type: "(to, content, energy) => Promise<TxResult>", description: "Send a message" },
      { name: "boost", type: "(messageId, energy) => Promise<TxResult>", description: "Boost message energy" },
      { name: "refresh", type: "() => void", description: "Manual re-fetch" },
    ],
    example: `const { inbox, send, boost } = useMessages(address);

// Send a mortal message
await send("0x...", "Hello!", 500);

// Boost a dying message
const dying = inbox.find(m => m.state === "Grace");
if (dying) await boost(dying.id, 200);`,
    requiresWallet: true,
  },
  {
    name: "useCollections",
    description: "NFT collection browser — lists all collections with floor energy and counts.",
    import: `import { useCollections } from "@evaporchain/wallet-sdk/react";`,
    returns: [
      { name: "collections", type: "NftCollection[]", description: "All NFT collections" },
      { name: "loading", type: "boolean", description: "Fetching in progress" },
      { name: "error", type: "string | null", description: "Error message" },
      { name: "refresh", type: "() => void", description: "Manual re-fetch" },
    ],
    example: `const { collections } = useCollections();

return collections.map(c => (
  <div key={c.id}>
    {c.name} — {c.count} NFTs, floor: {c.floorEnergy} energy
  </div>
));`,
    requiresWallet: false,
  },
];

const API_METHODS = [
  { category: "Chain", methods: ["getChainStatus()"] },
  { category: "Accounts", methods: ["getBalance(address)"] },
  { category: "Transactions", methods: ["transfer(from, to, amount, nonce)", "getTransactions(address?, limit?)"] },
  { category: "Faucet", methods: ["claimFaucet(address)"] },
  { category: "Objects", methods: ["getObjects(owner?)", "getObject(id)", "refreshObject(id, energy)", "batchRefresh(items)", "getObjectsByState(owner, state)"] },
  { category: "NFTs", methods: ["getNFTs(owner?)", "getNFT(id)", "mintNFT(params)", "transferNFT(id, to)", "refreshNFT(id, energy)", "getCollections()"] },
  { category: "Swap", methods: ["getSwapQuote(from, to, amount)", "executeSwap(from, to, amount, slippage)"] },
  { category: "Staking", methods: ["getStakingInfo(address)", "getValidators()", "stake(from, amount, nonce)", "unstake(from, amount, nonce)", "claimRewards(from, nonce)"] },
  { category: "Energy Pools", methods: ["getPools()", "getPool(id)", "getPoolContributors(id)", "getPoolActivity(id)", "createPool(name, creator, target?)", "stakeToPool(id, address, amount)", "unstakeFromPool(id, address, amount)"] },
  { category: "Messages", methods: ["sendMessage(from, to, content, energy)", "getInbox(address)", "getSentMessages(address)", "getMessage(id)", "boostMessage(id, energy)", "getMessageStats(address)"] },
];

const INSTALL_CODE = `npm install @evaporchain/wallet-sdk`;

const PROVIDER_CODE = `import { EvaporChainProvider } from "@evaporchain/wallet-sdk";

// The Provider wraps window.evaporchain injected by the browser extension
const provider = new EvaporChainProvider();

// Connect wallet
const { address, publicKey } = await provider.connect();

// Send a transaction (opens wallet popup for approval)
const result = await provider.sendTransaction({
  to: "0x...",
  amount: 1000,
  data: "Payment for services"
});

// Sign a message
const { signature } = await provider.signMessage({
  message: "Verify ownership of this address",
  label: "Authentication"
});

// Listen for events
provider.on("accountsChanged", (accounts) => {
  console.log("Active account:", accounts[0]);
});`;

const API_CLIENT_CODE = `import { EvaporChainAPI } from "@evaporchain/wallet-sdk/api";

// Create client (defaults to testnet)
const api = new EvaporChainAPI({ network: "testnet" });

// Or use a custom RPC URL
const api2 = new EvaporChainAPI({
  rpcUrl: "http://localhost:8080",
  timeout: 10_000
});

// Switch networks
api.setNetwork("mainnet");

// All responses are auto-converted from snake_case to camelCase
const status = await api.getChainStatus();
console.log(status.blockHeight);  // not block_height
console.log(status.ghostCount);   // not ghost_count`;

const HOOKS_SETUP_CODE = `import { configureApi } from "@evaporchain/wallet-sdk/react";

// Call once at app startup (e.g., in main.tsx)
configureApi({ network: "testnet" });

// Then use hooks anywhere in your component tree
import { useEvaporChain, useObjects, useStaking } from "@evaporchain/wallet-sdk/react";`;

export default function SdkReferencePage() {
  return (
    <>
      <Navbar />
      <main className="pt-24 pb-0">
        {/* Header */}
        <section className="px-6 py-16">
          <div className="max-w-5xl mx-auto">
            <Link
              href="/developers"
              className="inline-flex items-center gap-1.5 text-xs text-text-muted hover:text-accent-cyan transition-colors mb-6"
            >
              <ArrowLeft size={14} /> Back to Developers
            </Link>

            <motion.div {...fade(0)}>
              <div className="flex items-center gap-3 mb-4">
                <div className="w-12 h-12 rounded-xl bg-accent-purple/10 flex items-center justify-center">
                  <Code size={22} className="text-accent-purple" />
                </div>
                <div>
                  <h1 className="text-3xl font-bold">SDK Reference</h1>
                  <p className="text-sm text-text-muted">@evaporchain/wallet-sdk</p>
                </div>
              </div>
              <p className="text-sm text-text-secondary max-w-2xl mb-4">
                Zero-dependency TypeScript SDK for building dApps on EvaporChain.
                Three entry points: wallet provider for signing, API client for chain data, and React hooks for UI.
              </p>
              <div className="flex items-center gap-3">
                <code className="text-sm text-accent-cyan font-mono bg-white/5 px-3 py-1.5 rounded-lg">
                  {INSTALL_CODE}
                </code>
                <CopyButton text={INSTALL_CODE} />
              </div>
            </motion.div>

            {/* Quick nav */}
            <motion.div {...fade(0.1)} className="mt-8 flex flex-wrap gap-2">
              {[
                { label: "Provider", href: "#provider" },
                { label: "API Client", href: "#api-client" },
                { label: "React Hooks", href: "#hooks" },
                { label: "Types", href: "#types" },
              ].map((link) => (
                <a
                  key={link.label}
                  href={link.href}
                  className="text-xs px-3 py-1.5 rounded-lg border border-white/5 text-text-muted hover:text-accent-purple hover:border-accent-purple/30 transition-colors"
                >
                  {link.label}
                </a>
              ))}
            </motion.div>
          </div>
        </section>

        {/* Package Structure */}
        <section className="px-6 py-12 border-t border-white/5">
          <div className="max-w-5xl mx-auto">
            <motion.div {...fade(0)} className="mb-8">
              <h2 className="text-xl font-bold mb-2">Package Structure</h2>
              <p className="text-xs text-text-muted">Three subpath exports — import only what you need.</p>
            </motion.div>

            <div className="grid md:grid-cols-3 gap-4">
              {[
                {
                  icon: Plug,
                  path: "@evaporchain/wallet-sdk",
                  title: "Provider",
                  description: "Wallet connection, transaction signing, message signing. Wraps window.evaporchain.",
                  color: "accent-green",
                },
                {
                  icon: Server,
                  path: "@evaporchain/wallet-sdk/api",
                  title: "API Client",
                  description: "REST API client for reading chain data. No wallet connection needed.",
                  color: "accent-cyan",
                },
                {
                  icon: Package,
                  path: "@evaporchain/wallet-sdk/react",
                  title: "React Hooks",
                  description: "10 hooks for wallet, objects, staking, swap, pools, messages, and collections.",
                  color: "accent-purple",
                },
              ].map((entry, i) => (
                <motion.div
                  key={entry.path}
                  {...fade(i * 0.08)}
                  className="bg-bg-card border border-white/5 rounded-xl p-5"
                >
                  <div className={`w-9 h-9 rounded-lg bg-${entry.color}/10 flex items-center justify-center mb-3`}>
                    <entry.icon size={16} className={`text-${entry.color}`} />
                  </div>
                  <code className={`text-xs font-mono text-${entry.color} block mb-2`}>{entry.path}</code>
                  <h3 className="text-sm font-semibold text-text-primary mb-1">{entry.title}</h3>
                  <p className="text-xs text-text-muted leading-relaxed">{entry.description}</p>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Provider */}
        <section id="provider" className="px-6 py-16 border-t border-white/5">
          <div className="max-w-4xl mx-auto">
            <motion.div {...fade(0)} className="mb-6">
              <h2 className="text-xl font-bold mb-2">EvaporChainProvider</h2>
              <p className="text-sm text-text-muted">
                Wraps the <code className="text-accent-green font-mono text-xs">window.evaporchain</code> provider
                injected by the browser extension. Use for wallet connection, transaction signing, and event listening.
              </p>
            </motion.div>

            <motion.div {...fade(0.1)}>
              <CodeBlock code={PROVIDER_CODE} filename="provider-usage.ts" />
            </motion.div>

            <motion.div {...fade(0.2)} className="mt-6">
              <h3 className="text-sm font-semibold text-text-primary mb-3">Provider Methods</h3>
              <div className="space-y-2">
                {[
                  { method: "connect()", returns: "ConnectResult", desc: "Opens wallet popup, returns address and public key" },
                  { method: "disconnect()", returns: "void", desc: "Disconnects the wallet" },
                  { method: "getAccounts()", returns: "string[]", desc: "Returns connected account addresses" },
                  { method: "getBalance(address?)", returns: "Balance", desc: "Gets balance and nonce" },
                  { method: "getObjects(address?)", returns: "EvaporObject[]", desc: "Gets objects owned by address" },
                  { method: "getNfts(address?)", returns: "Nft[]", desc: "Gets NFTs owned by address" },
                  { method: "sendTransaction(tx)", returns: "TransactionResult", desc: "Signs and submits a transaction" },
                  { method: "signMessage(request)", returns: "{ signature }", desc: "Signs an arbitrary message with ML-DSA" },
                  { method: "refreshObject(id, energy)", returns: "TransactionResult", desc: "Refreshes an object's energy" },
                  { method: "createObject(params)", returns: "TransactionResult", desc: "Creates a new decaying object" },
                  { method: "getChainStatus()", returns: "ChainStatus", desc: "Gets current chain status" },
                  { method: "on(event, handler)", returns: "void", desc: "Subscribe to wallet events" },
                  { method: "off(event, handler)", returns: "void", desc: "Unsubscribe from events" },
                ].map((m) => (
                  <div key={m.method} className="flex items-start gap-3 px-4 py-2.5 bg-bg-card border border-white/5 rounded-lg">
                    <code className="text-xs font-mono text-accent-green whitespace-nowrap">{m.method}</code>
                    <span className="text-[10px] text-text-muted font-mono shrink-0">&rarr; {m.returns}</span>
                    <span className="text-xs text-text-muted">{m.desc}</span>
                  </div>
                ))}
              </div>
            </motion.div>

            <motion.div {...fade(0.3)} className="mt-6">
              <h3 className="text-sm font-semibold text-text-primary mb-3">Events</h3>
              <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
                {[
                  { event: "connect", desc: "Wallet connected" },
                  { event: "disconnect", desc: "Wallet disconnected" },
                  { event: "accountsChanged", desc: "Active account changed" },
                  { event: "chainChanged", desc: "Network switched" },
                ].map((e) => (
                  <div key={e.event} className="bg-bg-card border border-white/5 rounded-lg px-3 py-2.5">
                    <code className="text-xs font-mono text-accent-purple block mb-0.5">{e.event}</code>
                    <p className="text-[10px] text-text-muted">{e.desc}</p>
                  </div>
                ))}
              </div>
            </motion.div>
          </div>
        </section>

        {/* API Client */}
        <section id="api-client" className="px-6 py-16 border-t border-white/5">
          <div className="max-w-4xl mx-auto">
            <motion.div {...fade(0)} className="mb-6">
              <h2 className="text-xl font-bold mb-2">EvaporChainAPI</h2>
              <p className="text-sm text-text-muted">
                REST API client for reading chain data and submitting unsigned operations.
                No wallet connection required. Auto-converts snake_case responses to camelCase.
              </p>
            </motion.div>

            <motion.div {...fade(0.1)}>
              <CodeBlock code={API_CLIENT_CODE} filename="api-client.ts" />
            </motion.div>

            <motion.div {...fade(0.2)} className="mt-8">
              <h3 className="text-sm font-semibold text-text-primary mb-4">All Methods</h3>
              <div className="space-y-4">
                {API_METHODS.map((group) => (
                  <div key={group.category}>
                    <p className="text-[10px] text-text-muted uppercase tracking-wider mb-2">{group.category}</p>
                    <div className="flex flex-wrap gap-2">
                      {group.methods.map((method) => (
                        <code
                          key={method}
                          className="text-xs font-mono text-accent-cyan bg-white/5 px-2.5 py-1 rounded-lg"
                        >
                          {method}
                        </code>
                      ))}
                    </div>
                  </div>
                ))}
              </div>

              <div className="mt-6 px-4 py-3 rounded-lg bg-accent-cyan/5 border border-accent-cyan/10">
                <p className="text-xs text-text-secondary">
                  Every method listed above is fully typed. See the{" "}
                  <Link href="/developers/api" className="text-accent-cyan hover:underline">
                    API Reference
                  </Link>{" "}
                  for request/response shapes and endpoint details.
                </p>
              </div>
            </motion.div>
          </div>
        </section>

        {/* React Hooks */}
        <section id="hooks" className="px-6 py-16 border-t border-white/5">
          <div className="max-w-4xl mx-auto">
            <motion.div {...fade(0)} className="mb-4">
              <h2 className="text-xl font-bold mb-2">React Hooks</h2>
              <p className="text-sm text-text-muted">
                10 hooks that combine Provider (for signing) and API client (for data).
                Call <code className="text-accent-purple font-mono text-xs">configureApi()</code> once at app startup.
              </p>
            </motion.div>

            <motion.div {...fade(0.1)} className="mb-8">
              <CodeBlock code={HOOKS_SETUP_CODE} filename="main.tsx" />
            </motion.div>

            <div className="space-y-8">
              {HOOKS.map((hook, i) => (
                <motion.div
                  key={hook.name}
                  {...fade(0)}
                  id={hook.name.toLowerCase()}
                  className="bg-bg-card border border-white/5 rounded-2xl overflow-hidden"
                >
                  <div className="px-6 py-4 border-b border-white/5 flex items-center justify-between">
                    <div>
                      <h3 className="text-base font-semibold text-text-primary font-mono">{hook.name}()</h3>
                      {hook.params && (
                        <p className="text-[10px] text-text-muted font-mono mt-0.5">params: {hook.params}</p>
                      )}
                    </div>
                    <span className={`text-[10px] px-2 py-0.5 rounded ${
                      hook.requiresWallet
                        ? "bg-accent-amber/10 text-accent-amber"
                        : "bg-accent-green/10 text-accent-green"
                    }`}>
                      {hook.requiresWallet ? "Wallet required" : "No wallet needed"}
                    </span>
                  </div>

                  <div className="px-6 py-4 space-y-4">
                    <p className="text-xs text-text-secondary">{hook.description}</p>

                    <div>
                      <p className="text-[10px] text-text-muted uppercase tracking-wider mb-2">Returns</p>
                      <div className="space-y-1">
                        {hook.returns.map((r) => (
                          <div key={r.name} className="flex items-baseline gap-2 text-xs">
                            <code className="text-accent-purple font-mono">{r.name}</code>
                            <span className="text-text-muted font-mono text-[10px]">{r.type}</span>
                            <span className="text-text-muted">— {r.description}</span>
                          </div>
                        ))}
                      </div>
                    </div>

                    <div>
                      <div className="flex items-center justify-between mb-1.5">
                        <p className="text-[10px] text-text-muted uppercase tracking-wider">Example</p>
                        <CopyButton text={hook.example} />
                      </div>
                      <pre className="bg-[#0a0a0f] rounded-lg p-3 text-xs text-text-secondary overflow-x-auto font-mono leading-relaxed">
                        {hook.example}
                      </pre>
                    </div>
                  </div>
                </motion.div>
              ))}
            </div>
          </div>
        </section>

        {/* Types */}
        <section id="types" className="px-6 py-16 border-t border-white/5">
          <div className="max-w-4xl mx-auto">
            <motion.div {...fade(0)} className="mb-6">
              <h2 className="text-xl font-bold mb-2">Type Exports</h2>
              <p className="text-sm text-text-muted">
                All types are exported from the main entry point. The SDK is TypeScript-first with strict types throughout.
              </p>
            </motion.div>

            <motion.div {...fade(0.1)}>
              <div className="grid md:grid-cols-2 gap-4">
                {[
                  {
                    category: "Core",
                    types: ["EvaporObject", "ObjectState", "ChainStatus", "Balance", "Transaction", "TxResult"],
                  },
                  {
                    category: "NFTs",
                    types: ["Nft", "NftCollection", "MintNftParams"],
                  },
                  {
                    category: "DeFi",
                    types: ["StakingInfo", "Validator", "SwapQuote", "EnergyPool", "PoolContribution"],
                  },
                  {
                    category: "Messaging",
                    types: ["MortalMessage"],
                  },
                  {
                    category: "Wallet",
                    types: ["ConnectResult", "TransactionResult", "TransactionRequest", "SignMessageRequest", "InjectedProvider"],
                  },
                  {
                    category: "Config",
                    types: ["NetworkId", "NetworkConfig", "EvaporChainError", "EvaporChainErrorCode"],
                  },
                ].map((group) => (
                  <div key={group.category} className="bg-bg-card border border-white/5 rounded-xl p-4">
                    <p className="text-[10px] text-text-muted uppercase tracking-wider mb-2">{group.category}</p>
                    <div className="flex flex-wrap gap-1.5">
                      {group.types.map((t) => (
                        <code key={t} className="text-xs font-mono text-accent-cyan bg-white/5 px-2 py-0.5 rounded">
                          {t}
                        </code>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            </motion.div>
          </div>
        </section>

        {/* Error Handling */}
        <section className="px-6 py-16 border-t border-white/5">
          <div className="max-w-4xl mx-auto">
            <motion.div {...fade(0)} className="mb-6">
              <h2 className="text-xl font-bold mb-2">Error Handling</h2>
              <p className="text-sm text-text-muted">
                The SDK throws typed <code className="text-accent-red font-mono text-xs">EvaporChainError</code> instances
                with specific error codes.
              </p>
            </motion.div>

            <motion.div {...fade(0.1)}>
              <div className="space-y-2">
                {[
                  { code: "NOT_INSTALLED", desc: "Browser extension not found" },
                  { code: "USER_REJECTED", desc: "User rejected the wallet popup" },
                  { code: "NETWORK_ERROR", desc: "RPC or network communication failure" },
                  { code: "INSUFFICIENT_BALANCE", desc: "Not enough EVAP for the transaction" },
                  { code: "OBJECT_NOT_FOUND", desc: "Requested object does not exist on-chain" },
                ].map((err) => (
                  <div key={err.code} className="flex items-center gap-3 px-4 py-2.5 bg-bg-card border border-white/5 rounded-lg">
                    <code className="text-xs font-mono text-accent-red whitespace-nowrap">{err.code}</code>
                    <span className="text-xs text-text-muted">{err.desc}</span>
                  </div>
                ))}
              </div>
            </motion.div>

            <motion.div {...fade(0.2)} className="mt-6">
              <CodeBlock
                code={`import { EvaporChainError, EvaporChainErrorCode } from "@evaporchain/wallet-sdk";

try {
  await provider.connect();
} catch (err) {
  if (err instanceof EvaporChainError) {
    switch (err.code) {
      case EvaporChainErrorCode.NOT_INSTALLED:
        showInstallPrompt();
        break;
      case EvaporChainErrorCode.USER_REJECTED:
        // User cancelled — do nothing
        break;
      default:
        reportError(err.message, err.details);
    }
  }
}`}
                filename="error-handling.ts"
              />
            </motion.div>
          </div>
        </section>

        {/* CTA */}
        <section className="px-6 py-16 border-t border-white/5">
          <div className="max-w-3xl mx-auto text-center">
            <motion.div {...fade(0)}>
              <h3 className="text-lg font-semibold mb-3">Ready to build?</h3>
              <p className="text-sm text-text-muted mb-6">
                Install the SDK, connect to the testnet, and start building your first mortal dApp.
              </p>
              <div className="flex flex-wrap items-center justify-center gap-4">
                <Link
                  href="/developers/api"
                  className="gradient-bg text-bg-primary font-semibold px-8 py-3 rounded-full hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-shadow"
                >
                  API Reference &rarr;
                </Link>
                <Link
                  href="/developers"
                  className="border border-white/20 text-text-primary px-8 py-3 rounded-full hover:border-accent-cyan/40 transition-colors"
                >
                  Developer Hub
                </Link>
                <a
                  href="https://github.com/ss1738/EvaporChain/tree/main/wallet-sdk"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-sm text-text-muted hover:text-accent-cyan transition-colors"
                >
                  View Source &rarr;
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
