/**
 * TypeScript bridge to the ML-DSA WASM module.
 *
 * Initializes the WASM binary once, then exposes keygen / sign / verify
 * that call into real Dilithium3 (ML-DSA-65) post-quantum cryptography.
 *
 * Namespace import: different wasm-pack versions produce slightly
 * different `.d.ts` shapes — some emit named exports, some only emit a
 * default export with the functions as namespace members. Importing
 * the whole module + destructuring at runtime works for both, which
 * keeps the bridge buildable whether CI's wasm-pack is ahead or
 * behind whatever generated the locally-committed .d.ts.
 */

// eslint-disable-next-line @typescript-eslint/no-explicit-any
import * as wasm from "./wasm/evaporchain_crypto_wasm";

const init = (wasm as { default: () => Promise<unknown> }).default;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const mlDsaKeygen = (wasm as any).mlDsaKeygen as () => unknown;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const mlDsaSign = (wasm as any).mlDsaSign as (sk: Uint8Array, msg: Uint8Array) => Uint8Array;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const mlDsaVerify = (wasm as any).mlDsaVerify as (msg: Uint8Array, sig: Uint8Array, pk: Uint8Array) => boolean;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const wasmDeriveAddress = (wasm as any).deriveAddress as (pk: Uint8Array) => string;

let initialized = false;

/**
 * Initialize the WASM module. Must be called once before any crypto operation.
 * Safe to call multiple times — subsequent calls are no-ops.
 */
export async function initCrypto(): Promise<void> {
  if (initialized) return;
  await init();
  initialized = true;
}

/**
 * Generate a new ML-DSA-65 keypair.
 * Returns { publicKey, secretKey } as Uint8Arrays.
 *
 * Public key:  1952 bytes
 * Secret key: ~4000 bytes
 */
export async function generateMlDsaKeypair(): Promise<{
  publicKey: Uint8Array;
  secretKey: Uint8Array;
}> {
  await initCrypto();
  const kp = mlDsaKeygen() as { publicKey: Uint8Array; secretKey: Uint8Array };
  return kp;
}

/**
 * Sign a message with an ML-DSA-65 secret key.
 * Returns a 3293-byte Dilithium3 signature.
 */
export async function signWithMlDsa(
  secretKey: Uint8Array,
  message: Uint8Array
): Promise<Uint8Array> {
  await initCrypto();
  return mlDsaSign(secretKey, message);
}

/**
 * Verify an ML-DSA-65 signature.
 */
export async function verifyMlDsa(
  message: Uint8Array,
  signature: Uint8Array,
  publicKey: Uint8Array
): Promise<boolean> {
  await initCrypto();
  return mlDsaVerify(message, signature, publicKey);
}

/**
 * Derive an EvaporChain address from a public key.
 * Returns "0x" + hex(SHA-256(publicKey)).
 */
export function mlDsaDeriveAddress(publicKey: Uint8Array): string {
  return wasmDeriveAddress(publicKey);
}
