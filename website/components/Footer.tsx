const T = "https://testnet.evaporchain.com";

export default function Footer() {
  return (
    <footer className="border-t border-white/5 py-16 px-6">
      <div className="max-w-6xl mx-auto grid grid-cols-1 md:grid-cols-4 gap-12">
        <div>
          <div className="flex items-center gap-2.5 mb-3">
            <svg width="24" height="24" viewBox="0 0 32 32" fill="none">
              <defs>
                <linearGradient id="footerGrad" x1="0" y1="0" x2="32" y2="32">
                  <stop offset="0%" stopColor="#00f0ff" />
                  <stop offset="100%" stopColor="#8b5cf6" />
                </linearGradient>
              </defs>
              <path
                d="M16 2L28.66 9.5V24.5L16 32L3.34 24.5V9.5L16 2Z"
                stroke="url(#footerGrad)"
                strokeWidth="1.5"
                fill="none"
              />
              <path
                d="M16 10C16 10 13 14 13 17C13 18.66 14.34 20 16 20C17.66 20 19 18.66 19 17C19 14 16 10 16 10Z"
                fill="url(#footerGrad)"
                opacity="0.9"
              />
            </svg>
            <span className="text-base font-semibold">EvaporChain</span>
          </div>
          <p className="text-sm text-text-muted mb-4">
            Thermodynamic blockchain architecture
          </p>
          <p className="text-xs text-text-muted">&copy; 2026 EvaporChain</p>
        </div>

        <div>
          <h4 className="text-sm font-semibold mb-4">Testnet</h4>
          <ul className="space-y-2.5">
            <li><a href={`${T}/explorer`} className="text-sm text-text-muted hover:text-accent-cyan transition-colors">Explorer</a></li>
            <li><a href={`${T}/wallet`} className="text-sm text-text-muted hover:text-accent-cyan transition-colors">Wallet</a></li>
            <li><a href={`${T}/faucet`} className="text-sm text-text-muted hover:text-accent-cyan transition-colors">Faucet</a></li>
            <li><a href={`${T}/nft`} className="text-sm text-text-muted hover:text-accent-cyan transition-colors">NFT Marketplace</a></li>
            <li><a href={`${T}/tokens`} className="text-sm text-text-muted hover:text-accent-cyan transition-colors">Tokens</a></li>
            <li><a href={`${T}/staking`} className="text-sm text-text-muted hover:text-accent-cyan transition-colors">Staking</a></li>
            <li><a href={`${T}/dao`} className="text-sm text-text-muted hover:text-accent-cyan transition-colors">Governance</a></li>
          </ul>
        </div>

        <div>
          <h4 className="text-sm font-semibold mb-4">Resources</h4>
          <ul className="space-y-2.5">
            <li>
              <a
                href="/whitepaper"
                className="text-sm text-text-muted hover:text-accent-cyan transition-colors"
              >
                Whitepaper
              </a>
            </li>
            <li>
              <a
                href="https://github.com/ss1738/EvaporChain"
                target="_blank"
                rel="noopener noreferrer"
                className="text-sm text-text-muted hover:text-accent-cyan transition-colors"
              >
                GitHub
              </a>
            </li>
            <li>
              <a
                href="https://github.com/ss1738/EvaporChain/tree/main/standards"
                target="_blank"
                rel="noopener noreferrer"
                className="text-sm text-text-muted hover:text-accent-cyan transition-colors"
              >
                Standards (EVR-721, EVR-20)
              </a>
            </li>
          </ul>
        </div>

        <div>
          <h4 className="text-sm font-semibold mb-4">Community</h4>
          <ul className="space-y-2.5">
            <li>
              <a
                href="https://twitter.com/evaporchain"
                target="_blank"
                rel="noopener noreferrer"
                className="text-sm text-text-muted hover:text-accent-cyan transition-colors"
              >
                Twitter / X
              </a>
            </li>
            <li>
              <a
                href="https://discord.gg/evaporchain"
                target="_blank"
                rel="noopener noreferrer"
                className="text-sm text-text-muted hover:text-accent-cyan transition-colors"
              >
                Discord
              </a>
            </li>
            <li>
              <a
                href="mailto:hello@evaporchain.com"
                className="text-sm text-text-muted hover:text-accent-cyan transition-colors"
              >
                hello@evaporchain.com
              </a>
            </li>
          </ul>
        </div>
      </div>
    </footer>
  );
}
