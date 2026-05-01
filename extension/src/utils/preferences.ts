/**
 * User preferences backed by chrome.storage.local.
 * Falls back to localStorage in non-extension environments (dev / tests).
 */

export type NetworkKind = "mainnet" | "testnet" | "custom";

export interface UserPreferences {
  /** Resolved node URL — derived from {network, customNodeUrl}. Kept on
   *  the object so legacy consumers that read `prefs.nodeUrl` still work. */
  nodeUrl: string;
  /** Selected network. Defaults to "testnet" until mainnet ships. */
  network: NetworkKind;
  /** URL used when `network === "custom"`. Preserved across switches so
   *  the user's custom RPC isn't lost when toggling between networks. */
  customNodeUrl: string;
  currency: "USD" | "GBP" | "EUR";
  autoLockMinutes: number;
  defaultSlippage: number;
  hideSmallBalances: boolean;
  notificationsEnabled: boolean;
  /** Lock the wallet immediately when the popup loses focus. */
  lockOnBlur: boolean;
  /** Lock the wallet when the popup is closed (service-worker disconnect). */
  lockOnTabClose: boolean;
}

const STORAGE_KEY = "evaporchain_prefs";

/** Bumped whenever the preference shape changes in a non-additive way.
 *  v2 introduced the network/customNodeUrl/lockOnBlur/lockOnTabClose
 *  fields, migrating the legacy `nodeUrl` string in. */
export const PREFERENCES_VERSION = 2;

export const MAINNET_URL = "https://mainnet.evaporchain.com";
export const TESTNET_URL = "https://testnet.evaporchain.com";

const DEFAULTS: UserPreferences = {
  nodeUrl: TESTNET_URL,
  network: "testnet",
  customNodeUrl: TESTNET_URL,
  currency: "USD",
  autoLockMinutes: 15,
  defaultSlippage: 0.5,
  hideSmallBalances: false,
  notificationsEnabled: true,
  lockOnBlur: false,
  lockOnTabClose: true,
};

interface StoredPreferences extends Partial<UserPreferences> {
  version?: number;
}

function isChromeStorageAvailable(): boolean {
  return (
    typeof chrome !== "undefined" &&
    typeof chrome.storage !== "undefined" &&
    typeof chrome.storage.local !== "undefined"
  );
}

/**
 * Resolve the active node URL for a given preference set. Mainnet and
 * testnet have hard-coded canonical URLs; "custom" falls back to the
 * `customNodeUrl` slot (and `nodeUrl` if the slot is somehow empty).
 */
export function resolveNodeUrl(prefs: UserPreferences): string {
  switch (prefs.network) {
    case "mainnet":
      return MAINNET_URL;
    case "testnet":
      return TESTNET_URL;
    case "custom":
    default:
      return prefs.customNodeUrl || prefs.nodeUrl || TESTNET_URL;
  }
}

/**
 * Migrate a legacy preferences blob (version absent or < PREFERENCES_VERSION)
 * to the current shape. Idempotent — safe to run on already-migrated data.
 */
function migrate(stored: StoredPreferences | null): UserPreferences {
  if (!stored) return { ...DEFAULTS };

  // v1 had a single `nodeUrl: string` and no network/customNodeUrl.
  // We map the existing URL into the new structure: if it matches the
  // canonical mainnet/testnet URL we pick that network, otherwise the
  // URL becomes the customNodeUrl and network is set to "custom".
  let network: NetworkKind = stored.network ?? DEFAULTS.network;
  let customNodeUrl = stored.customNodeUrl ?? "";
  const legacyUrl = stored.nodeUrl;

  if (stored.network === undefined && legacyUrl) {
    if (legacyUrl === MAINNET_URL) {
      network = "mainnet";
    } else if (legacyUrl === TESTNET_URL) {
      network = "testnet";
    } else {
      network = "custom";
      customNodeUrl = legacyUrl;
    }
  }
  if (!customNodeUrl) {
    customNodeUrl = legacyUrl ?? DEFAULTS.customNodeUrl;
  }

  const merged: UserPreferences = {
    ...DEFAULTS,
    ...stored,
    network,
    customNodeUrl,
  };
  merged.nodeUrl = resolveNodeUrl(merged);
  return merged;
}

export async function loadPreferences(): Promise<UserPreferences> {
  try {
    if (isChromeStorageAvailable()) {
      return new Promise((resolve) => {
        chrome.storage.local.get(STORAGE_KEY, (result) => {
          const stored = result[STORAGE_KEY] as StoredPreferences | undefined;
          resolve(migrate(stored ?? null));
        });
      });
    }
    const raw = localStorage.getItem(STORAGE_KEY);
    return migrate(raw ? (JSON.parse(raw) as StoredPreferences) : null);
  } catch {
    return { ...DEFAULTS };
  }
}

export async function savePreferences(prefs: Partial<UserPreferences>): Promise<void> {
  const current = await loadPreferences();
  const merged: UserPreferences = { ...current, ...prefs };
  // Always recompute nodeUrl from the merged network selection so the
  // persisted value can't drift away from the canonical URL.
  merged.nodeUrl = resolveNodeUrl(merged);
  const persisted: StoredPreferences = { ...merged, version: PREFERENCES_VERSION };
  try {
    if (isChromeStorageAvailable()) {
      return new Promise((resolve) => {
        chrome.storage.local.set({ [STORAGE_KEY]: persisted }, resolve);
      });
    }
    localStorage.setItem(STORAGE_KEY, JSON.stringify(persisted));
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
