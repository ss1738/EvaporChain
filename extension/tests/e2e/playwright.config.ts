import { defineConfig } from "@playwright/test";
import { resolve } from "node:path";

/**
 * Playwright config for the EvaporChain wallet extension.
 *
 * MV3 extensions can only be loaded via `chromium.launchPersistentContext`
 * with `--disable-extensions-except` + `--load-extension`. The fixture in
 * `fixtures.ts` does the actual launch — this config just sets the global
 * defaults the user expects when they invoke `npm run test:e2e`.
 *
 * Node URL is read from $WALLET_NODE_URL (default http://satyawan.local:8080).
 * The test runner does a reachability check against `${WALLET_NODE_URL}/api/status`
 * in `globalSetup` — if it fails, the entire suite is aborted with a clear
 * error rather than letting individual specs time out one by one.
 */

const EXTENSION_DIST = resolve(__dirname, "..", "..", "dist");
const NODE_URL = process.env.WALLET_NODE_URL ?? "http://satyawan.local:8080";

export default defineConfig({
  testDir: "./specs",
  timeout: 120_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  forbidOnly: !!process.env.CI,
  reporter: [["list"]],
  globalSetup: resolve(__dirname, "global-setup.ts"),
  use: {
    baseURL: NODE_URL,
    trace: "retain-on-failure",
    video: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  // Single project. The extension is loaded by the `extensionContext`
  // fixture in fixtures.ts, not via `use.launchOptions` — Playwright's
  // standard `browser` fixture cannot load extensions.
  projects: [
    {
      name: "chromium",
      use: {
        // Surfaced for the fixture to read; no effect on the default
        // browser fixture (which the suite does not use).
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        ...(({ extensionPath: EXTENSION_DIST, nodeUrl: NODE_URL }) as any),
      },
    },
  ],
});

// Re-export so individual specs / fixtures can pull the resolved values
// without re-deriving them.
export const E2E_PATHS = {
  extensionDist: EXTENSION_DIST,
  nodeUrl: NODE_URL,
};
