import type { NextConfig } from "next";
import path from "node:path";

const config: NextConfig = {
  // Validator analytics is read-only; we proxy /api/* and /metrics to the
  // upstream EvaporChain node so the browser never sees CORS or origin issues.
  async rewrites() {
    const upstream = process.env.EVAPORCHAIN_RPC || "https://testnet.evaporchain.com";
    return [
      { source: "/api/:path*", destination: `${upstream}/api/:path*` },
      { source: "/metrics", destination: `${upstream}/metrics` },
    ];
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
