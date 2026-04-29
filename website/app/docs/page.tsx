import Link from "next/link";
import ApiDocs from "@/components/ApiDocs";

export const metadata = {
  title: "API Reference — EvaporChain",
  description:
    "Live catalog of every EvaporChain HTTP endpoint: identity, substrate primitives, HBCT lifecycle, autonomic Sentinel governance, demo, observability.",
};

export default function DocsPage() {
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
            <Link href="/identity" className="hover:text-neutral-900">
              Chain Identity
            </Link>
            <Link href="/whitepaper" className="hover:text-neutral-900">
              Whitepaper
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
              API Reference
            </p>
            <h1 className="text-4xl font-light tracking-tight text-neutral-900 sm:text-5xl">
              Every launch endpoint,{" "}
              <span className="text-neutral-500">live from the node.</span>
            </h1>
            <p className="mt-4 max-w-2xl text-sm text-neutral-600">
              Auto-generated from <code className="font-mono">/api/docs</code>.
              Each entry shows method, path, what the endpoint does, and
              (where applicable) an example payload you can copy into{" "}
              <code className="font-mono">curl</code>.
            </p>
          </div>
          <ApiDocs />
        </section>
        <footer className="border-t border-neutral-200">
          <div className="mx-auto max-w-6xl px-6 py-8 text-xs text-neutral-500">
            <p>
              Single-λ thermodynamic-decay L1 ·{" "}
              <Link
                href="/identity"
                className="underline hover:text-neutral-700"
              >
                live dashboard
              </Link>
            </p>
          </div>
        </footer>
      </main>
    </div>
  );
}
