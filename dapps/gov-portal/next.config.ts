import type { NextConfig } from "next";
import path from "node:path";

// EvaporChain testnet exposes the REST API on a different host; we proxy
// /api/* (other than this dApp's own /api/proposals[/...]) to it so the
// dApp's fetch calls stay relative.
//
// Note: the rewrite uses a `has` clause so /api/proposals/* falls through to
// our local route handlers and only chain endpoints hit the upstream node.
const config: NextConfig = {
  async rewrites() {
    const upstream = process.env.EVAPORCHAIN_RPC || "https://testnet.evaporchain.com";
    return {
      beforeFiles: [],
      afterFiles: [],
      // `fallback` runs only when no local route matches. /api/proposals[/:id]
      // resolves locally; everything else (governance/fork_choice_mode,
      // tx/upgrade_contract, validators, etc.) is proxied upstream.
      fallback: [
        {
          source: "/api/:path*",
          destination: `${upstream}/api/:path*`,
        },
      ],
    };
  },
  webpack(config) {
    config.resolve = config.resolve || {};
    config.resolve.alias = {
      ...(config.resolve.alias as Record<string, string> | undefined),
      "@evaporchain/wallet-sdk": path.resolve(__dirname, "../../wallet-sdk/src/index.ts"),
      "@evaporchain/wallet-sdk/react": path.resolve(__dirname, "../../wallet-sdk/src/react.ts"),
    };
    return config;
  },
};

export default config;
