/**
 * Browser-side encrypted keystore for EvaporChain wallet.
 *
 * Uses Web Crypto API:
 * - PBKDF2 for key derivation (password → AES key)
 * - AES-256-GCM for encrypting private keys at rest
 *
 * Key generation and signing use real ML-DSA-65 (Dilithium3) post-quantum
 * cryptography via WASM bridge to evaporchain-crypto-wasm.
 */

import { generateMlDsaKeypair, signWithMlDsa, mlDsaDeriveAddress } from "./wasm-bridge";

export interface KeyEntry {
  name: string;
  address: string;
  publicKey: string; // hex
  encryptedPrivateKey: string; // hex
  iv: string; // hex, 12 bytes
  salt: string; // hex, 32 bytes
  createdAt: string;
}

export interface KeyStoreData {
  version: number;
  entries: KeyEntry[];
  activeAccount: string | null;
}

const KEYSTORE_VERSION = 1;
const STORAGE_KEY = "evaporchain_keystore";

// ── Crypto helpers ──

async function deriveKey(password: string, salt: Uint8Array): Promise<CryptoKey> {
  const enc = new TextEncoder();
  const keyMaterial = await crypto.subtle.importKey(
    "raw",
    enc.encode(password) as BufferSource,
    "PBKDF2",
    false,
    ["deriveKey"]
  );
  return crypto.subtle.deriveKey(
    { name: "PBKDF2", salt: salt as BufferSource, iterations: 600_000, hash: "SHA-256" },
    keyMaterial,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"]
  );
}

async function encrypt(data: Uint8Array, password: string): Promise<{ ciphertext: Uint8Array; iv: Uint8Array; salt: Uint8Array }> {
  const salt = crypto.getRandomValues(new Uint8Array(32));
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const key = await deriveKey(password, salt);
  const ciphertext = new Uint8Array(
    await crypto.subtle.encrypt({ name: "AES-GCM", iv: iv as BufferSource }, key, data as BufferSource)
  );
  return { ciphertext, iv, salt };
}

async function decrypt(ciphertext: Uint8Array, iv: Uint8Array, salt: Uint8Array, password: string): Promise<Uint8Array> {
  const key = await deriveKey(password, salt);
  return new Uint8Array(
    await crypto.subtle.decrypt({ name: "AES-GCM", iv: iv as BufferSource }, key, ciphertext as BufferSource)
  );
}

function toHex(bytes: Uint8Array): string {
  return Array.from(bytes).map(b => b.toString(16).padStart(2, "0")).join("");
}

function fromHex(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    bytes[i / 2] = parseInt(hex.substring(i, i + 2), 16);
  }
  return bytes;
}

// ── Key generation (real ML-DSA-65 via WASM) ──

async function generateKeypair(): Promise<{ publicKey: Uint8Array; privateKey: Uint8Array }> {
  const { publicKey, secretKey } = await generateMlDsaKeypair();
  return { publicKey, privateKey: secretKey };
}

function deriveAddress(publicKey: Uint8Array): string {
  return mlDsaDeriveAddress(publicKey);
}

// ── Sign transaction (real ML-DSA-65 via WASM) ──

export async function signMessage(privateKey: Uint8Array, message: Uint8Array): Promise<Uint8Array> {
  return signWithMlDsa(privateKey, message);
}

// ── KeyStore class ──

export class BrowserKeyStore {
  private data: KeyStoreData;

  constructor(data?: KeyStoreData) {
    this.data = data ?? { version: KEYSTORE_VERSION, entries: [], activeAccount: null };
  }

  static async load(): Promise<BrowserKeyStore> {
    return new Promise((resolve) => {
      chrome.storage.local.get(STORAGE_KEY, (result) => {
        if (result[STORAGE_KEY]) {
          resolve(new BrowserKeyStore(JSON.parse(result[STORAGE_KEY])));
        } else {
          resolve(new BrowserKeyStore());
        }
      });
    });
  }

  async save(): Promise<void> {
    return new Promise((resolve) => {
      chrome.storage.local.set(
        { [STORAGE_KEY]: JSON.stringify(this.data) },
        () => resolve()
      );
    });
  }

  async generateKey(name: string, password: string): Promise<string> {
    if (this.data.entries.some(e => e.name === name)) {
      throw new Error(`Duplicate account name: ${name}`);
    }

    const { publicKey, privateKey } = await generateKeypair();
    const address = deriveAddress(publicKey);

    const { ciphertext, iv, salt } = await encrypt(privateKey, password);

    const entry: KeyEntry = {
      name,
      address,
      publicKey: toHex(publicKey),
      encryptedPrivateKey: toHex(ciphertext),
      iv: toHex(iv),
      salt: toHex(salt),
      createdAt: new Date().toISOString(),
    };

    this.data.entries.push(entry);
    if (!this.data.activeAccount) {
      this.data.activeAccount = name;
    }

    await this.save();
    return address;
  }

  /**
   * Import a previously recovered ML-DSA-65 keypair (e.g. from a mnemonic
   * backup) and re-encrypt it locally with the same PBKDF2-600k/AES-GCM
   * pipeline used by `generateKey`.
   */
  async importKeypair(
    name: string,
    password: string,
    publicKey: Uint8Array,
    secretKey: Uint8Array,
  ): Promise<string> {
    if (this.data.entries.some(e => e.name === name)) {
      throw new Error(`Duplicate account name: ${name}`);
    }

    const address = deriveAddress(publicKey);
    const { ciphertext, iv, salt } = await encrypt(secretKey, password);

    const entry: KeyEntry = {
      name,
      address,
      publicKey: toHex(publicKey),
      encryptedPrivateKey: toHex(ciphertext),
      iv: toHex(iv),
      salt: toHex(salt),
      createdAt: new Date().toISOString(),
    };

    this.data.entries.push(entry);
    if (!this.data.activeAccount) {
      this.data.activeAccount = name;
    }

    await this.save();
    return address;
  }

  async unlockKey(name: string, password: string): Promise<Uint8Array> {
    const entry = this.data.entries.find(e => e.name === name);
    if (!entry) throw new Error(`Account not found: ${name}`);

    return decrypt(
      fromHex(entry.encryptedPrivateKey),
      fromHex(entry.iv),
      fromHex(entry.salt),
      password
    );
  }

  getActiveAccount(): KeyEntry | null {
    if (!this.data.activeAccount) return null;
    return this.data.entries.find(e => e.name === this.data.activeAccount!) ?? null;
  }

  setActiveAccount(name: string): void {
    if (!this.data.entries.some(e => e.name === name)) {
      throw new Error(`Account not found: ${name}`);
    }
    this.data.activeAccount = name;
  }

  listAccounts(): KeyEntry[] {
    return [...this.data.entries];
  }

  removeAccount(name: string): boolean {
    const before = this.data.entries.length;
    this.data.entries = this.data.entries.filter(e => e.name !== name);
    if (this.data.activeAccount === name) {
      this.data.activeAccount = this.data.entries[0]?.name ?? null;
    }
    return this.data.entries.length < before;
  }

  get isEmpty(): boolean {
    return this.data.entries.length === 0;
  }

  get accountCount(): number {
    return this.data.entries.length;
  }
}
