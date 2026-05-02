/**
 * Mnemonic recovery — JS port of `wallet/src/mnemonic.rs`.
 *
 * Compatible byte-for-byte with the Rust CLI's `MnemonicBackup` flow:
 *
 *   - 24-word BIP-39 English wordlist
 *   - Custom checksum: BLAKE3(entropy)[0] == checksum_byte (NOT standard BIP-39
 *     SHA-256). This is why we can't use `@scure/bip39::validateMnemonic`.
 *   - Encryption key: BLAKE3("evaporchain-seed" || entropy || account_index_le4)
 *   - AES-256-GCM (12-byte nonce) decrypts into: pk_len_le4 || pk || sk
 *
 * Read `wallet/src/mnemonic.rs::derive_key_at` and `recover_keypair`. This file
 * is the JS twin and must stay in lock-step.
 */
import { wordlist as ENGLISH_WORDLIST } from "@scure/bip39/wordlists/english.js";
import { blake3 } from "@noble/hashes/blake3";

export interface MnemonicBackup {
  version: number;
  account_index: number;
  /** AES-256-GCM ciphertext, hex (encrypted: pk_len_le4 || pk || sk). */
  encrypted_keypair: string;
  /** AES-256-GCM nonce, hex (12 bytes). */
  nonce: string;
  /** Address derived from the embedded public key. */
  address: string;
}

export class MnemonicError extends Error {}

const WORD_COUNT = 24;
const SEED_DOMAIN = new TextEncoder().encode("evaporchain-seed");

// ──────────────────────────── Hex helpers ──────────────────────────────

function fromHex(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) throw new MnemonicError("invalid hex length");
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    const b = parseInt(hex.substring(i, i + 2), 16);
    if (Number.isNaN(b)) throw new MnemonicError("invalid hex character");
    bytes[i / 2] = b;
  }
  return bytes;
}

// ──────────────────────────── Mnemonic → entropy ──────────────────────

/**
 * Parse a 24-word phrase into its 32-byte entropy. Validates against the
 * Rust wallet's BLAKE3-based checksum (NOT standard BIP-39 SHA-256).
 */
export function phraseToEntropy(phrase: string): Uint8Array {
  const words = phrase
    .trim()
    .split(/\s+/)
    .map((w) => w.toLowerCase());
  if (words.length !== WORD_COUNT) {
    throw new MnemonicError(`expected ${WORD_COUNT} words, got ${words.length}`);
  }

  const indices: number[] = [];
  for (const w of words) {
    const idx = ENGLISH_WORDLIST.indexOf(w);
    if (idx < 0) throw new MnemonicError(`unknown word in mnemonic: '${w}'`);
    indices.push(idx);
  }

  // 24 words × 11 bits = 264 bits = 33 bytes. Each index supplies 11 bits MSB-first.
  const bits: number[] = new Array(264);
  let p = 0;
  for (const idx of indices) {
    for (let i = 10; i >= 0; i--) bits[p++] = (idx >> i) & 1;
  }

  const entropy = new Uint8Array(32);
  for (let i = 0; i < 32; i++) {
    let byte = 0;
    for (let j = 0; j < 8; j++) byte = (byte << 1) | bits[i * 8 + j];
    entropy[i] = byte;
  }
  let checksumByte = 0;
  for (let j = 0; j < 8; j++) checksumByte = (checksumByte << 1) | bits[256 + j];

  const expected = blake3(entropy)[0];
  if (checksumByte !== expected) {
    throw new MnemonicError("invalid mnemonic checksum");
  }
  return entropy;
}

/**
 * Derive the AES-256-GCM key for a specific account index.
 *
 * Mirrors `Mnemonic::derive_key_at` exactly:
 *   key = BLAKE3("evaporchain-seed" || entropy || index.to_le_bytes())
 */
export function deriveKeyAt(entropy: Uint8Array, accountIndex: number): Uint8Array {
  const indexLe = new Uint8Array(4);
  new DataView(indexLe.buffer).setUint32(0, accountIndex >>> 0, /* littleEndian */ true);

  const input = new Uint8Array(SEED_DOMAIN.length + entropy.length + 4);
  input.set(SEED_DOMAIN, 0);
  input.set(entropy, SEED_DOMAIN.length);
  input.set(indexLe, SEED_DOMAIN.length + entropy.length);
  return blake3(input);
}

// ──────────────────────────── Recovery ─────────────────────────────────

export interface RecoveredKeypair {
  publicKey: Uint8Array;
  secretKey: Uint8Array;
}

/**
 * Recover an ML-DSA-65 keypair from a `MnemonicBackup` JSON object using the
 * provided phrase. Throws `MnemonicError` on bad mnemonic, bad checksum,
 * malformed JSON fields, or AES tag failure (wrong mnemonic / corrupted
 * backup).
 */
export async function recoverKeypair(
  phrase: string,
  backup: MnemonicBackup,
): Promise<RecoveredKeypair> {
  if (typeof backup !== "object" || backup === null) {
    throw new MnemonicError("backup must be an object");
  }
  if (typeof backup.encrypted_keypair !== "string" || typeof backup.nonce !== "string") {
    throw new MnemonicError("backup is missing encrypted_keypair / nonce");
  }
  const accountIndex = backup.account_index ?? 0;

  const entropy = phraseToEntropy(phrase);
  const keyBytes = deriveKeyAt(entropy, accountIndex);
  const ciphertext = fromHex(backup.encrypted_keypair);
  const nonce = fromHex(backup.nonce);
  if (nonce.length !== 12) throw new MnemonicError("nonce must be 12 bytes");

  const key = await crypto.subtle.importKey(
    "raw",
    keyBytes as BufferSource,
    { name: "AES-GCM" },
    false,
    ["decrypt"],
  );
  let plaintext: Uint8Array;
  try {
    plaintext = new Uint8Array(
      await crypto.subtle.decrypt(
        { name: "AES-GCM", iv: nonce as BufferSource },
        key,
        ciphertext as BufferSource,
      ),
    );
  } catch {
    throw new MnemonicError("decryption failed: wrong mnemonic or corrupted backup");
  }

  if (plaintext.length < 4) throw new MnemonicError("backup too short");
  const view = new DataView(plaintext.buffer, plaintext.byteOffset, plaintext.byteLength);
  const pkLen = view.getUint32(0, /* littleEndian */ true);
  if (plaintext.length < 4 + pkLen) throw new MnemonicError("backup truncated");
  const publicKey = plaintext.slice(4, 4 + pkLen);
  const secretKey = plaintext.slice(4 + pkLen);
  return { publicKey, secretKey };
}
