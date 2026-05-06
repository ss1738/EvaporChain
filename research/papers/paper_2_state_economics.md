# State Economics: Why Infinite-State Blockchains Are Economically Unsustainable

**Working title** (alternates: *"The Storage Burden: A Formal Argument Against Permanent State"*; *"Thermodynamic Limits of Distributed Ledgers"*)

**Author:** Satyawan Singh
**Affiliation:** Independent Researcher, Leicester, United Kingdom
**Status:** Draft v0.1 (2026-04-27). Companion to *EvaporChain: A Thermodynamic State-Decay Blockchain* (whitepaper v1.0, March 2026).
**Target venues:** AFT (Advances in Financial Technologies), Financial Cryptography, or arXiv preprint as a standalone economics-of-systems paper.

---

## Abstract

Every general-purpose blockchain in production today operates on the implicit assumption that on-chain state, once written, persists indefinitely. We argue this assumption is not a neutral design choice but a structural debt — one whose cost compounds at a rate that no economic mechanism currently deployed can offset.

We construct a formal model of validator economics under unbounded state growth. The model has three inputs: a state-growth function $G(t)$, a per-byte cost function $C(t)$ for validator storage and bandwidth, and a fee-revenue function $R(t)$ for transactions whose marginal cost includes perpetual storage of their effects. We prove that for any blockchain whose state grows monotonically and whose fee revenue is paid only once at write time, there exists a finite time $T^*$ beyond which the present value of validating cost exceeds the present value of fee revenue, regardless of token price assumptions. The chain becomes economically dependent on inflation, treasury subsidy, or external rents to retain validators.

We catalogue the major existing mitigations — Ethereum's history pruning (EIP-4444), state expiry proposals (EIP-7745), statelessness via Verkle commitments, Solana's rent-exempt minimum balances, and Sui's object model — and show why each addresses a symptom rather than the underlying asymmetry between one-time fees and perpetual cost.

We then formalise *thermodynamic state decay* as an alternative protocol primitive: every on-chain object has an explicit half-life and an energy budget that depletes over time. Objects whose energy reaches zero evaporate, leaving only a cryptographic ghost record. We show that under this model, state size converges to a stationary distribution determined by sustained refresh demand, not by cumulative write history, and that validator economics become long-run sustainable.

Our contribution is not the design of a specific protocol — that work appears in the EvaporChain whitepaper — but the formal claim that **the design space of long-run-viable blockchains does not include those without a state-decay mechanism**. The paper is intended to make this claim explicit and falsifiable.

---

## 1. Introduction

### 1.1 The unstated assumption

Every major blockchain in production — Ethereum, Solana, Avalanche, Cosmos zones, Sui, Aptos, and the entire long tail of forks and L2s — implements an unstated invariant: state, once written, is preserved indefinitely. A user pays a one-time gas fee at write time. The protocol commits to remembering the resulting state forever. Every full node replicates this state forever. Every new validator that joins the network in year $t+1$ must download and serve the entire state accumulated since year 0.

This invariant is not stated in any whitepaper. It is the residue of an early design choice (Bitcoin's UTXO model, Ethereum's account state) that was treated as obvious by every project that followed. The few attempts to revisit it — EIP-4444 (proposed 2018, partially shipped 2024 [^1]), Ethereum statelessness research, Solana's rent (effectively abandoned), Filecoin's storage market (a different problem) — have either remained roadmap items or bolted on partial mitigations after the fact.

This paper argues that the invariant is structurally untenable. Not difficult — *untenable*. Not because of engineering inconvenience, but because of an irreducible asymmetry between one-time fee revenue and perpetual cost.

### 1.2 The asymmetry

Consider a single transaction that writes 100 bytes to state. The user pays a fee at block height $h$. From height $h$ onward, every full node in the network stores those 100 bytes, every state-sync transfer copies them, and every state-root computation hashes them. The cost of storing those bytes accumulates over the chain's lifetime. The fee was paid once.

For a chain with $N$ validators, growth rate $g$ bytes per block, block time $\tau$, and per-byte annual storage cost $c$ amortised over hardware refresh:

$$
\text{cumulative storage cost in year } t = N \cdot g \cdot \tau \cdot c \cdot t \cdot (\text{epochs per year})
$$

This is linear in $t$ and quadratic in $N$ if we account for the bandwidth of inter-validator state-sync. The fee revenue stream is, at best, also linear in $t$ — and only if transaction throughput remains at capacity. There is no mechanism in any production chain by which a transaction's writer is charged a cost proportional to how long their state persists.

In any productive industry, this is called externalisation. The transaction issuer captures the value; the network bears the cost. In a permissionless system with no external authority to socialise the externality, the network bears the cost via the validators.

### 1.3 The thesis

We argue:

1. **(Theorem 1)** For any blockchain with monotonically growing state and bounded fee throughput, there exists a finite time horizon beyond which validator participation becomes economically irrational without ongoing inflation or external subsidy.

2. **(Corollary)** Inflationary reward schedules — common in proof-of-stake — are not a solution; they are a transfer from token holders to validators, a tax that grows without bound as state grows without bound, and which compounds the chain's monetary debasement.

3. **(Constructive claim)** A blockchain that admits state decay as a first-class primitive — explicit half-lives, energy budgets, and cryptographic evaporation — converges to a stationary state size determined by demand, eliminating the asymmetry at the source.

4. **(Strong form)** No production blockchain that does not implement state decay or an equivalent mechanism will remain economically viable on a 30-year horizon.

The strong form is the falsifiable claim this paper is built around. We invite empirical refutation.

---

## 2. Background: State as a Resource

### 2.1 What "state" means in this paper

We use *state* to mean the part of a blockchain's data that any validator must hold in active storage to validate the next block. This excludes block headers (small, sequential), historical transaction data (which can be pruned without affecting validation), and proofs that can be regenerated.

For Ethereum, state is the account trie + contract storage tries. As of late 2025, the active state of Ethereum mainnet is approximately 250–300 GB depending on snapshot strategy [^2]. The historical chain (block bodies + receipts) is significantly larger but is not strictly state — EIP-4444 is the mechanism by which historical data is dropped without affecting consensus.

For Solana, state is the set of accounts. Solana's state is approximately 80 GB as of late 2025, with a growth profile constrained by its rent-exempt minimum (which we discuss in §5.2) and by aggressive ledger pruning.

For Sui, state is the set of objects. Sui's state has been estimated in the low tens of GB, again with active object lifecycle management.

For Cosmos zones, state varies wildly by zone but is typically small relative to Ethereum because of the focus on application-specific chains with bounded scope.

### 2.2 What state costs

Validator cost has three components:

1. **Storage**: the cost of keeping state on disk. Modern enterprise NVMe is approximately \$0.05–\$0.10 per GB-year, including redundancy and power [^3]. Consumer-grade hardware is cheaper but has lower expected service life.

2. **Bandwidth**: the cost of state-sync to new joiners and gossip during validation. Bandwidth in cloud regions is typically \$0.01–\$0.09 per GB egress; on-prem co-located bandwidth is roughly an order of magnitude cheaper.

3. **Computation**: the cost of executing state-touching opcodes. This is often larger than storage in absolute terms but does not grow with state size in the same monotonic way; it grows with throughput. For this paper we treat it as a separate term.

The total cost of validating per validator per year scales roughly as:

$$
C_v(t) = \underbrace{c_s \cdot |S(t)|}_{\text{storage}} + \underbrace{c_b \cdot |S(t)| \cdot k_{\text{sync}}}_{\text{state sync}} + \underbrace{c_c \cdot R(t)}_{\text{compute}}
$$

where $|S(t)|$ is state size at time $t$, $R(t)$ is throughput, and $k_{\text{sync}}$ is a sync-frequency parameter (how often a validator must re-download or transmit the state).

Empirically, $c_s \cdot |S(t)|$ dominates for any chain with state above ~50 GB on commodity validator hardware. The crossover happens earlier on cheap-bandwidth networks (where $c_b$ is small) and later on co-located networks.

### 2.3 What a transaction pays

In every production chain, a transaction pays a fee determined by the gas it consumes (or a flat rate, in some L1s). The fee is denominated in the chain's native token and, in EIP-1559-style fee markets, partially burned.

Critically, the fee is determined by *the cost of execution* and *block-space scarcity*, not by the durability of the resulting state. Two transactions that use identical gas pay identical fees, regardless of whether one writes a 32-byte balance update that will be overwritten next block or a 10-MB IPFS hash that will sit untouched for a decade.

Solana attempted to fix this with rent: an account paid an ongoing rent fee proportional to its size and lifetime. The mechanism was abandoned in favour of *rent-exempt minimum balances*: an account holding more than $\approx 0.002$ SOL pays no rent and persists indefinitely [^4]. In practice, this is not a fix; it is the original problem with an upfront deposit. Once the deposit is paid, the asymmetry returns.

Ethereum's EIP-7745 and related state-expiry proposals attempt to charge for storage at the protocol level but have remained roadmap items for over five years. Statelessness via Verkle trees [^5] reduces the burden on validators (proofs replace state) but pushes the burden onto provers and does not reduce the total state that must exist somewhere in the network.

The fundamental issue: **no production fee market prices the perpetual nature of storage**.

---

## 3. A Formal Model of State Economics

We now construct a deliberately simple model. The simplicity is the point — if the result holds in this model, it constrains the design space of any more elaborate model.

### 3.1 Setting

Let $t \in \mathbb{N}$ index epochs (one epoch per block, for simplicity). Let:

- $|S(t)|$ = active state size at epoch $t$, in bytes.
- $w(t)$ = bytes written per epoch (gross, before any deletion).
- $d(t)$ = bytes deleted per epoch (zero in production chains).
- $g(t) = w(t) - d(t)$ = net growth per epoch.
- $r(t)$ = fee revenue per epoch, in stable-value units.
- $N(t)$ = active validator count at epoch $t$.
- $c_s$ = per-byte-per-epoch storage cost (constant; we relax this in §3.4).
- $c_b$ = per-byte bandwidth cost amortised across the validator set.

State evolves as:

$$
|S(t+1)| = |S(t)| + g(t)
$$

In production chains, $d(t) = 0$, so $|S(t)| = |S(0)| + \sum_{i=0}^{t-1} w(i)$.

### 3.2 Validator cost

Per-validator cost per epoch is:

$$
C_v(t) = c_s \cdot |S(t)| + c_b \cdot |S(t)| \cdot \mathbb{1}[\text{sync}] + \text{constant terms}
$$

where $\mathbb{1}[\text{sync}]$ is 1 in epochs where state is transmitted (e.g. to a joining validator) and 0 otherwise. The expected per-epoch cost is $c_s \cdot |S(t)| + c_b \cdot |S(t)| \cdot p_{\text{sync}}$ for a sync probability $p_{\text{sync}}$.

Total network cost per epoch:

$$
C_{\text{net}}(t) = N(t) \cdot \big[ c_s \cdot |S(t)| + c_b \cdot |S(t)| \cdot p_{\text{sync}} \big] + O(N(t)^2)
$$

The $O(N(t)^2)$ term comes from inter-validator gossip / pairwise state-sync; we keep it separate because it dominates only for very large validator sets.

### 3.3 Net surplus

Define net network surplus per epoch:

$$
\Pi(t) = R(t) - C_{\text{net}}(t)
$$

where $R(t)$ is total fee revenue per epoch. In a fee-market equilibrium, $R(t)$ is bounded above by users' total willingness to pay for block space — itself bounded by demand, throughput, and price.

Validators participate (in a free market) iff their share of surplus exceeds their opportunity cost:

$$
\frac{\Pi(t)}{N(t)} \geq O_v
$$

for opportunity cost $O_v$ (e.g. yield on the same capital deployed elsewhere).

### 3.4 Adding inflation

In practice, validators receive inflationary rewards $I(t)$ in addition to fees. The participation condition becomes:

$$
\frac{R(t) + I(t) \cdot p(t) - C_{\text{net}}(t)}{N(t)} \geq O_v
$$

where $p(t)$ is the token price in stable-value units. Inflation pays validators in token units; the real-value pay-out depends on price.

Inflation itself imposes a cost: it dilutes existing holders. For long-run sustainability we cannot rely on inflation indefinitely without consequences for token value, which feeds back into $p(t)$.

### 3.5 The stationary-state condition

We say a blockchain has a *stationary state distribution* if there exists $S^* < \infty$ and $T^* < \infty$ such that for all $t > T^*$, $|S(t)| \leq S^*$.

Production chains do not satisfy this. Their state is monotonically non-decreasing with no upper bound. The asymmetry is structural: $w(t) > 0$ for any chain with non-zero usage; $d(t) = 0$ by design.

A blockchain with state decay — where every byte has a finite expected lifetime $\tau_b$ — does satisfy stationary-state. In equilibrium, the rate of writes equals the rate of decay:

$$
g(t) \to 0 \quad \text{as } t \to \infty
$$

and $|S(t)|$ converges to $S^* = w_{\text{sustained}} \cdot \bar{\tau}$ where $\bar{\tau}$ is the mean object lifetime.

This is the central asymmetry: **monotone-growth chains have no $S^*$; decay chains do**.

---

## 4. The Unsustainability Theorem

We can now state the main result formally.

### Theorem 1 (Unsustainability of Monotone-State Blockchains)

Consider a blockchain with:
- Monotonically non-decreasing state $|S(t)| = |S(0)| + \sum w(i)$ with $w(i) \geq w_{\min} > 0$ for all $i$ in some non-zero-density subset of epochs.
- Bounded fee revenue per epoch: $R(t) \leq R_{\max}$.
- Bounded validator count: $N_{\min} \leq N(t) \leq N_{\max}$.
- Constant per-byte storage cost $c_s > 0$.

Then there exists a finite epoch $T^*$ such that for all $t > T^*$, in the absence of inflation or external subsidy:

$$
\frac{R(t) - C_{\text{net}}(t)}{N(t)} < O_v
$$

i.e. the per-validator surplus falls below opportunity cost.

### Proof sketch

By assumption, $|S(t)| \to \infty$ as $t \to \infty$, since each epoch adds at least $w_{\min}$ bytes on average (we can replace "every epoch" with "infinitely many epochs"; the result holds).

$C_{\text{net}}(t) \geq N_{\min} \cdot c_s \cdot |S(t)|$, which is unbounded.

$R(t) \leq R_{\max}$ by assumption.

Therefore, $R(t) - C_{\text{net}}(t) \to -\infty$, so the per-validator surplus $\frac{R(t) - C_{\text{net}}(t)}{N(t)}$ falls below any positive constant $O_v$ for sufficiently large $t$.

By the same argument, even if $R(t)$ grows linearly in $t$ (which it does not, since it is bounded by demand), $C_{\text{net}}(t)$ grows in $|S(t)|$ which is at least linear in $t$, so the gap is at most constant — and the conclusion holds with the addition of any positive constant rate of state growth.

### Corollary 1 (Inflation as a debt instrument)

Under Theorem 1, the protocol can compensate validators only by supplying inflation $I(t) \cdot p(t) \geq C_{\text{net}}(t) - R(t)$. As $|S(t)|$ grows without bound, so does the required inflation in stable-value terms. If issuance is in fixed token units, the price $p(t)$ must rise without bound, contradicting any finite-supply asymptote. If issuance is denominated in stable value, the token supply must grow without bound, debasing existing holders.

In either case, the inflation is a transfer from holders to validators in payment for the externality the protocol failed to price at write time. The original fee was, in retrospect, mispriced.

### Corollary 2 (Centralisation pressure)

As $|S(t)|$ grows, the hardware cost of running a validator grows, and the population of entities that can profitably operate one shrinks. This is the centralisation pressure that Ethereum and Solana have both observed empirically [^6][^7]. Theorem 1 explains the *necessity* of this pressure: it is the equilibrium response to the asymmetry, not an accident of poor optimisation.

### Numerical illustration (illustrative, not a measurement)

To make the abstract argument concrete, consider parameters in the rough order of magnitude of Ethereum mainnet:

- $|S(0)| \approx 250\,\text{GB}$
- $w_{\min} \approx 0.5–1\,\text{GB / month}$ (sustained)
- $c_s \approx 0.05\,\$/\text{GB-year}$ (NVMe enterprise)
- $c_b$ negligible by comparison for permissionless validators
- $N \approx 500{,}000$ active validators
- $R \approx 1.5–4 \times 10^9 \,\$ / \text{year}$ (issuance + fees, late 2025) [^8]

Per-validator annual storage cost from state alone:

$$
c_s \cdot |S(t)| \approx 0.05 \cdot 250 = 12.5 \,\$/\text{validator-year}
$$

This is currently below the ~\$30–100 / validator-year that fees + issuance pay (depending on reward structure and exchange rate). The *cost is small relative to revenue today*. But $|S(t)|$ grows roughly 10–15 GB / year. In 50 years, ceteris paribus, storage cost reaches ~\$50/validator-year — still small. In 200 years, ~\$200/validator-year. The crossover with revenue depends on the future trajectory of fees and demand; the point is not that the crossover happens at year $T$, but that **a sufficient growth of $|S(t)|$ guarantees a finite $T^*$ exists for any bounded $R$**.

The illustrative figures should not be cited as predictions; they are sketch values to demonstrate the form of the argument. Empirical work on actual cost trajectories is open (§8).

---

## 5. Existing Approaches and Their Limits

We now consider the major mechanisms that have been deployed or proposed to address the asymmetry, and explain why each falls short.

### 5.1 History pruning (EIP-4444 and similar)

EIP-4444 partially shipped in Ethereum in 2024, allowing full nodes to drop pre-Merge history without affecting consensus [^9]. It addresses *historical chain data*, not active state. Validators still need the full state trie to validate the next block. EIP-4444 reduces archive-node burden, not state burden.

This is real progress on a separate problem. It does not address Theorem 1.

### 5.2 Rent (Solana 2020–2023, abandoned)

Solana's original design charged accounts an ongoing rent proportional to size and lifetime. A user could exempt their account from rent by maintaining a *rent-exempt minimum balance* (currently approximately 2 years of rent paid upfront, deposited in the account itself).

In practice, the exempt minimum became the universal pattern. Users (and, more importantly, dApp authors) did not want UX-level state-loss surprises. By 2023, effectively all production accounts were rent-exempt, and Solana shipped a soft deprecation of the rent mechanism [^10].

Solana shipped exactly one production-grade attempt at the right idea and the market rejected it on UX grounds. The lesson is not that the idea is wrong; it is that *rent must be a protocol invariant, not an opt-in fee*. Users cannot vote to exempt themselves from physics.

### 5.3 State expiry (EIP-7745 and ancestors)

Ethereum has worked on state expiry since 2018. The current proposal (EIP-7745, prior versions EIP-7732, EIP-3074) charges accounts a small ongoing fee or removes them after a long inactivity window. The fundamental design tension: any window long enough to be UX-acceptable (months to years) is short enough that high-value contracts will engineer around it (for example, by automated refresh transactions that cost little but keep state alive).

The result is a parameter that is either (a) too aggressive to be politically viable (deletes contracts users want kept) or (b) too lenient to bound state growth meaningfully. Ethereum has not shipped state expiry in 7+ years of work [^11].

This is not a failure of effort. It is an indication that *retroactive* state expiry on a chain not designed for it is genuinely intractable.

### 5.4 Statelessness (Verkle, KZG-witness models)

Statelessness moves the burden from validators (who hold full state) to provers (who hold full state + produce witnesses). Verkle trees [^5] reduce witness size enough to make this feasible.

Statelessness does not reduce $|S(t)|$. It re-allocates who pays the cost. The total state in the network is the same. The cost asymmetry of Theorem 1 simply shifts from validators to provers; the centralisation pressure (Corollary 2) shifts with it. Provers become the new bottleneck.

For a rigorous argument: Theorem 1 and Corollary 1 hold under statelessness with $N(t) \to N_{\text{prover}}(t)$. As long as someone, somewhere, holds and serves the state, the asymmetry exists.

### 5.5 Sharding (Ethereum, NEAR, others)

Sharding partitions state across shards so any individual validator holds only $|S(t)| / k$ for $k$ shards. The aggregate state is unchanged.

Sharding lowers per-validator cost without bound only if $k$ can grow without bound. In practice, $k$ is bounded by cross-shard messaging complexity, finality assumptions, and adversarial-resistance constraints (each shard must have enough honest stake). $k$ is in the low double digits at best for security-meaningful sharding. The dilution is real but not asymptotic.

Theorem 1 holds with $|S(t)| \to |S(t)|/k$ replacing $|S(t)|$; the $T^*$ is pushed out by a factor of $k$ but not eliminated.

### 5.6 Sui's object model

Sui's object lifecycle (created → owned → consumed) moves state in a graph-like fashion and is more efficient at handling natural object turnover than Ethereum's account model. However, Sui has no general decay primitive — an object once created persists until explicitly consumed. State growth is bounded by user-initiated deletion, not by protocol-enforced expiry.

Sui's growth is slower than Ethereum's empirically, but the underlying economic invariant is the same. Theorem 1 applies.

### 5.7 Summary

Every existing approach addresses a *symptom* of the asymmetry — high archive cost, validator hardware burden, prover specialisation — without addressing the asymmetry itself: that the protocol writes to state without ever charging for the duration of that state.

The minimum requirement for sustainability is a mechanism that makes write cost a function of the duration the state persists. Charging upfront for projected duration (rent) is one such mechanism but is UX-fragile. Charging at write + auto-decaying with refresh option is another.

---

## 6. Thermodynamic State Decay as an Alternative

We now show that a protocol with explicit state decay satisfies the stationary-state condition (§3.5) and therefore avoids Theorem 1's conclusion.

### 6.1 Definition

A blockchain has *thermodynamic state decay* if every state object has explicit fields:

- $E_0$ = initial energy at write time.
- $\tau$ = half-life in epochs.
- $E(t) = E_0 \cdot 2^{-(t - t_0)/\tau}$ = energy at epoch $t$, where $t_0$ is the last refresh epoch.

When $E(t) \leq 0$, the object enters a grace period of fixed length $g$ during which any participant may *refresh* the object (pay a fee, reset $E$ to a positive value). If grace expires without refresh, the object is *evaporated*: its data is hashed, the hash is recorded in a Merkle Mountain Range (MMR) accumulator, and the object's storage is reclaimed.

Refresh is a first-class transaction. Refresh fees flow to validators (or to a burn/mint pool). The fee structure prices the persistence of state explicitly.

### 6.2 Stationary state

In steady-state, write rate equals decay rate. Suppose users sustain a refresh demand of $w_R$ bytes per epoch and a fresh-write demand of $w_W$ bytes per epoch. Each byte has expected lifetime $\bar{\tau}$ (a function of the half-life distribution and refresh decisions). Then:

$$
|S^*| = (w_R + w_W) \cdot \bar{\tau}
$$

This is a finite, demand-determined equilibrium. There is no $T^*$ at which validator cost overruns revenue, because $|S(t)|$ does not grow without bound.

### 6.3 Refresh-fee economics

The refresh fee is the price the protocol charges for state persistence. In a competitive fee market:

- Users set $\bar{\tau}$ implicitly through their willingness to refresh.
- The protocol's revenue per byte-epoch is approximately the refresh fee divided by the inter-refresh interval.
- Validator revenue scales with the integral of state-size × refresh frequency, which is precisely the resource validators consume.

The asymmetry of §1.2 is dissolved: validator cost and fee revenue scale together, both proportional to active state demand. The economy is locally homeomorphic to traditional cloud-storage pricing, with the additional property of cryptographic auditability.

### 6.4 What objects need eternity

Some applications require state that genuinely should never be deleted: NFT provenance for cultural heritage, legal records, consensus-relevant validator stake records. The protocol can support these via:

- High initial energy + long half-life (effectively eternal under any reasonable refresh schedule).
- Explicit `set_immortal` flag with one-time large fee proportional to the projected validator-cost-of-eternity, paid into an endowment.
- Off-chain anchoring (only the hash on-chain; data in IPFS / Filecoin).

The point is that *eternity becomes an explicit, priced, opt-in property*, not a default. This aligns the cost with the economic value of permanence.

### 6.5 What objects don't

Most state in modern dApps does not need eternity:

- Active DeFi positions (closed within days/weeks/months).
- NFT marketplace listings (most expire within months).
- Oracle feeds (refresh periodically).
- L2 state-checkpoint commitments (used once, then historical).
- Game state (matches end).
- Cross-chain message receipts (consumed once).

In all of these, the natural lifecycle of the state matches the natural lifecycle of an energy-decaying object. The match is so good that it is not an accident — it reveals that the *intuitive* state model of every application is decay-first, and the *protocol-imposed* permanence is what is unnatural.

---

## 7. Empirical Projections

A serious version of this paper requires three empirical contributions, each of which is open work:

### 7.1 Methodology — historical state reconstruction

Reconstruct $|S(t)|$ for major chains (Ethereum, Solana, Sui, BNB) at quarterly granularity from 2020–2026. Sources: archive node snapshots, public dashboards (Etherscan, SolanaFM, SuiVision), academic datasets [^12].

Output: empirical $|S(t)|$ curves with growth-rate decompositions (fresh writes vs reactivations vs storage-related vs application-related).

### 7.2 Methodology — counterfactual simulation

For each historical chain, construct a hypothetical *decay-instrumented twin*: the same workload, but with each transaction's effects subject to a parameterised half-life. Sweep half-lives across $[1\,\text{week}, 100\,\text{years}]$. For each parameter, compute the implied $|S^*|$.

Output: a curve $|S^*|(\bar{\tau})$ for each chain. Identify the $\bar{\tau}$ at which $|S^*|$ falls below a target (e.g. 100 GB, the threshold below which consumer-hardware validation is plausible).

### 7.3 Methodology — validator-economics projection

Project total validator cost and revenue under both regimes (status-quo and counterfactual) over 10/30/100-year horizons. Use bands rather than point estimates for input parameters to make the result robust to parameter uncertainty.

Compare:
- Status-quo: $C_{\text{net}}(t)$ vs $R(t) + I(t)$ on existing chains.
- Counterfactual: $C_{\text{net}}^*(t)$ vs $R^*(t) + I^*(t)$ where state has decay.

Output: time-to-unsustainability $T^*$ under status-quo, and demonstration of finite $|S^*|$ under counterfactual.

### 7.4 What the simulations are not

The simulations are not predictions. They are illustrations of the form of the argument. The point is to make the unsustainability claim concrete enough to falsify, not to assign a specific year-of-collapse to any specific chain.

The simulations also do not account for innovations that may occur (new compression schemes, hardware improvements, zero-knowledge state checkpointing). These could push $T^*$ further out — but not to infinity, since $|S(t)|$ grows at least linearly under any positive write rate.

---

## 8. Discussion: Limitations and Open Questions

### 8.1 What this paper does not show

We do not show that EvaporChain's specific decay parameters are optimal, that thermodynamic decay is the only mechanism that achieves stationary state, or that all decay implementations are equivalent. The claim is narrow: *any* mechanism that implements per-byte decay-or-refresh dynamics suffices to satisfy stationary-state. We do not claim EvaporChain is the only such design.

### 8.2 The political economy of decay

Decay protocols face a hard incumbency problem: every existing dApp is written assuming permanent state. Migrating to a decay model requires all of those applications to change. We expect decay protocols to win first in *new* application categories where permanence was never an assumption — IoT telemetry, ephemeral messaging, expiring credentials, legal-mandate-driven data deletion (GDPR right-to-erasure, healthcare retention rules) — and to migrate from there.

This is not a critique of the technical argument. It is a separate question (politics of adoption) that the paper does not address.

### 8.3 Decay and adversarial behaviour

A naive decay protocol invites griefing: an attacker writes large state objects with high initial energy and long half-life, paying minimal upfront fees, to burden validators. EvaporChain's response (mandatory storage deposit, gas pricing proportional to size and half-life) is one design point. Other designs are possible. The space of fee structures consistent with stationary state is itself open research.

### 8.4 Decay and composability

If a contract depends on another contract whose state has evaporated, the dependent contract may fail unexpectedly. This is a real composability hazard. EvaporChain's resurrection mechanism (allowing ghosted objects to be revived for a fee) and the requirement that contracts explicitly handle the `on_grace` / `on_evaporate` lifecycle hooks are partial responses. A complete formal account of composability under decay is open.

### 8.5 What would falsify the argument

Theorem 1's strong form is falsifiable. Specifically:

- A demonstration of a long-lived blockchain (production for 30+ years) with monotonically growing state and economically rational validator participation, in a free market, without ongoing inflation funded externally.
- Or: a fundamentally different cost structure for storage that grows sublinearly in $|S(t)|$ — for example, a holographic-storage paradigm with per-byte cost in $O(1/|S(t)|)$ — would invalidate the assumption that storage cost is linear.

Neither is currently in evidence. We invite engagement.

---

## 9. Conclusion

The asymmetry between one-time fee and perpetual cost is not a quirk of any specific blockchain implementation. It is a structural property of every blockchain that does not enforce state decay. Theorem 1 makes the consequence formal: under any monotone-state regime, validator economics breaks at finite time.

The remediations deployed to date — history pruning, statelessness, sharding, opt-in rent — address the symptoms while preserving the asymmetry. None achieves stationary state. None can, by construction.

Thermodynamic state decay (or any equivalent per-byte temporal mechanism) is not a competing optimisation; it is the only known structural answer. The blockchains that adopt it will, on a long enough horizon, be the only blockchains that remain economically autonomous.

This paper is a deliberate provocation. The strong claim — that no chain without decay survives a 30-year horizon — is not yet an established fact. It is a falsifiable position, and we invite the empirical and theoretical work that will either confirm or refute it. EvaporChain is one constructive proof that decay works at the protocol level. Whether decay is the *only* answer is a question for the community.

---

## References (selected; bibliography is preliminary)

[^1]: Buterin, V. *EIP-4444: Bound Historical Data in Execution Clients*. Ethereum Improvement Proposals, 2021. https://eips.ethereum.org/EIPS/eip-4444

[^2]: Etherscan / Geth state-size dashboards, accessed 2025–2026. Indicative range of full execution-client state. Specific snapshots vary by client.

[^3]: AWS, GCP, on-prem co-location pricing surveys, 2024–2025. Per-GB-year cost depends on redundancy class, region, and procurement scale.

[^4]: Solana Foundation. *Rent and Rent Exemption*. Solana docs, 2023. https://docs.solana.com/implemented-proposals/rent

[^5]: Buterin, V., Ben-Sasson, E., Gabizon, A., et al. *Verkle Trees*. Ethereum research, 2021–2024.

[^6]: Eth2 staking economics community analysis, 2022–2025. Staking cost, MEV-share dynamics, validator concentration.

[^7]: Solana validator hardware requirements, validator concentration data, 2023–2025.

[^8]: Ultrasoundmoney.eth supply and burn dashboards, accessed 2025.

[^9]: Ethereum All Core Devs meeting notes, 2024. Partial activation of EIP-4444 history-pruning.

[^10]: Solana Improvement Documents (SIMD), rent deprecation discussion, 2022–2023.

[^11]: Ethereum Magicians forum, state-expiry threads, 2018–2024.

[^12]: Academic datasets on chain state reconstruction (e.g., Ren et al. on Ethereum state, NFTrade datasets, Bonneau et al. on UTXO sets).

---

## Appendix A — Notation reference

| Symbol | Meaning |
|---|---|
| $\|S(t)\|$ | Active state size at epoch $t$, in bytes |
| $w(t), d(t), g(t)$ | Bytes written, deleted, net per epoch |
| $N(t)$ | Validator count at epoch $t$ |
| $c_s, c_b, c_c$ | Per-byte storage / bandwidth / compute cost |
| $R(t)$ | Fee revenue per epoch |
| $I(t), p(t)$ | Inflation issuance and token price |
| $\Pi(t)$ | Net network surplus per epoch |
| $O_v$ | Validator opportunity cost |
| $T^*$ | Time at which surplus falls below opportunity cost |
| $S^*$ | Stationary state size under decay |
| $\tau, E_0, E(t)$ | Half-life, initial energy, energy-at-time of an object |

## Appendix B — Open questions for future work

- A more rigorous treatment of the relationship between the half-life distribution and the resulting stationary distribution.
- The optimal fee structure for refresh in a competitive market, given that users have heterogeneous time preferences.
- Composability formalism for decay-aware smart contracts.
- The impact of MEV under decay (does decay mitigate or exacerbate MEV?).
- Cross-chain state-decay coordination (a decay-aware bridge).
- Empirical work: actual reconstruction of $|S(t)|$ for major chains and counterfactual simulation per §7.

---

**End of draft v0.1.**

Notes for revision (do not publish):
- Numerical figures in §4 are illustrative and must be replaced or marked as such before submission.
- Literature review in §5 is selective; expand with citations to specific Ethereum Magicians threads and SIMD docs before submission.
- Appendix C (proofs in expanded form, including the regularity conditions for Theorem 1) to be added in v0.2.
- Reviewer 2 of any submission will object to the strong form of the claim in §1.3. We accept this. The strong form is the contribution; weakening it makes the paper a survey.
