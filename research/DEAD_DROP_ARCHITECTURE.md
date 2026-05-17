# Dead Drop — Self-Destructing On-Chain Data Architecture

> Flagship demo #2 for EvaporChain's energy-decay primitive: the
> *inverse* of immutability. Sibling of `SFSV_ARCHITECTURE.md` and
> `EVAPORCASH_ARCHITECTURE.md`; same doctrine, same substrate. Pairs
> with `contracts/evaporscript/dead_drop.es` (to build) and the
> `crates/evaporchain-dead-drop` mirror crate. Cheapest-to-build,
> highest watch-it-die visceral factor of the flagship set.

---

## 0. TL;DR for cold readers

Dead Drop lets you post a payload that the chain **physically forgets**
after its energy decays — not encryption, not access-control, not a
"please delete" request. The bytes leave chain state and become
**unrecoverable by protocol law**. Ethereum's defining property is
*immutable forever*; Dead Drop is the demonstrable opposite, on the
one chain where forgetting is a physical law rather than a retrofit
(contrast Ethereum's late-2026 Hegota state-expiry *bolt-on*).

It is the single most legible demo of the thesis sentence —
*"data without a half-life is a bug, not a feature"* — to a
non-crypto audience: a live counter, a payload, watch it evaporate,
try to read it, it is gone, on every node, forever. **UVP filter
(`APPLICATION_UNIVERSE.md` cat 9) passes maximally: an immutable chain
structurally cannot do this.** Solo-build budget: ~1.5–2 weeks
(simplest of the flagship set; no fungible accounting, no SDDC).

---

## 1. Mission & Doctrine Anchor

### 1.1 What Dead Drop is

A payload registry where each entry is a state object with an energy
budget *E* and the chain half-life *τ*. The entry is **readable while
Active**, **degraded in Grace**, and **gone once the evaporation
engine retires it** (Active → Grace → Ghost → Evaporated — the
chain's existing object lifecycle, not new machinery). Forgetting is
the *default*; remembering requires explicit, gas-paid `extend()`.
The protocol guarantees the negative: absent refresh, the content is
provably unrecoverable.

### 1.2 What Dead Drop is **not**

- **Not** encryption / disappearing-message UX. Encrypted data on an
  immutable chain is still *there* (harvest-now-decrypt-later, quantum
  later). Dead Drop deletes the **ciphertext-or-plaintext bytes
  themselves** from state. That distinction is the entire pitch —
  never frame it as "Signal on-chain."
- **Not** access control. There is no ACL that could be bypassed; the
  data does not exist post-evaporation to be accessed.
- **Not** archival/IPFS-pinning. The opposite: it guarantees
  *non-persistence*. Pinning it elsewhere is the user's choice and
  outside the protocol claim.
- **Not** a payment app/bank/game (the rejected suggestions) — those
  fail the UVP filter. Dead Drop is specifically the *forgetting*
  showcase.

### 1.3 Why this is a flagship demo

`evaporchain_application_universe.md` cat 9 (privacy + messaging:
self-destructing channels / whistleblower). Highest
impact-per-build-hour of the flagship set and the most instantly
graspable by a lay audience — the demo *is* the thesis. Where SFSV
shows "value released by decay" and EvaporCash "value preserved by
circulation," Dead Drop shows the rawest claim: **state that knows
when to die.**

---

## 2. The Two Primitives at Play

Even more minimal than SFSV (no SDDC market; no fungible ledger).

### 2.1 Energy-Decay + Evaporation Engine (Layer 0 + State)

The payload object's life is governed by `energy_at_epoch` (invariant
#1; never re-derived — Layer 0 lint) and the chain's existing
`Active → Grace → Ghost` evaporation engine in `evaporchain-state`.
Dead Drop adds **no new lifecycle machinery** — it is a thin `.es`
contract that *names* a use for the object lifecycle the chain already
runs for every state object. On `on_evaporate` the contract asserts
content is cleared; the state-root no longer commits the bytes, so a
node retaining them forks itself out (see §7.1).

### 2.2 EvaporScript VM (Layer 3)

`dead_drop.es` is the source of truth (invariant #2). No internal
method dispatch — the expiry computation is inlined into `read`,
`expires_at`, and the hooks, parity-pinned by
`tests/dead_drop_parity.rs` (port of `predicate_inlining_parity.rs`).
Read returns the **live** `energy_at_epoch`-gated availability — never
a frozen formula (the model-(a) lesson from
`VERIFICATION_2026_05_16.md`).

---

## 3. State Machine & Lifecycle

```
        post(payload, ttl_epochs)  → id, energy E sized so that
                                      energy_at_epoch(E, ttl) ≈ ε
                      │
                      ▼
   ┌──────────── Active ────────────┐   read(id) → bytes  ✅
   │  self.energy ≥ read_floor       │
   │  extend(id): refresh energy →   │   (pay-to-remember; the ONLY
   │    pushes expiry out            │    way to defeat forgetting)
   └───────────────┬─────────────────┘
                    │ energy < read_floor
                    ▼
   ┌──────────── Grace ─────────────┐   read(id) → ⚠ "fading"
   │  content still present but the  │    (degraded; on_grace emitted)
   │  contract refuses fresh reads   │
   └───────────────┬─────────────────┘
                    │ evaporation engine retires the object
                    ▼
   ┌──────── Ghost → Evaporated ────┐   read(id) → ✗ "forgotten"
   │  payload bytes pruned from      │    content unrecoverable;
   │  state; only a content-free     │    tombstone = blake3(id) only,
   │  tombstone may remain           │    NEVER the payload
   └─────────────────────────────────┘
```

`extend()` is the deliberate, bounded escape hatch (contrast SFSV
where refresh postpones a *release*; here refresh postpones
*forgetting*). Absent `extend`, forgetting is guaranteed — that
guarantee is the product.

---

## 4. Mathematical Foundation

### 4.1 Expiry function

```
available(id, e) = energy_at_epoch(E, e − posted_at) ≥ read_floor
```

Same canonical decay as SFSV §4.1 / EvaporCash §4.1. The deployer
sizes `E` and reads `ttl` off the inverse:

```
E ≈ read_floor · 2^( ttl / τ )
```

— the only arithmetic the deployer does; the contract itself never
re-derives decay (invariant #1).

### 4.2 The forgetting guarantee

Define *forgotten(id, e)* ≡ the payload bytes are absent from the
state committed by the block at height for epoch *e*. Claim:

```
∀ e ≥ evaporate_epoch(id):  forgotten(id, e)  ∧  ¬∃ revive API
```

Unlike SFSV (which has no revival) and EvaporCash (whose decay is
recoverable via touch), Dead Drop's evaporation is **terminal by
construction**: there is no `revive`, and `extend` is only callable
*while Active* (cannot resurrect an Evaporated entry). Candidate Coq
obligation `research/coq/DeadDropForgetting.v` (not yet written):
post-evaporation state-root excludes the payload pre-image — corollary
of the evaporation-engine pruning lemma + state-root binding.

### 4.3 Conservation

Trivial here (no fungible value): the only conserved quantity is gas
paid for `post`/`extend`, settled by the normal fee path. No
EvaporCash-style pool.

---

## 5. Contract Surface (`.es`)

```
fn post(payload: bytes, ttl_epochs: u64)  -> u64   # returns id; sizes energy from ttl
fn extend(id: u64)                                  # Active-only; refresh → push expiry
fn purge(id: u64)                                   # voluntary early forget (owner-only)

# Read-only (each inlines the SAME energy_at_epoch availability check)
fn read(id: u64)            -> bytes   # Active only; Grace/Evaporated → revert
fn expires_at(id: u64)      -> u64     # epoch at which available() crosses read_floor
fn is_forgotten(id: u64)    -> bool
fn poster_of(id: u64)       -> address

# Lifecycle hooks
on_grace()      -> emit("payload fading")
on_evaporate()  -> emit("payload forgotten")   # asserts content cleared
```

`read`, `expires_at`, `is_forgotten` inline the identical
availability expression (no method dispatch) — drift caught at PR time
by `tests/dead_drop_parity.rs`. Payload size is bounded by the node's
64 KB `source_code`/args cap (verified this session); large payloads
post a `blake3` commitment + off-chain blob, on-chain bytes for the
≤64 KB demo path.

---

## 6. Connect with mainnet

Every interface below was **verified live this session**
(`VERIFICATION_2026_05_16.md`); Dead Drop uses the identical node
contract as the live-verified `deploy-sfsv.sh`.

### 6.1 Deploy

`POST /api/tx/deploy-script` —
`DeployScriptRequest { deployer: u8, source_code, energy, half_life }`.
`deployer = 0` (genesis-funded faucet account; `addr_from_byte(0)`)
clears the balance pre-check with no admin-gated faucet. Poll
`GET /api/tx/:hash` → `.contract_id` once `finalised`.

### 6.2 Call

`POST /api/tx/call-script` —
`CallScriptRequest { caller: u8, contract_id: u64, method, args:
Vec<evaporchain_script::Value>, epoch: u64 }`. `args` externally
tagged: `post(payload, ttl)` → `[{"Str": "<payload>"} or
{"Address"/"U64"...}, {"U64": ttl}]`; bytes via `{"Str": ...}` or a
tagged byte array per the `.es` param type. `epoch` **required**.
Auth: register (testnet auto-verifies) → login → bearer token.

### 6.3 Observe — the demo's centrepiece

`GET /api/script/:id` (the **script** engine store, *not*
`/api/contract/:id` — the endpoint error corrected in `b76df4a2`).
The live "watch it die" panel polls this each epoch: `.state` shows
the payload present → fading → **absent**, and `energy` ticking down
toward `read_floor`. `GET /api/status` → `.epoch` for the countdown.
This single endpoint *is* the demo.

### 6.4 Runbook

`scripts/deploy-dead-drop.sh` — structural fork of the live-verified
`deploy-sfsv.sh`. Lifecycle proof sequence: deploy → `post` → read
(succeeds) → wait past `expires_at` → read (**reverts: forgotten**) →
`GET /api/script/:id` asserting the payload is **absent from
`.state`** (the strict, directly-observed assertion — the Dead Drop
analogue of SFSV's directly-observed `released:{Bool:true}`). A
non-vacuity guard: assert at least one *successful* read occurred
*before* expiry, so a green run cannot be a vacuous "never readable."

### 6.5 Invariant obligations (mainnet gate)

- Invariant #1: availability via `energy_at_epoch` only (Layer 0 lint).
- Invariant #2: `.es` source of truth; `crates/evaporchain-dead-drop`
  mirrors; `tests/dead_drop_parity.rs` pins equivalence.
- Forgetting guarantee (§4.2): no `revive`; `extend` Active-only;
  on_evaporate asserts content cleared. This is the one obligation
  unique to Dead Drop and the one the threat model centres on.

---

## 7. Threat Model

| # | Adversary | Defence |
|---|---|---|
| 7.1 | **Archival node** (refuses to prune; keeps payload) | The state-root commits the *post*-evaporation state; a node serving the old payload produces a divergent root and is forked out (the §8.4 State-Bloat-Spammer / evaporation-engine guarantee). Out-of-band copies the operator made are outside the protocol claim — the claim is precisely "the *chain* forgets," and that holds. State this honestly in the demo; do not overclaim "no copy can ever exist." |
| 7.2 | **Revival attack** (resurrect an Evaporated entry) | No `revive` API by construction; `extend` rejects non-Active ids (§4.2). Terminal by design — stronger than SFSV (no revival) and EvaporCash (touch-recoverable). |
| 7.3 | **Re-org past evaporation** | Energy is chain-state; a re-org rewinds the object's energy with it and re-evaluates availability deterministically at height *h* (SFSV §4.4 argument). A re-org cannot *un-forget* beyond its own depth, and finality (T0.1/T0.4) bounds depth. |
| 7.4 | **Read-DoS** (spam `read` near expiry) | `read` is gas-metered and pure; spamming burns the caller's gas, never extends or mutates the entry. |
| 7.5 | **Censor `post`/`extend`** | Standard mempool inclusion guarantees; censoring `extend` *accelerates* forgetting (fails safe — the protocol's bias is toward deletion, which is the desired direction). |
| 7.6 | **Quantum / harvest-now-decrypt-later** | The defining advantage: post-evaporation there is *no ciphertext to harvest*. This is Dead Drop's headline security claim vs every immutable chain. |

---

## 8. Doctrine Mapping

- **INEVITABILITY_STRATEGY:** the purest possible demonstration of
  *"data without a half-life is a bug, not a feature"* — the demo and
  the thesis are the same sentence.
- **APPLICATION_UNIVERSE cat 9** (privacy + messaging /
  self-destructing channels / whistleblower).
- **Doctrine primitives:** maps to Evaporated-Fork Certificates and
  Light-Cone pruning (`research/light_cone/`) — Dead Drop is their
  user-facing surface; the demo makes an internal consensus primitive
  legible to a lay audience.
- **Contrast artefact:** the cleanest side-by-side vs Ethereum Hegota
  state-expiry — *retrofit bolt-on* vs *protocol-native physical law*.

---

## 9. Reference Implementation Status

| Surface | State |
|---|---|
| `contracts/evaporscript/dead_drop.es` | **to build** (this spec) |
| `crates/evaporchain-dead-drop` mirror | to build (SFSV-class, smallest of the flagships) |
| `tests/dead_drop_parity.rs` | to build (port of `predicate_inlining_parity.rs`) |
| `tests/adversarial.rs` (§7) | to build (port of SFSV adversarial harness) |
| `scripts/deploy-dead-drop.sh` | to build (fork of live-verified `deploy-sfsv.sh`) |
| `research/coq/DeadDropForgetting.v` | candidate (not yet written) — §4.2 |
| Node API contract | **already verified live** — §6, no chain change |

No mainnet code change required: Dead Drop is pure EvaporScript over
the evaporation engine the chain already runs and this session
live-verified. Solo budget ~1.5–2 weeks, scoped — after the mainnet
sprint or a scoped parallel exactly as SFSV was, never at the sprint's
cost (`strategic_decision_2026_05_16_focus.md`).
