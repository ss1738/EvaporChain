import { useState, useEffect, useCallback } from "react";
import { useWallet } from "@/hooks/useWallet";
import {
  pluginManager,
  PERMISSION_LABELS,
  type PluginManifest,
  type PluginCategory,
  type InstalledPlugin,
} from "@/utils/plugins";

const CATEGORIES: PluginCategory[] = ["DeFi", "NFT", "Analytics", "Social", "Utilities"];

const CATEGORY_ICONS: Record<PluginCategory, string> = {
  DeFi: "$",
  NFT: "~",
  Analytics: "#",
  Social: "@",
  Utilities: "*",
};

type PluginTab = "browse" | "installed";

export function PluginStore() {
  const { setView } = useWallet();
  const [tab, setTab] = useState<PluginTab>("browse");
  const [search, setSearch] = useState("");
  const [activeCategory, setActiveCategory] = useState<PluginCategory | null>(null);
  const [selectedPlugin, setSelectedPlugin] = useState<PluginManifest | null>(null);
  const [installed, setInstalled] = useState<InstalledPlugin[]>([]);
  const [loading, setLoading] = useState(false);

  const refreshInstalled = useCallback(() => {
    setInstalled(pluginManager.getInstalled());
  }, []);

  useEffect(() => {
    pluginManager.loadPlugins().then(refreshInstalled);
  }, [refreshInstalled]);

  const handleInstall = async (id: string) => {
    setLoading(true);
    await pluginManager.installPlugin(id);
    refreshInstalled();
    setLoading(false);
  };

  const handleUninstall = async (id: string) => {
    setLoading(true);
    await pluginManager.uninstallPlugin(id);
    refreshInstalled();
    setLoading(false);
  };

  const getPlugins = (): PluginManifest[] => {
    if (search) return pluginManager.search(search);
    if (activeCategory) return pluginManager.getByCategory(activeCategory);
    return pluginManager.getAvailable();
  };

  // ── Detail view ──
  if (selectedPlugin) {
    return (
      <PluginDetail
        plugin={selectedPlugin}
        isInstalled={pluginManager.isInstalled(selectedPlugin.id)}
        onBack={() => setSelectedPlugin(null)}
        onInstall={() => handleInstall(selectedPlugin.id)}
        onUninstall={() => handleUninstall(selectedPlugin.id)}
        loading={loading}
      />
    );
  }

  // ── Main store view ──
  const plugins = getPlugins();

  return (
    <div className="flex flex-col h-full bg-white">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-zinc-100">
        <button
          onClick={() => setView("home")}
          className="w-8 h-8 flex items-center justify-center rounded-lg hover:bg-zinc-50 text-zinc-600 transition"
        >
          <span className="text-lg">&larr;</span>
        </button>
        <h1 className="text-base font-semibold text-zinc-800">Plugins</h1>
        <div className="ml-auto">
          <span className="text-xs text-zinc-400">{installed.length} installed</span>
        </div>
      </div>

      {/* Search */}
      <div className="px-4 pt-3 pb-2">
        <input
          type="text"
          placeholder="Search plugins..."
          value={search}
          onChange={e => { setSearch(e.target.value); setActiveCategory(null); }}
          className="w-full px-3 py-2 rounded-lg bg-zinc-50 border border-zinc-200 text-sm text-zinc-800 placeholder:text-zinc-400 focus:outline-none focus:border-cyan-400 focus:ring-1 focus:ring-cyan-400/30 transition"
        />
      </div>

      {/* Tabs */}
      <div className="flex px-4 gap-1 pb-2">
        <TabButton label="Browse" active={tab === "browse"} onClick={() => setTab("browse")} />
        <TabButton
          label={`Installed (${installed.length})`}
          active={tab === "installed"}
          onClick={() => setTab("installed")}
        />
      </div>

      {tab === "browse" ? (
        <>
          {/* Categories */}
          {!search && (
            <div className="flex gap-1.5 px-4 pb-3 overflow-x-auto scrollbar-hide">
              <CategoryPill
                label="All"
                active={activeCategory === null}
                onClick={() => setActiveCategory(null)}
              />
              {CATEGORIES.map(cat => (
                <CategoryPill
                  key={cat}
                  label={`${CATEGORY_ICONS[cat]} ${cat}`}
                  active={activeCategory === cat}
                  onClick={() => setActiveCategory(cat)}
                />
              ))}
            </div>
          )}

          {/* Plugin grid */}
          <div className="flex-1 overflow-y-auto px-4 pb-4">
            {plugins.length === 0 ? (
              <div className="flex items-center justify-center h-32">
                <p className="text-sm text-zinc-400">No plugins found</p>
              </div>
            ) : (
              <div className="grid grid-cols-1 gap-2">
                {plugins.map(plugin => (
                  <PluginCard
                    key={plugin.id}
                    plugin={plugin}
                    isInstalled={pluginManager.isInstalled(plugin.id)}
                    onTap={() => setSelectedPlugin(plugin)}
                    onInstall={() => handleInstall(plugin.id)}
                    onUninstall={() => handleUninstall(plugin.id)}
                    loading={loading}
                  />
                ))}
              </div>
            )}
          </div>
        </>
      ) : (
        /* Installed tab */
        <div className="flex-1 overflow-y-auto px-4 pb-4">
          {installed.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-32 gap-2">
              <p className="text-sm text-zinc-400">No plugins installed</p>
              <button
                onClick={() => setTab("browse")}
                className="text-xs text-cyan-600 hover:text-cyan-700 font-medium"
              >
                Browse plugins
              </button>
            </div>
          ) : (
            <div className="grid grid-cols-1 gap-2">
              {installed.map(ip => (
                <PluginCard
                  key={ip.manifest.id}
                  plugin={ip.manifest}
                  isInstalled={true}
                  onTap={() => setSelectedPlugin(ip.manifest)}
                  onInstall={() => {}}
                  onUninstall={() => handleUninstall(ip.manifest.id)}
                  loading={loading}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ── Sub-components ──

function TabButton({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className={`px-3 py-1.5 rounded-lg text-xs font-medium transition ${
        active
          ? "bg-cyan-50 text-cyan-700 border border-cyan-200"
          : "text-zinc-500 hover:text-zinc-700 border border-transparent"
      }`}
    >
      {label}
    </button>
  );
}

function CategoryPill({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className={`px-3 py-1 rounded-full text-xs font-medium whitespace-nowrap transition ${
        active
          ? "bg-cyan-600 text-white"
          : "bg-zinc-100 text-zinc-600 hover:bg-zinc-200"
      }`}
    >
      {label}
    </button>
  );
}

function StarRating({ rating }: { rating: number }) {
  const full = Math.floor(rating);
  const half = rating % 1 >= 0.5;
  const stars: string[] = [];
  for (let i = 0; i < 5; i++) {
    if (i < full) stars.push("*");
    else if (i === full && half) stars.push("*");
    else stars.push("-");
  }
  return (
    <span className="text-xs text-amber-500 font-mono tracking-tight">
      {stars.join("")} <span className="text-zinc-400">{rating.toFixed(1)}</span>
    </span>
  );
}

function PluginCard({
  plugin,
  isInstalled,
  onTap,
  onInstall,
  onUninstall,
  loading,
}: {
  plugin: PluginManifest;
  isInstalled: boolean;
  onTap: () => void;
  onInstall: () => void;
  onUninstall: () => void;
  loading: boolean;
}) {
  return (
    <div
      onClick={onTap}
      className="flex items-start gap-3 p-3 rounded-xl bg-white border border-zinc-200 hover:border-cyan-300 hover:shadow-sm cursor-pointer transition"
    >
      {/* Icon */}
      <div className="w-10 h-10 rounded-lg bg-zinc-50 border border-zinc-100 flex items-center justify-center text-xl flex-shrink-0">
        {plugin.icon}
      </div>

      {/* Info */}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <h3 className="text-sm font-semibold text-zinc-800 truncate">{plugin.name}</h3>
          <span className="text-xs text-zinc-400">v{plugin.version}</span>
        </div>
        <p className="text-xs text-zinc-500 mt-0.5">by {plugin.author}</p>
        <p className="text-xs text-zinc-500 mt-1 line-clamp-2 leading-relaxed">
          {plugin.description.slice(0, 80)}...
        </p>
        <div className="flex items-center gap-3 mt-1.5">
          <StarRating rating={plugin.rating} />
          <span className="text-xs text-zinc-400">
            {plugin.installCount.toLocaleString()} installs
          </span>
        </div>
      </div>

      {/* Install/Uninstall */}
      <button
        onClick={e => { e.stopPropagation(); isInstalled ? onUninstall() : onInstall(); }}
        disabled={loading}
        className={`px-3 py-1.5 rounded-lg text-xs font-medium flex-shrink-0 transition ${
          isInstalled
            ? "bg-zinc-100 text-zinc-600 hover:bg-red-50 hover:text-red-600 border border-zinc-200"
            : "bg-cyan-600 text-white hover:bg-cyan-700"
        }`}
      >
        {isInstalled ? "Remove" : "Install"}
      </button>
    </div>
  );
}

// ── Plugin Detail ──

function PluginDetail({
  plugin,
  isInstalled,
  onBack,
  onInstall,
  onUninstall,
  loading,
}: {
  plugin: PluginManifest;
  isInstalled: boolean;
  onBack: () => void;
  onInstall: () => void;
  onUninstall: () => void;
  loading: boolean;
}) {
  return (
    <div className="flex flex-col h-full bg-white">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-zinc-100">
        <button
          onClick={onBack}
          className="w-8 h-8 flex items-center justify-center rounded-lg hover:bg-zinc-50 text-zinc-600 transition"
        >
          <span className="text-lg">&larr;</span>
        </button>
        <h1 className="text-base font-semibold text-zinc-800 truncate">{plugin.name}</h1>
      </div>

      <div className="flex-1 overflow-y-auto">
        {/* Hero */}
        <div className="flex items-center gap-4 px-4 pt-4 pb-3">
          <div className="w-14 h-14 rounded-xl bg-zinc-50 border border-zinc-100 flex items-center justify-center text-3xl flex-shrink-0">
            {plugin.icon}
          </div>
          <div className="flex-1">
            <h2 className="text-lg font-bold text-zinc-800">{plugin.name}</h2>
            <p className="text-xs text-zinc-500">by {plugin.author}</p>
            <div className="flex items-center gap-3 mt-1">
              <StarRating rating={plugin.rating} />
              <span className="text-xs text-zinc-400">
                v{plugin.version}
              </span>
              <span className="text-xs text-zinc-400">
                {plugin.installCount.toLocaleString()} installs
              </span>
            </div>
          </div>
        </div>

        {/* Install button */}
        <div className="px-4 pb-4">
          <button
            onClick={isInstalled ? onUninstall : onInstall}
            disabled={loading}
            className={`w-full py-2.5 rounded-lg text-sm font-semibold transition ${
              isInstalled
                ? "bg-zinc-100 text-zinc-700 hover:bg-red-50 hover:text-red-600 border border-zinc-200"
                : "bg-cyan-600 text-white hover:bg-cyan-700"
            }`}
          >
            {loading ? "..." : isInstalled ? "Uninstall" : "Install Plugin"}
          </button>
        </div>

        {/* Description */}
        <div className="px-4 pb-4">
          <h3 className="text-xs font-semibold text-zinc-700 uppercase tracking-wide mb-1.5">Description</h3>
          <p className="text-sm text-zinc-600 leading-relaxed">{plugin.description}</p>
        </div>

        {/* Screenshots placeholder */}
        <div className="px-4 pb-4">
          <h3 className="text-xs font-semibold text-zinc-700 uppercase tracking-wide mb-1.5">Screenshots</h3>
          <div className="flex gap-2 overflow-x-auto scrollbar-hide">
            {[1, 2, 3].map(i => (
              <div
                key={i}
                className="w-32 h-20 rounded-lg bg-zinc-50 border border-zinc-200 flex items-center justify-center flex-shrink-0"
              >
                <span className="text-xs text-zinc-300">Screenshot {i}</span>
              </div>
            ))}
          </div>
        </div>

        {/* Permissions */}
        <div className="px-4 pb-4">
          <h3 className="text-xs font-semibold text-zinc-700 uppercase tracking-wide mb-1.5">Permissions Requested</h3>
          <div className="flex flex-col gap-1">
            {plugin.permissions.map(perm => (
              <div key={perm} className="flex items-center gap-2 px-3 py-2 rounded-lg bg-zinc-50 border border-zinc-100">
                <span className="text-xs text-zinc-400">
                  {perm.includes("sign") ? "!" : perm.includes("read") ? "R" : "~"}
                </span>
                <span className="text-xs text-zinc-600">{PERMISSION_LABELS[perm]}</span>
              </div>
            ))}
          </div>
        </div>

        {/* Reviews */}
        <div className="px-4 pb-6">
          <h3 className="text-xs font-semibold text-zinc-700 uppercase tracking-wide mb-1.5">
            Reviews ({plugin.reviews.length})
          </h3>
          <div className="flex flex-col gap-2">
            {plugin.reviews.map((review, idx) => (
              <div key={idx} className="p-3 rounded-lg bg-zinc-50 border border-zinc-100">
                <div className="flex items-center justify-between mb-1">
                  <span className="text-xs font-medium text-zinc-700">{review.author}</span>
                  <span className="text-xs text-zinc-400">{review.date}</span>
                </div>
                <StarRating rating={review.rating} />
                <p className="text-xs text-zinc-600 mt-1 leading-relaxed">{review.comment}</p>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
