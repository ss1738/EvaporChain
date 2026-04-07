/**
 * Browser-side encrypted keystore for EvaporChain wallet.
 *
 * Uses Web Crypto API:
 * - PBKDF2 for key derivation (password → AES key)
 * - AES-256-GCM for encrypting private keys at rest
 *
 * Note: ML-DSA signing is stubbed for now — will be replaced with WASM
 * bridge to evaporchain-crypto once compiled to wasm32-unknown-unknown.
 * For testnet, we use Ed25519-style key simulation via Web Crypto.
 */

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

// ── Key generation ──
// Temporary: uses Web Crypto Ed25519-style keys for testnet.
// Production: will use ML-DSA via WASM.

async function generateKeypair(): Promise<{ publicKey: Uint8Array; privateKey: Uint8Array }> {
  // Generate 32-byte random keypair (simulating key material)
  const privateKey = crypto.getRandomValues(new Uint8Array(32));
  // Derive public key via SHA-256 hash of private key (deterministic)
  const pubKeyHash = await crypto.subtle.digest("SHA-256", privateKey);
  const publicKey = new Uint8Array(pubKeyHash);
  return { publicKey, privateKey };
}

function deriveAddress(publicKey: Uint8Array): string {
  // BLAKE3 on Rust side — here we use first 32 bytes of SHA-256(pubkey) as address
  // This matches the testnet format: 0x + 64 hex chars
  return "0x" + toHex(publicKey);
}

// ── Sign transaction ──

export async function signMessage(privateKey: Uint8Array, message: Uint8Array): Promise<Uint8Array> {
  // Stub: HMAC-SHA256 signature for testnet compatibility
  // Production: ML-DSA signing via WASM
  const key = await crypto.subtle.importKey(
    "raw",
    privateKey as BufferSource,
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"]
  );
  return new Uint8Array(await crypto.subtle.sign("HMAC", key, message as BufferSource));
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
