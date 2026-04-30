/**
 * Secure Keystore for EvaporChain Mobile Wallet
 *
 * Stores ML-DSA-65 keypairs on-device via expo-secure-store, signs
 * messages with the real `@noble/post-quantum` ML-DSA implementation,
 * and produces / consumes the exact `MnemonicBackup` JSON envelope used
 * by the Rust wallet (wallet/src/mnemonic.rs).
 *
 * Backup envelope:
 *   {
 *     version: 1,
 *     account_index: u32,
 *     encrypted_keypair: hex,
 *     nonce: hex (12 bytes),
 *     address: "0x..." hex
 *   }
 *
 * Plaintext layout (before AES-256-GCM encrypt):
 *   u32_le(pk_len) || pk || sk
 *
 * KDF for the AES key:
 *   key = BLAKE3("evaporchain-seed" || entropy || u32_le(account_index))
 *
 * The 16-byte ASCII prefix is hard-coded to match
 * `Mnemonic::derive_key_at` in the Rust wallet.
 */

import * as SecureStore from 'expo-secure-store';
import { ml_dsa65 } from '@noble/post-quantum/ml-dsa';
import { gcm } from '@noble/ciphers/aes';
import { blake3 } from '@noble/hashes/blake3';

import { mnemonicToEntropyBytes } from './keygen';

// ─────────────────────────── secure-store keys ────────────────────────────

const KEYS = {
  PRIVATE_KEY: 'evap_private_key',
  PUBLIC_KEY: 'evap_public_key',
  ADDRESS: 'evap_address',
  SEED_PHRASE: 'evap_seed_phrase',
  PIN_HASH: 'evap_pin_hash',
  WALLET_CREATED: 'evap_wallet_created',
  AUTO_LOCK_TIMEOUT: 'evap_auto_lock_timeout',
  HW_ACCOUNTS: 'evap_hw_accounts',
  ACCOUNT_INDEX: 'evap_account_index',
} as const;

const SECURE_OPTIONS: SecureStore.SecureStoreOptions = {
  keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY,
};

// ─────────────────────────── backup envelope ──────────────────────────────

export interface MnemonicBackup {
  version: number;
  account_index: number;
  encrypted_keypair: string; // hex
  nonce: string; // hex (12 bytes)
  address: string; // 0x-prefixed
}

// ─────────────────────────── helpers ──────────────────────────────────────

function hexToBytes(hex: string): Uint8Array {
  const clean = hex.startsWith('0x') ? hex.slice(2) : hex;
  if (clean.length % 2 !== 0) throw new Error('invalid hex length');
  const out = new Uint8Array(clean.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(clean.substr(i * 2, 2), 16);
  }
  return out;
}

function bytesToHex(bytes: Uint8Array): string {
  let out = '';
  for (let i = 0; i < bytes.length; i++) {
    out += (bytes[i] as number).toString(16).padStart(2, '0');
  }
  return out;
}

function getRandomBytes(length: number): Uint8Array {
  const bytes = new Uint8Array(length);
  if (typeof globalThis.crypto !== 'undefined' && globalThis.crypto.getRandomValues) {
    globalThis.crypto.getRandomValues(bytes);
  } else {
    throw new Error(
      'Secure RNG unavailable: ensure react-native-get-random-values is imported at app entry'
    );
  }
  return bytes;
}

function u32LE(n: number): Uint8Array {
  const out = new Uint8Array(4);
  out[0] = n & 0xff;
  out[1] = (n >>> 8) & 0xff;
  out[2] = (n >>> 16) & 0xff;
  out[3] = (n >>> 24) & 0xff;
  return out;
}

function readU32LE(b: Uint8Array, off: number): number {
  return (
    ((b[off] as number) |
      ((b[off + 1] as number) << 8) |
      ((b[off + 2] as number) << 16) |
      ((b[off + 3] as number) << 24)) >>>
    0
  );
}

const SEED_PREFIX = new TextEncoder().encode('evaporchain-seed');
if (SEED_PREFIX.length !== 16) {
  // Defensive: matches Rust b"evaporchain-seed" — 16 ASCII bytes.
  throw new Error('seed prefix must be exactly 16 bytes');
}

function deriveBackupKey(entropy: Uint8Array, accountIndex: number): Uint8Array {
  if (entropy.length !== 32) throw new Error('entropy must be 32 bytes');
  const input = new Uint8Array(SEED_PREFIX.length + entropy.length + 4);
  input.set(SEED_PREFIX, 0);
  input.set(entropy, SEED_PREFIX.length);
  input.set(u32LE(accountIndex), SEED_PREFIX.length + entropy.length);
  return blake3(input);
}

/**
 * SHA-256-style PIN hash via BLAKE3 (sufficient for local PIN-gating;
 * the actual key material is not derived from the PIN).
 */
async function hashPin(pin: string): Promise<string> {
  const data = new TextEncoder().encode(`evap_salt_v1::${pin}`);
  return bytesToHex(blake3(data));
}

// ─────────────────────────── backup encode / decode ───────────────────────

/**
 * Encrypt an ML-DSA keypair under the mnemonic's derived key and emit
 * the exact MnemonicBackup JSON shape the Rust wallet expects.
 */
export function exportBackup(
  mnemonicPhrase: string,
  pkHex: string,
  skHex: string,
  addressHex: string,
  accountIndex = 0
): MnemonicBackup {
  const entropy = mnemonicToEntropyBytes(mnemonicPhrase);
  const key = deriveBackupKey(entropy, accountIndex);

  const pk = hexToBytes(pkHex);
  const sk = hexToBytes(skHex);

  // Plaintext = u32_le(pk_len) || pk || sk
  const plaintext = new Uint8Array(4 + pk.length + sk.length);
  plaintext.set(u32LE(pk.length), 0);
  plaintext.set(pk, 4);
  plaintext.set(sk, 4 + pk.length);

  const nonce = getRandomBytes(12);
  const ciphertext = gcm(key, nonce).encrypt(plaintext);

  // Best-effort zeroize.
  plaintext.fill(0);
  key.fill(0);
  entropy.fill(0);

  return {
    version: 1,
    account_index: accountIndex,
    encrypted_keypair: bytesToHex(ciphertext),
    nonce: bytesToHex(nonce),
    address: addressHex,
  };
}

/**
 * Decrypt a MnemonicBackup with the mnemonic that produced it.
 * Returns the recovered ML-DSA keypair as hex + the encoded address.
 */
export function importBackup(
  mnemonicPhrase: string,
  backup: MnemonicBackup
): { publicKey: string; privateKey: string; address: string; accountIndex: number } {
  if (backup.version !== 1) {
    throw new Error(`unsupported backup version: ${backup.version}`);
  }
  const entropy = mnemonicToEntropyBytes(mnemonicPhrase);
  const key = deriveBackupKey(entropy, backup.account_index);

  const ciphertext = hexToBytes(backup.encrypted_keypair);
  const nonce = hexToBytes(backup.nonce);
  if (nonce.length !== 12) throw new Error('backup nonce must be 12 bytes');

  let plaintext: Uint8Array;
  try {
    plaintext = gcm(key, nonce).decrypt(ciphertext);
  } catch {
    key.fill(0);
    entropy.fill(0);
    throw new Error('wrong mnemonic or corrupted backup');
  }

  if (plaintext.length < 4) throw new Error('backup payload too short');
  const pkLen = readU32LE(plaintext, 0);
  if (plaintext.length < 4 + pkLen) throw new Error('backup payload truncated');

  const pk = plaintext.slice(4, 4 + pkLen);
  const sk = plaintext.slice(4 + pkLen);

  const publicKey = bytesToHex(pk);
  const privateKey = bytesToHex(sk);
  const address = '0x' + bytesToHex(blake3(pk));

  // Best-effort zeroize.
  plaintext.fill(0);
  key.fill(0);
  entropy.fill(0);

  return {
    publicKey,
    privateKey,
    address,
    accountIndex: backup.account_index,
  };
}

// ─────────────────────────── signing ──────────────────────────────────────

/**
 * Sign a message with the on-device ML-DSA-65 secret key.
 * Returns the detached signature as a hex string.
 */
export async function signMessage(message: Uint8Array): Promise<string> {
  const skHex = await SecureStore.getItemAsync(KEYS.PRIVATE_KEY, SECURE_OPTIONS);
  if (!skHex) throw new Error('no wallet on device');
  const sk = hexToBytes(skHex);
  const sig = ml_dsa65.sign(sk, message);
  sk.fill(0);
  return bytesToHex(sig);
}

/**
 * Verify the PIN and return whether the wallet is unlockable.
 * (PIN-gating only — the secret key itself is in the secure enclave.)
 */
export async function unlockWallet(pin: string): Promise<boolean> {
  return keystore.verifyPin(pin);
}

// ─────────────────────────── keystore object ──────────────────────────────

export const keystore = {
  /**
   * Persist a freshly-generated wallet to secure storage.
   */
  async createWallet(
    privateKey: string,
    publicKey: string,
    address: string,
    seedPhrase: string,
    pin: string,
    accountIndex = 0
  ): Promise<void> {
    const pinHash = await hashPin(pin);
    await Promise.all([
      SecureStore.setItemAsync(KEYS.PRIVATE_KEY, privateKey, SECURE_OPTIONS),
      SecureStore.setItemAsync(KEYS.PUBLIC_KEY, publicKey, SECURE_OPTIONS),
      SecureStore.setItemAsync(KEYS.ADDRESS, address, SECURE_OPTIONS),
      SecureStore.setItemAsync(KEYS.SEED_PHRASE, seedPhrase, SECURE_OPTIONS),
      SecureStore.setItemAsync(KEYS.PIN_HASH, pinHash, SECURE_OPTIONS),
      SecureStore.setItemAsync(KEYS.WALLET_CREATED, 'true', SECURE_OPTIONS),
      SecureStore.setItemAsync(KEYS.ACCOUNT_INDEX, String(accountIndex), SECURE_OPTIONS),
    ]);
  },

  async hasWallet(): Promise<boolean> {
    const created = await SecureStore.getItemAsync(KEYS.WALLET_CREATED, SECURE_OPTIONS);
    return created === 'true';
  },

  async verifyPin(pin: string): Promise<boolean> {
    const stored = await SecureStore.getItemAsync(KEYS.PIN_HASH, SECURE_OPTIONS);
    if (!stored) return false;
    const candidate = await hashPin(pin);
    return stored === candidate;
  },

  async changePin(currentPin: string, newPin: string): Promise<boolean> {
    const valid = await this.verifyPin(currentPin);
    if (!valid) return false;
    const newHash = await hashPin(newPin);
    await SecureStore.setItemAsync(KEYS.PIN_HASH, newHash, SECURE_OPTIONS);
    return true;
  },

  async getAddress(): Promise<string | null> {
    return SecureStore.getItemAsync(KEYS.ADDRESS, SECURE_OPTIONS);
  },

  async getPublicKey(): Promise<string | null> {
    return SecureStore.getItemAsync(KEYS.PUBLIC_KEY, SECURE_OPTIONS);
  },

  async getPrivateKey(): Promise<string | null> {
    return SecureStore.getItemAsync(KEYS.PRIVATE_KEY, SECURE_OPTIONS);
  },

  async getSeedPhrase(): Promise<string | null> {
    return SecureStore.getItemAsync(KEYS.SEED_PHRASE, SECURE_OPTIONS);
  },

  async getAccountIndex(): Promise<number> {
    const v = await SecureStore.getItemAsync(KEYS.ACCOUNT_INDEX, SECURE_OPTIONS);
    return v ? parseInt(v, 10) || 0 : 0;
  },

  /**
   * Sign an arbitrary message with the on-device secret key (after the
   * caller has gated this behind a PIN / biometric).
   */
  signMessage,

  /** PIN-gated unlock convenience wrapper. */
  unlockWallet,

  /**
   * Export the on-device wallet as a MnemonicBackup envelope (JSON-safe).
   * PIN-gated.  Returns the JSON string the Rust wallet can ingest.
   */
  async exportKeystore(pin: string): Promise<string | null> {
    const valid = await this.verifyPin(pin);
    if (!valid) return null;

    const [pk, sk, address, phrase, idx] = await Promise.all([
      this.getPublicKey(),
      this.getPrivateKey(),
      this.getAddress(),
      this.getSeedPhrase(),
      this.getAccountIndex(),
    ]);
    if (!pk || !sk || !address || !phrase) return null;

    const backup = exportBackup(phrase, pk, sk, address, idx);
    return JSON.stringify(backup, null, 2);
  },

  /**
   * Re-export of the pure `importBackup` for symmetry; the screen wires
   * straight to the named export.
   */
  importBackup,

  /**
   * Re-export of the pure `exportBackup` so screens can build a backup
   * envelope without needing a stored wallet (e.g. immediately after
   * generation, before persistence).
   */
  exportBackup,

  async setAutoLockTimeout(minutes: number): Promise<void> {
    await SecureStore.setItemAsync(
      KEYS.AUTO_LOCK_TIMEOUT,
      minutes.toString(),
      SECURE_OPTIONS
    );
  },

  async getAutoLockTimeout(): Promise<number> {
    const val = await SecureStore.getItemAsync(KEYS.AUTO_LOCK_TIMEOUT, SECURE_OPTIONS);
    return val ? parseInt(val, 10) : 5;
  },

  async importHardwareAddress(address: string, path: string): Promise<void> {
    const raw = await SecureStore.getItemAsync(KEYS.HW_ACCOUNTS, SECURE_OPTIONS);
    const existing: Array<{ address: string; path: string }> = raw ? JSON.parse(raw) : [];
    if (!existing.find((a) => a.address === address)) {
      existing.push({ address, path });
    }
    await SecureStore.setItemAsync(KEYS.HW_ACCOUNTS, JSON.stringify(existing), SECURE_OPTIONS);
    const created = await SecureStore.getItemAsync(KEYS.WALLET_CREATED, SECURE_OPTIONS);
    if (created !== 'true') {
      await SecureStore.setItemAsync(KEYS.ADDRESS, address, SECURE_OPTIONS);
      await SecureStore.setItemAsync(KEYS.WALLET_CREATED, 'hw', SECURE_OPTIONS);
    }
  },

  async getHardwareAccounts(): Promise<Array<{ address: string; path: string }>> {
    const raw = await SecureStore.getItemAsync(KEYS.HW_ACCOUNTS, SECURE_OPTIONS);
    return raw ? JSON.parse(raw) : [];
  },

  async deleteWallet(): Promise<void> {
    await Promise.all(Object.values(KEYS).map((key) => SecureStore.deleteItemAsync(key)));
  },
};

export default keystore;
