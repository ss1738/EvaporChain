# EvaporChain Pitch Deck Content

15-slide seed round pitch deck. Text content for each slide.

---

## Slide 1: Title

**EvaporChain: The Blockchain That Gets Lighter Over Time**

Thermodynamic state decay + recursive proof folding.

Satyawan Singh — Independent Researcher · University of Leicester

---

## Slide 2: The Problem

State growth is the existential threat to blockchain decentralization.

- **Ethereum**: 300GB+ state, 18TB archive nodes, growing approximately 50GB/year
- **Solana**: 256GB RAM required for validators
- Every chain gets heavier every second. Forever. Until it can't.

The result: fewer validators, more centralization -- the opposite of the promise. The cost of running a full node increases monotonically. Hardware requirements compound. The decentralization that blockchains were built to provide erodes with every block.

---

## Slide 3: The Failed Solutions

- **State rent (Solana)**: UX disaster. Developers and users lose state unexpectedly. Adoption friction increases, not decreases.
- **State expiry (Ethereum)**: On the roadmap since 2018 (EIP-4444, EIP-7745). Still not shipped. Backwards compatibility makes implementation intractable.
- **Stateless clients**: Shifts the burden to provers. Does not reduce the total state that must exist somewhere.
- **History pruning**: Addresses history, not state. State is the part that grows and the part that validators must hold in memory.

These are patches applied to an architecture that assumed permanent state. The problem is architectural.

---

## Slide 4: The Insight

State should not be permanent. It should have an energy budget.

Objects that nobody uses should gracefully disappear -- automatically, without governance votes, without manual intervention. Like physical matter: unused things decay. This is thermodynamics, not punishment.

This is not a new idea. It is how every physical system works. Blockchains forgot to include it. EvaporChain corrects that omission.

---

## Slide 5: How EvaporChain Works

- Every state object has **energy** that depletes per epoch (configurable half-life)
- Energy reaches zero: **5-epoch grace period (~10s at 2-second block time)**, then **evaporation** (removal from active state)
- Evaporated objects leave a **cryptographic nullifier** (MMR (Merkle Mountain Range) nullifier accumulator membership proof) -- the object existed, and that fact is permanently provable
- **Revival**: one-click transaction with micro-payment to restore energy. Nothing is permanently lost; it is temporarily inactive.
- Result: the state trie **shrinks over time**. Active state reflects active usage. The chain reaches an equilibrium size determined by real demand, not accumulated history.

---

## Slide 6: The Second Innovation -- Constant-Size Chain Proof

Every block folds into a recursive proof using Nova IVC (Incremental Verifiable Computation).

- 1,000 blocks = same proof as 1 block
- 1,000,000 blocks = still the same proof
- A 10-year-old chain verifies in 15ms -- same as a 10-minute-old chain

New nodes sync by checking ONE proof. No replaying history. No downloading terabytes. A phone can verify the entire chain.

---

## Slide 7: Benchmark Results

**Nova IVC proving prototype (`prototypes/fold-a-block`): 1,000 blocks folded in 6.2 seconds, amortized 6.2ms per block.** Live testnet throughput is a separate metric — see project status.

| Metric | Value |
|---|---|
| Engine | Bn256/Grumpkin + HyperKZG |
| Batching | 5 blocks per fold step |
| Amortized fold time | 6.2ms per block |
| Compressed verification | 15.0ms |
| Proof size | 11.3KB (constant regardless of chain length) |
| Objects evaporated | 64/64 (thermodynamic decay verified in-circuit) |
| Verdict | PASS |

Open-source prototype: [github.com/ss1738/EvaporChain](https://github.com/ss1738/EvaporChain)

---

## Slide 8: Technical Architecture

| Layer | Technology | Rationale |
|---|---|---|
| Consensus | Tendermint BFT | Stake-weighted 2/3 quorum + BLS12-381 aggregate signatures + checked-arithmetic execution (`crates/evaporchain-consensus/src/tendermint.rs`) |
| Execution | EvaporScript VM | 44 gas-metered opcodes including temporal primitives (`EnergyOf`, `ComputeDecay`, `RequireEpochRange`); non-Turing-complete by design |
| ZK proving | Nova IVC folding | 6.2ms/block today; HyperNova/CCS + Binius binary-field backend on roadmap |
| Active state | Verkle trie | Smaller proofs than Merkle Patricia, bandwidth-efficient sync |
| Evaporated state | MMR (Merkle Mountain Range) nullifier accumulator | Append-only membership proofs for evaporated objects |
| Signatures | ML-DSA (Dilithium) | Post-quantum security from genesis, NIST standardized 2024 |

All components chosen for production readiness and cutting-edge performance. No novel cryptography -- novel combination of proven primitives.

---

## Slide 9: Why EvaporScript?

- **44 gas-metered opcodes**, non-Turing-complete by design — eliminates unbounded execution as an attack surface. Reentrancy guard enforced at the VM level.
- **Temporal primitives built into the VM**: `EpochNow`, `BlockNum`, `EnergyOf`, `RequireEpochRange`, `ComputeDecay`. State expiry isn't a contract pattern bolted on top — it's a first-class VM operation.
- **8 template contracts shipped** (`crates/evaporchain-contracts/src/lib.rs ContractTemplate`). Common decay patterns are off-the-shelf and audited.
- **Declarative rule engine** for triggers, conditions, and actions on contracts. Each contract instance carries its own energy and half-life — contracts themselves evaporate when unused.

---

## Slide 10: Market Timing

- **HyperNova** (2023): variable-time folding for customizable constraint systems
- **Binius** (2024): binary tower fields for 10-100x faster witness generation
- **Mysticeti** (2024): DAG-BFT consensus with sub-second finality
- **ML-DSA standardized** (2024): NIST post-quantum signature standard

These cryptographic primitives did not exist 3 years ago. 3 years from now, someone else will combine them. The window is 18 months wide. EvaporChain is building in the middle of it.

First-mover advantage in thermodynamic blockchain design.

---

## Slide 11: Competitive Landscape

**vs Ethereum**: 8 years of state expiry proposals, still not shipped. Backwards compatibility and governance make it intractable. EvaporChain ships state decay at genesis with no backwards-compatible baggage.

**vs Sui/Aptos**: Same Move language, same linear type safety. No state decay, no folded chain proofs. EvaporChain extends Move with temporal types and adds constant-size chain verification.

**vs Mina**: Constant-size proofs, but limited programmability. No general smart contracts, no DeFi composability. EvaporChain has full EvaporScript execution and constant-size proofs.

**vs Celestia/EigenDA**: They solve data availability. EvaporChain solves state growth. Orthogonal problems -- potentially complementary. EvaporChain could use Celestia for DA while managing its own state lifecycle.

---

## Slide 12: Business Model

- **EVAP token**: gas fees + validator staking + refresh fees for state revival
- **Refresh fees** create perpetual demand proportional to active state size. The more state people want to keep alive, the more demand for EVAP. This is demand driven by real usage, not speculation.
- **0.5-1% inflation floor** ensures validator security budget regardless of fee volume
- **Fee burn mechanism** when demand exceeds issuance creates deflationary pressure during high-activity periods
- Token economics align incentives: valuable state stays alive (users pay to refresh), abandoned state evaporates (reducing chain burden). The protocol charges for what it provides -- persistent state -- and stops charging when that service is no longer requested.

---

## Slide 13: Roadmap

| Milestone | Target | Status |
|---|---|---|
| Research phases 1-3 (1.2MB research corpus, 188KB whitepaper, 70 citations) | Q2 2025 | Complete |
| Fold-a-Block prototype -- 6.2ms/block, PASS | Q2 2025 | Complete |
| Project scaffold -- 9-crate Cargo workspace | Q3 2025 | Complete |
| EvaporScript VM (44 temporal opcodes) + 8 template contracts | Q4 2025 | Complete |
| Public testnet with Tendermint BFT consensus | Q3 2026 | Planned (after audit completion) |
| Security audits + mainnet genesis | Q4 2026 | Planned |

---

## Slide 14: Team and Ask

Founded by Satyawan Singh — ML Engineer, University of Leicester student — building at the intersection of cryptography, distributed systems, and applied thermodynamics.

**First 5 hires:**
1. ZK Cryptographer -- Nova/HyperNova implementation, circuit optimization
2. Smart Contract Engineer -- EvaporScript VM extensions, formal verification, developer tooling
3. Consensus Engineer -- Tendermint BFT hardening, networking
4. Systems/Infra Lead -- node architecture, DevOps, benchmarking infrastructure
5. DevRel -- documentation, developer ecosystem, grants program

**Raising**: $8-12M seed round at $40-60M post-money valuation

**Use of funds:**
- 60% engineering team
- 15% infrastructure
- 10% legal/compliance
- 10% treasury reserve
- 5% community/grants

24-month runway at current burn assumptions.

---

## Slide 15: The Vision

In 10 years, every blockchain adopts thermodynamic state models. "State growth" becomes a historical artifact, like "the 640KB barrier."

Full nodes run on consumer hardware again. True decentralization is restored -- not as an aspiration, but as a material property of the system.

The blockchain trilemma gets a fourth dimension: **sustainability**. Scalability, security, decentralization, and now temporal sustainability -- the ability of a chain to run indefinitely without unbounded resource growth.

**EvaporChain: the chain that made blockchains sustainable.**

[github.com/ss1738/EvaporChain](https://github.com/ss1738/EvaporChain)
