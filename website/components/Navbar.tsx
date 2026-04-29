"use client";

import { useState, useEffect, useRef } from "react";
import { Menu, X, ChevronDown } from "lucide-react";
import Link from "next/link";

const TESTNET = "https://testnet.evaporchain.com";

const productLinks = [
  { label: "Wallet", href: "/wallet" },
  { label: "NFT Marketplace", href: "/nft" },
  { label: "Tokens", href: "/tokens" },
  { label: "Staking", href: "/staking" },
  { label: "Governance", href: "/dao" },
  { label: "Explorer", href: "/explorer" },
  { label: "Chain Identity ↗", href: "/identity" },
];

const navLinks = [
  { label: "Technology", href: "/#technology" },
  { label: "Developers", href: "/developers" },
  { label: "Roadmap", href: "/#roadmap" },
  { label: "Whitepaper", href: "/whitepaper" },
];

function Logo() {
  return (
    <svg width="32" height="32" viewBox="0 0 32 32" fill="none" xmlns="http://www.w3.org/2000/svg">
      <defs>
        <linearGradient id="logoGrad" x1="0" y1="0" x2="32" y2="32">
          <stop offset="0%" stopColor="#00f0ff" />
          <stop offset="100%" stopColor="#8b5cf6" />
        </linearGradient>
      </defs>
      <path
        d="M16 2L28.66 9.5V24.5L16 32L3.34 24.5V9.5L16 2Z"
        stroke="url(#logoGrad)"
        strokeWidth="1.5"
        fill="none"
      />
      <path
        d="M16 10C16 10 13 14 13 17C13 18.66 14.34 20 16 20C17.66 20 19 18.66 19 17C19 14 16 10 16 10Z"
        fill="url(#logoGrad)"
        opacity="0.9"
      />
      <path
        d="M16 8C16 8 15.5 6 15 4.5"
        stroke="url(#logoGrad)"
        strokeWidth="1"
        strokeLinecap="round"
        opacity="0.5"
      />
      <path
        d="M17 7.5C17 7.5 17.5 5.5 17.2 4"
        stroke="url(#logoGrad)"
        strokeWidth="0.8"
        strokeLinecap="round"
        opacity="0.3"
      />
    </svg>
  );
}

export default function Navbar() {
  const [scrolled, setScrolled] = useState(false);
  const [mobileOpen, setMobileOpen] = useState(false);
  const [productsOpen, setProductsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 20);
    window.addEventListener("scroll", onScroll);
    return () => window.removeEventListener("scroll", onScroll);
  }, []);

  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setProductsOpen(false);
      }
    };
    document.addEventListener("mousedown", onClick);
    return () => document.removeEventListener("mousedown", onClick);
  }, []);

  return (
    <nav
      className={`fixed top-0 left-0 right-0 z-50 transition-all duration-300 ${
        scrolled
          ? "bg-[rgba(10,10,15,0.85)] backdrop-blur-xl border-b border-white/5"
          : "bg-transparent"
      }`}
    >
      <div className="max-w-7xl mx-auto px-6 h-16 flex items-center justify-between">
        <Link href="/" className="flex items-center gap-2.5">
          <Logo />
          <span className="text-lg font-semibold tracking-tight text-text-primary">
            EvaporChain
          </span>
        </Link>

        <div className="hidden md:flex items-center gap-8">
          {/* Products dropdown */}
          <div ref={dropdownRef} className="relative">
            <button
              onClick={() => setProductsOpen(!productsOpen)}
              className="flex items-center gap-1 text-sm text-text-secondary hover:text-accent-cyan transition-colors duration-200"
            >
              Products
              <ChevronDown
                size={14}
                className={`transition-transform duration-200 ${productsOpen ? "rotate-180" : ""}`}
              />
            </button>
            {productsOpen && (
              <div className="absolute top-full left-0 mt-2 w-52 bg-bg-card/95 backdrop-blur-xl border border-white/10 rounded-xl shadow-2xl py-2 overflow-hidden">
                {productLinks.map((link) => (
                  <Link
                    key={link.label}
                    href={link.href}
                    className="block px-4 py-2.5 text-sm text-text-secondary hover:text-accent-cyan hover:bg-white/5 transition-colors"
                    onClick={() => setProductsOpen(false)}
                  >
                    {link.label}
                  </Link>
                ))}
              </div>
            )}
          </div>

          {navLinks.map((link) => (
            <Link
              key={link.label}
              href={link.href}
              className="text-sm text-text-secondary hover:text-accent-cyan transition-colors duration-200"
            >
              {link.label}
            </Link>
          ))}
        </div>

        <div className="hidden md:block">
          <a
            href={TESTNET}
            target="_blank"
            rel="noopener noreferrer"
            className="gradient-bg text-sm font-medium text-bg-primary px-6 py-2 rounded-full hover:shadow-[0_0_20px_rgba(0,240,255,0.3)] transition-shadow duration-300"
          >
            Launch App &rarr;
          </a>
        </div>

        <button
          className="md:hidden text-text-primary"
          onClick={() => setMobileOpen(!mobileOpen)}
          aria-label="Toggle menu"
        >
          {mobileOpen ? <X size={24} /> : <Menu size={24} />}
        </button>
      </div>

      {mobileOpen && (
        <div className="md:hidden bg-[rgba(10,10,15,0.95)] backdrop-blur-xl border-t border-white/5">
          <div className="px-6 py-4 flex flex-col gap-1">
            <p className="text-xs uppercase tracking-widest text-text-muted mb-2 mt-2">Products</p>
            {productLinks.map((link) => (
              <Link
                key={link.label}
                href={link.href}
                className="text-text-secondary hover:text-accent-cyan transition-colors py-2 pl-2"
                onClick={() => setMobileOpen(false)}
              >
                {link.label}
              </Link>
            ))}

            <div className="border-t border-white/5 my-3" />

            {navLinks.map((link) => (
              <Link
                key={link.label}
                href={link.href}
                className="text-text-secondary hover:text-accent-cyan transition-colors py-2"
                onClick={() => setMobileOpen(false)}
              >
                {link.label}
              </Link>
            ))}

            <a
              href={TESTNET}
              target="_blank"
              rel="noopener noreferrer"
              className="gradient-bg text-center text-sm font-medium text-bg-primary px-6 py-2.5 rounded-full mt-4"
              onClick={() => setMobileOpen(false)}
            >
              Launch App &rarr;
            </a>
          </div>
        </div>
      )}
    </nav>
  );
}
