import { test, expect } from "../fixtures";
import { nodeClient } from "../lib/node-client";

/**
 * Spec 02 — Faucet → balance.
 *
 * Creates a wallet, points it at the test node (so refreshBalance hits
 * the Mini, not testnet.evaporchain.com), then claims from the faucet.
 * Asserts the in-popup balance updates within 5s.
 *
 * We trigger the claim via the store directly so we don't depend on
 * the exact label of the QuickAction tile (currently "Faucet" with a
 * 💧 emoji — refactor-prone).
 */

test.describe("02 — faucet + balance", () => {
  test("balance updates after faucet claim", async ({ popupPage }) => {
    const page = popupPage;

    // Point the wallet at the test node.
    await page.evaluate((url: string) => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const s = (globalThis as any).__zustandStore.getState();
      s.setNodeUrl(url);
    }, process.env.WALLET_NODE_URL ?? "http://satyawan.local:8080");

    // Create wallet.
    await page.getByPlaceholder("Account name (e.g. main)").fill("alice");
    await page.getByPlaceholder("Password (min 8 characters)").fill("hunter2hunter2");
    await page.getByPlaceholder("Confirm password").fill("hunter2hunter2");
    await page.getByRole("button", { name: /create wallet/i }).click();

    const address = await page.evaluate(() => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      return (globalThis as any).__zustandStore.getState().activeAccount.address as string;
    });

    // Sanity-check: node-side balance is 0.
    const before = await nodeClient.getBalance(address).catch(() => ({ balance: 0, address, nonce: 0 }));
    expect(before.balance).toBe(0);

    // Claim via the store action — bypasses UI labelling churn.
    await page.evaluate(async () => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      await (globalThis as any).__zustandStore.getState().claimFaucet();
    });

    // Wait for Zustand to reflect the new balance. refreshBalance() is
    // called inside claimFaucet and runs against the node. 5s budget.
    await expect
      .poll(
        async () =>
          page.evaluate(() => {
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            return (globalThis as any).__zustandStore.getState().balance as number;
          }),
        { timeout: 5_000, intervals: [250, 500, 1_000] },
      )
      .toBeGreaterThan(0);

    // Cross-check against node directly.
    const after = await nodeClient.getBalance(address);
    expect(after.balance).toBeGreaterThan(0);
  });
});
