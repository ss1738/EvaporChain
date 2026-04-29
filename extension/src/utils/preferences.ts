/**
 * User preferences backed by chrome.storage.local.
 * Falls back to localStorage in non-extension environments (dev / tests).
 */

export interface UserPreferences {
  nodeUrl: string;
  currency: "USD" | "GBP" | "EUR";
  autoLockMinutes: number;
  defaultSlippage: number;
  hideSmallBalances: boolean;
  notificationsEnabled: boolean;
}

const STORAGE_KEY = "evaporchain_prefs";

const DEFAULTS: UserPreferences = {
  nodeUrl: "https://testnet.evaporchain.com",
  currency: "USD",
  autoLockMinutes: 15,
  defaultSlippage: 0.5,
  hideSmallBalances: false,
  notificationsEnabled: true,
};

function isChromeStorageAvailable(): boolean {
  return (
    typeof chrome !== "undefined" &&
    typeof chrome.storage !== "undefined" &&
    typeof chrome.storage.local !== "undefined"
  );
}

export async function loadPreferences(): Promise<UserPreferences> {
  try {
    if (isChromeStorageAvailable()) {
      return new Promise((resolve) => {
        chrome.storage.local.get(STORAGE_KEY, (result) => {
          const stored = result[STORAGE_KEY];
          resolve(stored ? { ...DEFAULTS, ...stored } : { ...DEFAULTS });
        });
      });
    }
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? { ...DEFAULTS, ...JSON.parse(raw) } : { ...DEFAULTS };
  } catch {
    return { ...DEFAULTS };
  }
}

export async function savePreferences(prefs: Partial<UserPreferences>): Promise<void> {
  const current = await loadPreferences();
  const updated = { ...current, ...prefs };
  try {
    if (isChromeStorageAvailable()) {
      return new Promise((resolve) => {
        chrome.storage.local.set({ [STORAGE_KEY]: updated }, resolve);
      });
    }
    localStorage.setItem(STORAGE_KEY, JSON.stringify(updated));
  } catch {
    // Non-fatal — in-memory state still reflects the change
  }
}

export async function clearPreferences(): Promise<void> {
  try {
    if (isChromeStorageAvailable()) {
      return new Promise((resolve) => {
        chrome.storage.local.remove(STORAGE_KEY, resolve);
      });
    }
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // ignore
  }
}
