/**
 * Plugin runtime for EvaporChain wallet extension.
 * Manages plugin lifecycle, sandboxed execution, and registry.
 */

// ── Types ──

export interface PluginManifest {
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  icon: string;
  category: PluginCategory;
  permissions: PluginPermission[];
  entrypoint: string;
  installCount: number;
  rating: number;
  reviews: PluginReview[];
}

export type PluginCategory = "DeFi" | "NFT" | "Analytics" | "Social" | "Utilities";

export type PluginPermission =
  | "read_balance"
  | "read_address"
  | "read_objects"
  | "read_nfts"
  | "sign_transactions"
  | "chain_status"
  | "notifications"
  | "plugin_storage";

export interface PluginReview {
  author: string;
  rating: number;
  comment: string;
  date: string;
}

export interface InstalledPlugin {
  manifest: PluginManifest;
  installedAt: number;
  enabled: boolean;
}

export interface PluginContext {
  wallet: {
    getBalance: () => Promise<number>;
    getAddress: () => string;
    getObjects: () => Promise<any[]>;
    getNfts: () => Promise<any[]>;
    requestTransaction: (tx: { to: string; amount: number }) => Promise<{ approved: boolean; hash?: string }>;
    getChainStatus: () => Promise<any>;
  };
  ui: {
    showNotification: (message: string) => void;
  };
  storage: {
    get: (key: string) => string | null;
    set: (key: string, value: string) => void;
  };
}

// ── Built-in plugins registry ──

export const PLUGIN_REGISTRY: PluginManifest[] = [
  {
    id: "energy-optimizer",
    name: "Energy Optimizer",
    version: "1.2.0",
    author: "EvaporChain Labs",
    description: "Automatically analyzes your objects' energy levels and suggests the most cost-effective refresh strategies. Monitors decay curves and alerts you before objects enter the Grace period.",
    icon: "⚡",
    category: "Utilities",
    permissions: ["read_balance", "read_objects", "notifications", "plugin_storage"],
    entrypoint: "energy-optimizer.js",
    installCount: 12_480,
    rating: 4.7,
    reviews: [
      { author: "0x7a..3f", rating: 5, comment: "Saved me tons of EVAP on refresh costs!", date: "2026-03-15" },
      { author: "0xb2..1c", rating: 4, comment: "Great tool, wish it had batch mode.", date: "2026-03-20" },
      { author: "0x91..dd", rating: 5, comment: "Essential for managing many objects.", date: "2026-04-01" },
    ],
  },
  {
    id: "whale-tracker",
    name: "Whale Tracker",
    version: "2.0.1",
    author: "ChainWatch",
    description: "Monitors the EvaporChain network for large EVAP transfers and significant object creation events. Get real-time alerts when whale wallets move funds or create high-energy objects.",
    icon: "🐋",
    category: "Analytics",
    permissions: ["chain_status", "notifications", "plugin_storage"],
    entrypoint: "whale-tracker.js",
    installCount: 8_340,
    rating: 4.3,
    reviews: [
      { author: "0xf4..ab", rating: 4, comment: "Useful for staying ahead of market moves.", date: "2026-03-18" },
      { author: "0xc1..09", rating: 5, comment: "Caught a 500k EVAP transfer early!", date: "2026-03-25" },
    ],
  },
  {
    id: "collection-tracker",
    name: "Collection Tracker",
    version: "1.0.3",
    author: "NFT Pulse",
    description: "Track floor prices, volume, and energy health across your NFT collections. Get alerts when collection floors move significantly or when your NFTs are approaching evaporation.",
    icon: "🖼",
    category: "NFT",
    permissions: ["read_nfts", "read_objects", "notifications", "plugin_storage"],
    entrypoint: "collection-tracker.js",
    installCount: 5_210,
    rating: 4.5,
    reviews: [
      { author: "0xd3..77", rating: 5, comment: "Best NFT tracker for EvaporChain.", date: "2026-03-22" },
      { author: "0xe5..4a", rating: 4, comment: "Needs more collection support.", date: "2026-04-03" },
    ],
  },
  {
    id: "gas-oracle",
    name: "Gas Oracle",
    version: "1.1.0",
    author: "EvaporChain Labs",
    description: "Real-time fee estimation and optimization for EvaporChain transactions. Suggests optimal timing for transactions based on network congestion and energy costs.",
    icon: "⛽",
    category: "DeFi",
    permissions: ["chain_status", "read_balance", "notifications"],
    entrypoint: "gas-oracle.js",
    installCount: 15_720,
    rating: 4.8,
    reviews: [
      { author: "0xa8..f2", rating: 5, comment: "Pays for itself with the savings.", date: "2026-03-10" },
      { author: "0x33..b8", rating: 5, comment: "Simple and effective.", date: "2026-03-28" },
      { author: "0x6e..c4", rating: 4, comment: "Would love historical gas charts.", date: "2026-04-05" },
    ],
  },
  {
    id: "address-book",
    name: "Address Book",
    version: "1.3.2",
    author: "WalletUtils",
    description: "Save and manage named contacts for frequent transfers. Quickly select recipients from your address book when sending EVAP or NFTs. Supports tags and notes.",
    icon: "📇",
    category: "Social",
    permissions: ["read_address", "plugin_storage", "notifications"],
    entrypoint: "address-book.js",
    installCount: 9_870,
    rating: 4.6,
    reviews: [
      { author: "0x54..ea", rating: 5, comment: "No more copy-pasting addresses!", date: "2026-03-12" },
      { author: "0x8b..37", rating: 4, comment: "Works well, would like import/export.", date: "2026-04-02" },
    ],
  },
];

// ── Permission labels ──

export const PERMISSION_LABELS: Record<PluginPermission, string> = {
  read_balance: "Read wallet balance",
  read_address: "Read wallet address",
  read_objects: "View state objects",
  read_nfts: "View NFT collection",
  sign_transactions: "Request transaction signing",
  chain_status: "Read chain status",
  notifications: "Show notifications",
  plugin_storage: "Store plugin data",
};

// ── Storage keys ──

const INSTALLED_PLUGINS_KEY = "evaporchain_installed_plugins";

// ── Plugin Manager ──

export class PluginManager {
  private installed: Map<string, InstalledPlugin> = new Map();

  /** Load installed plugins from localStorage */
  async loadPlugins(): Promise<void> {
    try {
      const raw = localStorage.getItem(INSTALLED_PLUGINS_KEY);
      if (raw) {
        const list: InstalledPlugin[] = JSON.parse(raw);
        this.installed = new Map(list.map(p => [p.manifest.id, p]));
      }
    } catch {
      // Corrupted storage — start fresh
      this.installed = new Map();
    }
  }

  /** Save installed plugins to localStorage */
  private save(): void {
    try {
      const list = Array.from(this.installed.values());
      localStorage.setItem(INSTALLED_PLUGINS_KEY, JSON.stringify(list));
    } catch {
      // Storage might be full
    }
  }

  /** Install a plugin by ID from the registry */
  async installPlugin(id: string): Promise<boolean> {
    const manifest = PLUGIN_REGISTRY.find(p => p.id === id);
    if (!manifest) return false;
    if (this.installed.has(id)) return false;

    this.installed.set(id, {
      manifest,
      installedAt: Date.now(),
      enabled: true,
    });
    this.save();
    return true;
  }

  /** Uninstall a plugin by ID */
  async uninstallPlugin(id: string): Promise<boolean> {
    if (!this.installed.has(id)) return false;

    // Clean up plugin-scoped storage
    try {
      const prefix = `evap_plugin_${id}_`;
      const keys: string[] = [];
      for (let i = 0; i < localStorage.length; i++) {
        const key = localStorage.key(i);
        if (key?.startsWith(prefix)) keys.push(key);
      }
      keys.forEach(k => localStorage.removeItem(k));
    } catch {
      // Ignore storage errors
    }

    this.installed.delete(id);
    this.save();
    return true;
  }

  /** Get all installed plugins */
  getInstalled(): InstalledPlugin[] {
    return Array.from(this.installed.values());
  }

  /** Check if a plugin is installed */
  isInstalled(id: string): boolean {
    return this.installed.has(id);
  }

  /** Get all available plugins from registry */
  getAvailable(): PluginManifest[] {
    return PLUGIN_REGISTRY;
  }

  /** Get available plugins filtered by category */
  getByCategory(category: PluginCategory): PluginManifest[] {
    return PLUGIN_REGISTRY.filter(p => p.category === category);
  }

  /** Search plugins by name or description */
  search(query: string): PluginManifest[] {
    const q = query.toLowerCase();
    return PLUGIN_REGISTRY.filter(
      p =>
        p.name.toLowerCase().includes(q) ||
        p.description.toLowerCase().includes(q) ||
        p.author.toLowerCase().includes(q) ||
        p.category.toLowerCase().includes(q),
    );
  }

  /** Execute a plugin with a sandboxed API context */
  async executePlugin(id: string, context: PluginContext): Promise<string> {
    const plugin = this.installed.get(id);
    if (!plugin) return "Plugin not installed";
    if (!plugin.enabled) return "Plugin is disabled";

    // Build sandboxed API surface based on granted permissions
    const sandboxedContext: Partial<PluginContext> = {};
    const perms = plugin.manifest.permissions;

    if (perms.includes("read_balance") || perms.includes("read_address") ||
        perms.includes("read_objects") || perms.includes("read_nfts") ||
        perms.includes("sign_transactions") || perms.includes("chain_status")) {
      const wallet: Partial<PluginContext["wallet"]> = {};
      if (perms.includes("read_balance")) wallet.getBalance = context.wallet.getBalance;
      if (perms.includes("read_address")) wallet.getAddress = context.wallet.getAddress;
      if (perms.includes("read_objects")) wallet.getObjects = context.wallet.getObjects;
      if (perms.includes("read_nfts")) wallet.getNfts = context.wallet.getNfts;
      if (perms.includes("sign_transactions")) wallet.requestTransaction = context.wallet.requestTransaction;
      if (perms.includes("chain_status")) wallet.getChainStatus = context.wallet.getChainStatus;
      sandboxedContext.wallet = wallet as PluginContext["wallet"];
    }

    if (perms.includes("notifications")) {
      sandboxedContext.ui = context.ui;
    }

    if (perms.includes("plugin_storage")) {
      const prefix = `evap_plugin_${id}_`;
      sandboxedContext.storage = {
        get: (key: string) => {
          try { return localStorage.getItem(prefix + key); } catch { return null; }
        },
        set: (key: string, value: string) => {
          try { localStorage.setItem(prefix + key, value); } catch { /* full */ }
        },
      };
    }

    // In a real implementation, this would load and execute the plugin entrypoint
    // in an isolated iframe or web worker. For now, return a simulated response.
    return `Plugin "${plugin.manifest.name}" executed successfully`;
  }
}

/** Singleton instance */
export const pluginManager = new PluginManager();
