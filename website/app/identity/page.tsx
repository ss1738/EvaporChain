import Link from "next/link";
import IdentityDashboard from "@/components/IdentityDashboard";

export const metadata = {
  title: "Chain Identity — EvaporChain",
  description:
    "Live snapshot of every distinguishing EvaporChain primitive in one view: four-act narrative spine, light-cone DAG, TUR liveness, Lambda-Fold accumulator, autonomic Sentinel governance.",
};

export default function IdentityPage() {
  return (
    <div className="min-h-screen bg-white text-neutral-900">
      <header className="sticky top-0 z-10 border-b border-neutral-200 bg-white/85 backdrop-blur">
        <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
          <Link
            href="/"
            className="flex items-center gap-2 text-sm font-medium text-neutral-900 hover:text-neutral-600"
          >
            <svg
              width="20"
              height="20"
              viewBox="0 0 32 32"
              fill="none"
              xmlns="http://www.w3.org/2000/svg"
            >
              <path
                d="M16 2L28.66 9.5V24.5L16 32L3.34 24.5V9.5L16 2Z"
                stroke="currentColor"
                strokeWidth="1.5"
                fill="none"
              />
              <path
                d="M16 10C16 10 13 14 13 17C13 18.66 14.34 20 16 20C17.66 20 19 18.66 19 17C19 14 16 10 16 10Z"
                fill="currentColor"
                opacity="0.85"
              />
            </svg>
            EvaporChain
          </Link>
          <nav className="flex items-center gap-5 text-xs text-neutral-500">
            <Link href="/" className="hover:text-neutral-900">
              Home
            </Link>
            <Link href="/whitepaper" className="hover:text-neutral-900">
              Whitepaper
            </Link>
            <Link href="/explorer" className="hover:text-neutral-900">
              Explorer
            </Link>
            <a
              href="https://github.com/ss1738/EvaporChain"
              target="_blank"
              rel="noopener noreferrer"
              className="hover:text-neutral-900"
            >
              GitHub
            </a>
          </nav>
        </div>
      </header>
      <main>
        <section className="mx-auto max-w-6xl px-6 pb-20 pt-12">
          <div className="mb-10">
            <p className="mb-3 text-xs font-semibold uppercase tracking-[0.2em] text-neutral-500">
              Chain Identity
            </p>
            <h1 className="text-4xl font-light tracking-tight text-neutral-900 sm:text-5xl">
              What makes EvaporChain different,{" "}
              <span className="text-neutral-500">in real time.</span>
            </h1>
          </div>
          <IdentityDashboard />
        </section>
        <footer className="border-t border-neutral-200">
          <div className="mx-auto max-w-6xl px-6 py-8 text-xs text-neutral-500">
            <p>
              Single-λ thermodynamic-decay L1 · 30+ wired primitives ·{" "}
              <Link
                href="/whitepaper"
                className="underline hover:text-neutral-700"
              >
                whitepaper
              </Link>
            </p>
          </div>
        </footer>
      </main>
    </div>
  );
}
