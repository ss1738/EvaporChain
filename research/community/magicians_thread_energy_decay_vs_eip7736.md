# Ethereum Magicians Thread Draft

**Purpose:** post this on forum.ethereum.org to establish technical presence in the state-expiry conversation.
**Timing:** post this week. Refine as needed but don't over-polish — Magicians culture prefers substance over polish.
**Where to post:** forum.ethereum.org → "Cat Herders" or "ARC (Application Layer)" section → likely "EIP" subcategory under EIP-7736 discussion thread, or as a new thread if no active discussion exists.
**Tags to use:** `eip-7736`, `state-expiry`, `verkle`, `statelessness`, `research`
**Status:** drafted 2026-05-13, not yet posted

---

## Suggested Title

> **Continuous decay as a generalisation of binary leaf-expiry — design tradeoffs vs EIP-7736**

(Alternative titles if the above feels too aggressive: *"Comparing continuous energy-decay and binary leaf-expiry primitives"*, or *"EIP-7736 follow-up: continuous-time vs threshold-time state expiry"*)

---

## Body

Hi all,

Reading EIP-7736 (leaf-level state expiry in Verkle, @gballet @wei-han-ng, July 2024) and the related partial-state-node thread from @vbuterin, I've been working on a related primitive that generalises the binary expiry semantics into a continuous decay function. I'd like to compare design tradeoffs and surface some open questions.

This isn't an EIP proposal — it's a comparison post to inform the existing state-expiry conversation. I have a reference implementation but the chain it lives in is separate context; the primitive itself is what I'd like to discuss.

### The core idea

EIP-7736 expires a leaf at a fixed threshold epoch. Below that threshold the leaf is active and pays standard gas; at the threshold it transitions to expired and requires a resurrection proof to access.

The generalisation I've been studying:

```
weight(leaf, epoch) = base_energy * exp(-decay_rate * (epoch - last_touched))
```

Each leaf carries a per-leaf `last_touched` epoch and an explicit `decay_rate` parameter. The leaf's effective weight in the state commitment decays continuously rather than transitioning binarily. Resurrection becomes "refresh" — paying gas to push `last_touched` forward, restoring full weight.

Concretely, this means:

- A leaf accessed at `epoch + 100k` with `decay_rate = log(2)/1M` has half its original weight contributing to the Verkle commitment
- The same leaf at `epoch + 5M` has ~1/32 weight — accessible but heavily decayed
- The leaf at `epoch + 10M` has near-zero weight; resurrection is required for any meaningful weight

This is structurally compatible with Verkle: the per-leaf decay is computed deterministically from `(last_touched, current_epoch, decay_rate)` and folded into the leaf's contribution to the parent commitment via the standard Pedersen homomorphism. No new cryptographic machinery needed.

### Specific comparisons against EIP-7736

| Property | EIP-7736 (binary) | Continuous decay |
|---|---|---|
| Activation transition | Threshold epoch → discontinuous | Continuous; weight monotonically decays |
| Resurrection proof | Single inclusion proof for expired leaf | Equivalent inclusion proof; "refresh" updates `last_touched` |
| Gas semantics | Standard until threshold, then refresh-fee | Decay-aware: cost scales with weight (or flat with explicit refresh) |
| State commitment | Active leaves contribute fully; expired leaves contribute zero | All leaves contribute scaled by decay; commitment updates lazily |
| Gaming surface | Touch-near-threshold loophole | Smoother but introduces decay-rate-selection question |
| Implementation complexity | Lower — binary state machine | Higher — per-leaf decay math, but constant-time |
| UX (developer model) | "Things expire" — clean semantic | "Things decay" — closer to physical/economic intuition |

### Three open design questions I'd value input on

1. **Resurrection vs continuous refresh.** EIP-7736 makes resurrection a discrete event (proof + payment). Continuous decay opens the possibility of "graduated refresh" — pay a fraction to restore a fraction of weight, with the option of full refresh. Is this an attractive UX/economic primitive, or just complexity for its own sake?

2. **Decay-rate selection.** EIP-7736 effectively encodes one decay rate (binary, after threshold). Continuous decay forces an explicit `decay_rate` per leaf — leaving it up to the application designer. Should this be application-controlled, governance-controlled, or globally fixed? @vbuterin has noted concerns about governance-controlled state-expiry parameters; does continuous-decay's per-application parameter avoid or amplify that concern?

3. **Adversarial models.** With binary expiry, the adversarial surface is "touch state right before threshold to delay resurrection cost." With continuous decay, the adversarial surface is different — possibly easier (no cliff to exploit) or possibly harder (decay-rate-gaming attacks). I have a partial analysis but haven't found prior work specific to continuous-decay state. Pointers?

### What I'm not claiming

- This is not a proposal to replace EIP-7736 in Ethereum's roadmap. EIP-7736 is well-targeted at the Verkle migration and the binary semantics are easier to reason about.
- This is not advocating for a specific decay rate. The math allows any rate; the rate is the application's parameter, not a protocol decision.
- This is not pitching a chain or a token. I have a working reference implementation but the chain's separate context isn't what I'd like to discuss here.

### Where the work lives

The continuous-decay primitive lives in a research codebase that I'm preparing to publish more formally. I'm separately preparing an IETF Internet-Draft `draft-singh-state-decay-00` and a paper submission for FC 2027 / SBC 2027 / RWC 2027.

If anyone is interested in the implementation specifics — particularly the Verkle commitment update math under continuous decay, or the resurrection-vs-graduated-refresh tradeoff — happy to share more in this thread.

Tagging @gballet @wei-han-ng @dankrad @vbuterin since you've been most active on the state-expiry primitives. No expectation of a reply — but if you have a moment, I'd particularly value pointers on (1) adversarial-model literature I might be missing and (2) whether the per-application decay-rate parameter strikes you as feature or footgun.

Thanks for any input.

---

## Posting checklist

Before posting:

- [ ] Verify EIP-7736 author handles are current (@gballet @wei-han-ng) — these are based on April-2024-era Magicians activity. If they've changed handles since, update.
- [ ] Skim the most recent EIP-7736 Magicians thread for current open questions, and weave 1-2 into your 3 questions if relevant. Live citations beat stale ones.
- [ ] Read @vbuterin's most recent state-expiry / partial-state-node posts (verify the "governance-controlled parameter" quote you reference exists — paraphrase or cite directly).
- [ ] Verify the Pedersen-homomorphism claim ("the per-leaf decay is computed deterministically ... folded into the leaf's contribution to the parent commitment") — this is correct for additive blinding but may need a more precise statement about the actual Verkle commit operation. Reviewer-grade detail matters here.
- [ ] Pick the title — recommend "Continuous decay as a generalisation of binary leaf-expiry — design tradeoffs vs EIP-7736" unless current Magicians discussion suggests a softer title.
- [ ] Account: post from a real account associated with your name (`satyawan.singh`, `ssingh`, or similar). Don't use a pseudonymous handle — credibility is from the real-name technical contribution.

## After posting

1. **Within 24h**: reply to your own thread with a small worked example (concrete numbers — decay over 1M / 5M / 10M epochs with specific decay rate). Substance comes from worked examples, not just framing.
2. **Within 48h**: cross-post the link on Farcaster tagging @vitalik.eth @dankrad @gballet (sparingly, one Farcaster post is enough).
3. **Within a week**: if @gballet or @wei-han-ng has replied, follow up substantively — engage their specific points, don't just thank them.
4. **Long-term**: every 4-6 weeks, post a follow-up with new technical findings. The thread becomes a research log. This pattern is what gets cited.

## What this thread is NOT for

- ❌ Mention EvaporChain by name (research framing only)
- ❌ Link to evaporchain.com (signals competitor-chain agenda)
- ❌ Promote EVP, mainnet plans, tokenomics
- ❌ Ask for EF grant
- ❌ Pitch enterprise integration
- ❌ Compare to Solana / Sui / Aptos state models

This thread is purely about the continuous-vs-binary state-expiry primitive comparison. Anything else dilutes the technical signal and triggers the "competing-L1 advocacy" filter that EF reviewers have.

## Expected outcomes

| Outcome | Probability |
|---|---|
| Zero replies, dies quietly | ~30% |
| 1-3 replies from community, no big names | ~40% |
| @gballet or @wei-han-ng reply substantively | ~15% |
| @vbuterin or @dankrad engage | ~10% |
| Gets cited in next EIP-7736 update or related EIP | ~5% |
| Sparks a working group discussion | ~3-5% |

Even the "zero replies" outcome has value — the thread exists on Magicians forever as a citable artifact for IETF draft + Paper 1.

## Cross-references

- IETF draft to follow: `draft-singh-state-decay-00`
- Paper to follow: state-decay paper for FC 2027 / SBC 2027
- Repo strategic doc: `research/INEVITABILITY_STRATEGY.md`, `research/APPLICATION_UNIVERSE.md`
