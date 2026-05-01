import type { Metadata } from "next";
import Link from "next/link";
import "../styles/globals.css";

export const metadata: Metadata = {
  title: "EvaporChain Validator Analytics",
  description:
    "Read-only analytics dashboard for EvaporChain validators: stake, block-production, delegations, slashes, finality and network health.",
};

const NAV: Array<{ href: string; label: string }> = [
  { href: "/", label: "Overview" },
  { href: "/validators", label: "Validators" },
  { href: "/finality", label: "Finality" },
  { href: "/network", label: "Network" },
];

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body className="min-h-screen bg-evap-bg font-sans text-zinc-900 antialiased">
        <nav className="border-b border-evap-border bg-white">
          <div className="mx-auto flex max-w-6xl items-center justify-between px-4 py-3">
            <Link href="/" className="flex items-center gap-2">
              <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-gradient-to-br from-evap-cyan to-evap-purple">
                <span className="text-sm font-bold text-white">V</span>
              </div>
              <div>
                <p className="text-sm font-bold text-zinc-900">Validator Analytics</p>
                <p className="text-[10px] text-zinc-400">EvaporChain · read-only</p>
              </div>
            </Link>
            <div className="flex items-center gap-1">
              {NAV.map((n) => (
                <Link
                  key={n.href}
                  href={n.href}
                  className="rounded-md px-3 py-1.5 text-xs font-medium text-zinc-500 hover:bg-zinc-50 hover:text-zinc-900"
                >
                  {n.label}
                </Link>
              ))}
            </div>
          </div>
        </nav>
        <main className="mx-auto max-w-6xl px-4 py-6">{children}</main>
        <footer className="mt-12 border-t border-evap-border py-6">
          <p className="mx-auto max-w-6xl px-4 text-[10px] text-zinc-400">
            Validator Analytics — EvaporChain. Refresh interval: 15s. Source:{" "}
            <span className="font-mono">/api/validators · /api/network/* · /api/finality/gap · /metrics</span>
          </p>
        </footer>
      </body>
    </html>
  );
}
