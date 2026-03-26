"use client";

import { useState, useEffect } from "react";
import { Menu, X } from "lucide-react";

const navLinks = [
  { label: "Technology", href: "#technology" },
  { label: "Contracts", href: "#contracts" },
  { label: "Roadmap", href: "#roadmap" },
  { label: "Whitepaper", href: "/whitepaper" },
  { label: "Explorer", href: "https://testnet.evaporchain.com/explorer" },
  { label: "Ecosystem", href: "#contracts" },
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

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 20);
    window.addEventListener("scroll", onScroll);
    return () => window.removeEventListener("scroll", onScroll);
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
        <a href="#home" className="flex items-center gap-2.5">
          <Logo />
          <span className="text-lg font-semibold tracking-tight text-text-primary">
            EvaporChain
          </span>
        </a>

        <div className="hidden md:flex items-center gap-8">
          {navLinks.map((link) => (
            <a
              key={link.label}
              href={link.href}
              className="text-sm text-text-secondary hover:text-accent-cyan transition-colors duration-200"
            >
              {link.label}
            </a>
          ))}
        </div>

        <div className="hidden md:block">
          <a
            href="https://testnet.evaporchain.com"
            className="gradient-bg text-sm font-medium text-bg-primary px-6 py-2 rounded-full hover:shadow-[0_0_20px_rgba(0,240,255,0.3)] transition-shadow duration-300"
          >
            Try the Testnet
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
          <div className="px-6 py-4 flex flex-col gap-4">
            {navLinks.map((link) => (
              <a
                key={link.label}
                href={link.href}
                className="text-text-secondary hover:text-accent-cyan transition-colors py-2"
                onClick={() => setMobileOpen(false)}
              >
                {link.label}
              </a>
            ))}
            <a
              href="https://testnet.evaporchain.com"
              className="gradient-bg text-center text-sm font-medium text-bg-primary px-6 py-2.5 rounded-full mt-2"
              onClick={() => setMobileOpen(false)}
            >
              Try the Testnet
            </a>
          </div>
        </div>
      )}
    </nav>
  );
}
