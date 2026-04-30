import { test, expect } from "../fixtures";
import { nodeClient } from "../lib/node-client";

/**
 * Spec 03 — Send + track.
 *
 * Creates two wallets (alice + bob) inside one keystore — alice is
 * funded from the faucet, then sends 100 EVAP to bob. We assert:
 *   • Header shows "N pending" pill while the tx is in flight
 *   • The node returns state="finalised" within 60s
 *   • bob's balance reflects the transfer node-side
 *
 * Bob's address is generated in a second wallet inside the same popup
 * (multi-account support), then we switch back to alice to send.
 */

const NODE_URL = process.env.WALLET_NODE_URL ?? "http://satyawan.local:8080";

test.describe("03 — send + track", () => {
  test("send 100 EVAP and observe finalisation", async ({ popupPage }) => {
    const page = popupPage;

    // Point at the test node.
    await page.evaluate((url: string) => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (globalThis as any).__zustandStore.getState().setNodeUrl(url);
    }, NODE_URL);

    // ── Create alice ───────────────────────────────────────────────
    await page.getByPlaceholder("Account name (e.g. main)").fill("alice");
    await page.getByPlaceholder("Password (min 8 characters)").fill("hunter2hunter2");
    await page.getByPlaceholder("Confirm password").fill("hunter2hunter2");
    await page.getByRole("button", { name: /create wallet/i }).click();
    await expect(page.locator("text=Total Balance")).toBeVisible();

    const aliceAddr = await getActiveAddress(page);

    // ── Create bob in the same keystore via the store ──────────────
    // (No "add account" UI flow exposed yet — the store action is the
    // canonical path.) After this, the active account flips to bob.
    await page.evaluate(async () => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      await (globalThis as any).__zustandStore.getState().createAccount("bob", "hunter2hunter2");
    });
    const bobAddr = await getActiveAddress(page);
    expect(bobAddr).not.toEqual(aliceAddr);

    // Switch back to alice for sending.
    await page.evaluate(() => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (globalThis as any).__zustandStore.getState().switchAccount("alice");
    });

    // ── Faucet alice ───────────────────────────────────────────────
    await page.evaluate(async () => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      await (globalThis as any).__zustandStore.getState().claimFaucet();
    });
    await expect
      .poll(() => page.evaluate(() =>
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        (globalThis as any).__zustandStore.getState().balance as number,
      ), { timeout: 8_000 })
      .toBeGreaterThanOrEqual(100);

    const bobBefore = await nodeClient.getBalance(bobAddr).catch(() => ({ balance: 0, address: bobAddr, nonce: 0 }));

    // ── Send 100 to bob via the store, capture the hash ────────────
    const sendResult = await page.evaluate(async (to: string) => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const s = (globalThis as any).__zustandStore.getState();
      const r = await s.sendTransfer(to, 100);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const pending = (globalThis as any).__zustandStore.getState().pendingTxs as Array<{ hash: string }>;
      return { result: r, hash: pending[pending.length - 1]?.hash };
    }, bobAddr);
    expect(sendResult.result.success).toBe(true);
    expect(sendResult.hash).toMatch(/^0x?[0-9a-f]+/i);

    // ── Pending pill appears in Header ─────────────────────────────
    // Header renders `{N} pending` while inflight count > 0. Match by
    // text containing /pending/i to survive count changes.
    await expect(page.locator("text=/\\d+ pending/i").first()).toBeVisible({ timeout: 10_000 });

    // ── Wait for finalisation node-side (authoritative) ────────────
    const rec = await nodeClient.pollUntilFinalised(sendResult.hash!, 60_000);
    expect(rec.status).toBe("success");

    // ── ActivityScreen row should promote to "Finalised" badge ─────
    // The store's poll loop runs every 3s; give it one extra cycle.
    await page.evaluate(() => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (globalThis as any).__zustandStore.getState().setView("activity");
    });
    await expect
      .poll(
        () =>
          page.evaluate(() => {
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            const txs = (globalThis as any).__zustandStore.getState().pendingTxs as Array<{ status: string }>;
            return txs.some((t) => t.status === "finalised");
          }),
        { timeout: 15_000, intervals: [500, 1_000, 2_000] },
      )
      .toBe(true);

    // ── bob's balance moved by 100 (node-side) ─────────────────────
    const bobAfter = await nodeClient.getBalance(bobAddr);
    expect(bobAfter.balance - bobBefore.balance).toBe(100);
  });
});

async function getActiveAddress(page: import("@playwright/test").Page): Promise<string> {
  return page.evaluate(() => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    return (globalThis as any).__zustandStore.getState().activeAccount.address as string;
  });
}
