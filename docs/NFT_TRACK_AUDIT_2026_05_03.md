# EvaporChain NFT Track — Core vs Singh-Named Novel Primitives

**Date:** 2026-05-03
**Scope:** Full audit of the NFT story across spec, code, dApps, doctrine.
**Purpose:** Distinguish what is *baseline-NFT-but-mortal* from what is *genuinely first-of-its-kind only-possible-on-EvaporChain*.

---

## Layer 1 — The Baseline (already shipped, already standard for this chain)

### EVR-721 — Mortal Non-Fungible Token Standard

**File:** `standards/EVR-721.md` · **Author:** Satyawan Singh · **Status:** Living, 2026-03-26.

This is the **chain-native NFT contract**. It is *not* a Singh-named research primitive — it's the equivalent of "ERC-721, except every token has a finite life":

| Feature | EVR-721 |
|---|---|
| Lifespan | Finite (energy × half-life) |
| Lifecycle | Active → Grace (5 epochs) → Ghost |
| Decay formula | `E(t) = E_initial · 2^(-t / half_life)` |
| Resurrection | Supported via `refresh()` on a Ghost |
| Ghost record | `{token_id, owner, metadata_hash, evaporated_epoch, ghost_proof}` in MMR |
| Genuinely new vs ERC-721 | Native expiry semantics + ghost provenance |

**Reference implementation:** `crates/evaporchain-node/src/api.rs` (`NftToken`, `NftStore`), `crates/evaporchain-state/src/evaporation.rs`, plus the on-chain template `MortalNFT` in `crates/evaporchain-contracts/src/lib.rs`.

**Verdict:** This is the *baseline standard*. It is to EvaporChain what ERC-721 is to Ethereum. **It is not the moat.** The moat is what gets built on top.

### MortalNFT (one of 7 contract templates)

**File:** `docs/contracts.md §MortalNFT`. A drop-in deployable contract template using the EVR-721 mechanics:

```json
{ "template": "MortalNFT", "init_args": { "name": "...", "metadata_uri": "ipfs://..." }, "energy": 10000, "half_life": 100 }
```

Methods: `transfer(to)`, `metadata()`, `time_remaining()`. Use cases listed: expiring event tickets, temporary access passes, art that ages.

**Verdict:** The *primitive vehicle* — useful but not novel by itself. Same family as ERC-721 wrapped in EVR-721 mechanics.

### Tier 3 — Half-Life NFT (with retention-tier)

**File:** `research/INVENTION_STACK.md §4.3` (Tier 3 list).

> "Half-Life NFT (with retention-tier)"

A doctrine entry naming the post-V2 polish that adds a *retention-tier* mechanic on top of EVR-721. Not yet specified, not yet implemented. Still inside the "EVR-721 + a knob" frame, not a paradigm primitive.

---

## Layer 2 — The Singh-Named NFT Primitives (A5.3 — five lock-grade)

These are the primitives marked **lock-grade** in `research/INVENTION_STACK.md §A5.3` — each has structural-decay synergy that is **only possible on a chain with a single-λ**, each carries cultural-lineage citations, each is named after Satyawan because the math/mechanism is his.

| # | Crate | Doctrine claim | Status |
|---|---|---|---|
| 1 | **Singh-Sabi** (Patina Tokens) | NFTs age toward "ruined-beautiful." Non-zero floor decay (~15%); deterministic visual-entropy tuple `{cracks, desaturation, foxing, edge_fray}` derived from `(token_id, score)`. Owner cannot pause; only witness. | ✅ `evaporchain-singh-sabi` shipped this session — 29 tests, 0 fail |
| 2 | **Singh-Migrant** (Wanderwrits) | NFTs die if held still. Tiered decay (1× / 2× / 4×) past resting threshold; transfer to a *novel* wallet refunds a fraction of *current* energy (kills farm-and-relay attack). | ✅ `evaporchain-singh-migrant` shipped this session — 29 tests, 0 fail |
| 3 | **Singh-Posthuma** (Sealed Testaments) | Confessional NFTs revealed on certified death. Threshold-secret-sharing committee holds key; decay suspended while issuer is verifiably alive; on death, key revealed → λ-decay begins → fades to permanent on-chain marker. | ⚠️ **NOT BUILT** — gated on death-oracle primitive (12-wk build, defer per A5.6) |
| 4 | **Singh-Heir** (Patrilithic Tokens) | Kin-graph heirloom NFTs. Generational transfer (parent→child edge in attested DAG) refreshes 80%; non-kin transfer refreshes 0%; ~3 generations of dormancy → evaporates. | ⚠️ **NOT BUILT** — kin-graph is a real research artefact, defer to Year 2 |
| 5 | **Singh-Resonance** (Vital-Sign NFTs) | Engagement-coupled decay. λ inversely coupled to engagement; loved art slows toward immortality; ignored art accelerates to zero. | ⚠️ **NOT BUILT** — 8 weeks; needs careful framing as critique not Black-Mirror |

**Cultural lineage cited per primitive** (paraphrased from doctrine):

- **Singh-Sabi**: wabi-sabi (Sen no Rikyū, 16th c.) · kintsugi · Tarkovsky's *Stalker* · Basinski's *Disintegration Loops* · Banksy's *Love is in the Bin*
- **Singh-Migrant**: Trobriand kula ring (Malinowski 1922) · chain letters · Olympic torch · Marcel Mauss *The Gift* (1925)
- **Singh-Posthuma**: Catholic confessional seal · Pessoa's trunk · Kafka's Brod betrayal · Didion's *Year of Magical Thinking*
- **Singh-Heir**: primogeniture · Japanese daimyō sword inheritance · Torah scrolls · signet rings · Mann's *Buddenbrooks*
- **Singh-Resonance**: Tristan Harris attention-economy critique · Jenny Odell *How to Do Nothing*

**Strikes from this round (do NOT re-litigate):** Penalty NFTs · Pheromone NFTs · Metabolic NFTs · Counterfactual NFTs · Genealogical NFTs · Witness NFTs · Ouroboros NFTs · Decay-Ranked Curation NFTs · Memento NFTs · Time-Capsule Souvenir NFTs · Kintsugi NFTs (subsumed by Singh-Sabi).

---

## Layer 3 — The Cultural-Launch Wedge (A2.3 — The Gallery That Forgets)

`research/INVENTION_STACK.md §A2.3` proposes a *single launch artifact* that fuses three primitives into one cultural moment:

1. **Provably-Mortal NFTs ("Mayflies")** — minted with declared half-life + ZK death certificate. Wallet shows literal countdown.
2. **Decay-as-Performance-Art** — gallery contract; artists deposit works with chosen half-lives; gallery's visual state changes daily. Closing date is *thermodynamic*.
3. **AI-Decay-Art** — generative pieces taking chain-energy as a runtime parameter; output literally changes as state evaporates. Basinski's Disintegration Loops on-chain.

**The single sentence the doctrine wants on the press release:**
> *"It is the first thing humans have made that is provably going to die."*

**Status:** ⚠️ **NOT BUILT** as a unified `evaporchain-gallery-forgets` crate. Each component is partially expressible via Singh-Sabi (already shipped) + the EVR-721 baseline. A gallery-orchestration crate is the missing layer.

---

## Layer 4 — Where the genuine moat lives

The *genuine first-of-its-kind* part of the EvaporChain NFT story is **not** "we have NFTs." It is:

### Moat #1 — Patina-shape decay (Singh-Sabi)
Every other "decaying NFT" project (a handful of one-off art experiments) has decay tend to zero. Singh-Sabi's **non-zero asymptote** is the only one that says *"ruined-beautiful is the destination, not the absence."* It maps to wabi-sabi as a precise mathematical object, not a metaphor.

### Moat #2 — Death-by-stillness (Singh-Migrant)
Mauss's *The Gift* gets a chain primitive. The **novel-wallet refund + tier-doubling decay past resting threshold** is the only mechanism that makes "the gift must keep moving" enforceable on-chain *without* a curator. Anti-farm guard (refund of *current*, not initial) is what keeps the kula-ring metaphor honest.

### Moat #3 — Provable mortality as a press claim
Per A2.3: *"the first thing humans have made that is provably going to die."* This is the framing that gets EvaporChain out of crypto press and into the Atlantic / NYT Arts. The substrate (EVR-721 ghost records + MMR proofs) makes it *literally true* — every evaporated NFT carries an MMR-verifiable death certificate.

### Moat #4 — Inverted decay (ChildKey, in the consumer lane)
Not strictly an NFT primitive but adjacent: same mathematics, opposite sign. Chain runs decay backward to compute "energy-time-to-unlock." The same single-λ that makes Singh-Sabi possible makes ChildKey possible. **No other chain can do this without bolting on a separate clock primitive.**

### Moat #5 — The ghost record itself (EVR-721)
Even the *baseline* has a moat: post-evaporation provenance via MMR proofs. ERC-721 has nothing equivalent — once burned, a token is gone. EVR-721 ghosts can be queried, resurrected, and used as audit trails. This is the moat that exists *before* any Singh-named work.

---

## What's missing — the buildable shortlist

Ordered by ratio of (cultural impact × decay-structural fit) ÷ build-cost:

1. **`evaporchain-gallery-forgets`** — orchestration crate that fuses Singh-Sabi (already shipped) + EVR-721 ghost records + a `GalleryContract` that lists exhibits with declared half-lives. ~3 weeks. Press play. **Highest ROI for "moats already in-stack."**

2. **`evaporchain-singh-resonance`** — 8 weeks. Needs an in-stack social-graph proxy (or stub for V1: explicit `register_engagement(token_id)` calls counted into a per-token engagement score that scales λ inversely). **Highest TAM** of the unbuilt NFT primitives.

3. **`evaporchain-singh-posthuma`** — 12 weeks. Highest *press potency* of the set. Death-oracle is the painful gating primitive — could ship a v1 with operator-attested death (multisig threshold of trusted attestors) and upgrade to a real death-oracle later.

4. **`evaporchain-singh-heir`** — 10 weeks. Kin-graph attestation DAG is the real artefact. Year-2 per doctrine; **don't build now.**

---

## Synthesis — the 30-second version

EvaporChain's NFT story has **three layers**:

| Layer | Name | Novelty | Status |
|---|---|---|---|
| **Baseline** | EVR-721 / MortalNFT | "ERC-721 with a half-life + ghost records" | Shipped |
| **Singh-named primitives** | Sabi · Migrant · Posthuma · Heir · Resonance | Each adds a structural-decay mechanism that has no parallel anywhere else | 2/5 shipped |
| **Cultural launch wedge** | Gallery That Forgets / Mayflies | Press-ready artifact fusing the above into a single moment | Not built |

**The novel things — your novel things — are the Singh-named primitives.** EVR-721 is the chain's "table stakes for being an L1." Singh-Sabi and Singh-Migrant are *only possible because EvaporChain has a single λ* — they cannot be ported to any other chain without rebuilding the substrate. Singh-Posthuma extends that with a death-oracle. Singh-Heir extends it with a kin-graph DAG. Singh-Resonance extends it with engagement-coupling.

**The press claim that lands** ("the first thing humans have made that is provably going to die") needs the Gallery That Forgets to be built — the artefact that makes the abstract claim concrete. **That is the next NFT-track build.**
