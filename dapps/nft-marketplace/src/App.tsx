import { useState, useEffect, useCallback } from "react";
import { api } from "@/utils/api";
import type { Nft, ChainStatus } from "@/utils/types";
import { useWalletConnect } from "@/hooks/useWalletConnect";
import { NftCard } from "@/components/NftCard";
import { MintModal } from "@/components/MintModal";
import { RefreshModal } from "@/components/RefreshModal";
import { TransferModal } from "@/components/TransferModal";
import { SponsorModal } from "@/components/SponsorModal";
import { RefreshPoolFooter } from "@/components/RefreshPoolFooter";

type Tab = "all" | "mine" | "ghosts";

export function App() {
  const wallet = useWalletConnect();
  const [nfts, setNfts] = useState<Nft[]>([]);
  const [status, setStatus] = useState<ChainStatus | null>(null);
  const [tab, setTab] = useState<Tab>("all");
  const [showMint, setShowMint] = useState(false);
  const [refreshTarget, setRefreshTarget] = useState<Nft | null>(null);
  const [transferTarget, setTransferTarget] = useState<Nft | null>(null);
  const [sponsorTarget, setSponsorTarget] = useState<Nft | null>(null);
  const [patronageNsHex, setPatronageNsHex] = useState<string>("");
  const [loading, setLoading] = useState(true);

  const fetchData = useCallback(async () => {
    try {
      const [nftData, statusData] = await Promise.all([
        api.getNfts(),
        api.getStatus(),
      ]);
      setNfts(nftData);
      setStatus(statusData);
    } catch {
      // Retry on next poll
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
  }, [fetchData]);

  // Fetch the chain's patronage namespace once — required to open a covenant.
  useEffect(() => {
    let cancelled = false;
    api
      .getPatronageStatus()
      .then((res) => {
        if (!cancelled && res.patronage_ns_hex) setPatronageNsHex(res.patronage_ns_hex);
      })
      .catch(() => {
        /* covenants disabled or namespace not yet provisioned */
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const filteredNfts = nfts.filter(nft => {
    if (tab === "mine") return wallet.address && nft.owner.toLowerCase() === wallet.address.toLowerCase();
    if (tab === "ghosts") return nft.state === "Ghost";
    return true;
  });

  const activeCount = nfts.filter(n => n.state === "Active").length;
  const graceCount = nfts.filter(n => n.state === "Grace").length;
  const ghostCount = nfts.filter(n => n.state === "Ghost").length;
  const myCount = wallet.address
    ? nfts.filter(n => n.owner.toLowerCase() === wallet.address!.toLowerCase()).length
    : 0;

  return (
    <div className="min-h-screen bg-evap-bg">
      {/* Navbar */}
      <nav className="bg-white border-b border-evap-border sticky top-0 z-40">
        <div className="max-w-6xl mx-auto px-4 py-3 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-evap-cyan to-evap-purple flex items-center justify-center">
              <span className="text-sm font-bold text-white">E</span>
            </div>
            <div>
              <h1 className="text-sm font-bold text-zinc-900">Mortal NFTs</h1>
              <p className="text-[10px] text-zinc-400">EvaporChain Marketplace</p>
            </div>
          </div>

          <div className="flex items-center gap-3">
            {status && (
              <div className="hidden sm:flex items-center gap-3 text-[10px] text-zinc-400">
                <span className="flex items-center gap-1">
                  <span className="w-1.5 h-1.5 rounded-full bg-evap-green animate-pulse" />
                  Block {status.block_height.toLocaleString()}
                </span>
                <span>Epoch {status.epoch.toLocaleString()}</span>
              </div>
            )}

            {wallet.connected ? (
              <div className="flex items-center gap-2">
                <span className="text-[10px] text-zinc-500 font-mono hidden sm:inline">
                  {wallet.address?.slice(0, 6)}...{wallet.address?.slice(-4)}
                </span>
                <button
                  onClick={wallet.disconnect}
                  className="text-[10px] text-zinc-400 hover:text-zinc-600 px-2 py-1 rounded border border-evap-border"
                >
                  Disconnect
                </button>
              </div>
            ) : (
              <button
                onClick={wallet.connect}
                disabled={wallet.connecting}
                className="px-4 py-2 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-xs font-semibold text-white hover:opacity-90 transition disabled:opacity-50"
              >
                {wallet.connecting ? "Connecting..." : "Connect Wallet"}
              </button>
            )}
          </div>
        </div>
      </nav>

      {/* Hero stats */}
      <div className="max-w-6xl mx-auto px-4 py-6">
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-6">
          <StatCard label="Active NFTs" value={activeCount} color="text-evap-green" />
          <StatCard label="Decaying" value={graceCount} color="text-evap-amber" />
          <StatCard label="Evaporated" value={ghostCount} color="text-evap-ghost" />
          <StatCard label="My NFTs" value={myCount} color="text-evap-purple" />
        </div>

        {/* Tabs + Mint button */}
        <div className="flex items-center justify-between mb-4">
          <div className="flex bg-white rounded-lg border border-evap-border p-0.5">
            <TabBtn label="All" active={tab === "all"} onClick={() => setTab("all")} count={nfts.length} />
            <TabBtn label="Mine" active={tab === "mine"} onClick={() => setTab("mine")} count={myCount} />
            <TabBtn label="Ghost Gallery" active={tab === "ghosts"} onClick={() => setTab("ghosts")} count={ghostCount} />
          </div>

          <button
            onClick={() => setShowMint(true)}
            className="px-4 py-2 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-xs font-semibold text-white hover:opacity-90 transition"
          >
            + Mint NFT
          </button>
        </div>

        {/* NFT Grid */}
        {loading ? (
          <div className="flex items-center justify-center py-20">
            <div className="w-8 h-8 border-2 border-evap-cyan/30 border-t-evap-cyan rounded-full animate-spin" />
          </div>
        ) : filteredNfts.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-20 bg-white rounded-xl border border-evap-border">
            <span className="text-4xl mb-3">{tab === "ghosts" ? "👻" : "◈"}</span>
            <p className="text-sm text-zinc-500">
              {tab === "mine" ? "You don't own any NFTs yet" :
               tab === "ghosts" ? "No evaporated NFTs" :
               "No NFTs found"}
            </p>
            <p className="text-xs text-zinc-400 mt-1">
              {tab === "mine" ? "Mint your first mortal NFT above" : "Be the first to mint!"}
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
            {filteredNfts.map(nft => (
              <NftCard
                key={nft.id}
                nft={nft}
                isOwner={wallet.address?.toLowerCase() === nft.owner.toLowerCase()}
                onRefresh={setRefreshTarget}
                onTransfer={setTransferTarget}
                onSponsor={patronageNsHex ? setSponsorTarget : undefined}
                currentEpoch={status?.epoch}
              />
            ))}
          </div>
        )}

        {/* Wallet not connected hint */}
        {!wallet.connected && (
          <div className="mt-6 px-4 py-3 rounded-lg bg-evap-cyan/5 border border-evap-cyan/20 text-center">
            <p className="text-xs text-evap-cyan">
              Connect your EvaporChain Wallet to mint, refresh, and transfer NFTs
            </p>
            {wallet.error && (
              <p className="text-[10px] text-evap-red mt-1">{wallet.error}</p>
            )}
          </div>
        )}

        {/* Refresh-pool stake widget */}
        <RefreshPoolFooter />
      </div>

      {/* Footer */}
      <footer className="border-t border-evap-border mt-12 py-6">
        <div className="max-w-6xl mx-auto px-4 flex items-center justify-between">
          <p className="text-[10px] text-zinc-400">
            Mortal NFTs — Powered by EvaporChain
          </p>
          <p className="text-[10px] text-zinc-400">
            NFTs decay in real-time. Refresh to keep alive.
          </p>
        </div>
      </footer>

      {/* Modals */}
      {showMint && (
        <MintModal
          ownerAddress={wallet.address}
          onClose={() => setShowMint(false)}
          onMinted={fetchData}
        />
      )}
      {refreshTarget && (
        <RefreshModal
          nft={refreshTarget}
          onClose={() => setRefreshTarget(null)}
          onRefreshed={fetchData}
        />
      )}
      {transferTarget && (
        <TransferModal
          nft={transferTarget}
          onClose={() => setTransferTarget(null)}
          onTransferred={fetchData}
        />
      )}
      {sponsorTarget && status && patronageNsHex && (
        <SponsorModal
          nft={sponsorTarget}
          currentEpoch={status.epoch}
          patronageNsHex={patronageNsHex}
          onClose={() => setSponsorTarget(null)}
          onSponsored={() => {
            setSponsorTarget(null);
            fetchData();
          }}
        />
      )}
    </div>
  );
}

function StatCard({ label, value, color }: { label: string; value: number; color: string }) {
  return (
    <div className="bg-white rounded-xl border border-evap-border px-4 py-3">
      <p className={`text-xl font-bold ${color}`}>{value}</p>
      <p className="text-[10px] text-zinc-400">{label}</p>
    </div>
  );
}

function TabBtn({ label, active, onClick, count }: {
  label: string; active: boolean; onClick: () => void; count: number;
}) {
  return (
    <button
      onClick={onClick}
      className={`px-3 py-1.5 rounded-md text-xs font-medium transition ${
        active ? "bg-zinc-100 text-zinc-900" : "text-zinc-500 hover:text-zinc-700"
      }`}
    >
      {label} <span className="text-zinc-400">({count})</span>
    </button>
  );
}
