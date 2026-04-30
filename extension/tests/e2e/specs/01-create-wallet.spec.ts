import { test, expect } from "../fixtures";

/**
 * Spec 01 — Create wallet, lock, unlock.
 *
 * Pure popup-side; no node interaction. Verifies the most fundamental
 * flow: a fresh install lands on the create screen, the user can mint
 * a wallet, the address renders in Header, and the lock/unlock cycle
 * round-trips through the password.
 */

test.describe("01 — wallet creation + lock cycle", () => {
  test("creates a wallet and unlocks after lock", async ({ popupPage, extensionId, extensionContext }) => {
    const page = popupPage;

    // Fresh install → CreateAccount screen. We assert by checking the
    // create form's password fields are visible (the screen has a
    // hard-coded heading "Create Wallet" but matching by role+name is
    // more resilient to copy tweaks).
    await expect(page.getByRole("heading", { name: /create wallet/i })).toBeVisible();

    // Fill the form. Selectors are by placeholder which the component
    // renders verbatim (`CreateAccount.tsx`).
    await page.getByPlaceholder("Account name (e.g. main)").fill("alice");
    await page.getByPlaceholder("Password (min 8 characters)").fill("hunter2hunter2");
    await page.getByPlaceholder("Confirm password").fill("hunter2hunter2");
    await page.getByRole("button", { name: /create wallet/i }).click();

    // After creation we land on home; Header renders the active address
    // truncated as `0x….….` via `shortAddress`. We pull the raw address
    // from the Zustand store rather than parsing the truncation.
    const address = await page.evaluate(() => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const s = (globalThis as any).__zustandStore.getState();
      return s.activeAccount?.address as string | undefined;
    });
    expect(address).toBeTruthy();
    expect(address!).toMatch(/^0x[0-9a-f]+/i);

    // Header shows the account name we picked.
    await expect(page.locator("text=alice").first()).toBeVisible();

    // ── Close + reopen popup ───────────────────────────────────────
    await page.close();
    const popup2 = await extensionContext.newPage();
    await popup2.goto(`chrome-extension://${extensionId}/popup.html`);
    await popup2.waitForFunction(
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      () => typeof (globalThis as any).__zustandStore !== "undefined",
      undefined,
      { timeout: 15_000 },
    );

    // Wallet should be locked. LockScreen renders a Password input +
    // an "Unlock" button; CreateAccount does not. Distinguish by the
    // presence of the unlock CTA.
    await expect(popup2.getByRole("button", { name: /^unlock$/i })).toBeVisible();

    await popup2.getByPlaceholder("Password").fill("hunter2hunter2");
    await popup2.getByRole("button", { name: /^unlock$/i }).click();

    // Home screen again — quickest stable signal is the "Total Balance" label.
    await expect(popup2.locator("text=Total Balance")).toBeVisible({ timeout: 10_000 });
    await popup2.close();
  });
});
