#!/usr/bin/env node
/**
 * verify-wasm.mjs — guards the extension build against a tampered or stale
 * pre-built WASM blob.
 *
 * Reads extension/src/crypto/wasm/checksums.json and recomputes the sha256
 * of the actual .wasm + .js files in that directory. Exits 0 on match, 1
 * otherwise. Wired in as the `prebuild` script in package.json.
 *
 * Pure Node — no npm dependencies. Uses crypto + fs from stdlib.
 */
import { createHash } from "node:crypto";
import { readFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const WASM_DIR = resolve(__dirname, "..", "src", "crypto", "wasm");
const CHECKSUMS_PATH = resolve(WASM_DIR, "checksums.json");

const RED = "\x1b[31m";
const GREEN = "\x1b[32m";
const YELLOW = "\x1b[33m";
const BOLD = "\x1b[1m";
const RESET = "\x1b[0m";

function fail(msg) {
  process.stderr.write(`${RED}${BOLD}verify-wasm: ${msg}${RESET}\n`);
  process.exit(1);
}

function info(msg) {
  process.stdout.write(`verify-wasm: ${msg}\n`);
}

if (!existsSync(CHECKSUMS_PATH)) {
  fail(
    `${CHECKSUMS_PATH} not found. ` +
      `Run 'bash scripts/build-wasm.sh' first to produce it (must be run on a ` +
      `machine with the pinned Rust + wasm-pack + wasm-opt toolchain).`,
  );
}

let manifest;
try {
  manifest = JSON.parse(readFileSync(CHECKSUMS_PATH, "utf8"));
} catch (err) {
  fail(`failed to parse ${CHECKSUMS_PATH}: ${err.message}`);
}

// pinned_commit drift is allowed; it's provenance, not enforcement.
const REQUIRED_FILE_FIELDS = [
  "evaporchain_crypto_wasm_bg.wasm",
  "evaporchain_crypto_wasm.js",
];

const ZERO_HASH = "0".repeat(64);

const mismatches = [];

for (const fileName of REQUIRED_FILE_FIELDS) {
  const expected = manifest[fileName];
  if (typeof expected !== "string") {
    fail(`checksums.json missing required field '${fileName}'.`);
  }
  if (expected === ZERO_HASH) {
    fail(
      `checksums.json contains the placeholder zero hash for '${fileName}'. ` +
        `Run 'bash scripts/build-wasm.sh' on a Mini with the pinned toolchain ` +
        `to produce real hashes, then commit the resulting checksums.json.`,
    );
  }

  const filePath = resolve(WASM_DIR, fileName);
  if (!existsSync(filePath)) {
    fail(`expected artifact ${filePath} is missing.`);
  }

  const actual = createHash("sha256")
    .update(readFileSync(filePath))
    .digest("hex");

  if (actual !== expected) {
    mismatches.push({ fileName, expected, actual });
  }
}

if (mismatches.length > 0) {
  process.stderr.write(`${RED}${BOLD}verify-wasm: checksum mismatch — refusing to build.${RESET}\n`);
  for (const m of mismatches) {
    process.stderr.write(`  ${BOLD}${m.fileName}${RESET}\n`);
    process.stderr.write(`    expected: ${GREEN}${m.expected}${RESET}\n`);
    process.stderr.write(`    actual:   ${RED}${m.actual}${RESET}\n`);
  }
  process.stderr.write(
    `\n${YELLOW}If you intentionally rebuilt the WASM, run 'bash scripts/build-wasm.sh' ` +
      `to regenerate checksums.json and commit it alongside the new artifacts.${RESET}\n`,
  );
  process.exit(1);
}

info(
  `ok — wasm + js checksums match checksums.json ` +
    `(built_at=${manifest.built_at ?? "?"}, commit=${manifest.pinned_commit ?? "?"}).`,
);
process.exit(0);
