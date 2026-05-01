import type { Metadata } from "next";
import Link from "next/link";
import { Vote, FileText, Plus } from "lucide-react";
import "../styles/globals.css";

export const metadata: Metadata = {
  title: "EvaporChain Governance Portal",
  description:
    "Draft, endorse, and submit stake-quorum amendments for the EvaporChain L1 — fork-choice mode changes and contract upgrades.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body className="min-h-screen bg-evap-bg font-sans text-zinc-900 antialiased">
        <nav className="border-b border-evap-border bg-white">
          <div className="mx-auto flex max-w-6xl items-center justify-between px-4 py-3">
            <Link href="/" className="flex items-center gap-2">
              <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-gradient-to-br from-evap-violet to-evap-cyan">
                <Vote className="h-4 w-4 text-white" strokeWidth={2.5} />
              </div>
              <div>
                <p className="text-sm font-bold text-zinc-900">Governance Portal</p>
                <p className="text-[10px] text-zinc-400">EvaporChain L1 amendments</p>
              </div>
            </Link>
            <div className="flex items-center gap-1">
              <Link
                href="/"
                className="flex items-center gap-1 rounded-md px-3 py-1.5 text-xs font-medium text-zinc-500 hover:bg-zinc-50 hover:text-zinc-900"
              >
                <FileText className="h-3.5 w-3.5" />
                Proposals
              </Link>
              <Link
                href="/proposals/new"
                className="flex items-center gap-1 rounded-md bg-gradient-to-r from-evap-violet to-evap-cyan px-3 py-1.5 text-xs font-semibold text-white hover:opacity-90"
              >
                <Plus className="h-3.5 w-3.5" />
                Draft
              </Link>
            </div>
          </div>
        </nav>
        <main className="mx-auto max-w-6xl px-4 py-6">{children}</main>
        <footer className="mt-12 border-t border-evap-border py-6">
          <p className="mx-auto max-w-6xl px-4 text-[10px] text-zinc-400">
            Off-chain coordination forum. The chain re-validates every quorum
            at submit time — endorsements stored here are an aggregation gate,
            not an authorisation.
          </p>
        </footer>
      </body>
    </html>
  );
}
