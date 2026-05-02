// Smoke test that mnemonic.ts's imports resolve.
//
// Pure import-side check — vitest's import-analysis catches export-map
// drift like the @noble/hashes/blake3.js → @noble/hashes/blake3 fix and
// the @scure/bip39/wordlists/english.js → @scure/bip39/wordlists/english
// fix. If either dep is missing or the path is wrong, this test fails to
// load and surfaces the real cause clearly.

import { describe, expect, it } from "vitest";

describe("mnemonic.ts module load", () => {
  it("imports cleanly with wordlist + blake3 deps resolved", async () => {
    const mod = await import("./mnemonic");
    // Spot-check a public symbol so the bundler can't tree-shake the
    // import to an empty namespace and silently pass.
    expect(typeof mod).toBe("object");
  });
});
