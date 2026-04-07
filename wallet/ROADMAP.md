# EvaporChain Wallet Roadmap

> Last updated: 2026-04-07

## Vision

**"The wallet where your assets are alive."**

EvaporChain is the **only L1 blockchain shipping a built-in wallet** — no MetaMask, no Phantom, no third-party dependency. We own the full stack from consensus to wallet. Objects breathe, decay, and die in real-time. No other wallet on Earth can show that.

We cannot beat MetaMask at being MetaMask. We win by being **the only wallet where blockchain objects have lifespans** — energy bars depleting, decay forecasts, ghost resurrection, refresh urgency. That's baked into our consensus. It cannot be copied by bolting on a plugin.

---

## Honest Assessment: Where We Stand vs The Giants

| | MetaMask | Phantom | Trust Wallet | **EvaporChain** |
|--|---------|---------|-------------|-------------|
| Real users | 30M+ monthly | 15M+ | 60M+ | **0** |
| Usable product | Extension + Mobile | Extension + Mobile | Mobile + Extension | **Rust code only** |
| Chains supported | 100+ EVM | 4 | 100+ | **1 (testnet)** |
| dApp ecosystem | 10,000+ | 3,000+ | 5,000+ | **0** |
| In-app swaps | Yes | Yes | Yes | **No** |
| Fiat on-ramp | Yes | Yes | Yes | **No** |
| Security audits | Multiple firms | Multiple firms | Multiple firms | **None** |
| Funding | ConsenSys $450M+ | $118M | Binance-backed | **Bootstrapped** |
| Team size | 100+ | 50+ | 100+ | **1** |
| Post-quantum sigs | **No** | **No** | **No** | **ML-DSA (FIPS 204)** |
| Energy decay UX | **No** | **No** | **No** | **Yes — unique** |
| Full-stack ownership | **No** | **No** | **No** | **Yes — consensus to wallet** |

**Bottom line:** We have a powerful engine (137 modules, 57 tests). We haven't built the car. Users interact with buttons, not Rust crates.

---

## What Exists Today (Completed)

### CLI Wallet (evaporchain-wallet crate)
- [x] 137 modules, 57 behavior tests, zero unsafe
- [x] AES-256-GCM encrypted keystore with Argon2id KDF
- [x] ML-DSA (FIPS 204) post-quantum signatures
- [x] All 9 transaction types (Transfer, Refresh, CreateObject, DeployContract, CallContract, DeployScript, CallScript, ValidatorStake, ValidatorExit)
- [x] BIP-39 24-word mnemonic backup/recovery
- [x] Offline air-gapped signing workflow
- [x] Multi-account management with active switching
- [x] 40+ CLI subcommands
- [x] DeFi primitives: liquidity pools, yield farming, DCA, limit orders
- [x] Security: social recovery, escrow, multi-sig vault, threat monitoring
- [x] Analytics: portfolio, P&L, tax tracking, whale tracker

### Web Wallet (testnet.evaporchain.com/wallet)
- [x] Browser-based, no install needed
- [x] Create/import wallet, send EVAP, view balance
- [x] Object viewer with live energy decay bars
- [x] Transaction history, faucet integration
- [x] Client-side ML-DSA key generation

---

## The 4-Tier Roadmap

### TIER 1: Make It Touchable (NOW — Without This, Nothing Else Matters)

> Users must be able to click a button and use the wallet. Code in a repo is not a product.

#### Step 1.1: Browser Extension Wallet
> **This is the single most important thing to build next.**

- [x] Chrome extension (Manifest V3) with popup UI
- [x] Account creation with mnemonic display
- [x] Balance display with EVAP amount
- [x] Send EVAP — address input, amount, confirm, broadcast
- [x] Receive — show address + QR code
- [x] Object viewer — list owned objects with energy bars
- [x] Transaction history — recent sends/receives
- [x] dApp injection (`window.evaporchain` provider API)
- [x] Transaction approval popup with energy cost preview
- [x] Import/export keystore (compatible with CLI wallet)
- [x] One-click faucet claim (testnet)
- [x] Network switcher (testnet → mainnet when ready)

**Tech stack:** TypeScript + React + Vite, WebAssembly bridge to evaporchain-crypto for ML-DSA signing in-browser, Manifest V3 service worker.

#### Step 1.2: Three Reference dApps
> A wallet with no dApps is useless. Ship 3 that showcase energy decay.

- [x] **Decaying NFT Marketplace** — Mint NFTs that visually decay. Users refresh to keep alive.
- [x] **Energy Pool** — Pool EVAP energy across objects. Cooperative refresh game.
- [x] **Mortal Messages** — Post messages that evaporate after their energy runs out. Ephemeral social.

Each dApp connects via `window.evaporchain` → extension approval popup → signed transaction.

#### Step 1.3: Integration Tests Against Live Node
- [x] Behavior tests that spin up a local testnet node
- [x] End-to-end: create wallet → faucet → send → verify balance
- [x] Extension ↔ node ↔ dApp full loop test

---

### TIER 2: Match Table Stakes (Without This, Users Leave Immediately)

> Every serious wallet has these. Missing any one is a reason to uninstall.

#### Step 2.1: In-App Token Swap
- [x] Simple swap UI: select token pair, input amount, preview rate, confirm
- [x] Connect to on-chain liquidity pool (or DEX contract)
- [x] Slippage tolerance setting
- [x] Transaction preview with energy cost

#### Step 2.2: Mobile Wallet
- [x] React Native app (iOS + Android)
- [x] Biometric unlock (Face ID / fingerprint)
- [x] QR code scanning for addresses and payments
- [x] Push notifications: incoming transfers + decay warnings ("Your NFT has 2 hours left")
- [x] Same keystore format as CLI + extension (import/export)

#### Step 2.3: Fiat On-Ramp
- [x] MoonPay or Transak integration
- [x] Buy EVAP with credit card / Apple Pay / Google Pay
- [x] KYC flow handled by partner (not us)

#### Step 2.4: Transaction Simulation
- [x] "This will cost X energy" preview before signing
- [x] "This object will survive Y more days at current decay" forecast
- [x] Warning if sending to a ghost address (evaporated account)

#### Step 2.5: NFT Gallery
- [x] Visual grid of owned NFTs
- [x] Each shows: image, name, energy bar, time-to-evaporation countdown
- [x] "Refresh" button on each NFT (one-click energy top-up)
- [x] Sort by: most urgent (lowest energy first)

---

### TIER 3: Differentiate (This Is Where We WIN)

> Features no other wallet can copy because they're baked into EvaporChain's consensus.

#### Step 3.1: "Quantum-Safe" Badge and Marketing
- [x] Prominent "Post-Quantum Secured" badge in wallet UI
- [x] Explainer: "Your keys are safe even against quantum computers"
- [x] Comparison page: "MetaMask uses ECDSA (quantum-vulnerable). We use ML-DSA (quantum-safe)."
- [x] This is a **fear-based differentiator** — and it's real, not marketing fluff

#### Step 3.2: Energy Dashboard
- [x] "3 objects expiring today" — urgency notification
- [x] Portfolio energy chart: total energy over time (line chart, declining curve)
- [x] "Weekly energy report" — objects refreshed, objects evaporated, energy spent
- [x] Color-coded health: green (>50%), yellow (10-50%), red (<10%), skull (ghost)

#### Step 3.3: One-Click Refresh
- [x] "Keep Alive" button on every object
- [x] Batch refresh: "Refresh all objects below 20% energy" in one transaction
- [x] Auto-refresh scheduler: "Keep this object above 30% automatically"
- [x] Energy cost calculator: "Refreshing costs X EVAP, extends life by Y days"

#### Step 3.4: Decay Forecasting
- [x] "At current rate, your portfolio loses 12% energy this week"
- [x] Per-object: "This NFT evaporates on April 23 at 2:15 PM"
- [x] "Cheapest refresh strategy" optimizer — which objects to refresh first

#### Step 3.5: Ghost Recovery
- [x] "This object evaporated 3 days ago. Resurrect for 500 EVAP?"
- [x] Ghost browser — explore evaporated objects with Merkle proofs
- [x] "Recovery window" indicator — how long until ghost proof expires

#### Step 3.6: Social Login (No Seed Phrases)
- [x] Google / Apple sign-in → automatic wallet creation
- [x] No seed phrase shown on first use (stored encrypted, revealed on demand)
- [x] Progressive security: start simple → graduate to self-custody
- [x] "Your first decaying object" interactive tutorial

---

### TIER 4: Scale (Once We Have Users)

> Build these when we have >1,000 active wallets.

#### Step 4.1: WalletConnect v2
- [x] Standard WalletConnect integration for third-party dApps
- [x] Session management (connect/disconnect/switch)
- [x] dApp browser inside mobile wallet

#### Step 4.2: Hardware Wallet (Ledger)
- [x] Custom Ledger app for ML-DSA signing
- [x] Transaction preview on Ledger screen
- [x] Multi-sig: hardware + software co-signing

#### Step 4.3: Multi-Chain Bridge
- [x] Bridge UI for cross-chain asset transfers
- [x] Wrapped assets from Ethereum/Solana
- [x] Bridge transaction tracking with status updates

#### Step 4.4: Developer SDK
- [x] `@evaporchain/wallet-sdk` npm package
- [x] `connect()`, `signTransaction()`, `getBalance()`, `getObjects()`
- [x] 5-minute integration guide for dApp developers
- [x] TypeScript types for all transaction types

#### Step 4.5: Plugin System
- [x] Third-party developers can add wallet features
- [x] Plugin marketplace inside wallet
- [x] Sandboxed execution (no access to private keys)

#### Step 4.6: AI Assistant
- [x] "What should I refresh before it evaporates?"
- [x] "Optimize my energy spend this week"
- [x] Natural language transaction building: "Send 100 EVAP to alice.evap"

---

## Design Principles

1. **Decay is visible.** Every object shows its energy bar. Users feel the thermodynamics.
2. **Post-quantum by default.** ML-DSA everywhere. No legacy ECDSA. No compromise.
3. **Progressive complexity.** New users see "Send/Receive". Power users unlock CLI, multi-sig, air-gapped signing.
4. **Offline-first.** Sign transactions without internet. Broadcast when ready.
5. **Own your keys.** No custodial accounts by default. Social login graduates to self-custody.
6. **Ship fast, iterate.** Extension MVP first. Polish later. Users over perfection.

---

## How We Win

We don't compete on features. MetaMask will always have more chains. Trust Wallet will always have more users. We compete on **experience**:

- MetaMask shows static balances → We show **energy bars depleting in real-time**
- Phantom shows a token list → We show **objects with heartbeats and lifespans**
- Trust Wallet is a vault → We're a **living ecosystem viewer**
- All wallets use ECDSA → We use **ML-DSA (the only quantum-safe wallet)**

**The moat is the consensus.** Energy decay is not a feature we can be out-shipped on. It's baked into every block, every object, every transaction. To copy us, they'd have to fork our entire chain.

---

## Execution Order

```
NOW        → Step 1.1: Browser Extension (THE priority)
           → Step 1.2: 3 Reference dApps
           → Step 1.3: Integration Tests
           
NEXT       → Step 2.1: Token Swap
           → Step 2.2: Mobile Wallet  
           → Step 2.3: Fiat On-Ramp
           → Step 2.4: Transaction Simulation
           → Step 2.5: NFT Gallery

THEN       → Step 3.1-3.6: Differentiation features
           
LATER      → Step 4.1-4.6: Scale features
```

**Rule: Do not start Tier N+1 until Tier N is shipped and tested.**
