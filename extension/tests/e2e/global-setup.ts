import { existsSync } from "node:fs";
import { E2E_PATHS } from "./playwright.config";

/**
 * Global setup: aborts the whole run if either
 *   1. the built `dist/` folder is missing (run `npm run build` first), or
 *   2. the node at $WALLET_NODE_URL isn't reachable on /api/status.
 *
 * Surfacing both as a single fatal error is cleaner than letting every
 * spec time out independently in CI logs.
 */
export default async function globalSetup() {
  const { extensionDist, nodeUrl } = E2E_PATHS;

  if (!existsSync(extensionDist)) {
    throw new Error(
      `[e2e] Extension dist not found at ${extensionDist}. ` +
        `Run \`npm run build\` from extension/ before \`npm run test:e2e\`.`,
    );
  }

  const statusUrl = `${nodeUrl.replace(/\/+$/, "")}/api/status`;
  let ok = false;
  let lastErr: unknown = null;
  try {
    const ctrl = new AbortController();
    const tid = setTimeout(() => ctrl.abort(), 5_000);
    const res = await fetch(statusUrl, { signal: ctrl.signal });
    clearTimeout(tid);
    ok = res.ok;
    if (!ok) lastErr = `HTTP ${res.status}`;
  } catch (e) {
    lastErr = e;
  }

  if (!ok) {
    throw new Error(
      `[e2e] Node not reachable at ${statusUrl} (${String(lastErr)}). ` +
        `Set WALLET_NODE_URL to a running node — e.g. ` +
        `WALLET_NODE_URL=http://localhost:8080 npm run test:e2e on a Mini.`,
    );
  }

  // eslint-disable-next-line no-console
  console.log(`[e2e] Node OK at ${nodeUrl}; extension at ${extensionDist}`);
}
