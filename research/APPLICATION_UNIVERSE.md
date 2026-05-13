# EvaporChain Application Universe

**Status:** Strategic reference doc — what applications structurally need EvaporChain's energy-decay primitive (vs running on Ethereum / Solana / Cosmos).
**Pairs with:** `INEVITABILITY_STRATEGY.md` (the thesis), `INVENTION_STACK.md` (the primitives), `TOKENOMICS.md` (the economics).
**Last updated:** 2026-05-13.
**Purpose:** Catalog the application universe so (a) Satyawan picks 1-2 reference dApps to build personally, (b) the Foundation knows which grant categories to fund, (c) ecosystem builders see where to focus.

---

## The unique-value-prop filter

Before any dApp gets built or funded on EvaporChain, run it through this filter:

> **Would this dApp work as well on Ethereum / Solana / Cosmos?**
> - If yes: **don't build it on EvaporChain.** You'll just lose the comparison.
> - If no (because it needs native data expiry / decay / forgetting): this is exactly the target.

Examples of the filter applied:

| Application | Filter result | Reason |
|---|---|---|
| Uniswap-clone on EvaporChain | ❌ Skip | Works fine on Ethereum, no advantage |
| GDPR-Erasure-as-a-Service | ✅ Build | Impossible on Ethereum (immutability conflict) |
| NFT marketplace | ❌ Skip | Works fine on Ethereum, no advantage |
| Decaying event tickets | ✅ Build | Unique to energy-decay primitive |
| IoT sensor data marketplace with auto-pruning | ✅ Build | Cost-economics break on Ethereum |
| Stablecoin like USDC | ❌ Skip | No decay advantage |
| Wörgl-style demurrage stablecoin | ✅ Build | Demurrage is built into the protocol |

**The filter is brutal but accurate.** Anything that doesn't pass it is wasted ecosystem effort.

---

## The 12 application categories

### 1. Regulatory-compliant data infrastructure (the largest commercial wedge)
- **GDPR Right-to-Erasure-as-a-Service** — on-chain immutable consent + processing log + auto-expiry per regulation
- **MiFID-II audit trail** — trade records retained 7 years then auto-deleted
- **HIPAA-compliant clinical trial data** — patient consent + data lifecycle on-chain
- **KYC vaults with expiry** — financial institutions, gambling sites, age-gated platforms
- **Marketing-consent registers** — revocable, time-bound permissions

**Addressable market estimate:** $50–200B (every EU SaaS company + every regulated financial institution).

### 2. Time-bound digital assets
- Concert / event tickets that auto-expire after the event (no scalping risk after event)
- Flight reservations + boarding passes that decay after travel
- Subscription tokens (gym memberships, software licenses, time-share access)
- Coupons / voucher systems with built-in expiry
- Hotel-room access tokens (on-chain rental keys)

### 3. Decaying NFTs and digital collectibles
- Rental NFTs — borrow an asset for N blocks, auto-return
- Insurance policies as NFTs — expire after term, no manual close-out
- Bail bonds, escrow with mathematical time-decay
- Seasonal game NFTs that fade after tournament / season ends

### 4. Reputation + identity systems
- **Skill certifications (SHLM)** — `evaporchain-shlm` substrate; professional certs that decay if not re-validated
- Credit scores with decay — credit history doesn't haunt you forever (matches GDPR spirit)
- DAO governance with attendance-decay — voting power drops if you don't participate
- Trust networks — decentralised reputation that decays without activity
- Anonymous credentials (age verification, etc.) that auto-erase the proof after use

### 5. Healthcare and life sciences
- Patient consent management — granular, time-bound, revocable
- Clinical trial data lifecycle — automatic retention-then-delete per regulation
- Medical records with HIPAA + GDPR minimum-necessary enforcement
- Drug supply-chain tracking that releases after consumption / expiry date

### 6. IoT + industrial telemetry (primary enterprise thesis wedge)
- Sensor data marketplaces — fresh data has full weight, stale data decays automatically
- Industrial monitoring with cost-optimised retention
- Connected vehicle telemetry (insurance windows, then delete)
- Smart city data — traffic, environmental, mobility with automatic decay
- **Target enterprises:** Bosch.IO Stuttgart (Dirk Slama's group has published on blockchain-IoT cost), Siemens Cre8Ventures, Schneider Electric, ABB

### 7. DeFi with native decay primitives
- Decaying yield vaults — return decreases as deposits sit idle (forces utilisation)
- **Wörgl-style demurrage stablecoins** — Silvio Gesell anti-hoarding money
- Lending with built-in collateral decay — encourages active management
- Time-locked options markets — natural expiry, no manual close
- **Self-Future Vault (SFSV)** — `evaporchain-sfsv` substrate; lock assets to your future self

### 8. Markets and auctions
- Sealed-bid auctions (using encrypted mempool) with automatic reveal-then-decay
- Prediction markets with auto-settlement and history-purge
- Concert ticket exchanges with anti-scalping decay

### 9. Privacy + messaging
- Self-destructing message channels — Signal-style but on-chain immutable until decay
- Whistleblower platforms with mathematically guaranteed auto-purge
- Time-bound disclosures — embargoed information that auto-releases

### 10. Governance and DAOs
- Voting power that decays with inactivity — anti-zombie governance
- Proposal sunset clauses — DAO decisions auto-expire if not renewed
- **Memento commit-and-forget contracts** — `evaporchain-memento` substrate; conditional commitments that evaporate

### 11. Real-World Asset (RWA) tokenization with lifecycle
- Tokenized bonds that auto-mature
- Tokenized invoices that decay after settlement
- Tokenized derivatives (expire by definition — strong fit)
- Tokenized insurance contracts with native term-decay
- **Target enterprises:** HSBC Orion (UK DIGIT digital-gilt mandate), Franklin Templeton (BENJI pattern), tokenised-RWA platforms

### 12. Climate, ESG, carbon
- Carbon credits with retirement-on-use + automatic certificate decay
- Renewable energy certificates (RECs) with vintage-decay (older RECs less valuable)
- Climate-monitoring data with regulatory retention windows

---

## What Satyawan should build personally

Solo founder. Cannot build 50 dApps. Pick **1–2 reference dApps** that demonstrate the primitive at scale, then **let everyone else build the rest**.

### Pick 1 (mandatory) — Viral demonstration
**SFSV (Self-Future Vault)** — already in substrate as `evaporchain-sfsv`.
- Time-locked self-message / self-asset to future self
- Use cases: digital wills, addiction-recovery commitments, retirement self-savings, time capsules, "letters to your future self"
- Why it works: simple primitive, viral mechanism, makes energy-decay obvious to non-technical users
- Build cost: low (substrate crate already exists)
- Commercial: zero direct revenue, but **shows the primitive viscerally**. Every blog post / paper / talk demos this.

### Pick 2 (optional, commercial wedge) — Enterprise revenue path
Choose ONE of:

**A. GDPR-Erasure-as-a-Service** (recommended)
- Highest signal-to-effort commercial path
- B2B subscription model
- Every EU company is a potential customer
- Build time: 4–8 months solo
- Commercial: £50–200k contracts realistic Year 1
- Doctrine-clean (compliance is universally needed)
- Forces clear differentiation vs Ethereum (which can't natively expire data)

**B. SHLM (Skill Half-Life Market)**
- Existing crate (`evaporchain-shlm`), big commercial framing
- $50B B2B TAM
- More complex to build full product layer
- Commercial: per-certificate fees, enterprise licensing

**C. Decaying Event Tickets**
- Most viral, consumer-facing
- Partnership model (small concert promoters first)
- Build time: 6–12 months
- Commercial: per-ticket fees

**Recommendation: GDPR-Erasure-as-a-Service** as commercial wedge.

---

## What Satyawan should NOT build personally

Everything else from the 12 categories. Specifically:
- IoT marketplaces — Bosch / Siemens build these (use EvaporChain as backend)
- DeFi protocols — third-party teams build on EvaporChain once mainnet ships
- Identity systems — DID-focused teams adopt EvaporChain as a backend
- Healthcare apps — Roche / Novartis pilot programs build these
- Gaming items — game studios build these
- Governance systems — DAO teams build these
- Privacy / messaging — specialist teams build these
- Climate / ESG — sustainability-focused builders adopt EvaporChain

**Doctrine: build the primitive, let others build the products.** Linux didn't build Microsoft Office. Bitcoin didn't build Coinbase. Ethereum didn't build Uniswap. EvaporChain shouldn't build everything — it should be the chain that makes those things buildable.

---

## Satyawan's personal build queue (3-year horizon)

| Priority | Build | When | Why |
|---|---|---|---|
| 1 | EvaporChain mainnet | 2026–2027 | The chain itself |
| 2 | EvaporScript SDK + docs + tutorials | 2026 | So others can build |
| 3 | SFSV reference dApp | 2027 (early) | Viral demonstration |
| 4 | Paper 1 (mechanism) | 2027 | Establishes primitive |
| 5 | GDPR-Erasure-as-a-Service (or alternative commercial wedge) | 2027–2028 | First real revenue |
| 6 | Foundation grants programme | 2028 | Ecosystem development |
| 7 | Paper 2 (economics) | 2028 | Reinforces primitive |

**Stop there. That's all you build personally for the next 3 years.** Everything else comes from the ecosystem.

---

## Foundation grants programme (Phase 2+, ~2028)

Once Foundation has treasury, fund ecosystem grants. Pattern that worked for Solana: $5–50k per grant × 50–100 grants per year = 200+ dApps built by others over 3 years.

### Prioritise grant categories (in order)
1. **GDPR / regulatory tooling** — direct enterprise pipeline
2. **IoT data infrastructure** — Bosch / Siemens enterprise wedge
3. **RWA with lifecycle** — HSBC Orion / Franklin Templeton wedge
4. **Healthcare data lifecycle** — Roche / Novartis wedge
5. **DAO governance with decay primitives** — community-builder wedge
6. **Decaying NFT / time-bound digital assets** — consumer / viral wedge

### Deprioritise / reject categories
- "Another DEX" — would work on Ethereum, no EvaporChain advantage
- "Another lending protocol" — same
- "Another L2" — wrong layer
- "Another stablecoin" without decay primitive — no advantage
- "Another NFT marketplace" — generic

**Filter rule:** every grant application must answer "why EvaporChain specifically, not Ethereum?" If they can't, deny.

---

## Target enterprise integrations (from `INEVITABILITY_STRATEGY.md`)

Each maps to an application category above:

| Enterprise target | Maps to category | Realistic timeline | Realistic contract size |
|---|---|---|---|
| **HSBC Orion** (UK DIGIT digital-gilt mandate) | 11 — RWA tokenization | 2028 | £100–500k POC |
| **Bosch.IO Stuttgart** (Dirk Slama's IoT-blockchain research) | 6 — IoT telemetry | 2027–2028 | £30–150k research engagement |
| **Siemens Cre8Ventures** | 6 — IoT telemetry | 2028–2029 | £50–200k POC |
| **Roche / Novartis (Basel)** | 5 — Healthcare | 2029–2030 | £100–500k pilot |
| **Monzo / Starling / Revolut** (challenger banks) | 1 — Regulatory data | 2027–2028 | £20–100k POC |
| **EEX (European Energy Exchange)** | 12 — Climate / carbon credits | 2029–2030 | £100–500k POC |
| **LSEG (London Stock Exchange Group)** | 1 — MiFID-II audit trail | 2029–2030 | £100–500k POC |
| **JPMorgan Kinexys** | 11 — RWA, enterprise settlement | 2031–2032 | £500k–2M if landed |

---

## The "ONE killer integration" — per `INEVITABILITY_STRATEGY.md`

From the strategy doc:
> *"One Integration That Makes EvaporChain Load-Bearing... ONE massive use case where it becomes the thing that can't be ripped out."*

Realistic candidates for solo founder capacity in 2027–2028:
1. **GDPR-Erasure-as-a-Service** for 1–3 EU SaaS companies → procurement-realistic, doctrine-clean
2. **Bosch.IO IoT research engagement** → research-grade pilot, builds enterprise credibility
3. **HSBC Orion digital-gilt post-settlement data lifecycle** → via FCA Sandbox cohort entry

**Pick one. Land it. The protocol's reputation will compound from one undeniable integration.**

---

## Anti-patterns to refuse

Watch for these requests / suggestions that conflict with the EvaporChain doctrine:

| Anti-pattern | Why it conflicts |
|---|---|
| "Let's also build a memecoin launcher on EvaporChain" | Pure token-play signal; no decay primitive advantage; deters serious enterprise + academic audiences |
| "Build a Solana-killer DEX experience" | Wrong category — EvaporChain isn't a Solana competitor; it's a state-primitive |
| "Add a token launchpad / IDO platform" | Token-play signal; not differentiated; conflicts with "Don't make it a token play" doctrine |
| "Build everything yourself so you control the ecosystem" | Solo founder cannot; doctrine is "spec, not chain — let others build" |
| "License the primitive to competitors who pay" | Wrong model — primitive must stay open-source to win adoption (Linux pattern) |
| "Optimise for TVL number" | Wrong metric for state-primitive thesis; optimise for adoption-as-primitive instead |

---

## Cross-references

- `INEVITABILITY_STRATEGY.md` — the master thesis
- `INVENTION_STACK.md` — the locked primitives + Tier-2/Tier-3 substrates
- `TOKENOMICS.md` — economic model
- `MAINNET_READINESS.md` — operational readiness
- Substrate crates: `evaporchain-sfsv`, `evaporchain-shlm`, `evaporchain-memento`, `evaporchain-cap-decay-vm`, `evaporchain-total-evaporscript`, `evaporchain-app-templates-*`

## When to update this doc

- New application category discovered that passes the filter → add to the 12
- Enterprise target signs first contract → update the realistic-timeline column
- Foundation grants programme launches → publish the actual grant criteria here
- New substrate crate ships that enables a new category → link it inline
