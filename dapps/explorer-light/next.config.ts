import type { NextConfig } from "next";
import path from "node:path";

// EvaporChain testnet exposes the REST API on a different host; we proxy
// /api/* to it so the dApp's fetch calls stay relative.
const config: NextConfig = {
  async rewrites() {
    const upstream = process.env.EVAPORCHAIN_RPC || "https://testnet.evaporchain.com";
    return [
      {
        source: "/api/:path*",
        destination: `${upstream}/api/:path*`,
      },
    ];
  },
  // Resolve workspace-local wallet-sdk so we can use it without publishing.
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
