import type { Metadata } from "next";
import Link from "next/link";
import { Activity, Network, ScanSearch } from "lucide-react";
import "../styles/globals.css";

export const metadata: Metadata = {
  title: "EvaporChain Explorer · Light",
  description:
    "Light-client block explorer for EvaporChain. Pulls compact CSLC headers, verifies tx-inclusion + Verkle state proofs in-browser. The indexer is never trusted.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body className="min-h-screen bg-evap-bg font-sans text-zinc-900 antialiased">
        <nav className="border-b border-evap-border bg-white">
          <div className="mx-auto flex max-w-6xl items-center justify-between px-4 py-3">
            <Link href="/" className="flex items-center gap-2">
              <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-gradient-to-br from-evap-cyan to-evap-purple">
                <ScanSearch className="h-4 w-4 text-white" />
              </div>
              <div>
                <p className="text-sm font-bold text-zinc-900">EvaporChain Explorer</p>
                <p className="text-[10px] text-zinc-400">Light · client-verified</p>
              </div>
            </Link>
            <div className="flex items-center gap-1">
              <Link
                href="/"
                className="inline-flex items-center gap-1 rounded-md px-3 py-1.5 text-xs font-medium text-zinc-500 hover:bg-zinc-50 hover:text-zinc-900"
              >
                <Activity className="h-3 w-3" /> Headers
              </Link>
              <Link
                href="/state-graph"
                className="inline-flex items-center gap-1 rounded-md px-3 py-1.5 text-xs font-medium text-zinc-500 hover:bg-zinc-50 hover:text-zinc-900"
              >
                <Network className="h-3 w-3" /> ε-Machine
              </Link>
            </div>
          </div>
        </nav>
        <main className="mx-auto max-w-6xl px-4 py-6">{children}</main>
        <footer className="mt-12 border-t border-evap-border py-6">
          <p className="mx-auto max-w-6xl px-4 text-[10px] text-zinc-400">
            Light explorer — compact headers only, all proofs verified
            client-side. The indexer can&apos;t lie to you about inclusion.
          </p>
        </footer>
      </body>
    </html>
  );
}
