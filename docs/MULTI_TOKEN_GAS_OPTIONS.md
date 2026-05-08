# Multi-Token Gas — Research + Decision Document

**Status:** research / decision artifact (NOT a build commitment).
**Last updated:** 2026-05-08.
**Author:** parent session, EvaporChain afternoon arc.

This document exists to convert the question *"should EvaporChain accept ETH/USDC/other tokens for gas, and if yes, how"* from informal conversation into a structured comparison the team can act on. Companion to the 1-month roadmap in `SESSION_PROGRESS.md`. Reference, not commitment.

---

## 0. TL;DR

| Approach | Effort | UX gain | Strength gain | Risk | Recommended for V1? |
|---|---|---|---|---|---|
| **A. Status quo** (EVP-only gas) | 0 | 0 | baseline | none | **YES** |
| **B. Wallet paymaster** (app-layer) | 1 week | high | medium | low | post-mainnet (V1.5) |
| **C. Protocol-level multi-token gas** | 2–3 weeks | high | low net (loses native-token demand anchor) | high (consensus complexity) | **NO**, ever |

**Recommendation: ship V1 mainnet with EVP-only gas (Option A). Plan Option B for V1.5 (~3 months post-mainnet). Do not build Option C.**

---

## 1. Why this question exists

EvaporChain's gas constants live at `crates/evaporchain-execution/src/lib.rs`:

```rust
pub const GAS_TRANSFER: u64 = 21_000;          // EVP units
pub const GAS_CREATE_OBJECT_BASE: u64 = 50_000; // EVP units
pub const GAS_REFUND: u64 = 5_000;              // EVP units
```

The deduction path is:

```rust
let required = req.amount.saturating_add(GAS_TRANSFER);
if acct.balance < required { reject }
```

Where `acct.balance` is denominated in EVP. The chain has no concept of any other token at the gas layer.

**The question:** users come from Ethereum / Solana / Cosmos with existing wallet balances of ETH / USDC / SOL. Buying EVP just to interact with EvaporChain is a friction step. Should the chain (or its surrounding infra) let them pay gas in those existing tokens?

---

## 2. What other chains do — comparative research

| Chain | Gas-token model | Mechanism | When introduced |
|---|---|---|---|
| **Bitcoin** | BTC only | hardcoded at protocol | 2009 |
| **Ethereum L1** | ETH only at protocol | ERC-4337 paymasters at app layer (2023+) | 2015 / 2023 |
| **Solana** | SOL only at protocol | "fee delegation" via wallet wrappers | 2020 |
| **Avalanche C-Chain** | AVAX only at protocol | ERC-4337 paymasters available | 2020 |
| **Cosmos hubs** | Native chain token | Bridged-token gas via specific governance deals (rare) | 2019+ |
| **NEAR** | NEAR only at protocol | "meta transactions" — relayer pays gas, user reimburses in any token off-chain | 2020 |
| **StarkNet** | STRK or ETH (since v0.13) | Native account abstraction + paymaster contracts | 2024 |
| **Polygon zkEVM** | MATIC at protocol | ERC-4337 paymasters | 2022+ |
| **zkSync Era** | ETH at protocol | Native paymasters in account abstraction model | 2023 |
| **Arbitrum** | ETH only at protocol | ERC-4337 paymasters available | 2021 |
| **Stripe Tempo** (stablecoin chain) | USDC for gas natively | Centralized chain design, USDC-first | 2024 |

### Pattern identified

**Every successful L1 keeps gas in its native token at the protocol level.** Multi-token gas — when it exists — is implemented via **paymaster patterns at the application layer** (ERC-4337 standard).

The only exceptions are stablecoin-first L2s that explicitly chose USDC-as-gas as their differentiator (Stripe's Tempo, some L2 experiments). These are not L1s in EvaporChain's sense; they're application-specific chains.

**Why this pattern is dominant:**

1. **Native token demand anchor.** ETH gas → demand for ETH → price floor. Same for SOL, AVAX, NEAR. Removing this is a known price-floor weakness.

2. **Consensus simplicity.** Validators don't need to agree on token prices each block. Native-token-only gas means deterministic, oracle-free fee calculation.

3. **Reorg safety.** When a tx pays gas in token X, the chain holds X. On reorg, that X has to be returned somewhere. Multi-token gas multiplies this cleanup surface.

4. **Audit predictability.** External auditors charge by complexity. Native-token gas has been audited thousands of times across all chains; multi-token gas has limited audit precedent.

### What ERC-4337 (Ethereum's solution) actually does

ERC-4337 doesn't change Ethereum's protocol. It standardizes a smart-contract pattern:

```
User → signs UserOperation (intent) with paymaster: 0x...
Bundler → groups UserOps into a single tx
Paymaster contract → covers ETH gas at the entry point
User → reimburses paymaster in any agreed-upon token via separate flow
Entry point contract → validates, executes, settles
```

The chain still sees normal ETH gas. The user perceives "I paid in USDC." All complexity is at the smart-contract layer.

This is the pattern Option B (below) would adapt for EvaporChain.

### What protocol-level multi-token gas actually requires

Looking at the few chains/L2s that tried protocol-level multi-token gas (NEAR's early experiments, some governance-token chains):

1. **Per-block oracle price.** Every validator queries an oracle, agrees within tolerance. Disagreement → block rejection.
2. **Custody at consensus layer.** Token X received by chain → who holds X? Producer? Pool?
3. **Slippage protection.** User's "1 USDC = 0.0001 EVP gas" agreement → if EVP price spikes, who eats the loss?
4. **State conservation.** EvaporChain's §1.2 invariant tracks compartment sums. Multi-token compartments mean tracking N tokens through reorgs.

**These problems are solvable but expensive.** None of the major L1s judged them worth solving at the protocol level.

---

## 3. Strategic frame for EvaporChain specifically

EvaporChain's differentiators (per `INVENTION_STACK.md`):

1. **Thermodynamic state decay** (Active→Grace→Ghost lifecycle, demurrage, HBCT)
2. **Lambda-Fold Nova IVC** sublinear light-client verification
3. **Energy-Verkle Trie** state commitment
4. **MMR nullifier accumulator** for evaporated objects
5. **Causal-CHSH cartel detector**
6. **Singh Pool** decay-aware AMM
7. **Post-quantum signatures** (ML-DSA Dilithium3)

**Multi-token gas is NOT in this list.** Adding it would shift the narrative from *"the chain that decays state by physics"* to *"another chain with flexible gas"* — and the second framing has zero defensible moat. Every chain will have flexible gas via paymasters within 18 months; the decay-thesis differentiator is the durable claim.

**The question becomes:** does multi-token gas help land the V1 mainnet ship, or does it dilute the narrative + consume sprint runway?

Honest answer: **dilute + consume**. Defer.

---

## 4. Three options — detailed breakdown

### Option A — Status quo (EVP-only gas)

**What:** Ship V1 mainnet with native-token-only gas. Standard model, matches ETH/SOL/AVAX/etc.

**Effort:** 0. Already shipped.

**UX cost:** Users coming from other chains must acquire EVP first (via bridge or DEX or faucet during testnet phase).

**Strategic gain:** Native token demand anchor preserved. Consensus simplicity preserved. Audit scope preserved.

**When it's wrong:** never, for V1. Maybe constraining for niche use-cases post-V2 (e.g., enterprise users who can't custody non-stable tokens), but those use-cases have other paths (paymaster service, custodial bridge).

### Option B — Wallet paymaster pattern

**What:** A paymaster service (single account or a network of competing services) covers EVP gas on the user's behalf. User pays the paymaster in their preferred token via a separate flow.

**The chain protocol doesn't change.** The chain still sees standard EVP-gas txs. The paymaster is a normal account that happens to be funded with EVP and accepts payment in other tokens off-protocol.

**Effort:** ~1 week (wallet UX + 1 paymaster service + small protocol additions).

**Steps (numbered, executable):**

1. **Day 1 — Tx envelope changes** (4 hours):
   - Add optional field to `Transaction` types: `paymaster: Option<AccountAddress>`. Backwards-compatible (`#[serde(default)]`).
   - When `paymaster` is set, gas is deducted from `paymaster.balance` instead of `from.balance`.
   - When `paymaster` is set, the tx must include a paymaster signature alongside the user's signature (verified at tx-validity time).

2. **Day 2 — Paymaster service binary** (8 hours):
   - New crate `evaporchain-paymaster`. Long-running service. Listens for "paymaster requests" via HTTP or gRPC.
   - Holds an EVP-funded account. Has a price table (or oracle hookup) for accepted tokens (initially USDC, ETH).
   - On request: validates user's intent, computes EVP gas needed, computes equivalent in user's token, signs the tx as paymaster, returns signed tx to client.
   - Out-of-protocol: paymaster collects user's payment via standard transfer (e.g., USDC tx on Ethereum, or in-EvaporChain token transfer when EvaporChain has wrapped USDC).

3. **Day 3 — Wallet UX integration** (8 hours):
   - Wallet (browser extension + mobile) gets a "Pay with..." dropdown showing user's available balances across tokens.
   - On selection: wallet contacts the paymaster service, gets a quote, presents to user.
   - On confirm: wallet sends paymaster's payment + the paymaster-signed tx to the chain in parallel.

4. **Day 4 — Tests + smoke** (4 hours):
   - Unit tests for the new tx envelope fields.
   - Integration test: user pays in token X, paymaster covers EVP gas, chain processes normally.
   - Live-cluster smoke against the running cluster.

5. **Day 5 — Operations + docs** (4 hours):
   - Operator runbook for running a paymaster service.
   - Documentation for third parties to run competing paymasters.
   - Pricing-policy doc (how the paymaster sets exchange rates, slippage handling).

**Total: ~28 hours = 1 working week.**

**UX gain:** Users see "Pay 0.50 USDC" or "Pay 0.0001 ETH" in their wallet. Friction-equivalent to Coinbase or any custodial product.

**Risk:**
- Paymaster is initially centralized (single trusted party). Reputational risk if it goes down.
- Paymaster has counterparty risk for the user (paymaster could vanish with user's payment before signing).
- Mitigation: launch with a foundation-run paymaster, document how to run competing ones, transition to multi-paymaster network over months.

**Strategic gain:** Real UX win. Users from other chains can interact without buying EVP first.

**Strategic cost:** ~10% native-token demand softening (some users keep their EVP at zero, paymaster holds the float). Manageable; not catastrophic.

### Option C — Protocol-level multi-token gas

**What:** The chain natively accepts ETH / USDC / etc. as gas. Validators query oracles each block, validate that the tx's gas-token amount × oracle-price ≥ required EVP gas equivalent.

**Effort:** 2-3 weeks of senior protocol-engineering time. Multiple consensus-affecting changes.

**Steps (numbered, executable, but listed for completeness — DO NOT DO THIS):**

1. **Week 1 — Oracle integration** (5 days):
   - Wire a per-block oracle bridge into consensus. Validators query a price source (Pyth, Chainlink, or custom BFT-aggregated) and include the price in their proposal.
   - Block-validity rule: 2/3 of validator-attested prices must agree within tolerance (e.g., 0.5%) for the block to be valid.
   - Liveness risk: oracle outage → block rejection → chain halts.

2. **Week 1.5 — Tx envelope** (2 days):
   - `Transaction::*` variants get a `gas_token: TokenId` field and `gas_amount_in_token: u64`.
   - `gas_token = 0` reserved for EVP (backwards-compat).
   - Validity check: `gas_amount_in_token × oracle_price[gas_token] ≥ gas_required_in_evp`.

3. **Week 2 — Custody + execution** (5 days):
   - When a tx pays in token X, the chain holds X. Decision: where does it go?
     - Option: producer's account in token X (mirrors existing producer reward path)
     - Option: refresh pool in a per-token compartment (preserves §1.2 conservation per-token)
     - Option: auto-swap to EVP via an internal Singh Pool (introduces routing dependency; circular)
   - Each option is its own 2-3 day implementation.

4. **Week 2.5 — Conservation invariant** (3 days):
   - §1.2 audit currently tracks `accounts + stake + refresh_pool + slashed`. With multi-token, this becomes per-token sums. The audit fires N times per block (N = number of supported tokens).
   - Each per-token sum must be monotone-decreasing under λ. Cross-token swaps break this (energy moves between compartments).
   - This is the hardest piece.

5. **Week 3 — Testing + audit prep** (5 days):
   - All existing tx tests gain multi-token variants.
   - New consensus-divergence tests (price disagreement, oracle outage, slippage).
   - External audit scope grows ~30%.

**Total: ~20 working days = 4 weeks. Beyond the 1-month sprint window.**

**Risks (these are why no major L1 has shipped this):**

- **Liveness risk.** Oracle outage = block rejection = chain halt.
- **Consensus risk.** Price disagreement among validators = block rejection. Tolerance bands are oracle-of-oracles for chain liveness.
- **Reorg complexity.** Token X paid for gas in block N. Block N reorgs out. Token X must be refunded. Multi-token reorg = cascading custody updates.
- **MEV.** Multi-token gas means MEV bots can manipulate token prices to make their txs cheaper. New attack surface.
- **Audit scope.** Adds ~30% to external audit cost + duration.

**UX gain:** marginal vs Option B. Same user-facing experience.

**Strategic gain:** **negative.** Loses native-token demand floor. Adds attack surface. Dilutes narrative.

**Strategic cost:** All of the above PLUS sprint-runway consumption (4 weeks of 20 weeks = 20% of remaining mainnet time on a feature with negative net strategic value).

---

## 5. Decision criteria

Choose the option that fits the answer to these questions:

| Question | Answer favouring A (status quo) | Answer favouring B (paymaster) | Answer favouring C (protocol-level) |
|---|---|---|---|
| Is multi-token UX a *V1 mainnet differentiator* or a *post-mainnet polish*? | post-mainnet | post-mainnet | V1 |
| Are we OK with a paymaster being a centralised party initially? | n/a | yes | n/a |
| Are we willing to spend 4 weeks of sprint runway on this? | no | no (1 week) | yes |
| Do we believe the chain's narrative is "decay" or "flexible gas"? | decay | decay | flexible gas |
| Will external auditors charge meaningfully more for protocol-level multi-token? | n/a | no | yes (~30%) |
| Are we willing to risk consensus liveness on oracle uptime? | no | no | yes |

If you answer "decay" + "decay-narrative-V1" + "audit-cost-matters" + "liveness-matters", you arrive at Option A for V1, Option B for V1.5.

If you answer "flexible gas" + "we want to be a UX-first chain", you arrive at Option C — and you should re-read INVENTION_STACK.md to make sure that's the chain you're building.

---

## 6. The recommended path

```
NOW (next 5 months → mainnet Oct 2026):  Option A (EVP-only gas)
                                          Focus on what's already differentiated.
                                          Don't dilute the decay narrative.

V1.5 (3 months post-mainnet, ~Jan 2027): Option B (wallet paymaster)
                                          1 week to build, real UX wedge.
                                          Single foundation-run paymaster initially.

V2+ (mid-2027):                           Multi-paymaster competitive network
                                          Documented path for third parties to run paymasters.
                                          Standard ERC-4337-style abstraction at the wallet layer.

NEVER:                                    Option C (protocol-level multi-token gas)
                                          The complexity-to-strength ratio is wrong.
                                          Every chain that's tried has had to back out.
```

---

## 7. What this document does NOT say

- It does NOT say multi-token UX is unimportant. It is. Just deliver it via Option B at the right time.
- It does NOT say the chain should be hostile to ETH/USDC users. The bridge (`evaporchain-cone-bridge`) plus future paymaster makes them first-class.
- It does NOT close off Option C forever. If 5 years from now the consensus complexity is solved by a primitive we don't see today, the question can be reopened.

---

## 8. References

### EvaporChain code

- `crates/evaporchain-execution/src/lib.rs` — gas constants
- `crates/evaporchain-execution/src/parallel.rs` — gas deduction in execute_block
- `crates/evaporchain-cone-bridge/` — cross-chain bridge substrate (replay-immune via decay-cone intersection)
- `research/INVENTION_STACK.md` — canonical doctrine (decay thesis)

### External research

- **EIP-4337**: https://eips.ethereum.org/EIPS/eip-4337 — Ethereum's account abstraction standard. The canonical paymaster pattern reference.
- **StarkNet account abstraction docs**: https://docs.starknet.io/architecture-and-concepts/accounts/ — native account abstraction including multi-token gas via paymasters.
- **NEAR meta transactions**: https://docs.near.org/concepts/abstraction/meta-transactions — NEAR's relayer-pays-gas model, the closest L1 analogue to Option B.
- **Cosmos IBC fee abstraction**: ICS-29 — fee payment in arbitrary IBC tokens between chains. Closest cosmos-side analogue.
- **Stripe Tempo announcement** (2024): stablecoin-first L2 design choosing USDC-as-gas natively. Niche, app-specific.

### Adjacent EvaporChain decisions

- `SESSION_PROGRESS.md` 2026-05-08 (afternoon) — captures the conversation that produced this doc.
- `docs/runbooks/cluster-deploy.md` — deploy procedure that any of these options would have to ride.
- `TOKENOMICS.md` — economic model context. Multi-token gas decisions interact with §2.1 (recipient policy) and §2.5 (staking-APY controller).

---

## 9. Future-decision triggers

Re-read this document if any of these become true:

- ⚪ Mainnet has launched and is stable for ≥3 months. (Reopen Option B.)
- ⚪ A wallet partner (e.g., MetaMask, Phantom) explicitly requests multi-token gas integration. (Reopen Option B.)
- ⚪ A novel cryptoeconomic primitive solves the per-block oracle agreement problem at zero liveness cost. (Reopen Option C.)
- ⚪ The chain's narrative pivots away from decay-thesis to UX-flexibility. (Reopen Option C — but check INVENTION_STACK first.)

Until then, the recommended path stands.
