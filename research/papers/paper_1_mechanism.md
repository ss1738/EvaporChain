# Energy-Decay State Management: A Thermodynamic Primitive for Blockchain State

**Author:** Satyawan Singh
**Affiliation:** Independent Researcher, Leicester, United Kingdom
**Status:** Draft v0.1 (2026-04-27). The mechanism paper. Companion to *State Economics: Why Infinite-State Blockchains Are Economically Unsustainable* (Paper 2, this corpus) and to the EvaporChain whitepaper.
**Target venues:** arXiv (cs.CR / cs.DC) preprint; AFT or Financial Cryptography for venue.

---

## Abstract

We present *energy-decay state management*: a protocol primitive in which every on-chain object carries an explicit energy budget that depletes over time according to a configurable half-life. Objects whose energy reaches zero enter a bounded grace period and, if not refreshed, are *evaporated* — removed from active state and committed only as a cryptographic ghost record in a Merkle Mountain Range nullifier accumulator. Refresh and resurrection are first-class transactions with explicit fee structures.

The mechanism is implemented in EvaporChain, a production-grade Layer-1 blockchain. We give the formal definition, describe the object lifecycle, present the deterministic integer arithmetic that drives decay (avoiding floating-point non-determinism in consensus), specify the dual cryptographic commitment scheme (Verkle trie over active objects + MMR over ghost records), and analyse composition with Tendermint BFT consensus.

The contribution is not a single new cryptographic primitive but the *combination* of energy decay, evaporation with cryptographic auditability, and dual commitment as a complete state-management protocol. We argue that this combination achieves three properties no production blockchain currently provides: (a) bounded steady-state size as a function of demand, (b) protocol-priced state persistence, and (c) cryptographic provenance for evaporated state.

We characterise the security model, the integer-arithmetic determinism property, and the edge cases (refresh races, anchor composition, resurrection across long horizons). We also catalogue what the mechanism does *not* solve — sharding, MEV, finality latency — and clarify the relationship between this paper and Paper 2 (the economic-sustainability argument).

---

## 1. Introduction

### 1.1 The state-management gap

The state-management literature for blockchains is dominated by *post-hoc* approaches. State expiry, history pruning, statelessness, and rent are mechanisms designed to retrofit lifetime semantics onto state models that originally assumed permanence. As Paper 2 argues, this retrofit is structurally incomplete: every existing approach addresses a symptom while preserving the asymmetry between one-time fee revenue and perpetual storage cost.

This paper takes the dual approach. Rather than retrofit, we ask: what would a state-management protocol look like if lifetime semantics were *primitive*? Every object has an explicit energy and half-life from the moment it is created. Decay is automatic. Persistence is explicitly priced via refresh transactions. Evaporated state leaves cryptographic provenance.

We do not claim this is the only valid design. We claim it is a complete one, and we present its mechanism precisely enough to be implemented, formally verified, and deployed.

### 1.2 Contributions

1. **Formal definition** (§2) of energy-decay state, including the deterministic integer-arithmetic decay formula.
2. **Object lifecycle specification** (§3): Active → Grace → Ghost → Resurrected, with transition triggers and cryptographic invariants.
3. **Dual commitment scheme** (§4): Verkle trie over active state + Merkle Mountain Range over evaporated nullifiers, with the combined state root as the canonical commitment.
4. **Refresh and resurrection economics** (§5), with explicit fee structures that compose cleanly with EIP-1559-style fee markets.
5. **Composition with Tendermint BFT consensus** (§6), using the Rule-Based Consensus anchor scheme to avoid per-block state-root agreement (formal treatment in `RuleBasedConsensus.tla` and its proof companion).
6. **Security analysis** (§7) covering refresh races, anchor composition error bounds, and adversarial behaviour against the decay protocol.
7. **Comparative analysis** (§8) with EIP-7745 / Solana rent / statelessness / Sui's object lifecycle, distinguishing what each addresses and what they do not.

### 1.3 Out of scope

This paper does not cover:

- The economic-sustainability argument for why state decay is necessary (Paper 2).
- The data-availability layer (Frontier Paper 1: PoHA Decaying DA).
- The energy-annotated Verkle trie's subtree-compression mechanism (Frontier Paper 2).
- The full Rule-Based Consensus protocol details beyond the anchor model (Frontier Paper 3).
- The smart-contract VM (EvaporScript) and the rule engine.
- Empirical performance benchmarks and validator-economics projections.

These are companion contributions, separately drafted or in flight.

---

## 2. Energy-Decay State: Formal Definition

### 2.1 The state object

Let $\mathcal{O}$ denote the set of on-chain state objects. Each object $o \in \mathcal{O}$ is a tuple:

$$
o = \langle \text{id}, \text{owner}, E_0, \tau, t_{\text{create}}, t_{\text{refresh}}, \text{state}, \text{data} \rangle
$$

where:
- $\text{id} \in \{0,1\}^{256}$ is the unique object identifier (BLAKE3 of creator + nonce).
- $\text{owner} \in \mathcal{A}$ is the current owner's account address.
- $E_0 \in \mathbb{N}_{>0}$ is the *deposited* energy at last refresh, in protocol energy units.
- $\tau \in \mathbb{N}_{>0}$ is the half-life of the object, in epochs.
- $t_{\text{create}}, t_{\text{refresh}} \in \mathbb{N}$ are the creation and most-recent-refresh epochs.
- $\text{state} \in \{\text{Active}, \text{Grace}, \text{Ghost}, \text{Resurrected}\}$ is the lifecycle phase.
- $\text{data} \in \{0,1\}^*$ is opaque application payload.

We treat the protocol as operating in discrete epochs, one per block in the simplest case; extensions to variable block time are addressed in §6.

### 2.2 The energy function

The current energy of an Active or Resurrected object at epoch $t$ is:

$$
E(o, t) = \mathrm{LazyEnergy}(E_0, \tau, t - t_{\text{refresh}})
$$

where $\mathrm{LazyEnergy}$ is the deterministic integer formula:

$$
\mathrm{LazyEnergy}(E_0, \tau, \Delta) =
\begin{cases}
0 & \text{if } \tau = 0 \\
\left\lfloor \dfrac{E_0}{2^{\lfloor \Delta / \tau \rfloor}} \right\rfloor - \left\lfloor \dfrac{\lfloor E_0 / 2^{\lfloor \Delta / \tau \rfloor} \rfloor \cdot (\Delta \bmod \tau)}{2\tau} \right\rfloor & \text{if } \lfloor \Delta / \tau \rfloor < 64 \\
0 & \text{if } \lfloor \Delta / \tau \rfloor \geq 64
\end{cases}
$$

This formula is the integer approximation of $E_0 \cdot 2^{-\Delta / \tau}$. The key properties:

1. **Determinism**: every validator computing $\mathrm{LazyEnergy}(E_0, \tau, \Delta)$ produces the same result, since the formula uses only integer division and bit-shifts.
2. **Monotonic non-increasing**: $\Delta_1 \leq \Delta_2 \implies \mathrm{LazyEnergy}(E_0, \tau, \Delta_1) \geq \mathrm{LazyEnergy}(E_0, \tau, \Delta_2)$.
3. **Saturation at zero**: $E$ never goes negative; $\mathrm{LazyEnergy}$ saturates to 0.
4. **Bounded error under composition**: $\mathrm{LazyEnergy}(\mathrm{LazyEnergy}(E_0, \tau, \Delta_1), \tau, \Delta_2)$ differs from $\mathrm{LazyEnergy}(E_0, \tau, \Delta_1 + \Delta_2)$ by at most 1 unit per composition step. (Discussed in §6.3.)

The choice of integer arithmetic is deliberate: floating-point operations are non-deterministic across architectures and consensus-relevant computation must be exactly reproducible.

### 2.3 The half-life $\tau$

The half-life is set at object creation. Different applications need different temporal characteristics:

- **DeFi position state**: $\tau$ short (days), since positions are closed quickly.
- **NFT provenance**: $\tau$ long (years), with refresh subsidised by collection treasury.
- **Oracle feeds**: $\tau$ medium (hours), with refresh as part of the oracle update flow.
- **Long-term immutable records**: special object class with $\tau = \infty$ (modelled as a separate state machine, §3.5).

The protocol does not prescribe specific $\tau$ values. Applications choose their own, paying the corresponding fees.

---

## 3. Object Lifecycle

### 3.1 The four states

Every object is in exactly one of four states:

```
                          refresh
                  ┌────────────────────────┐
                  ▼                        │
              ┌──────┐                   ┌─┴────┐
              │Active│ ────E(t)≤0────▶│ Grace│
              └───┬──┘                   └─┬────┘
                  │                        │
              resurrect                grace expires
                  ▲                        │
              ┌───┴────────┐             ┌─▼───┐
              │Resurrected │ ◀───────────│Ghost│
              └────────────┘  resurrect  └─────┘
```

**Active**: object exists in full active state. Decay is implicit; energy at any query epoch is computed by `LazyEnergy`.

**Grace**: $E(o, t) \leq 0$. Object remains in active storage for a configurable grace period $g$ (default 5 epochs, configurable via genesis parameters and tunable via on-chain governance). Any participant may refresh during grace, restoring the object to Active.

**Ghost**: grace period expired without refresh. Object's data is hashed (BLAKE3), the hash is stored in a `GhostRecord`, the ghost is appended to the MMR nullifier accumulator, and the object's data is removed from active storage. The original payload is retained in the ghost (for resurrection) up to a per-protocol *cold-storage horizon* beyond which only the hash survives.

**Resurrected**: a ghost has been restored to active state via the resurrection transaction. The MMR entry persists as historical proof of the evaporation event; the object is freshly active.

### 3.2 The decay engine

The protocol runs an `EvaporationEngine` once per block, after transaction execution. In pseudocode:

```
EVAPORATE(state, current_epoch, grace_period):
  for each object in state.active_objects():
    if object.state == Active:
      current_energy = LazyEnergy(object.E0, object.tau,
                                   current_epoch - object.t_refresh)
      if current_energy == 0:
        object.state = Grace
        object.grace_epoch = current_epoch
    elif object.state == Grace:
      if current_epoch >= object.grace_epoch + grace_period:
        ghost = make_ghost_record(object)
        state.append_ghost(ghost)
        state.delete_object(object.id)
```

The engine is deterministic (no system clock, no randomness) and its output is committed in the block.

### 3.3 Refresh and resurrection

**Refresh**: a transaction that deposits additional energy into an Active or Grace object. Its effect:

- For Active/Resurrected: $E_0 \leftarrow E(o, t) + \text{deposit}$, $t_{\text{refresh}} \leftarrow t$.
- For Grace: $E_0 \leftarrow \text{deposit}$, $t_{\text{refresh}} \leftarrow t$, $\text{state} \leftarrow \text{Active}$.

A refresh requires the deposit to exceed `MIN_REFRESH` (configurable, currently 1000 energy units) and that the caller pays the standard transaction gas plus a per-byte refresh-fee proportional to object size.

**Resurrection**: a transaction that restores a Ghost to Active. Its effect:

- New `StateObject` is created with state `Resurrected`.
- $E_0$ set by the deposit.
- The ghost's original data is recovered (if within cold-storage horizon).
- A resurrection fee is charged: 60% of the original creation cost, with a floor of 500 energy units. The floor exists to prevent griefing-by-mass-resurrection of cheaply-created ghosts.

### 3.4 Lifecycle invariants

The protocol enforces, as cryptographic invariants:

**Inv-1** (state-monotonicity within block): Active → Grace → Ghost transitions happen in this order; no skipping.
**Inv-2** (energy non-negative): $E(o, t) \geq 0$ for all $o, t$ where $o$ is Active.
**Inv-3** (ghost provenance): every Ghost has a corresponding MMR entry committed at the epoch of evaporation.
**Inv-4** (resurrection uniqueness): a Ghost can be resurrected at most once. Re-resurrection is prevented by construction: `resurrect()` calls `db.remove_ghost(object_id)`, which deletes the ghost record; any subsequent `RefreshTx` for the same id returns `ObjectNotFound` before any state mutation occurs (see `state/src/refresh.rs`). The MMR entry persists as an append-only historical proof of the evaporation event — it is not consumed on resurrection. (GHOST-A, audit 2026-05-17: prior text said "MMR entry is consumed, marked with a nullifier hash"; corrected to reflect V1 implementation. The invariant holds by different means than stated; the MMR is not a double-spend registry in V1.)
**Inv-5** (data redaction post-horizon): once a Ghost passes the cold-storage horizon, its `original_data` is purged; only the hash survives. Resurrection past this point requires off-chain data recovery.

These invariants are enforced by the execution engine and verified by the consensus engine.

### 3.5 The eternal-object exception

Some applications genuinely require state that should never be evaporated: legal records, validator stake records, certain compliance-mandated audit logs.

The protocol supports these via an explicit `Immortal` flag set at creation time. An Immortal object pays a one-time *endowment fee*, deposited into a protocol-wide endowment pool. The object never decays; it bypasses the decay engine entirely.

The endowment fee is calibrated so that the present value of perpetually validating the object equals the fee. This makes the cost of eternity explicit and prepaid, rather than externalised onto future validators.

The protocol caps the fraction of state that can be Immortal (currently 5% of total state by byte-count) to prevent the eternal-object exception from undoing the decay primitive's economic benefit.

---

## 4. Cryptographic Commitments

### 4.1 The dual-commitment scheme

EvaporChain commits to state via a *dual commitment*:

$$
\text{StateRoot}(t) = \text{BLAKE3}\big( \text{VerkleRoot}(\text{Active}(t)) \;\Vert\; \text{MMRRoot}(\text{Ghosts}(t)) \big)
$$

where:
- $\text{Active}(t)$ is the set of active objects (not yet ghost) at epoch $t$.
- $\text{Ghosts}(t)$ is the cumulative set of evaporated objects up to epoch $t$.
- $\text{VerkleRoot}$ is the standard Verkle-tree commitment (Pedersen-IPA over BLS12-381 or KZG over BN254 — both implementations supported).
- $\text{MMRRoot}$ is the Merkle Mountain Range root over append-only ghost records.

The combined state root is what is committed in the block header.

### 4.2 Active-state commitments via Verkle

The Verkle trie commits to active accounts and active objects. Standard Verkle properties apply: small witnesses (≈ 200 bytes for a key-value proof), constant verification time, post-quantum security depending on the choice of underlying scheme.

EvaporChain extends the standard Verkle trie with energy-aware metadata in internal nodes (Frontier Paper 2). For the purposes of this paper, treat the Verkle root as a black-box commitment to the active state.

### 4.3 Evaporated-state commitments via MMR

The Merkle Mountain Range (MMR) is an append-only commitment data structure. Each appended leaf is a `GhostRecord`:

$$
g = \langle \text{object\_id}, \text{owner}, t_{\text{evap}}, \text{BLAKE3}(\text{data}), \text{mmr\_position} \rangle
$$

The MMR is updated only via append (when a new ghost is created). The MMR root is updated in the block in which the evaporation occurs.

MMR has two desirable properties for this use:

1. **O(log n) inclusion proofs** for individual ghosts. Sufficient for resurrection transactions.
2. **O(log n) range proofs** for batched ghost queries. Useful for cross-shard auditing and bridge contracts.

The MMR is canonical; once a ghost is appended, the MMR commitment can be rebuilt from the (height, position, object_id) tuple. This means a node that has lost ghost data can resync from peers without trusting them: each ghost record is hashed into the MMR, and the resulting root must match the chain's committed state root.

### 4.4 The combined commitment

The choice to combine Verkle (for active) and MMR (for evaporated) reflects different access patterns:

- Active state needs efficient *update*: every block changes some active objects. Verkle's per-block update cost is logarithmic.
- Evaporated state is *append-only*: ghosts are added but never modified. MMR's append cost is amortised constant.

Using the same primitive for both would compromise one or the other. The dual scheme matches the operational profile.

The combined root has the security of the weaker of the two (currently both at >120-bit security under standard assumptions). Composition is via a single BLAKE3 hash, which adds negligible overhead.

### 4.5 Light-client implications

Light clients verifying a state query:

- For an active object: download a Verkle witness (∼200 bytes) + verify against the committed VerkleRoot.
- For a ghost: download an MMR inclusion proof (O(log |Ghosts|) hashes) + verify against the committed MMRRoot.
- For non-existence (object never existed or has evaporated): combination — Verkle non-membership proof for active set + MMR membership proof for ghost set, OR Verkle non-membership + MMR non-membership for "object never existed".

Total light-client proof size for any single query is on the order of hundreds of bytes to a few kilobytes, regardless of total state size. This is significant: in a chain that grows without bound, light-client proof size grows with state. In EvaporChain, proof size grows with $\log |\text{Active}| + \log |\text{Ghosts}|$, both of which are bounded under the steady-state argument of Paper 2.

---

## 5. Refresh and Resurrection Economics

### 5.1 The fee structure

Every operation on a state object has an associated fee. The fee schedule:

| Operation | Fee structure |
|---|---|
| Create object | $f_{\text{create}} = \text{base\_create\_fee} + c_{\text{byte}} \cdot \|\text{data}\| + c_{\text{energy}} \cdot E_0 \cdot \tau$ |
| Refresh (Active/Grace) | $f_{\text{refresh}} = \text{base\_refresh\_fee} + c_{\text{energy}} \cdot \text{deposit}$ |
| Resurrection (from Ghost) | $f_{\text{resurrect}} = 0.6 \cdot f_{\text{create}}^{\text{original}}$, floor 500 |
| Read query (light client) | 0 (off-chain) |

The $c_{\text{energy}}$ coefficient prices the chain's storage commitment over the object's expected lifetime. By construction, the create fee internalises the perpetual-storage externality that monotone-state chains externalise (Paper 2 §1.2).

### 5.2 Why creation cost includes $E_0 \cdot \tau$

The product $E_0 \cdot \tau$ is, to first order, the expected lifetime byte-epoch cost of the object: $E_0$ is the energy budget, and an object with half-life $\tau$ has an expected lifetime proportional to $\tau \cdot \log(E_0 / E_{\text{floor}})$ for a small floor energy.

Pricing the create fee proportional to $E_0 \cdot \tau$ means:

- Short-lived, low-energy objects pay a small fee.
- Long-lived, high-energy objects pay a proportionally larger fee.
- Eternal objects (Immortal flag) pay the present value of perpetual storage, computed from the protocol's discount rate.

In other words: at write time, the user pays the protocol's expected cost of remembering. The asymmetry of Paper 2 §1.2 is removed at the source.

### 5.3 The refresh market

Refresh is a competitive market. Anyone — not just the owner — may refresh any object. The economic incentive: refreshing an object you depend on (a contract whose state you rely on, an oracle feed you query) is cheaper than recreating it.

Importantly, refresh fees flow primarily to validators (after fee burn). This aligns the protocol with the validators' interest: their compensation increases with state activity (refresh transactions) rather than with state quantity.

### 5.4 Resurrection economics

Resurrection at 60% of original creation cost discourages frivolous use of the resurrection mechanism while keeping it accessible for legitimate restoration. The 60% is empirical; its calibration is an open economics question (Paper 2 §8 discusses).

The resurrection fee floor (500 energy) prevents an attacker from creating millions of cheap ghosts and resurrecting them en masse to grief validators with a large surge of resurrection-validation work. Without the floor, the attack would be cheap; with it, the attack is bounded by the number of ghosts the attacker can afford to create.

### 5.5 Composition with EIP-1559-style fee markets

EvaporChain implements an EIP-1559-style base fee with PID controller, separate from the operation-specific fees. The total fee is:

$$
f_{\text{total}} = f_{\text{base}}(t) + f_{\text{operation}} + f_{\text{tip}}
$$

The base fee adjusts based on block-space demand; the operation fee is determined by the schedule above. The tip is at user discretion and influences inclusion priority.

Fee burn, as configured in genesis, removes a fixed fraction of $f_{\text{base}}$ from the supply each block. Operation fees accrue to the validator reward pool. The split provides both deflationary pressure (fee burn) and validator-aligned incentives (operation fees).

---

## 6. Composition with Tendermint BFT Consensus

### 6.1 The integration challenge

EvaporChain runs Tendermint BFT for consensus on blocks. Standard Tendermint commits to a state-root at every block. With energy decay, this creates a challenge: the state at epoch $t$ depends on $t$ explicitly (object energies have evolved), so two validators evaluating state at slightly different wall-clock moments may compute different state roots.

We have observed this in practice: cluster nodes Mini2 and Mini3 reported `ghost_count = 4330` and `ghost_count = 4290` respectively at what each believed was the same epoch. The cause was epoch-evaluation timing, not consensus disagreement.

### 6.2 The Rule-Based Consensus solution

Rule-Based Consensus (Frontier Paper 3) addresses this by separating *consensus on anchors* from *evaluation of state*:

1. At regular intervals (every $A$ epochs, default $A = 100$), validators reach BFT consensus on an *anchor state*: a snapshot of all active objects' energies at the anchor epoch.
2. Between anchors, no per-block state-root agreement is required. State at any epoch $t$ in $[t_{\text{anchor}}, t]$ is computed lazily by applying $\mathrm{LazyEnergy}$ to the anchor.
3. Block headers commit to the anchor reference (anchor epoch + anchor root) plus the transaction Merkle root and the data-availability commitment, but NOT to the per-block state root.

This achieves two things:

- **Determinism is restored**: every validator computing state from the same anchor produces the same answer (formal proof in `RuleBasedConsensus.tla`, this corpus).
- **Per-block consensus is faster**: validators no longer need to compute and agree on a full state root every block. They agree on transaction order and DA, with state evaluation as a derivable function.

### 6.3 The integer-rounding error in anchor composition

An honest treatment requires acknowledging a subtlety in the integer-arithmetic decay. The frontier doc claims that lazy evaluation across multiple anchors is exactly equivalent to lazy evaluation from genesis. This holds in continuous mathematics but **fails for integer arithmetic** by at most 1 unit per re-anchor step.

For typical parameters ($A = 100$, sustained system runtime $T = 10^7$ epochs, energy units in millions), the cumulative rounding error after $T/A = 10^5$ re-anchors is bounded by $10^5$ energy units against a typical $10^6$ initial — a relative error below 0.1%.

Crucially, the error is **deterministic**: every validator accumulates the same error, so determinism is preserved. The error is a property of the decay formula, not of the validators.

The proof companion (`research/frontier/03-rule-based-consensus-proof.md`) derives the bound and explains the design choice: EvaporChain accepts bounded deterministic drift in exchange for $O(1)$ anchor cost, rather than re-deriving from genesis on every query (which would be $O(T)$).

### 6.4 Block header structure

With Rule-Based Consensus, the EvaporChain block header is:

```
BlockHeader {
    height: u64,
    parent_hash: [u8; 32],
    tx_merkle_root: [u8; 32],
    data_root: [u8; 32],          // 2D-erasure DA commitment
    anchor_epoch: u64,             // most recent anchor's epoch
    anchor_state_root: [u8; 32],   // anchor's state commitment
    epoch_at_block: u64,           // current epoch (informational, not consensus-critical between anchors)
    timestamp: u64,
    validator_set_root: [u8; 32],
    signature: BLS aggregate,
}
```

State queries by external clients reference (anchor_state_root, query_epoch) pairs, deriving the canonical state via `LazyEnergy`.

---

## 7. Security Analysis

### 7.1 Threat model

Adversaries we model:

- **External transaction submitter**: can submit any well-formed transaction. Constrained by signature validity, nonce, and balance.
- **Byzantine validator**: can equivocate, withhold, propose invalid blocks, vote for conflicting blocks. Constrained by 1/3 stake bound on byzantines.
- **State miner**: an attacker attempting to bloat state by creating low-energy short-lifetime objects in volume.
- **Refresh griefer**: an attacker attempting to keep state alive past its useful life by sustained refresh.
- **Resurrection griefer**: an attacker attempting to mass-resurrect ghosts to burden validators.
- **Anchor disrupter**: an attacker attempting to delay or corrupt anchor consensus.

### 7.2 Attack analysis

**State-bloat attack**: An attacker tries to bloat active state by creating large numbers of cheap, short-lifetime objects. Defence: the create fee includes $c_{\text{byte}} \cdot \|\text{data}\|$, so large objects cost proportionally to size. Short half-life means the objects evaporate quickly, returning state to baseline. Cumulative cost per byte of state-life is bounded below by the fee schedule; an attacker pays for state they create.

**Refresh-griefing attack**: An attacker repeatedly refreshes a ghost-of-no-value to keep it alive, burdening validators. Defence: refresh requires $f_{\text{refresh}} \geq \text{base\_refresh\_fee} + c_{\text{energy}} \cdot \text{deposit}$. The attacker pays in proportion to the energy they're depositing. There is no economic benefit to refreshing valueless state; the protocol treats this as legitimate paid persistence.

**Resurrection-flood attack**: An attacker creates many cheap ghosts (over time, paying create fees) and resurrects them all at once. Defence: the resurrection fee floor (500 energy) makes each resurrection costly even for cheaply-created ghosts. The total attack cost is bounded by the number of ghosts × 500. Validators are protected by the per-block gas limit, which caps the number of resurrections per block.

**MMR-corruption attack**: An attacker attempts to insert a forged ghost into the MMR. Defence: MMR is committed to via the canonical state root in every block. A forged ghost would change the MMR root, which would fail consensus.

**Anchor disruption**: An attacker (a byzantine validator with up to 1/3 stake) attempts to prevent or corrupt an anchor commit. Defence: anchor consensus runs through the same Tendermint BFT path as normal block commits. Standard BFT safety guarantees apply: 2/3 honest stake is sufficient for liveness, and equivocation slashing enforces accountability.

**Replay attack on anchors**: An attacker replays an old anchor commit to confuse light clients. Defence: each anchor is height-indexed. Replays would be detected by the height field. Additionally, the Rule-Based Consensus spec's `QueryDeterminism` invariant ensures that lazy queries against an old anchor produce well-defined, deterministic results (the result is what the chain *says* the value was at that anchor + lazy derivation, which is consistent with what light clients have already verified).

### 7.3 Formal verification status

Formal verification artefacts in this corpus:

- `research/tla/EvaporChainBFT.tla` — Tendermint BFT consensus model with safety + liveness invariants.
- `research/tla/RuleBasedConsensus.tla` — Rule-Based Consensus state-function-commitment model with `QueryDeterminism`, `MonotoneDecay`, `BoundedByInitial`, `AnchorSanity` invariants.
- `research/frontier/03-rule-based-consensus-proof.md` — proof companion explaining the integer-rounding subtlety and bounding the error.

Open formal-verification work:

- Mechanized proof in Coq or Lean of the `LazyEnergy` composition error bound (sketch in proof companion §5.1).
- Composition proof of `EvaporChainBFT.tla` × `RuleBasedConsensus.tla` to obtain the full *"consensus on anchor implies determinism of all subsequent queries"* statement.
- Formal modelling of resurrection across anchors (currently informal in the proof companion §5.3).

---

## 8. Comparison with Existing Approaches

### 8.1 Ethereum state expiry (EIP-7745 and ancestors)

State expiry proposals have been on the Ethereum roadmap since 2018 [^1]. The current proposal expires inactive accounts after a long inactivity window (years). EvaporChain differs in:

- **Decay is continuous, not threshold**: every object decays each epoch, rather than being suddenly expired after a threshold.
- **Decay parameters are per-object**: applications choose appropriate $\tau$ for their data, rather than the protocol prescribing a single window.
- **Refresh is permissionless**: any participant can refresh any object, not just the owner.
- **Cryptographic provenance is preserved**: ghost records remain in the MMR after evaporation; Ethereum state expiry simply deletes the data.

EVMs cannot easily adopt energy-decay because the EVM and account model are designed around permanent state. EvaporChain's design starts from decay-first and is unable to use the EVM directly; it uses EvaporScript instead.

### 8.2 Solana rent (deprecated)

Solana shipped rent in 2020, charging accounts ongoing fees proportional to size. By 2023, the rent-exempt minimum balance pattern had become universal: developers deposit enough SOL to exempt accounts indefinitely [^2]. The mechanism became a no-op.

EvaporChain differs in:

- **Decay is not bypassable by upfront deposit**: the energy depletes regardless of any minimum balance.
- **Decay is a protocol invariant**, not a fee that applications can structure around.
- **Refresh is the user-facing equivalent of rent**: pay to keep state alive, but as an explicit, recurring action, not a one-time exemption.

### 8.3 Statelessness via Verkle / KZG witnesses

Statelessness reduces validator state by moving state into witnesses [^3]. Each transaction carries a witness proving the state it touches. Validators only verify witnesses; they don't store full state.

EvaporChain differs in:

- **Statelessness re-distributes the cost; decay reduces it**: someone, somewhere, holds the full state in stateless designs.
- **Decay and statelessness are complementary**: an EvaporChain validator can be stateless, and Frontier Paper 2's energy-annotated Verkle trie reduces witness size for cold regions.

The two approaches address different layers and are not mutually exclusive.

### 8.4 Sui's object lifecycle

Sui's object model has explicit creation and consumption [^4]. Objects exist until consumed by a transaction. The model is more lifecycle-aware than Ethereum's account model but does not provide automatic decay; objects persist until explicitly removed.

EvaporChain differs in:

- **Automatic vs explicit lifecycle management**: EvaporChain requires no per-object cleanup transaction; decay handles this.
- **Energy budget vs binary alive/dead**: EvaporChain objects have continuous energy values usable for application logic (e.g., "if energy > X, allow operation").

A possible future direction: a Move-language extension with energy-decay primitives, exposing EvaporChain-like semantics to Sui developers. This is research, not implementation.

### 8.5 Filecoin / Arweave / IPFS

Filecoin and Arweave are storage networks with explicit pricing for permanence. Filecoin has time-bounded storage deals; Arweave has one-time payment for "permanent" storage with an endowment model.

EvaporChain differs in:

- **Storage networks are external to the chain**: clients pay storage networks separately. EvaporChain prices storage natively in its state model.
- **Application-layer coupling**: an EvaporChain object can directly depend on its own decay; a Filecoin file does not interact with smart-contract logic.

### 8.6 Summary

No production blockchain currently provides energy-decay state management as a primitive. The closest analogues are state expiry (still unshipped on Ethereum) and Sui's object lifecycle (explicit, not automatic). EvaporChain's contribution is the combination of automatic decay, dual cryptographic commitment, and explicit fee-priced persistence.

---

## 9. Discussion

### 9.1 What energy-decay state does well

- **Bounded steady-state size**: $|S^*|$ is determined by sustained refresh demand, not by cumulative write history.
- **Internalised storage cost**: writers pay for the lifetime of the state they create; the externality of Paper 2 §1.2 is removed.
- **Cryptographic provenance for evaporated state**: the MMR preserves an audit trail without preserving the data.
- **Application alignment with natural data lifecycle**: most application state has a natural lifetime; decay matches it.

### 9.2 What energy-decay state does not solve

- **MEV**: separate problem, addressed by encrypted mempool (orthogonal).
- **Sharding**: separate problem, addressed by EvaporChain's sharding crate (orthogonal).
- **Finality latency**: addressed by Tendermint's block time, not affected by decay.
- **Cross-chain composability**: bridging an EvaporChain object to a chain without decay is non-trivial; the bridge spec must handle the case where the bridged-from object evaporates while the bridged-to representation persists. Open work.

### 9.3 The composability question

A contract whose state has evaporated is non-functional. Contracts that depend on each other must coordinate refresh, or accept that dependent state may become unavailable. The protocol provides:

- **`on_grace` hook**: fires when an object enters Grace; allows the contract to take refresh action or notify owner.
- **`on_evaporate` hook**: fires when an object is about to be evaporated; last-chance refresh.
- **Resurrection mechanism**: restores ghost-state if needed.

These are partial responses to the composability problem. A complete formal account of contract-composability under decay is open work.

### 9.4 Adoption

Energy-decay state requires applications to explicitly choose their own $\tau$ values and pay corresponding fees. Existing applications written assuming permanent state cannot be ported without modification.

We expect adoption to come first from new application categories where the natural data lifecycle is finite — IoT telemetry, oracle feeds, ephemeral game state, GDPR-compliant data, derivative and option contracts that expire by definition. Once these categories are demonstrated, other applications can migrate.

This is an adoption-strategy question, not a technical one. Technical correctness does not guarantee adoption; the protocol must also be operationally robust (covered by Paper 2 §8.2 on political economy).

---

## 10. Conclusion

We have presented energy-decay state management: a protocol primitive in which state has a half-life, and the chain shrinks rather than grows when usage declines. The mechanism is implemented in EvaporChain. The formal definition is in §2; the lifecycle in §3; the cryptographic commitments in §4; the economics in §5; the consensus integration in §6.

The contribution is not novel cryptography — Verkle trees, MMR accumulators, BLS signatures, and integer arithmetic are all standard. The contribution is the *combination* of these into a complete state-management protocol with explicit energy accounting.

The companion paper (Paper 2) argues that this combination is not merely a design preference but a structural requirement for blockchain economic sustainability. This paper provides the constructive proof that the combination is implementable.

Open work spans the formal-verification mechanization (§7.3), the resurrection-across-anchors formalisation, the composability of decay-aware smart contracts, and adoption-strategy questions outside the protocol itself. Each is a separate research thread; this paper provides the foundation they build on.

---

## References (preliminary)

[^1]: Buterin, V., et al. *State Expiry and EIP-7745 / 4444*. Ethereum Magicians threads, 2018-2024.

[^2]: Solana Foundation. *Rent and Rent Exemption*. Solana documentation, 2020-2023.

[^3]: Buterin, V., Ben-Sasson, E., Gabizon, A. *Verkle Trees and Statelessness*. Ethereum research, 2021-2024.

[^4]: Mysten Labs. *Sui Object Model*. Sui documentation, 2022-2024.

[^5]: Lamport, L. *Specifying Systems*. Addison-Wesley, 2002.

[^6]: Yu, Y., et al. *Model Checking TLA+ Specifications*. CHARME 1999.

[^7]: Aumasson, J.-P., Neves, S., Wilcox-O'Hearn, Z., Winnerlein, C. *BLAKE3*. 2020.

[^8]: Boneh, D., Lynn, B., Shacham, H. *Short Signatures from the Weil Pairing*. Journal of Cryptology, 2004.

[^9]: NIST. *FIPS 204: Module-Lattice-Based Digital Signature Standard*. 2024.

[^10]: Kothapalli, A., Setty, S., Tzialla, I. *Nova: Recursive Zero-Knowledge Arguments from Folding Schemes*. CRYPTO 2022.

[^11]: Boneh, D., Bunz, B., Fisch, B. *Batching Techniques for Accumulators with Applications to IOPs and Stateless Blockchains*. CRYPTO 2019. (MMR / RSA accumulators)

[^12]: Companion paper: Singh, S. *State Economics: Why Infinite-State Blockchains Are Economically Unsustainable*. EvaporChain corpus, 2026.

[^13]: Companion paper: Singh, S. *Rule-Based Consensus for Time-Dependent State*. EvaporChain corpus (Frontier Paper 3), 2026. (in flight)

---

## Appendix A — Glossary of symbols

| Symbol | Meaning |
|---|---|
| $\mathcal{O}$ | Set of state objects |
| $o$ | An individual state object |
| $E_0$ | Initial energy at last refresh |
| $E(o, t)$ | Current energy at epoch $t$ |
| $\tau$ | Half-life in epochs |
| $\Delta$ | Elapsed epochs since last refresh |
| $g$ | Grace period (default 5 epochs) |
| $A$ | Anchor interval (default 100 epochs) |
| $\mathrm{LazyEnergy}$ | The integer-arithmetic decay function |
| $\text{Active}(t), \text{Ghosts}(t)$ | The active and ghost sets at epoch $t$ |
| $f_{\text{create}}, f_{\text{refresh}}, f_{\text{resurrect}}$ | Operation fees |
| $f_{\text{base}}(t)$ | EIP-1559-style base fee at $t$ |

## Appendix B — Open questions

- The optimal calibration of $c_{\text{energy}}$ in the create fee (relates to the discount rate).
- The right resurrection-fee fraction (currently 60% of original create cost; empirical justification open).
- Cross-chain bridging semantics for evaporable state.
- Composability formalism for decay-aware contracts.
- Mechanized proof in Coq / Lean of the `LazyEnergy` composition bound.
- Empirical performance benchmarks at scale (separate paper, "Paper 3: Benchmarks" in `REMAINING_WORK.md`).

---

**End of draft v0.1.**

Notes for revision (do not publish):
- Bibliography is preliminary; expand with specific arXiv IDs and DOIs in v0.2.
- §7.2 attack analyses are sketches; expand to formal arguments with explicit cost calculations in v0.2.
- Numerical examples (defaults like $g = 5$, $A = 100$) should be verified against the most recent `genesis-mainnet.json` and `crates/evaporchain-node/src/main.rs` constants before submission.
- Reviewer 2 will ask about the choice of $\tau = \infty$ representation for Immortal objects — formalise this as a separate state machine in Appendix C in v0.2.
- This paper assumes Paper 2 (economics) and Frontier Paper 3 (Rule-Based Consensus) are available. If submitted standalone, §6 would need to be self-contained.
