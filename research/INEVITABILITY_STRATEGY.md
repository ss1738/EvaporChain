# EvaporChain: Path to Inevitability

**Author:** Satyawan Singh
**Date:** 2026-04-21
**Status:** Strategic foundation document — guides all decisions from May 2026 onward

---

## Core Thesis

Data without a half-life is a bug, not a feature. Every system that stores state forever is accumulating technical debt that compounds exponentially. EvaporChain is the first blockchain where data knows when to die. The goal is not to build a successful blockchain — it is to make energy-decay state a universal primitive that every future system adopts.

---

## The Problem That Only Gets Worse

Ethereum's state: 300GB+ and growing. Solana: terabytes of historical data. Every blockchain, every database, every system that stores data forever is a ticking bomb. No one is solving this at the protocol level. They bolt on pruning, archival nodes, state expiry proposals — all patches on a fundamental design flaw.

EvaporChain doesn't patch the problem. It eliminates the category.

---

## What Inevitability Requires

### 1. Become a Standard, Not a Product

TCP/IP won because nobody owns it. Linux won because everyone builds on it. Ethereum won its category because it became the layer others assume exists.

EvaporChain must follow the same path: open protocol, open source, reference implementation. The moat is being first, being right, and being the canonical implementation.

### 2. Make Energy-Decay a Primitive

Don't position EvaporChain as "a blockchain with decay." Make **energy-decay** the concept that other systems adopt — like how "proof of stake" went from one project's idea to an industry standard.

The spec must be so clear that even if someone forks the code, they're still using the EvaporChain primitive. The concept must carry the origin.

```rust
// This is what inevitability looks like:
let object = EvaporObject::new(data)
    .energy(1000)
    .half_life(blocks(30_days));

// Every developer should think this is obvious.
// "Why WOULDN'T data have a half-life?"
```

When developers start asking "why doesn't Ethereum have this?" — the battle is won.

### 3. One Integration That Makes EvaporChain Load-Bearing

Linux became inevitable the day IBM bet on it. Ethereum became inevitable the day stablecoins chose it. EvaporChain needs ONE massive use case where it becomes the thing that can't be ripped out.

Target domains where "data must die" is a legal or physical requirement — not a preference:
- **IoT telemetry** — millions of sensor readings per second, worthless after processing
- **GDPR-compliant data** — right to erasure baked into the protocol, not bolted on
- **Satellite/space data** — orbital telemetry with natural temporal relevance
- **Financial settlement** — derivatives, options, and contracts that expire by definition

### 4. The Developer Standard

Inevitability is decided by developers, not executives. The developer experience must make energy-decay feel obvious — like garbage collection felt obvious after manual memory management.

Every SDK, every tutorial, every example should make developers think: "Why would I ever store data without an expiry?"

### 5. Never Sell the Company

The builders who changed the world — Vitalik, Linus, Satoshi — never sold. That's why we know their names. EvaporChain is not an exit. It is infrastructure.

---

## What NOT to Do

- **Don't make it a token play.** The moment EvaporChain looks like a money grab, engineers walk away. Credibility comes from not cashing out.
- **Don't chase every use case.** One vertical, proven undeniably, is worth more than twenty demos.
- **Don't let anyone abstract you away.** If Chainlink or AWS puts a wrapper over the chain and becomes the interface, they own the relationship. EvaporChain becomes replaceable plumbing.
- **Don't close the source. Ever.** Inevitability requires that anyone can build, fork, and extend.
- **Don't sell the company.** Not for 50M. Not for 500M. Infrastructure outlives acquisitions.

---

## The Research Moat

Three papers, not one:

### Paper 1: The Mechanism
Energy-decay state management — formal definition, correctness proofs, the EvaporChain protocol specification. Establishes the primitive.

### Paper 2: The Economics (the one that changes minds)
Mathematical proof that infinite-state blockchains are economically unsustainable. State growth models, storage cost projections, validator economics over 10-50 year horizons. This paper doesn't say "use EvaporChain." It proves "every chain without decay will eventually fail." EvaporChain becomes the only existing answer.

### Paper 3: The Benchmark
EvaporChain vs Ethereum/Solana/Sui on state growth over simulated 5-year periods under real-world workloads. Empirical evidence that decay works and that the tradeoffs are manageable.

---

## The Enterprise Path

Target companies where evaporating state solves a genuine technical need:

| Company | Why |
|---------|-----|
| JPMorgan | Built Onyx/Quorum, runs blockchain internally, knows state grows forever |
| Maersk | Killed TradeLens partly due to state bloat — supply chain data is only relevant in transit |
| Bosch/Siemens | 14B+ IoT sensors, sensor data worthless after hours |
| Roche/Philips | Clinical trial data with mandatory expiry, patient monitoring with GDPR obligations |
| DTCC | Clears $2.5 quadrillion/year, settlement records have natural expiry |
| CME Group | Derivatives and options expire by definition |
| SpaceX/NASA | Satellite telemetry — millions of data points per orbit, worthless after processing |

These companies don't become "clients" on day one. The path: audit completed, paper published, one mid-tier pilot proven, then the giants evaluate.

---

## Timeline to Inevitability

| Year | Milestone |
|------|-----------|
| **2026** | Security audit + Paper 1 + expanded devnet (10-20 validators). EvaporChain becomes credible. |
| **2027** | One production use case (IoT or regulated data) + Paper 2. EvaporChain becomes real. |
| **2028** | Other chains start copying energy-decay. EvaporChain is recognised as the original. |
| **2029** | Standards body (IEEE/W3C/IETF) formalises temporal state primitives. The spec carries EvaporChain's DNA. |
| **2030** | Every new blockchain includes decay by default. EvaporChain is inevitable. |

---

## Budget Allocation (2026, £100K)

| Item | Allocation | Purpose |
|------|-----------|---------|
| External security audit | £30-50K | Non-negotiable for credibility. Trail of Bits, OtterSec, or equivalent. |
| Research papers | £0-5K | Mostly time investment. Conference fees, potential co-author stipends. |
| Expanded devnet | £10-15K | Cloud nodes, geographic distribution, stress testing at scale. |
| Killer use case pilot | £10-20K | One domain, one integration, one undeniable proof. |
| Reserve | £20-30K | Runway buffer. Don't spend everything. |

---

## The Measure of Success

EvaporChain succeeds not when it has the most users, but when the idea of energy-decay state becomes so obvious that people forget it was ever a new idea. Like garbage collection. Like proof of stake. Like the half-life of a radioactive isotope — it was always there, someone just had to apply it to data.

That someone is Satyawan Singh, from Leicester.

---

*"Make the world realise that data without a half-life is a bug, not a feature. Once that idea is accepted, EvaporChain is the only answer that already exists."*
