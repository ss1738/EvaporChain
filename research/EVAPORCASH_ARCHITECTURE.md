# EvaporCash — Native-Demurrage Money Architecture

> Flagship demo #1 for EvaporChain's energy-decay primitive applied to
> *money itself*. Sibling of `SFSV_ARCHITECTURE.md`; same doctrine,
> same substrate (`evaporchain-script`, `evaporchain-sddc`,
> `evaporchain_types::energy_at_epoch`), different application of the
> one primitive. Pairs with `contracts/evaporscript/evaporcash.es`
> (to build) and the `crates/evaporchain-evaporcash` mirror crate.

---

## 0. TL;DR for cold readers

EvaporCash is a token whose **balances physically decay if you hoard
them**. Holding is decay; circulation is the only way to preserve
value. There is no keeper bot, no rebase contract, no off-chain
cron — the chain's Single-λ Principle does the demurrage for free,
exactly as SFSV's vault energy decays for free.

This is the 1932 **Wörgl experiment** (Silvio Gesell's *Freigeld*,
praised by Keynes in *General Theory* ch.23) made native at protocol
level. On Ethereum a demurrage coin needs an external burner script
touching every balance every block (centralised, gas-prohibitive,
unreliable). On EvaporChain it is *the absence of a refresh* — the
default physical behaviour of state. **UVP filter
(`APPLICATION_UNIVERSE.md` cat 7) passes cleanly: this cannot be built
as well anywhere without native energy-decay.**

It doubles as **Paper 2 economics ammunition**: a live instance of
"infinite-state money is unsustainable; money with a half-life
circulates." Solo-build budget: ~3 weeks on the shipped substrate
(SFSV-class), *not* a sprawling product. Scope discipline per
`strategic_decision_2026_05_16_focus.md` — one tight flagship, not a
bank.

---

## ⛔ BLOCKING FINDING (2026-05-17) — §4 NOT implementable in EvaporScript today

Verified against the real VM before any contract was written (the same
verify-before-build discipline that caught the Dead Drop / mortal_message
duplication and the /api/script endpoint):

- EvaporScript exposes **exactly four** builtins to `.es`:
  `caller | owner | epoch | energy` (`evaporchain-script/src/
  compiler.rs:368`). **There is no `energy_at_epoch` builtin**, no
  pow/shift/exp, no float — it is a 44-opcode *total* VM.
- Every shipped decay contract (`future_self_vault.es`,
  `mortal_message.es`, `energy_pool.es`) uses the **contract's single
  own `energy`** (engine-decayed) as the one decaying quantity.
  `energy_pool.es` stores per-account balances as **raw u64**
  (`stakes[caller] = prev + amount`) — it does **not** demurrage them.

Consequently §4.1/§4.2's core mechanism —
`spendable = energy_at_epoch(touched_value, e − last_touched)`
**per balance** — cannot be expressed: there is no in-script primitive
to decay an arbitrary stored value, re-deriving the half-life in-script
would violate invariant #1 (Layer 0 lint) *and* is impossible (no
exp/shift), and the one in-script decaying quantity (the contract's
own `energy`) cannot represent N independent per-account balances.

**This is a real VM-capability gap, not a coding detail.** EvaporCash
needs a design decision before any `.es` is written (do NOT fabricate
a contract around a non-existent builtin):

- **(A) Bearer-note model** — one balance = one mortal contract
  instance; its *own* `energy` demurrages natively (exactly the
  proven SFSV/mortal_message/energy_pool pattern). Transfer =
  retire+reissue. **Implementable today, doctrinally pure**, but it is
  "demurraging bearer notes," not a classic fungible map ledger.
- **(B) Host-layer demurrage** — expose `energy_at_epoch` to scripts
  as a host function, or engine-decayed map entries. Faithful to §4
  but it is **mainnet/VM work** that touches the (verified-complete)
  chain — out of scope for a scoped demo + a sprint risk.
- **(C) Pool-only decay** — only the contract's own energy (the pool)
  decays; per-balance hoarding is not penalised. Implementable, but
  loses the "your money rots if you hoard it" punchline — i.e. loses
  the demo's entire point.

**DECISION (2026-05-17): model (A) — bearer-note — CHOSEN by the
operator and IMPLEMENTED.** Contract:
`contracts/evaporscript/evaporcash_note.es` (one note = one mortal
contract instance; the note's own engine-decayed `energy` builtin IS
its spendable value — invariant-#1-clean, no in-script formula;
`spend(to)` retires the note + emits, the off-chain coordinator
reissues a fresh note carrying the live value; `on_evaporate` of an
unspent note = value lost to hoarding = the demo's punchline, by
physics). §3–§5 below describe the *superseded* map model and are
retained only as the pre-finding record — **do not build to them; the
authoritative contract is `evaporcash_note.es`.**

---

## 1. Mission & Doctrine Anchor

### 1.1 What EvaporCash is

A fungible balance ledger where every balance is an **energy-bearing
state object**. A balance's spendable value at epoch *e* is:

```
spendable(addr, e) = energy_at_epoch(touched_value, e − last_touched_epoch)
```

— the *same* canonical decay the whole chain uses. "Touching" a
balance (sending or receiving) resets that balance's demurrage clock
for the moved amount. Hoarding (no touch) bleeds value into a
protocol **redistribution pool** at rate λ. Circulating money keeps
its value; idle money funds the commons. That is Gesell's stamp
scrip, with the stamp replaced by physics.

### 1.2 What EvaporCash is **not**

- **Not** a stablecoin peg play. No oracle, no collateral, no RWA
  backing. The "stable" property it demonstrates is *velocity*, not a
  USD peg. Calling it a "stablecoin" in marketing is a UVP-filter
  failure — frame it as **demurrage money / circulation money**.
- **Not** a yield/interest product (anti-feature, same as SFSV §1.2).
  Demurrage is negative interest *by physics*, not a payout.
- **Not** a token sale. No ICO/IDO. Satoshi-pattern. The demo mints
  test units from a faucet-funded deployer; mainnet issuance policy is
  out of scope for the primitive demo.
- **Not** a bank/payment app (the rejected suggestions) — those fail
  the UVP filter. EvaporCash is specifically the *decay-of-money*
  showcase, nothing more.

### 1.3 Why this is a flagship demo

Most viscerally explosive on-thesis narrative: "money that rots if you
sit on it — native, no bot." Direct line to Paper 2. From
`evaporchain_application_universe.md` cat 7 (DeFi with native decay)
alongside SFSV; SFSV proved the *vault* pattern, EvaporCash proves the
*fungible-balance* pattern — together they cover the two structural
shapes every downstream decay-dApp forks.

---

## 2. The Three Primitives at Play

Minimal by design (same philosophy as SFSV §2 — every extra primitive
is downstream fragility).

### 2.1 Energy-Decay (Layer 0)

Each balance entry stores `(touched_value, last_touched_epoch)`.
Spendable value is **always** computed via
`evaporchain_types::energy_at_epoch` — never re-derived (Layer 0 lint;
invariant #1). The demurrage rate *is* the chain Single-λ; EvaporCash
adds **zero** new decay math. The decayed delta
`touched_value − spendable(addr,e)` is conserved: it moves to the
redistribution pool, never destroyed (conservation invariant §4.3).

### 2.2 EvaporScript VM (Layer 3)

`evaporcash.es` is the **source of truth** (invariant #2: new business
logic is EvaporScript-first; TS/Rust are thin mirrors). No internal
method dispatch (`evaporchain_evaporscript_grammar_gotchas.md`) — the
demurrage computation is inlined into every balance-reading method and
held bit-identical by a `*_parity.rs` suite, exactly as SFSV's
predicate parity works. **Map gotcha:** EvaporScript map defaults are
`U64(0)` regardless of declared type — the balance map keys on address
with a parallel `u64` presence map for "has this account ever been
touched" (mirrors the SFSV listing-state parallel-map pattern).

### 2.3 SDDC Marketplace Pattern (Layer 6, Substrate — optional)

Reused verbatim from SFSV/SHLM for an *optional* secondary FX market:
a holder can Dutch-clear-auction a forward claim on a demurrage-free
tranche. Not required for the core demo; documented so the fork
recipe is uniform across the SDDC family.

---

## 3. State Machine & Lifecycle

```
                mint(to, amount)
                      │
                      ▼
   ┌────────── Active balance ──────────┐
   │  spendable = energy_at_epoch(...)   │
   │                                     │
   │  transfer(to, amt):                 │
   │    debit sender (touch → reset both │
   │      legs' demurrage clocks for the │
   │      moved value), credit receiver  │
   │                                     │
   │  no activity → value bleeds to the  │
   │  redistribution pool at λ           │
   └───────────────┬─────────────────────┘
                    │ spendable → 0 (fully demurraged)
                    ▼
            Dormant (zero spendable; entry retained as
            tombstone until evaporation-engine prunes it
            — Active→Grace→Ghost per the chain engine)
                    │ redistribute()
                    ▼
       pool → pro-rata credit to currently-Active holders
       (Gesell circulation incentive; conserves total)
```

Touch semantics are the whole design: a `transfer` is *also* a
refresh for the touched value on both legs. Velocity ≈ value
preservation; that equivalence is the demo's punchline.

---

## 4. Mathematical Foundation

### 4.1 Demurrage = the canonical decay

```
spendable(addr, e) = energy_at_epoch(v, e − t)
                    = v · 2^(−(e − t) / τ)
```

`v = touched_value`, `t = last_touched_epoch`, `τ` = chain half-life
constant. **Identical** formula to SFSV §4.1 — EvaporCash is SFSV's
decay applied to a fungible map instead of a single vault.

### 4.2 Transfer / touch algebra

`transfer(from, to, amt)` at epoch *e*:

1. require `amt ≤ spendable(from, e)`
2. `from.touched_value := spendable(from,e) − amt`; `from.last := e`
3. `to.touched_value   := spendable(to,e)   + amt`; `to.last   := e`
4. pool absorbs each leg's pre-touch decayed delta (step 2/3 fold it
   in automatically — the decayed remainder is exactly what is *not*
   carried forward).

Pure arithmetic over chain-state; re-org-safe and refresh-safe for the
same reasons as SFSV §4.4.

### 4.3 Conservation invariant

```
Σ spendable(all addrs, e)  +  pool_balance(e)
  = Σ minted − Σ burned                       ∀ e
```

The decayed value is **moved, not destroyed**. Candidate Coq proof
`research/coq/EvaporCashConservation.v` (not yet written) is a
corollary of `EnergyDecayPreservation.v` + the touch-algebra lemma —
same structure as the SFSV conservation candidate.

### 4.4 Why demurrage must read live energy (model (a))

Lesson carried verbatim from the SFSV reconciliation
(`VERIFICATION_2026_05_16.md`): the predicate/decay must read the
**engine-supplied live energy**, never a frozen
`(initial, half_life, created_at)` formula re-derived in the contract.
EvaporCash `balance_of` returns the *live* `energy_at_epoch` value;
any frozen-formula shortcut is the exact bug model (a) fixed for SFSV.
The `*_parity.rs` suite pins `.es` ≡ Rust on this.

---

## 5. Contract Surface (`.es`)

```
fn mint(to: address, amount: u64)              # deployer-only (owner == creator)
fn transfer(to: address, amount: u64)          # touch-resets both legs
fn redistribute()                              # pool → pro-rata to Active holders
fn collect_to_pool()                           # fold idle decay into pool (permissionless, idempotent)

# Read-only (each inlines the SAME energy_at_epoch call — parity-pinned)
fn balance_of(addr: address)        -> u64     # LIVE spendable, not stored
fn pool_balance()                   -> u64
fn last_touched(addr: address)      -> u64
fn total_supply()                   -> u64     # Σ spendable + pool

# Lifecycle hooks
on_grace()      -> emit("balance fading")
on_refresh()    -> emit("balance touched")
on_evaporate()  -> emit("balance dormant")
```

`balance_of` and every internal value-read inline the identical
demurrage expression (no method dispatch) — drift caught at PR time by
`tests/evaporcash_parity.rs` (port of `predicate_inlining_parity.rs`).

---

## 6. Connect with mainnet

This is the load-bearing section — every interface below was
**verified live this session** (`VERIFICATION_2026_05_16.md`, the SFSV
e2e PASS) against `evaporchain-node`. EvaporCash uses the *exact same*
node contract as the verified `deploy-sfsv.sh` path.

### 6.1 Deploy

`POST /api/tx/deploy-script` —
`DeployScriptRequest { deployer: u8, source_code: String, energy: u64,
half_life: u64 }`. `deployer` is a **u8 devnet account index** (node
maps `i → addr_from_byte(i)`); index **0** is the genesis-funded
faucet account — use it so the balance pre-check passes without the
admin-gated faucet. Poll `GET /api/tx/:hash` until `finalised`; read
`.contract_id` (there is no `by-deploy` index endpoint).

### 6.2 Call

`POST /api/tx/call-script` —
`CallScriptRequest { caller: u8, contract_id: u64, method: String,
args: Vec<evaporchain_script::Value>, epoch: u64 }`. `args` is an
**externally-tagged** serde enum — addresses are
`{"Address":[b0..b31]}` (32-byte; `addr_from_byte(i)` = `[i,0,…,0]`),
u64 are `{"U64":n}`. Bare positionals are rejected (the bug the SFSV
e2e caught). `epoch` is **required**. Auth: register
(testnet auto-verifies) → login → `Authorization: Bearer <token>`.

### 6.3 Observe

`GET /api/script/:id` (the **script** engine store — *not*
`/api/contract/:id`, which is the unrelated template store and 404s
for an `.es` contract; this was the endpoint error corrected in
`b76df4a2`). Returns the externally-tagged `.state` map. `GET
/api/status` → `.epoch`.

> **OBSERVABILITY REALITY (verified 2026-05-17, model A):** this
> endpoint's `.energy` is the **static deploy value**, not the
> live-decaying one — directly probed: `60000` at issue and `60000`
> ~10 epochs later, then a straight flip to `evaporated:true` (the
> Dead Drop transcript shows the same). So there is **no
> "watch-the-balance-tick-down" panel** — gradual decay is real
> (it drives the eventual evaporation) but **not API-surfaced**. The
> demo's provable, directly-observable claim is the **terminal loss**:
> an unspent note `evaporated:true` while `spent:false` = value lost
> to hoarding (same observable bar as the live-verified Dead Drop).
> The pre-finding `balance_of`/`pool_balance` "decay panel" wording
> belongs to the superseded map model — do not build to it.

### 6.4 Runbook

`scripts/deploy-evaporcash.sh` — a verbatim structural fork of the
live-verified `scripts/deploy-sfsv.sh`: same auth flow, same
tagged-arg builder, same `GET /api/script/:id` strict verify, same
non-vacuity discipline (assert demurrage actually moved value to the
pool across ≥1 epoch before declaring green — the EvaporCash analogue
of SFSV's predicate-gate non-vacuity guard).

### 6.5 Invariant obligations (mainnet gate)

- Invariant #1: all decay via `energy_at_epoch` — no bit-shifts
  (Layer 0 CI lint).
- Invariant #2: `.es` is source of truth; `crates/evaporchain-evaporcash`
  mirrors; `tests/evaporcash_parity.rs` pins equivalence.
- Conservation (§4.3) enforced under `conservation_enforcement =
  "enforce"` (already the chain default per T1.13 `76d95590`).

---

## 7. Threat Model

| # | Adversary | Defence |
|---|---|---|
| 7.1 | **Hoard-via-many-wallets (Sybil split)** | Demurrage is per-*balance* on *energy*, not per-identity. Splitting 1000 across N wallets decays identically (Σ energy unchanged). Sybil buys nothing — the canonical anti-Sybil property of physical decay. |
| 7.2 | **Refresh-spam** (touch every block to dodge demurrage) | Each touch is a gas-metered tx; sustaining zero net demurrage costs ≥ the demurrage avoided (fee floor ≥ λ·value per touch by construction). Economically self-defeating, same shape as SFSV §8.4. |
| 7.3 | **Re-org** | Balances are chain-state energy; a re-org rewinds `touched_value`/`last` with it. Deterministic on every honest fork at height *h* (SFSV §4.4 argument). |
| 7.4 | **Operator-censor** (suppress `collect_to_pool`) | `collect_to_pool` is permissionless + idempotent; any node can call it; the next honest block folds the backlog. Decay is computed on read regardless — censorship delays pool accounting, never inflates a balance. |
| 7.5 | **Replay / double-spend** | Tx nonce + canonical `signable_bytes` hash (verified in the SFSV e2e — `submit_tx`). |
| 7.6 | **Quantum** | ML-DSA (Dilithium3) signatures, chain default. |

---

## 8. Doctrine Mapping

- **Paper 2 (economics):** EvaporCash is the *empirical instance* of
  "money with a half-life circulates; infinite-state money
  stagnates." A live testnet with a measurable velocity/pool curve is
  the figure Paper 2 needs.
- **APPLICATION_UNIVERSE cat 7** (DeFi with native decay) — sibling of
  SFSV; the fungible-balance shape vs SFSV's single-vault shape.
- **INEVITABILITY_STRATEGY:** reinforces "data without a half-life is
  a bug" at the most provocative possible surface — money.

---

## 9. Reference Implementation Status

| Surface | State |
|---|---|
| **design decision (A/B/C)** | ✅ **(A) bearer-note CHOSEN 2026-05-17** |
| `contracts/evaporscript/evaporcash_note.es` | ✅ **written** (model A; faithful to verified grammar — single-instance, 4 builtins, no maps, no in-script decay). Mirrors future_self_vault.es / mortal_message.es. |
| Mini parse/exec verification | **pending** — deploy on the smoke node (next increment, like SFSV/Dead Drop) |
| `scripts/deploy-evaporcash.sh` | to build (author fresh against the corrected `/api/script/:id`, NOT fork the on-branch stale deploy-sfsv.sh) |
| live forgetting/demurrage e2e | pending — run after the runbook, capture the directly-observed loss-to-hoarding |
| `crates/evaporchain-evaporcash` mirror / parity | not required for the demo (single-instance pilot, like mortal_message — no second impl to drift from) |
| Node API contract | already verified live — §6, no chain change needed |

**Honest status:** EvaporCash is NOT a "pure EvaporScript, ~3-week,
SFSV-class" build as the pre-finding text claimed. Under model **(A)**
(bearer notes) it is roughly SFSV-class and needs no chain change.
Under **(B)** it requires mainnet/VM work (host-exposed decay) — a
different, larger, sprint-touching effort. **(C)** is cheap but
defeats the demo's purpose. Dead Drop, by contrast, is done +
live-verified (= `mortal_message.es`). The only thing to "start
building" from the two flagships is EvaporCash, and it is
**design-blocked**, not code-blocked — the next step is a decision,
not a contract.
