import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "path";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": resolve(__dirname, "src"),
      "@evaporchain/wallet-sdk": resolve(__dirname, "../../wallet-sdk/src/index.ts"),
      "@evaporchain/wallet-sdk/react": resolve(__dirname, "../../wallet-sdk/src/react.ts"),
    },
  },
  server: {
    port: 5177,
    proxy: {
      "/api": {
        target: "https://testnet.evaporchain.com",
        changeOrigin: true,
      },
    },
  },
});
