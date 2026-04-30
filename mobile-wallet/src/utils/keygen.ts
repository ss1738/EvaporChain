/**
 * Wallet Key Generation for EvaporChain
 *
 * Real ML-DSA-65 keypairs via @noble/post-quantum, plus a 24-word
 * BIP-39 mnemonic with the EvaporChain custom checksum BLAKE3(entropy)[0]
 * (matches wallet/src/mnemonic.rs).
 *
 * Address derivation: address = "0x" + hex(BLAKE3(pk)) — matches
 * wallet/src/address.rs.
 *
 * NOTE: ML-DSA does not support seed-derived keypair generation, so
 * recovery requires both the mnemonic AND an encrypted backup envelope
 * (see keystore.ts).  Mnemonic-only recovery is intentionally
 * unsupported.
 */

import { ml_dsa65 } from '@noble/post-quantum/ml-dsa';
import { blake3 } from '@noble/hashes/blake3';
// `bip39` re-exports the English wordlist as `wordlists.english`
// (a 2048-element string array).  This avoids depending on a JSON
// import path that varies across module bundlers.
import * as bip39 from 'bip39';

export interface WalletKeys {
  /** Hex-encoded ML-DSA-65 public key. */
  publicKey: string;
  /** Hex-encoded ML-DSA-65 secret key. */
  privateKey: string;
  /** 0x-prefixed 32-byte BLAKE3(pk) hex string. */
  address: string;
  /** 24-word BIP-39 mnemonic with custom EvaporChain checksum. */
  seedPhrase: string;
  /** 32 raw entropy bytes that produced the mnemonic. */
  entropy: Uint8Array;
}

// ─────────────────────────────── helpers ──────────────────────────────────

function getRandomBytes(length: number): Uint8Array {
  const bytes = new Uint8Array(length);
  if (typeof globalThis.crypto !== 'undefined' && globalThis.crypto.getRandomValues) {
    globalThis.crypto.getRandomValues(bytes);
  } else {
    // No fallback — refusing to mint keys with Math.random.
    throw new Error(
      'Secure RNG unavailable: ensure react-native-get-random-values is imported at app entry'
    );
  }
  return bytes;
}

function bytesToHex(bytes: Uint8Array): string {
  let out = '';
  for (let i = 0; i < bytes.length; i++) {
    out += (bytes[i] as number).toString(16).padStart(2, '0');
  }
  return out;
}

// ─────────────────────────────── wordlist ─────────────────────────────────

// Resolve the BIP-39 English wordlist across the various shapes the
// `bip39` package can present at runtime (CJS default-export, ESM named
// export, bundler-rewritten module).
const WORDLIST: string[] = (() => {
  const mod = bip39 as unknown as Record<string, unknown> & {
    default?: Record<string, unknown>;
  };
  const candidates: unknown[] = [
    (mod.wordlists as Record<string, unknown> | undefined)?.english,
    (mod.default?.wordlists as Record<string, unknown> | undefined)?.english,
    (mod as Record<string, unknown>).english,
    mod.default?.english,
  ];
  for (const c of candidates) {
    if (Array.isArray(c) && c.length === 2048) return c as string[];
  }
  throw new Error('bip39 English wordlist failed to load (expected 2048 words)');
})();

// ─────────────────────────────── mnemonic ─────────────────────────────────

/**
 * Encode 32 bytes of entropy + the EvaporChain checksum byte into a
 * 24-word phrase.  Checksum = BLAKE3(entropy)[0] — matches
 * Mnemonic::from_entropy in wallet/src/mnemonic.rs.
 */
function entropyToMnemonic(entropy: Uint8Array): string {
  if (entropy.length !== 32) {
    throw new Error(`entropy must be 32 bytes, got ${entropy.length}`);
  }
  const checksum = blake3(entropy)[0] as number;

  // 256 entropy bits + 8 checksum bits = 264 bits = 24 words × 11 bits.
  const bits: number[] = [];
  for (let i = 0; i < 32; i++) {
    const byte = entropy[i] as number;
    for (let b = 7; b >= 0; b--) bits.push((byte >> b) & 1);
  }
  for (let b = 7; b >= 0; b--) bits.push((checksum >> b) & 1);

  const words: string[] = [];
  for (let w = 0; w < 24; w++) {
    let idx = 0;
    for (let b = 0; b < 11; b++) {
      idx = (idx << 1) | (bits[w * 11 + b] as number);
    }
    words.push(WORDLIST[idx] as string);
  }
  return words.join(' ');
}

/**
 * Parse a phrase into 32 entropy bytes.  Verifies the EvaporChain checksum.
 */
export function mnemonicToEntropyBytes(phrase: string): Uint8Array {
  const words = phrase.trim().toLowerCase().split(/\s+/);
  if (words.length !== 24) {
    throw new Error(`mnemonic must be 24 words, got ${words.length}`);
  }

  const indexFor = new Map<string, number>();
  for (let i = 0; i < WORDLIST.length; i++) indexFor.set(WORDLIST[i] as string, i);

  const indices: number[] = [];
  for (const w of words) {
    const i = indexFor.get(w);
    if (i === undefined) throw new Error(`unknown word in mnemonic: '${w}'`);
    indices.push(i);
  }

  const bits: number[] = [];
  for (const idx of indices) {
    for (let b = 10; b >= 0; b--) bits.push((idx >> b) & 1);
  }

  const entropy = new Uint8Array(32);
  for (let i = 0; i < 32; i++) {
    let byte = 0;
    for (let b = 0; b < 8; b++) byte = (byte << 1) | (bits[i * 8 + b] as number);
    entropy[i] = byte;
  }

  let checksum = 0;
  for (let b = 0; b < 8; b++) checksum = (checksum << 1) | (bits[256 + b] as number);

  const expected = blake3(entropy)[0] as number;
  if (checksum !== expected) throw new Error('invalid mnemonic checksum');

  return entropy;
}

/** Generate a fresh 24-word mnemonic. */
export function generateMnemonic(): string {
  const entropy = getRandomBytes(32);
  return entropyToMnemonic(entropy);
}

/** Validate phrase + EvaporChain checksum. */
export function validateMnemonic(phrase: string): boolean {
  try {
    mnemonicToEntropyBytes(phrase);
    return true;
  } catch {
    return false;
  }
}

// ─────────────────────────────── keys ─────────────────────────────────────

/**
 * Derive an EvaporChain address from a raw ML-DSA public key.
 *   address = "0x" + hex(BLAKE3(pk))
 */
export function deriveAddressFromPubKey(pk: Uint8Array): string {
  return '0x' + bytesToHex(blake3(pk));
}

/**
 * Generate a new wallet: real ML-DSA-65 keypair + fresh mnemonic.
 *
 * `ml_dsa65.keygen()` accepts a 32-byte seed; we feed it secure RNG
 * bytes (independent from the mnemonic entropy — ML-DSA cannot be
 * deterministically re-derived from a mnemonic, so the keypair is
 * backed up via encrypted envelope, not regenerated from seed words).
 */
export async function generateWallet(): Promise<WalletKeys> {
  return generateWalletSync();
}

/**
 * Synchronous variant for code paths that already run inside an
 * `await` boundary or non-blocking startup.
 */
export function generateWalletSync(): WalletKeys {
  const entropy = getRandomBytes(32);
  const seedPhrase = entropyToMnemonic(entropy);

  const keySeed = getRandomBytes(32);
  const kp = ml_dsa65.keygen(keySeed);

  const publicKey = bytesToHex(kp.publicKey);
  const privateKey = bytesToHex(kp.secretKey);
  const address = deriveAddressFromPubKey(kp.publicKey);

  return { publicKey, privateKey, address, seedPhrase, entropy };
}
