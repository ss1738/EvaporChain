"use client";

import { motion } from "framer-motion";
import Link from "next/link";
import Navbar from "@/components/Navbar";
import Footer from "@/components/Footer";
import {
  ArrowLeft,
  Server,
  Activity,
  Wallet,
  Send,
  Droplets,
  Box,
  Image,
  Repeat,
  Coins,
  MessageSquare,
  Zap,
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

type Method = "GET" | "POST";

interface Endpoint {
  method: Method;
  path: string;
  description: string;
  params?: Array<{ name: string; type: string; required?: boolean; description: string }>;
  body?: Array<{ name: string; type: string; required?: boolean; description: string }>;
  response: string;
}

interface EndpointGroup {
  icon: typeof Server;
  title: string;
  color: string;
  baseUrl: string;
  endpoints: Endpoint[];
}

const API_GROUPS: EndpointGroup[] = [
  {
    icon: Activity,
    title: "Chain",
    color: "accent-cyan",
    baseUrl: "/api",
    endpoints: [
      {
        method: "GET",
        path: "/api/status",
        description: "Get current chain status including block height, epoch, active objects, ghost count, and peer count.",
        response: `{
  "chain_name": "EvaporChain Testnet",
  "version": "0.1.0",
  "block_height": 142857,
  "epoch": 2381,
  "active_objects": 4201,
  "ghost_count": 892,
  "total_evaporated": 12450,
  "peer_count": 24
}`,
      },
      {
        method: "POST",
        path: "/api/faucet",
        description: "Claim testnet EVAP tokens. Rate limited to one claim per address per hour.",
        body: [
          { name: "address", type: "string", required: true, description: "Recipient address" },
        ],
        response: `{
  "success": true,
  "balance": 10000,
  "message": "Claimed 10,000 EVAP"
}`,
      },
    ],
  },
  {
    icon: Wallet,
    title: "Accounts",
    color: "accent-purple",
    baseUrl: "/api",
    endpoints: [
      {
        method: "GET",
        path: "/api/address/{address}",
        description: "Get balance and nonce for an address.",
        params: [
          { name: "address", type: "string", required: true, description: "EvaporChain address" },
        ],
        response: `{
  "address": "0x1a2b3c...",
  "balance": 50000,
  "nonce": 12
}`,
      },
    ],
  },
  {
    icon: Send,
    title: "Transactions",
    color: "accent-green",
    baseUrl: "/api",
    endpoints: [
      {
        method: "GET",
        path: "/api/transactions",
        description: "Get recent transactions, optionally filtered by address.",
        params: [
          { name: "address", type: "string", description: "Filter by sender or recipient" },
          { name: "limit", type: "number", description: "Max results (default: 50)" },
        ],
        response: `[{
  "hash": "0xabc...",
  "type": "transfer",
  "detail": "Sent 500 EVAP",
  "from": "0x1a2b...",
  "to": "0x3c4d...",
  "amount": "500",
  "timestamp": 1712678400
}]`,
      },
      {
        method: "POST",
        path: "/api/tx/transfer",
        description: "Submit a transfer transaction.",
        body: [
          { name: "from", type: "string", required: true, description: "Sender address" },
          { name: "to", type: "string", required: true, description: "Recipient address" },
          { name: "amount", type: "number", required: true, description: "Amount in EVAP" },
          { name: "nonce", type: "number", required: true, description: "Sender nonce" },
        ],
        response: `{
  "success": true,
  "message": "Transfer submitted",
  "tx_hash": "0xdef..."
}`,
      },
    ],
  },
  {
    icon: Box,
    title: "Objects",
    color: "accent-amber",
    baseUrl: "/api",
    endpoints: [
      {
        method: "GET",
        path: "/api/objects",
        description: "Get all objects, optionally filtered by owner and/or state.",
        params: [
          { name: "owner", type: "string", description: "Filter by owner address" },
          { name: "state", type: "string", description: "Filter by state: Active, Grace, Ghost, Risen" },
        ],
        response: `[{
  "id": "obj_abc123",
  "name": "My Object",
  "owner": "0x1a2b...",
  "energy": 5000,
  "max_energy": 10000,
  "half_life": 100,
  "state": "Active",
  "current_energy": 7500,
  "decay_percentage": 25.0,
  "estimated_ghost_time": 1712764800,
  "created_epoch": 2100,
  "last_refreshed": 2350
}]`,
      },
      {
        method: "GET",
        path: "/api/object/{objectId}",
        description: "Get a single object by ID.",
        params: [
          { name: "objectId", type: "string", required: true, description: "Object ID" },
        ],
        response: `{
  "id": "obj_abc123",
  "name": "My Object",
  "owner": "0x1a2b...",
  "state": "Active",
  "current_energy": 7500,
  "max_energy": 10000,
  "half_life": 100,
  ...
}`,
      },
      {
        method: "POST",
        path: "/api/tx/refresh",
        description: "Refresh an object's energy to prevent decay. Adds energy and resets the decay timer.",
        body: [
          { name: "object_id", type: "string", required: true, description: "Object to refresh" },
          { name: "energy_deposit", type: "number", required: true, description: "Energy to add" },
        ],
        response: `{
  "success": true,
  "message": "Object refreshed",
  "tx_hash": "0x..."
}`,
      },
      {
        method: "POST",
        path: "/api/tx/batch-refresh",
        description: "Refresh multiple objects in a single transaction.",
        body: [
          { name: "items", type: "Array<{id, energy}>", required: true, description: "Objects and energy amounts" },
        ],
        response: `[
  { "success": true, "message": "Refreshed", "tx_hash": "0x..." },
  { "success": true, "message": "Refreshed", "tx_hash": "0x..." }
]`,
      },
    ],
  },
  {
    icon: Image,
    title: "NFTs",
    color: "accent-red",
    baseUrl: "/api",
    endpoints: [
      {
        method: "GET",
        path: "/api/nfts",
        description: "Get all NFTs, optionally filtered by owner.",
        params: [
          { name: "owner", type: "string", description: "Filter by owner address" },
        ],
        response: `[{
  "id": "nft_xyz",
  "name": "Mortal Punk #42",
  "collection": "col_abc",
  "collection_name": "Mortal Punks",
  "owner": "0x1a2b...",
  "image_uri": "ipfs://...",
  "current_energy": 3000,
  "max_energy": 10000,
  "half_life": 200,
  "state": "Active",
  "epochs_remaining": 150
}]`,
      },
      {
        method: "POST",
        path: "/api/nft/mint",
        description: "Mint a new mortal NFT with initial energy and decay rate.",
        body: [
          { name: "name", type: "string", required: true, description: "NFT name" },
          { name: "collection", type: "string", required: true, description: "Collection ID" },
          { name: "image_uri", type: "string", description: "Image URI (IPFS recommended)" },
          { name: "energy", type: "number", required: true, description: "Initial energy" },
          { name: "half_life", type: "number", required: true, description: "Decay half-life in epochs" },
          { name: "data", type: "object", description: "Additional metadata" },
        ],
        response: `{
  "success": true,
  "message": "NFT minted",
  "tx_hash": "0x...",
  "nft_id": "nft_new123"
}`,
      },
      {
        method: "POST",
        path: "/api/nft/transfer",
        description: "Transfer an NFT to another address.",
        body: [
          { name: "nft_id", type: "string", required: true, description: "NFT to transfer" },
          { name: "to", type: "string", required: true, description: "Recipient address" },
        ],
        response: `{ "success": true, "message": "NFT transferred", "tx_hash": "0x..." }`,
      },
      {
        method: "POST",
        path: "/api/nft/refresh",
        description: "Refresh an NFT's energy to extend its life.",
        body: [
          { name: "nft_id", type: "string", required: true, description: "NFT to refresh" },
          { name: "energy_deposit", type: "number", required: true, description: "Energy to add" },
        ],
        response: `{ "success": true, "message": "NFT refreshed", "tx_hash": "0x..." }`,
      },
      {
        method: "GET",
        path: "/api/nft/collections",
        description: "Get all NFT collections.",
        response: `[{
  "id": "col_abc",
  "name": "Mortal Punks",
  "creator": "0x1a2b...",
  "count": 42,
  "floor_energy": 2000
}]`,
      },
    ],
  },
  {
    icon: Repeat,
    title: "Swap",
    color: "accent-cyan",
    baseUrl: "/api",
    endpoints: [
      {
        method: "POST",
        path: "/api/swap/quote",
        description: "Get a swap quote. Returns expected output amount and price impact.",
        body: [
          { name: "from_token", type: "string", required: true, description: "Input token symbol" },
          { name: "to_token", type: "string", required: true, description: "Output token symbol" },
          { name: "amount", type: "number", required: true, description: "Input amount" },
        ],
        response: `{
  "from_token": "EVAP",
  "to_token": "GHOST",
  "amount_in": 1000,
  "amount_out": 4850,
  "rate": 4.85,
  "price_impact": 0.02
}`,
      },
      {
        method: "POST",
        path: "/api/swap/execute",
        description: "Execute a token swap with slippage protection.",
        body: [
          { name: "from_token", type: "string", required: true, description: "Input token symbol" },
          { name: "to_token", type: "string", required: true, description: "Output token symbol" },
          { name: "amount", type: "number", required: true, description: "Input amount" },
          { name: "slippage", type: "number", required: true, description: "Max slippage (0.01 = 1%)" },
        ],
        response: `{ "success": true, "message": "Swap executed", "tx_hash": "0x..." }`,
      },
    ],
  },
  {
    icon: Coins,
    title: "Staking",
    color: "accent-purple",
    baseUrl: "/api",
    endpoints: [
      {
        method: "GET",
        path: "/api/staking/{address}",
        description: "Get staking info for an address — staked amount, rewards, unbonding status.",
        params: [
          { name: "address", type: "string", required: true, description: "Staker address" },
        ],
        response: `{
  "staked": 25000,
  "rewards": 1200,
  "is_validator": false,
  "epoch": 2381,
  "staking_start_epoch": 2100,
  "unbonding_amount": 0,
  "unbonding_complete_epoch": null
}`,
      },
      {
        method: "GET",
        path: "/api/validators",
        description: "Get all validators with stake, commission, and uptime.",
        response: `[{
  "address": "0xval...",
  "name": "Validator Alpha",
  "stake": 500000,
  "commission": 0.05,
  "uptime": 0.998,
  "status": "active"
}]`,
      },
      {
        method: "POST",
        path: "/api/tx/stake",
        description: "Stake EVAP tokens to earn rewards.",
        body: [
          { name: "from", type: "string", required: true, description: "Staker address" },
          { name: "amount", type: "number", required: true, description: "Amount to stake" },
          { name: "nonce", type: "number", required: true, description: "Account nonce" },
        ],
        response: `{ "success": true, "message": "Staked", "tx_hash": "0x..." }`,
      },
      {
        method: "POST",
        path: "/api/tx/unstake",
        description: "Begin unstaking (starts unbonding period).",
        body: [
          { name: "from", type: "string", required: true, description: "Staker address" },
          { name: "amount", type: "number", required: true, description: "Amount to unstake" },
          { name: "nonce", type: "number", required: true, description: "Account nonce" },
        ],
        response: `{ "success": true, "message": "Unstaking initiated", "tx_hash": "0x..." }`,
      },
      {
        method: "POST",
        path: "/api/tx/claim-rewards",
        description: "Claim accumulated staking rewards.",
        body: [
          { name: "from", type: "string", required: true, description: "Staker address" },
          { name: "nonce", type: "number", required: true, description: "Account nonce" },
        ],
        response: `{ "success": true, "message": "Rewards claimed", "tx_hash": "0x..." }`,
      },
    ],
  },
  {
    icon: Droplets,
    title: "Energy Pools",
    color: "accent-green",
    baseUrl: "/api",
    endpoints: [
      {
        method: "GET",
        path: "/api/pools",
        description: "Get all community energy pools.",
        response: `[{
  "id": "pool_abc",
  "name": "Protect Genesis Objects",
  "creator": "0x1a2b...",
  "total_energy": 150000,
  "contributors": 24,
  "target_object": "obj_genesis",
  "created_epoch": 100
}]`,
      },
      {
        method: "GET",
        path: "/api/pool/{poolId}",
        description: "Get a single pool with details.",
        params: [
          { name: "poolId", type: "string", required: true, description: "Pool ID" },
        ],
        response: `{ "id": "pool_abc", "name": "...", "total_energy": 150000, ... }`,
      },
      {
        method: "GET",
        path: "/api/pool/{poolId}/contributors",
        description: "Get all contributors to a pool.",
        params: [
          { name: "poolId", type: "string", required: true, description: "Pool ID" },
        ],
        response: `[{ "address": "0x...", "amount": 5000, "timestamp": 1712678400 }]`,
      },
      {
        method: "POST",
        path: "/api/pool/create",
        description: "Create a new energy pool.",
        body: [
          { name: "name", type: "string", required: true, description: "Pool name" },
          { name: "creator", type: "string", required: true, description: "Creator address" },
          { name: "target_object", type: "string", description: "Object to protect (optional)" },
        ],
        response: `{ "success": true, "message": "Pool created", "tx_hash": "0x...", "pool_id": "pool_new" }`,
      },
      {
        method: "POST",
        path: "/api/pool/stake",
        description: "Stake energy into a pool.",
        body: [
          { name: "pool_id", type: "string", required: true, description: "Pool ID" },
          { name: "address", type: "string", required: true, description: "Contributor address" },
          { name: "amount", type: "number", required: true, description: "Energy to stake" },
        ],
        response: `{ "success": true, "message": "Staked to pool", "tx_hash": "0x..." }`,
      },
    ],
  },
  {
    icon: MessageSquare,
    title: "Messages",
    color: "accent-amber",
    baseUrl: "/api",
    endpoints: [
      {
        method: "POST",
        path: "/api/messages/send",
        description: "Send a mortal message with initial energy.",
        body: [
          { name: "from", type: "string", required: true, description: "Sender address" },
          { name: "to", type: "string", required: true, description: "Recipient address" },
          { name: "content", type: "string", required: true, description: "Message content" },
          { name: "energy", type: "number", required: true, description: "Initial energy" },
        ],
        response: `{ "success": true, "message": "Sent", "tx_hash": "0x...", "message_id": "msg_abc" }`,
      },
      {
        method: "GET",
        path: "/api/messages/inbox/{address}",
        description: "Get inbox messages for an address.",
        params: [
          { name: "address", type: "string", required: true, description: "Recipient address" },
        ],
        response: `[{
  "id": "msg_abc",
  "from": "0x...",
  "to": "0x...",
  "content": "Hello world",
  "energy": 500,
  "max_energy": 1000,
  "current_energy": 750,
  "state": "Active",
  "timestamp": 1712678400
}]`,
      },
      {
        method: "GET",
        path: "/api/messages/sent/{address}",
        description: "Get sent messages for an address.",
        params: [
          { name: "address", type: "string", required: true, description: "Sender address" },
        ],
        response: `[{ "id": "msg_abc", "from": "0x...", "to": "0x...", ... }]`,
      },
      {
        method: "POST",
        path: "/api/message/boost",
        description: "Boost a message's energy to extend its life.",
        body: [
          { name: "message_id", type: "string", required: true, description: "Message to boost" },
          { name: "energy", type: "number", required: true, description: "Energy to add" },
        ],
        response: `{ "success": true, "message": "Boosted", "tx_hash": "0x..." }`,
      },
      {
        method: "GET",
        path: "/api/messages/stats/{address}",
        description: "Get messaging statistics for an address.",
        params: [
          { name: "address", type: "string", required: true, description: "Address" },
        ],
        response: `{ "sent": 42, "received": 38, "active": 12, "ghosted": 8 }`,
      },
    ],
  },
];

function MethodBadge({ method }: { method: Method }) {
  return (
    <span
      className={`text-[10px] font-bold font-mono px-2 py-0.5 rounded ${
        method === "GET"
          ? "bg-accent-green/10 text-accent-green"
          : "bg-accent-amber/10 text-accent-amber"
      }`}
    >
      {method}
    </span>
  );
}

function EndpointCard({ endpoint }: { endpoint: Endpoint }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="border border-white/5 rounded-xl overflow-hidden">
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full px-4 py-3 flex items-center gap-3 hover:bg-white/[0.02] transition-colors text-left"
      >
        <MethodBadge method={endpoint.method} />
        <code className="text-sm text-text-primary font-mono flex-1">{endpoint.path}</code>
        <span className="text-[10px] text-text-muted hidden sm:inline max-w-[200px] truncate">
          {endpoint.description}
        </span>
        <span className={`text-text-muted transition-transform ${expanded ? "rotate-180" : ""}`}>
          <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
            <path d="M2 4l4 4 4-4" stroke="currentColor" strokeWidth="1.5" fill="none" />
          </svg>
        </span>
      </button>

      {expanded && (
        <div className="px-4 pb-4 border-t border-white/5 pt-3 space-y-4">
          <p className="text-xs text-text-secondary">{endpoint.description}</p>

          {endpoint.params && endpoint.params.length > 0 && (
            <div>
              <p className="text-[10px] text-text-muted uppercase tracking-wider mb-2">Parameters</p>
              <div className="space-y-1.5">
                {endpoint.params.map((p) => (
                  <div key={p.name} className="flex items-baseline gap-2 text-xs">
                    <code className="text-accent-cyan font-mono">{p.name}</code>
                    <span className="text-text-muted font-mono text-[10px]">{p.type}</span>
                    {p.required && <span className="text-accent-red text-[10px]">required</span>}
                    <span className="text-text-muted">— {p.description}</span>
                  </div>
                ))}
              </div>
            </div>
          )}

          {endpoint.body && endpoint.body.length > 0 && (
            <div>
              <p className="text-[10px] text-text-muted uppercase tracking-wider mb-2">Request Body</p>
              <div className="space-y-1.5">
                {endpoint.body.map((p) => (
                  <div key={p.name} className="flex items-baseline gap-2 text-xs">
                    <code className="text-accent-purple font-mono">{p.name}</code>
                    <span className="text-text-muted font-mono text-[10px]">{p.type}</span>
                    {p.required && <span className="text-accent-red text-[10px]">required</span>}
                    <span className="text-text-muted">— {p.description}</span>
                  </div>
                ))}
              </div>
            </div>
          )}

          <div>
            <div className="flex items-center justify-between mb-1.5">
              <p className="text-[10px] text-text-muted uppercase tracking-wider">Response</p>
              <CopyButton text={endpoint.response} />
            </div>
            <pre className="bg-bg-card rounded-lg p-3 text-xs text-text-secondary overflow-x-auto font-mono leading-relaxed">
              {endpoint.response}
            </pre>
          </div>
        </div>
      )}
    </div>
  );
}

export default function ApiReferencePage() {
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
                <div className="w-12 h-12 rounded-xl bg-accent-cyan/10 flex items-center justify-center">
                  <Server size={22} className="text-accent-cyan" />
                </div>
                <div>
                  <h1 className="text-3xl font-bold">API Reference</h1>
                  <p className="text-sm text-text-muted">REST API for EvaporChain testnet</p>
                </div>
              </div>
              <p className="text-sm text-text-secondary max-w-2xl">
                All endpoints use JSON. The base URL for testnet is{" "}
                <code className="text-accent-cyan font-mono text-xs bg-white/5 px-1.5 py-0.5 rounded">
                  https://testnet.evaporchain.com
                </code>
                . Responses return snake_case — the SDK auto-converts to camelCase.
              </p>
            </motion.div>

            {/* Quick nav */}
            <motion.div {...fade(0.1)} className="mt-8 flex flex-wrap gap-2">
              {API_GROUPS.map((group) => (
                <a
                  key={group.title}
                  href={`#${group.title.toLowerCase()}`}
                  className={`text-xs px-3 py-1.5 rounded-lg border border-white/5 text-text-muted hover:text-${group.color} hover:border-${group.color}/30 transition-colors`}
                >
                  {group.title}
                </a>
              ))}
            </motion.div>
          </div>
        </section>

        {/* Endpoint Groups */}
        <section className="px-6 pb-20">
          <div className="max-w-5xl mx-auto space-y-12">
            {API_GROUPS.map((group, gi) => (
              <motion.div
                key={group.title}
                id={group.title.toLowerCase()}
                {...fade(0)}
              >
                <div className="flex items-center gap-3 mb-4">
                  <div className={`w-9 h-9 rounded-lg bg-${group.color}/10 flex items-center justify-center`}>
                    <group.icon size={16} className={`text-${group.color}`} />
                  </div>
                  <div>
                    <h2 className="text-lg font-semibold text-text-primary">{group.title}</h2>
                    <p className="text-[10px] text-text-muted">
                      {group.endpoints.length} endpoint{group.endpoints.length !== 1 ? "s" : ""}
                    </p>
                  </div>
                </div>

                <div className="space-y-2">
                  {group.endpoints.map((endpoint) => (
                    <EndpointCard key={`${endpoint.method}-${endpoint.path}`} endpoint={endpoint} />
                  ))}
                </div>
              </motion.div>
            ))}
          </div>
        </section>

        {/* SDK Note */}
        <section className="px-6 py-16 border-t border-white/5">
          <div className="max-w-3xl mx-auto text-center">
            <motion.div {...fade(0)}>
              <h3 className="text-lg font-semibold mb-3">Prefer TypeScript?</h3>
              <p className="text-sm text-text-muted mb-6">
                The Wallet SDK wraps every endpoint above into typed methods with automatic
                snake_case to camelCase conversion, timeouts, and network switching.
              </p>
              <div className="flex items-center justify-center gap-4">
                <Link
                  href="/developers/sdk"
                  className="gradient-bg text-bg-primary font-semibold px-8 py-3 rounded-full hover:shadow-[0_0_24px_rgba(0,240,255,0.3)] transition-shadow"
                >
                  SDK Reference &rarr;
                </Link>
                <code className="text-sm text-accent-cyan font-mono bg-white/5 px-4 py-2.5 rounded-lg">
                  npm i @evaporchain/wallet-sdk
                </code>
              </div>
            </motion.div>
          </div>
        </section>
      </main>
      <Footer />
    </>
  );
}
