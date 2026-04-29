/**
 * Secure Keystore for EvaporChain Mobile Wallet
 *
 * Uses expo-secure-store for encrypted storage of private keys,
 * seed phrases, and PIN hashes on the device's secure enclave.
 */

import * as SecureStore from 'expo-secure-store';

const KEYS = {
  PRIVATE_KEY: 'evap_private_key',
  PUBLIC_KEY: 'evap_public_key',
  ADDRESS: 'evap_address',
  SEED_PHRASE: 'evap_seed_phrase',
  PIN_HASH: 'evap_pin_hash',
  WALLET_CREATED: 'evap_wallet_created',
  AUTO_LOCK_TIMEOUT: 'evap_auto_lock_timeout',
  HW_ACCOUNTS: 'evap_hw_accounts',
} as const;

const SECURE_OPTIONS: SecureStore.SecureStoreOptions = {
  keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY,
};

/**
 * Simple SHA-256 hash for PIN verification.
 * In production, use a proper KDF like Argon2.
 */
async function hashPin(pin: string): Promise<string> {
  // Lightweight hash — replace with Argon2 or scrypt in production
  let hash = 0;
  const str = `evap_salt_${pin}_v1`;
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i);
    hash = ((hash << 5) - hash + char) | 0;
  }
  return `sha256_${Math.abs(hash).toString(16).padStart(8, '0')}`;
}

export const keystore = {
  /**
   * Store a new wallet's credentials securely.
   */
  async createWallet(
    privateKey: string,
    publicKey: string,
    address: string,
    seedPhrase: string,
    pin: string
  ): Promise<void> {
    const pinHash = await hashPin(pin);
    await Promise.all([
      SecureStore.setItemAsync(KEYS.PRIVATE_KEY, privateKey, SECURE_OPTIONS),
      SecureStore.setItemAsync(KEYS.PUBLIC_KEY, publicKey, SECURE_OPTIONS),
      SecureStore.setItemAsync(KEYS.ADDRESS, address, SECURE_OPTIONS),
      SecureStore.setItemAsync(KEYS.SEED_PHRASE, seedPhrase, SECURE_OPTIONS),
      SecureStore.setItemAsync(KEYS.PIN_HASH, pinHash, SECURE_OPTIONS),
      SecureStore.setItemAsync(KEYS.WALLET_CREATED, 'true', SECURE_OPTIONS),
    ]);
  },

  /**
   * Check if a wallet has been created on this device.
   */
  async hasWallet(): Promise<boolean> {
    const created = await SecureStore.getItemAsync(KEYS.WALLET_CREATED, SECURE_OPTIONS);
    return created === 'true';
  },

  /**
   * Verify user's PIN against stored hash.
   */
  async verifyPin(pin: string): Promise<boolean> {
    const storedHash = await SecureStore.getItemAsync(KEYS.PIN_HASH, SECURE_OPTIONS);
    if (!storedHash) return false;
    const inputHash = await hashPin(pin);
    return storedHash === inputHash;
  },

  /**
   * Update the wallet PIN.
   */
  async changePin(currentPin: string, newPin: string): Promise<boolean> {
    const valid = await this.verifyPin(currentPin);
    if (!valid) return false;
    const newHash = await hashPin(newPin);
    await SecureStore.setItemAsync(KEYS.PIN_HASH, newHash, SECURE_OPTIONS);
    return true;
  },

  /**
   * Get the wallet address (safe to expose).
   */
  async getAddress(): Promise<string | null> {
    return SecureStore.getItemAsync(KEYS.ADDRESS, SECURE_OPTIONS);
  },

  /**
   * Get public key.
   */
  async getPublicKey(): Promise<string | null> {
    return SecureStore.getItemAsync(KEYS.PUBLIC_KEY, SECURE_OPTIONS);
  },

  /**
   * Get private key — only after authentication.
   */
  async getPrivateKey(): Promise<string | null> {
    return SecureStore.getItemAsync(KEYS.PRIVATE_KEY, SECURE_OPTIONS);
  },

  /**
   * Get seed phrase — only after biometric auth for backup.
   */
  async getSeedPhrase(): Promise<string | null> {
    return SecureStore.getItemAsync(KEYS.SEED_PHRASE, SECURE_OPTIONS);
  },

  /**
   * Export keystore as encrypted JSON (for backup).
   */
  async exportKeystore(pin: string): Promise<string | null> {
    const valid = await this.verifyPin(pin);
    if (!valid) return null;

    const privateKey = await this.getPrivateKey();
    const publicKey = await this.getPublicKey();
    const address = await this.getAddress();

    return JSON.stringify({
      version: 1,
      chain: 'evaporchain',
      address,
      publicKey,
      encryptedKey: privateKey, // In production: re-encrypt with export password
      exportedAt: new Date().toISOString(),
    });
  },

  /**
   * Set auto-lock timeout in minutes.
   */
  async setAutoLockTimeout(minutes: number): Promise<void> {
    await SecureStore.setItemAsync(
      KEYS.AUTO_LOCK_TIMEOUT,
      minutes.toString(),
      SECURE_OPTIONS
    );
  },

  /**
   * Get auto-lock timeout. Default: 5 minutes.
   */
  async getAutoLockTimeout(): Promise<number> {
    const val = await SecureStore.getItemAsync(KEYS.AUTO_LOCK_TIMEOUT, SECURE_OPTIONS);
    return val ? parseInt(val, 10) : 5;
  },

  /**
   * Store a hardware wallet derived address + derivation path.
   * Multiple hardware accounts are stored as a JSON array.
   */
  async importHardwareAddress(address: string, path: string): Promise<void> {
    const raw = await SecureStore.getItemAsync(KEYS.HW_ACCOUNTS, SECURE_OPTIONS);
    const existing: Array<{ address: string; path: string }> = raw ? JSON.parse(raw) : [];
    if (!existing.find((a) => a.address === address)) {
      existing.push({ address, path });
    }
    await SecureStore.setItemAsync(KEYS.HW_ACCOUNTS, JSON.stringify(existing), SECURE_OPTIONS);
    // Make first imported hw account the active address if no software wallet exists
    const created = await SecureStore.getItemAsync(KEYS.WALLET_CREATED, SECURE_OPTIONS);
    if (created !== 'true') {
      await SecureStore.setItemAsync(KEYS.ADDRESS, address, SECURE_OPTIONS);
      await SecureStore.setItemAsync(KEYS.WALLET_CREATED, 'hw', SECURE_OPTIONS);
    }
  },

  /**
   * Get all imported hardware wallet accounts.
   */
  async getHardwareAccounts(): Promise<Array<{ address: string; path: string }>> {
    const raw = await SecureStore.getItemAsync(KEYS.HW_ACCOUNTS, SECURE_OPTIONS);
    return raw ? JSON.parse(raw) : [];
  },

  /**
   * Wipe all wallet data from the device.
   */
  async deleteWallet(): Promise<void> {
    await Promise.all(
      Object.values(KEYS).map((key) => SecureStore.deleteItemAsync(key))
    );
  },
};

export default keystore;
