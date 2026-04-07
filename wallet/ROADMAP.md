# EvaporChain Wallet Roadmap

> Last updated: 2026-04-07

## Vision

EvaporChain is the **only L1 blockchain shipping a built-in wallet** — no third-party extensions, no MetaMask, no Phantom. Every other chain (Ethereum, Solana, Sui, Aptos, NEAR, Avalanche, Cosmos, Polkadot) relies on third-party wallets. This is our competitive moat. We own the full stack from consensus to wallet.

The goal: **make EvaporChain the easiest blockchain to use from day one** — no extension installs, no seed phrase anxiety, no gas confusion. Energy decay is visual, objects are tangible, and the wallet is where users experience it all.

---

## What Exists Today

### Web Wallet (testnet.evaporchain.com/wallet)
- Browser-based, no install needed
- Create/import wallet, send EVAP, view balance
- Object viewer with live energy decay bars
- Transaction history, faucet integration
- Client-side key generation (ML-DSA post-quantum)

### CLI Wallet (evaporchain-wallet crate)
- 137 modules, 57 behavior tests, zero unsafe
- AES-256-GCM encrypted keystore with Argon2id KDF
- ML-DSA (FIPS 204) post-quantum signatures
- All 9 transaction types supported
- BIP-39 mnemonic backup/recovery
- Offline air-gapped signing
- Multi-account management
- DeFi primitives: liquidity pools, yield farming, DCA, limit orders
- Security: social recovery, escrow, multi-sig vault, threat monitoring
- Analytics: portfolio, P&L, tax tracking, whale tracker

---

## Roadmap

### Phase W1: Foundation (Current — Q2 2026)
- [x] Core keystore with AES-256-GCM + Argon2id
- [x] ML-DSA signing for all 9 transaction types
- [x] BIP-39 mnemonic backup/recovery
- [x] Offline air-gapped signing workflow
- [x] Multi-account management
- [x] Behavior test suite (57 tests passing)
- [x] Web wallet with live energy decay visualization
- [ ] **Commit and stabilize CLI wallet**
- [ ] Integration tests against running testnet node
- [ ] CLI wallet published to crates.io

### Phase W2: Browser Extension (Q2–Q3 2026)
> **Why:** Every serious L1 has a browser extension. Users expect one-click dApp interaction.

- [ ] Chrome/Firefox extension (Manifest V3)
- [ ] Popup UI: balance, send, receive, object viewer
- [ ] dApp injection (window.evaporchain provider)
- [ ] Transaction approval popup with energy cost preview
- [ ] One-click faucet claim
- [ ] Import/export from CLI wallet keystore
- [ ] Energy decay notifications ("Your NFT has 12 hours left")

### Phase W3: Mobile Wallet (Q3 2026)
> **Why:** Sui has Sui Wallet mobile, Solana has Phantom mobile, Aptos has Petra mobile. Mobile is where mainstream users live.

- [ ] React Native or Flutter app (iOS + Android)
- [ ] Biometric unlock (Face ID / fingerprint)
- [ ] QR code scanning for addresses and payments
- [ ] Push notifications for incoming transfers and decay warnings
- [ ] Camera-based seed phrase backup (encrypted QR export)
- [ ] NFC tap-to-pay for EVAP transfers (stretch)

### Phase W4: WalletConnect & dApp Ecosystem (Q3 2026)
> **Why:** dApps need a standard protocol to connect wallets. WalletConnect is the industry standard.

- [ ] WalletConnect v2 integration
- [ ] Session management (connect/disconnect/switch account)
- [ ] Transaction signing via WalletConnect relay
- [ ] dApp browser inside mobile wallet
- [ ] EIP-6963-style provider discovery for web wallet

### Phase W5: Hardware Wallet Support (Q4 2026)
> **Why:** Institutional users and whales require hardware security. Ledger/Trezor support signals maturity.

- [ ] Ledger integration (custom EvaporChain app)
- [ ] ML-DSA signing on hardware (post-quantum on secure element)
- [ ] Transaction preview on Ledger screen
- [ ] Multi-sig with hardware + software co-signing
- [ ] Air-gapped QR signing as fallback

### Phase W6: Smart Onboarding (Q4 2026)
> **Why:** The #1 barrier to crypto adoption is seed phrases and gas fees. Remove both.

- [ ] Social login (Google/Apple → zkLogin-style account creation)
- [ ] Email-based recovery (encrypted key shards to email)
- [ ] Gasless first transaction (relayer-sponsored)
- [ ] Human-readable names (alice.evap → 0xabc...)
- [ ] Progressive security: start custodial → graduate to self-custody
- [ ] Interactive onboarding tutorial ("Your first decaying object")

### Phase W7: Advanced Features (2027+)
- [ ] Multi-chain bridge UI (cross-chain swaps)
- [ ] DAO governance dashboard in wallet
- [ ] Staking management with APR calculator
- [ ] NFT gallery with decay timeline visualization
- [ ] AI assistant ("What should I refresh before it evaporates?")
- [ ] Batch transactions with gas optimization
- [ ] Privacy mode (shielded transfers)

---

## Competitive Positioning

| Feature | EvaporChain | Ethereum | Solana | Sui | Aptos |
|---------|-------------|----------|--------|-----|-------|
| Built-in web wallet | **Yes** | No | No | No | No |
| Built-in CLI wallet | **Yes** | No | Yes | Yes | Yes |
| Browser extension | Planned W2 | MetaMask | Phantom | Sui Wallet | Petra |
| Mobile wallet | Planned W3 | MetaMask | Phantom | Sui Wallet | Petra |
| Post-quantum sigs | **ML-DSA** | No | No | No | No |
| Energy decay UX | **Unique** | N/A | N/A | N/A | N/A |
| Social login | Planned W6 | No | No | zkLogin | Aptos Connect |
| Hardware wallet | Planned W5 | Ledger | Ledger | Ledger | Ledger |

**Our unique advantages:**
1. Only L1 with built-in wallet (no third-party dependency)
2. Only L1 with post-quantum signatures in wallet
3. Only wallet with energy decay visualization and lifecycle management
4. Full-stack ownership: consensus → execution → wallet → dApp

---

## Design Principles

1. **No extensions required.** The web wallet works in any browser, zero install.
2. **Decay is visible.** Every object shows its energy bar. Users feel the thermodynamics.
3. **Post-quantum by default.** ML-DSA everywhere. No legacy ECDSA option.
4. **Progressive complexity.** New users see "Send/Receive". Power users unlock CLI, multi-sig, air-gapped signing.
5. **Offline-first.** Sign transactions without internet. Broadcast when ready.
6. **Own your keys.** No custodial accounts by default. Social login graduates to self-custody.
