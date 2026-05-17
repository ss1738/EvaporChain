# Dead Drop — Self-Destructing On-Chain Data Architecture

> Flagship demo #2 for EvaporChain's energy-decay primitive: the
> *inverse* of immutability. Sibling of `SFSV_ARCHITECTURE.md` and
> `EVAPORCASH_ARCHITECTURE.md`; same doctrine, same substrate.
>
> **Contract: `contracts/evaporscript/mortal_message.es` (the canonical
> EvaporScript pilot — already shipped).** Dead Drop is the
> mortal-messages dApp *positioned* as the flagship "prove the chain"
> forgetting demo — NOT a new contract. Per invariant #2 the `.es` is
> the source of truth; a separate `dead_drop.es` would be a redundant
> near-duplicate of the proven pilot (the exact redundant-scaffold
> anti-pattern the SFSV strategic record warns against). The genuinely
> new artifact is the live-e2e runbook `scripts/deploy-dead-drop.sh`
> (this change) that *directly observes the forgetting* on a node.

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

`mortal_message.es` is the source of truth (invariant #2). No internal
method dispatch. Crucially, the contract does **not** hand-roll any
decay/expiry arithmetic at all — forgetting is the chain runtime
driving the instance Active→Grace→Ghost via its own `energy` builtin
(the `on_grace`/`on_evaporate` hooks just emit). That structurally
*avoids* the model-(a) frozen-formula trap (the
`VERIFICATION_2026_05_16.md` lesson) because there is no formula in the
contract to drift — the strongest possible form of the invariant.

---

## 3. State Machine & Lifecycle

One Dead Drop = one `mortal_message.es` instance. The chain runtime
(not contract code) drives the lifecycle off the instance's own
`energy` builtin; `set_payload` seals the content once.

```
   deploy mortal_message.es with energy=E, half_life=τ   (E,τ = the TTL)
                      │
              set_payload(body, recipient)   (owner-only, once)
                      │
                      ▼
   ┌──────────── Active ────────────┐   read() → body  ✅
   │  instance energy healthy        │   (caller == recipient || owner)
   │  on_refresh / record_boost:     │   (chain-applied energy refresh;
   │    chain pushes evaporation out │    the ONLY defer path, gas-paid)
   └───────────────┬─────────────────┘
                    │ energy decays (evaporation engine)
                    ▼
   ┌──────────── Grace ─────────────┐   on_grace → emit("energy low")
   │  instance still present         │   read() still works here
   └───────────────┬─────────────────┘
                    │ engine retires the instance
                    ▼
   ┌──────── Ghost → Evaporated ────┐   on_evaporate → emit; instance
   │  contract + state{body} cease   │   + state gone. GET /api/script/
   │  to exist; unrecoverable        │   :id no longer returns it.
   └─────────────────────────────────┘
```

The escape hatch is the chain's energy **refresh** (surfaced via
`on_refresh`/`record_boost`), not a contract method — contrast SFSV
where refresh postpones a *release*; here it postpones *forgetting*.
Absent refresh, forgetting is guaranteed by physics — that guarantee
is the product.

---

## 4. Mathematical Foundation

### 4.1 Lifespan function

The instance's own energy follows the canonical chain decay (SFSV §4.1
/ EvaporCash §4.1):

```
instance_energy(e) = energy_at_epoch(E, e − deploy_epoch)
                    = E · 2^(−(e − deploy_epoch) / τ)
```

`E` = deploy `energy`, `τ` = deploy `half_life`. The deployer picks
(E, τ) so the instance survives the intended read window then
evaporates — the only sizing arithmetic anyone does. The contract
contains **no** decay math (invariant #1, maximally: nothing to lint).

### 4.2 The forgetting guarantee

Define *forgotten(e)* ≡ the instance's `state { body }` is absent from
the state committed at epoch *e*. Claim:

```
∀ e ≥ evaporate_epoch:  forgotten(e)  ∧  ¬∃ revive API
```

Evaporation is **terminal by construction**: there is no `revive`; the
only defer path is the chain-applied energy refresh (`on_refresh`),
which cannot resurrect an already-Evaporated instance. Stronger than
SFSV (no revival) and EvaporCash (touch-recoverable). Candidate Coq
obligation `research/coq/DeadDropForgetting.v` (not yet written; not
demo-blocking): the post-evaporation state-root excludes the body
pre-image — a corollary of the evaporation-engine pruning lemma +
state-root binding, both already exercised by the chain.

### 4.3 Conservation

Trivial here (no fungible value): the only conserved quantity is gas
paid for `post`/`extend`, settled by the normal fee path. No
EvaporCash-style pool.

---

## 5. Contract Surface (`.es`) — `mortal_message.es` (actual, shipped)

Reconciled to the real grammar of the shipped pilot (verified by
reading `contracts/evaporscript/mortal_message.es`). EvaporScript
supports `string` (used here), `address`, `u64`, `bool`; **no `bytes`
type, no maps/ids** in the surface. **One Dead Drop = one contract
instance** (exactly as SFSV is one vault per instance) — the
instance's *own* decaying `energy` builtin IS the forgetting timer;
deploy `energy`/`half_life` ARE the TTL.

```
fn set_payload(payload_body: string, payload_recipient: address)
                          # owner-only, once (caller == owner); seals
fn read() -> string       # require sealed; caller == recipient || owner;
                          #   returns body while the instance is Active
fn record_boost()         # telemetry for chain-applied energy refresh
fn inspect() -> u64       # boost_count, without exposing the body

on_grace()      -> emit("message energy low — boost to keep alive")
on_refresh()    -> boost_count += 1; emit("message boosted")
on_evaporate()  -> emit("message evaporated")   # terminal: instance + state gone
```

The forgetting is **structural, not a contract branch**: when the
instance's energy decays out, the chain's evaporation engine retires
the whole contract (Active → Grace → Ghost → Evaporated) and its
`state { body }` ceases to exist — `GET /api/script/:id` stops
returning it. There is no `revive`; `record_boost`/`on_refresh` is the
only (bounded, gas-paid) way to defer forgetting. Payload is a
`string` bounded by the node's 64 KB `source_code`/args cap (verified
this session); larger payloads post a `blake3` commitment string +
off-chain blob. No new parity test is required — `mortal_message.es`
is itself the canonical proven pilot.

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
tagged: `set_payload(body, recipient)` →
`[{"Str":"<body>"}, {"Address":[r,0×31]}]`; `read()` → `[]`. `epoch`
**required**. Auth: register (testnet auto-verifies) → login → bearer
token. (TTL is the deploy `energy`/`half_life`, not a call arg.)

### 6.3 Observe — the demo's centrepiece

`GET /api/script/:id` (the **script** engine store, *not*
`/api/contract/:id` — the endpoint error corrected in `b76df4a2`).
The live "watch it die" panel polls this each epoch: `.state.body`
present → instance `energy` ticking down → the whole entry **absent**
(404 / `.evaporated`). `GET /api/status` → `.epoch` for the countdown.
This single endpoint *is* the demo.

### 6.4 Runbook — `scripts/deploy-dead-drop.sh` (shipped, this change)

Authored directly against the **corrected** node contract (the
`/api/script/:id` endpoint per `b76df4a2`) — NOT forked from the
on-branch `deploy-sfsv.sh`, which on some branches is still the old
`/api/contract/:id` version; forking it would re-introduce that bug.
Proof sequence against `mortal_message.es`:

1. deploy with small `energy`/`half_life` (the TTL knob)
2. poll `/api/tx/:hash` → `.contract_id`
3. `set_payload(body, recipient)` — tagged args
   `[{"Str":body},{"Address":[r,0×31]}]`, `epoch` required
4. **non-vacuity guard:** `read()` finalises AND `GET /api/script/:id`
   `.state.body.Str == body` → `saw_readable=1`. A run that never
   confirmed a readable payload **fails** (exit 5) — it cannot prove
   forgetting from a never-readable drop.
5. poll `GET /api/script/:id` until it `has("error")` / `.evaporated`
   / body gone → **forgotten**.

Verdict: PASS iff `saw_readable==1` **and** it disappeared. Inverse
polarity to SFSV: here a post-readable `/api/script/:id` 404 **is the
success** (a state object that physically ceased to exist), guarded
against vacuity by step 4. "Still readable past TTL" → exit 6
(forgetting guarantee violated).

### 6.5 Invariant obligations (mainnet gate)

- Invariant #1: forgetting is the chain's `energy`-driven evaporation
  engine; the contract holds no decay arithmetic (Layer 0 lint moot —
  nothing to lint).
- Invariant #2: `mortal_message.es` is the source of truth and is
  itself the proven canonical pilot — no mirror crate / parity test
  needed (there is no second implementation to drift from).
- Forgetting guarantee (§4.2): no `revive`; the only defer path is the
  chain-applied energy refresh surfaced via `on_refresh`/`record_boost`
  (bounded, gas-paid); on_evaporate is terminal — the instance and its
  `state { body }` cease to exist. This is the obligation the threat
  model centres on.

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
| `contracts/evaporscript/mortal_message.es` | **already shipped** — the canonical pilot IS the Dead Drop contract (no `dead_drop.es`; redundant by invariant #2) |
| mirror crate / parity test | **not required** — `mortal_message.es` is itself the proven reference pilot |
| `scripts/deploy-dead-drop.sh` | **shipped (this change)** — forgetting e2e, corrected `/api/script/:id`, non-vacuity-guarded |
| `tests/adversarial.rs` (§7) | optional follow-up (port of SFSV adversarial harness) — not blocking the demo |
| `research/coq/DeadDropForgetting.v` | candidate (not yet written) — §4.2; nice-to-have, not demo-blocking |
| live forgetting e2e on a node | **pending** — run `deploy-dead-drop.sh` against a node (Mini) to capture the directly-observed PASS, as SFSV's was |
| Node API contract | **already verified live** — §6, no chain change |

No mainnet code change required: Dead Drop is pure EvaporScript over
the evaporation engine the chain already runs and this session
live-verified. Solo budget ~1.5–2 weeks, scoped — after the mainnet
sprint or a scoped parallel exactly as SFSV was, never at the sprint's
cost (`strategic_decision_2026_05_16_focus.md`).
