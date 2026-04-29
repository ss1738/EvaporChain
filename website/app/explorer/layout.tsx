"use client";

import { useState, type ReactNode } from "react";
import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { Search, ArrowLeft } from "lucide-react";

const TABS = [
  { label: "Overview", href: "/explorer" },
  { label: "Validators", href: "/explorer/validators" },
  { label: "Contracts", href: "/explorer/contracts" },
  { label: "Simulate", href: "/explorer/simulate" },
];

export default function ExplorerLayout({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const router = useRouter();
  const [query, setQuery] = useState("");

  const TAB_PATHS = new Set(TABS.map((t) => t.href));
  const isDetail = !TAB_PATHS.has(pathname);

  const handleSearch = (e: React.FormEvent) => {
    e.preventDefault();
    const q = query.trim();
    if (!q) return;

    if (q.length === 64 || q.startsWith("0x")) {
      // Could be tx hash, address, or object ID — try address first
      if (q.length > 50) {
        router.push(`/explorer/tx/${q}`);
      } else {
        router.push(`/explorer/address/${q}`);
      }
    } else if (/^\d+$/.test(q)) {
      // Block height — show on main page (future: block detail)
      router.push(`/explorer`);
    } else {
      router.push(`/explorer/address/${q}`);
    }
    setQuery("");
  };

  return (
    <div className="min-h-screen bg-bg-primary">
      {/* Explorer Header */}
      <div className="border-b border-white/5 bg-bg-card/50 backdrop-blur-xl sticky top-0 z-40">
        <div className="max-w-7xl mx-auto px-6">
          {/* Top row: Logo + Search */}
          <div className="flex items-center justify-between h-16 gap-4">
            <div className="flex items-center gap-3">
              {isDetail && (
                <button
                  onClick={() => router.back()}
                  className="text-text-muted hover:text-accent-cyan transition-colors mr-1"
                >
                  <ArrowLeft size={18} />
                </button>
              )}
              <Link href="/explorer" className="flex items-center gap-2.5">
                <div className="w-7 h-7 rounded-lg gradient-bg flex items-center justify-center">
                  <span className="text-xs font-bold text-bg-primary">E</span>
                </div>
                <span className="text-sm font-semibold text-text-primary hidden sm:inline">
                  EvaporChain Explorer
                </span>
              </Link>
              <span className="text-[10px] px-2 py-0.5 rounded-full bg-accent-cyan/10 text-accent-cyan font-medium">
                Testnet
              </span>
            </div>

            <form onSubmit={handleSearch} className="flex-1 max-w-xl">
              <div className="relative">
                <Search
                  size={14}
                  className="absolute left-3 top-1/2 -translate-y-1/2 text-text-muted"
                />
                <input
                  type="text"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="Search by address, tx hash, or object ID..."
                  className="w-full bg-white/5 border border-white/5 rounded-lg pl-9 pr-4 py-2 text-sm text-text-primary placeholder:text-text-muted focus:outline-none focus:border-accent-cyan/30 transition-colors"
                />
              </div>
            </form>

            <Link
              href="/"
              className="text-xs text-text-muted hover:text-accent-cyan transition-colors hidden sm:inline"
            >
              evaporchain.com
            </Link>
          </div>

          {/* Tab row */}
          {!isDetail && (
            <div className="flex gap-1 -mb-px">
              {TABS.map((tab) => {
                const active = pathname === tab.href;
                return (
                  <Link
                    key={tab.href}
                    href={tab.href}
                    className={`px-4 py-2.5 text-sm font-medium border-b-2 transition-colors ${
                      active
                        ? "text-accent-cyan border-accent-cyan"
                        : "text-text-muted border-transparent hover:text-text-secondary"
                    }`}
                  >
                    {tab.label}
                  </Link>
                );
              })}
            </div>
          )}
        </div>
      </div>

      {/* Content */}
      <div className="max-w-7xl mx-auto px-6 py-8">{children}</div>
    </div>
  );
}
