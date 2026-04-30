import { test as base, chromium, type BrowserContext, type Page, type Worker } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { E2E_PATHS } from "./playwright.config";

/**
 * Playwright fixtures for MV3 wallet testing.
 *
 *   extensionContext  – persistent Chromium context with the built dist/
 *                       loaded as an unpacked extension. Each test gets
 *                       a brand-new on-disk profile so chrome.storage.local
 *                       and IndexedDB are guaranteed-clean.
 *   extensionId       – resolved chrome-extension://<id> for this run.
 *   popupPage         – page open at chrome-extension://<id>/popup.html.
 *
 * Resolving the extension ID:
 *   The recommended Playwright recipe is to wait for the background
 *   service worker to register — its URL has the form
 *   `chrome-extension://<id>/background.js`. We grab the first one.
 *   This works for MV3 extensions where the manifest declares a
 *   `background.service_worker`.
 */

type Fixtures = {
  extensionContext: BrowserContext;
  extensionId: string;
  popupPage: Page;
};

export const test = base.extend<Fixtures>({
  // eslint-disable-next-line no-empty-pattern
  extensionContext: async ({}, use) => {
    const userDataDir = mkdtempSync(join(tmpdir(), "evap-wallet-e2e-"));
    const ctx = await chromium.launchPersistentContext(userDataDir, {
      headless: false, // MV3 service workers don't run reliably in headless
      args: [
        `--disable-extensions-except=${E2E_PATHS.extensionDist}`,
        `--load-extension=${E2E_PATHS.extensionDist}`,
        "--no-first-run",
        "--no-default-browser-check",
      ],
      // Speeds up CI; MV3 service workers still work.
      viewport: { width: 1280, height: 800 },
    });

    await use(ctx);

    await ctx.close();
    try {
      rmSync(userDataDir, { recursive: true, force: true });
    } catch {
      // best-effort cleanup
    }
  },

  extensionId: async ({ extensionContext }, use) => {
    const id = await resolveExtensionId(extensionContext);
    await use(id);
  },

  popupPage: async ({ extensionContext, extensionId }, use) => {
    const page = await extensionContext.newPage();
    await page.goto(`chrome-extension://${extensionId}/popup.html`);
    // Wait for React to mount + Zustand init() to finish. The store
    // exposes itself on globalThis when MODE === "test" (see
    // useWallet.ts bottom). We use it as our readiness signal.
    await page.waitForFunction(
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      () => typeof (globalThis as any).__zustandStore !== "undefined",
      undefined,
      { timeout: 15_000 },
    );

    // Wipe wallet state from previous test (defence in depth — the
    // persistent context is also fresh per test, but if anything is
    // re-used this guarantees a clean slate).
    await page.evaluate(async () => {
      try {
        await chrome.storage.local.clear();
      } catch {
        /* ignore */
      }
      try {
        localStorage.clear();
      } catch {
        /* ignore */
      }
    });

    // Reload so init() re-runs against the cleared storage.
    await page.reload();
    await page.waitForFunction(
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      () => typeof (globalThis as any).__zustandStore !== "undefined",
      undefined,
      { timeout: 15_000 },
    );

    await use(page);
    await page.close();
  },
});

export const expect = test.expect;

/**
 * Find the loaded extension's ID by waiting for its service worker to
 * appear. The URL is of the form `chrome-extension://<id>/background.js`.
 *
 * Falls back to scanning open background pages (legacy MV2-style) if no
 * service worker shows up within the timeout.
 */
async function resolveExtensionId(ctx: BrowserContext): Promise<string> {
  const fromWorker = (w: Worker): string | null => {
    const m = /^chrome-extension:\/\/([a-p]{32})\//.exec(w.url());
    return m ? m[1] : null;
  };

  const existing = ctx.serviceWorkers().map(fromWorker).find(Boolean);
  if (existing) return existing;

  const w = await ctx.waitForEvent("serviceworker", { timeout: 20_000 });
  const id = fromWorker(w);
  if (!id) throw new Error(`Could not parse extension ID from worker URL: ${w.url()}`);
  return id;
}
