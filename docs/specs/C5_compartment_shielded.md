# C5 Stage B — Compartment::Shielded type-level enforcement

**Status:** spec (no code) — pending design approval before implementation.
**Closes:** AUDIT_2026_05_13 C5 Stage B (architectural follow-up).
**Touches:** `evaporchain-types`, `evaporchain-execution`, `evaporchain-state`.
**Type:** internal refactor; no consensus-format change; no hard fork.

---

## 1. Problem statement

C5 Stage A (PR #73, merged) closed the specific bug where the
shielded-pool delta wasn't pre-credited as a legitimate redirect,
letting an attacker forge a balance discrepancy across the
shield/unshield boundary. The fix added a per-step pre-credit
accounting layer at runtime.

What Stage A did **not** fix: every balance in the codebase is
still a bare `u64`. There is no compile-time barrier preventing a
future bug like:

```rust
// Two balances, syntactically indistinguishable:
let transparent_balance: u64 = account.balance;
let shielded_pool_balance: u64 = db.get_shielded_pool_balance();

// All four of these compile, three are wrong:
sender.balance = sender.balance.saturating_sub(amount);     // OK
sender.balance = shielded_pool_balance;                     // WRONG (silently)
db.put_shielded_pool_balance(sender.balance);               // WRONG (silently)
let total = transparent_balance + shielded_pool_balance;    // WRONG: mixing compartments
```

The chain has TWO economic compartments:

- **Transparent.** Per-account `balance` field. Visible on-chain.
  Sum across all accounts = the public supply.
- **Shielded.** `shielded_pool_balance` aggregate. Notes are
  per-owner but the pool-level invariant is "sum of all live
  notes' values == shielded_pool_balance" — verified by the
  Pedersen commitment to the pool delta on every shield/unshield/
  private-transfer.

These two compartments are intentionally separate; a unit of value
can move from one to the other only via shield (T→S) or unshield
(S→T), both of which fire on-chain events that change the
respective compartment's aggregate. Inside one compartment, the
two aggregates must NEVER mix.

The Stage A fix protects this invariant **dynamically** (the
conservation audit at consensus tick rejects blocks where the
pool delta doesn't equal the matching transparent delta). Stage B
protects it **statically** — the compiler refuses to let a
developer write code that mixes the two.

---

## 2. Goal

Lift the compartment distinction into the type system. Concretely:

- (a) Introduce `Compartment` discriminant with two variants:
  `Transparent` and `Shielded`. Each balance / amount / delta type
  carries the discriminant in its type.
- (b) Arithmetic between same-compartment values is permitted
  (e.g., transparent + transparent → transparent). Arithmetic
  between different compartments is a compile error.
- (c) Boundary-crossing operations (shield, unshield) require
  explicit conversion via a single well-named function that
  produces an auditable trail.
- (d) Existing call sites are migrated; the conservation audit at
  the consensus layer retains its runtime check as a belt-and-
  braces second layer.

(a) + (b) means a mistake like `account.balance = pool_balance`
becomes a build failure, not a deferred-to-runtime audit reject.

---

## 3. Design

### 3.1 Core types

In `evaporchain-types::compartment`:

```rust
/// Statically-disjoint economic compartments. The discriminant is
/// a phantom marker — zero runtime cost — but enforces at compile
/// time that values from different compartments cannot be
/// accidentally combined.
pub trait Compartment: sealed::Sealed + 'static {
    const NAME: &'static str;
}

mod sealed { pub trait Sealed {} }

pub struct Transparent;
pub struct Shielded;

impl sealed::Sealed for Transparent {}
impl sealed::Sealed for Shielded {}

impl Compartment for Transparent { const NAME: &'static str = "transparent"; }
impl Compartment for Shielded     { const NAME: &'static str = "shielded";   }

/// A value pinned to a specific compartment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Balance<C: Compartment>(pub u64, #[serde(skip)] PhantomData<C>);

impl<C: Compartment> Balance<C> {
    pub const fn new(v: u64) -> Self { Self(v, PhantomData) }
    pub const fn raw(&self) -> u64 { self.0 }

    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self::new)
    }
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self::new)
    }
    pub fn saturating_add(self, other: Self) -> Self {
        Self::new(self.0.saturating_add(other.0))
    }
    pub fn saturating_sub(self, other: Self) -> Self {
        Self::new(self.0.saturating_sub(other.0))
    }
}

/// Signed delta — used by shield/unshield event records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delta<C: Compartment>(pub i128, #[serde(skip)] PhantomData<C>);

pub type TransparentBalance = Balance<Transparent>;
pub type ShieldedBalance    = Balance<Shielded>;
pub type TransparentDelta   = Delta<Transparent>;
pub type ShieldedDelta      = Delta<Shielded>;
```

### 3.2 Boundary-crossing operation

The ONLY way to convert across compartments is via an explicit
`BoundaryEvent`:

```rust
/// Records a value crossing the transparent ↔ shielded boundary.
/// Constructed only by the privacy executor; consumed by the
/// state-update path. Each event is logged to the chain so the
/// boundary crossing is auditable.
pub struct BoundaryEvent {
    pub kind: BoundaryKind,
    pub amount_transparent: TransparentDelta,
    pub amount_shielded:    ShieldedDelta,
    pub fee_transparent:    TransparentBalance,
    /// Pedersen commitment binding the boundary delta to the
    /// per-note value commitments. Already exists today; we just
    /// move it into the type.
    pub pedersen_binding: [u8; 32],
}

pub enum BoundaryKind {
    /// Transparent → shielded. `amount_transparent < 0`,
    /// `amount_shielded > 0`, and `|amount_transparent| == amount_shielded`.
    Shield,
    /// Shielded → transparent. Signs reversed.
    Unshield,
}

impl BoundaryEvent {
    /// Construct a shield boundary event. Returns Err if the
    /// transparent debit doesn't match the shielded credit
    /// (the runtime check Stage A added — preserved here as a
    /// belt-and-braces second layer beyond the type discipline).
    pub fn shield(
        from_transparent: TransparentBalance,
        to_shielded:      ShieldedBalance,
        fee:              TransparentBalance,
        pedersen_binding: [u8; 32],
    ) -> Result<Self, BoundaryError> { /* ... */ }

    pub fn unshield(
        from_shielded:    ShieldedBalance,
        to_transparent:   TransparentBalance,
        fee:              TransparentBalance,
        pedersen_binding: [u8; 32],
    ) -> Result<Self, BoundaryError> { /* ... */ }
}
```

### 3.3 Migration of existing types

| Today                                | Stage B                              |
|--------------------------------------|--------------------------------------|
| `Account::balance: u64`              | `Account::balance: TransparentBalance` |
| `Account::storage_deposit: u64`      | `TransparentBalance`                 |
| `db.put_shielded_pool_balance(u64)`  | `db.put_shielded_pool_balance(ShieldedBalance)` |
| `db.get_shielded_pool_balance() -> u64` | `-> ShieldedBalance`             |
| `TransferTx::amount: u64`            | `TransparentBalance`                 |
| `ShieldTx::amount: u64`              | (Boundary input — stays `u64` at the
                                         tx level, converted at executor
                                         entry via `BoundaryEvent::shield`)
                                         |
| `UnshieldTx::amount: u64`            | (Boundary input — same) |
| `PrivateTransferTx` value fields     | `ShieldedBalance`                    |
| `RefundTx::amount: u64`              | `TransparentBalance`                 |
| `StakeRecord::staked_amount: u64`    | `TransparentBalance`                 |
| `DelegationRecord::amount: u64`      | `TransparentBalance`                 |
| Conservation-audit aggregate         | `TransparentBalance` and `ShieldedBalance`
                                         tracked separately |

### 3.4 Serde compatibility

`Balance<C>` serializes as the bare `u64` (via `#[serde(transparent)]`).
On-disk format and on-wire format are byte-identical to today's
`u64` — no migration of stored blocks, no hard fork.

The `PhantomData<C>` is skipped at serde time (`#[serde(skip)]`).
At deserialize time the type-tag is reconstructed from the
deserialization context (i.e., the field's declared type already
pins `C`).

### 3.5 Conservation audit

Stage A's runtime check moves from this shape:

```rust
let transparent_delta = sum_account_balance_changes;
let pool_delta        = new_pool_balance - old_pool_balance;
require(transparent_delta + pool_delta == 0)
```

To this shape:

```rust
let t: TransparentDelta = sum_account_balance_deltas();
let s: ShieldedDelta    = compute_pool_delta();

// `+` between different compartments is a compile error,
// so we use the explicit BoundaryEvent ledger:
let boundary_t: TransparentDelta = boundary_events.sum_transparent();
let boundary_s: ShieldedDelta    = boundary_events.sum_shielded();

require(t == -boundary_t);
require(s ==  boundary_s);
```

The check is now expressed in terms that the type system has
already partly verified: every transparent change either came
from a transparent → transparent transfer (net zero) or from a
boundary event that recorded an equal-and-opposite shielded
change.

---

## 4. What this does NOT solve

- **Pedersen-commitment validity.** The shielded-side
  arithmetic still relies on the zero-knowledge proof binding
  the pool delta to per-note commitments. Compartment types
  catch "accidentally mixed `u64`s", not "valid proof but
  malicious value". The crypto layer keeps that responsibility.
- **Per-account shielded balances.** The shielded compartment is
  pool-aggregate only; per-owner balances live in encrypted
  notes whose values are not visible to the executor by design.
  Stage B keeps that property — `ShieldedBalance` typed values
  are only the pool aggregate.
- **Off-chain integrations.** RPC payloads cross the API
  boundary as JSON numbers. The serde-transparent representation
  means RPC clients see exactly today's wire format. Stage B
  is internal to the node binary.

---

## 5. Test plan

Unit (`evaporchain-types::compartment`):
- `Balance<Transparent>` + `Balance<Transparent>` compiles; arithmetic
  matches `u64` semantics.
- `Balance<Transparent>` + `Balance<Shielded>` is a compile error
  (proved via a `compile_fail` rustdoc test).
- `Balance<C>` round-trips through bincode unchanged from `u64`.
- `BoundaryEvent::shield` rejects mismatched amounts.

Integration (`evaporchain-execution`):
- Migrated `execute_shield` produces a `BoundaryEvent` that the
  state-apply layer consumes. Net balance changes are identical
  to today's implementation byte-for-byte (regression-pin via
  hash of the post-state).
- Migrated `execute_unshield`, `execute_private_transfer`: same.
- Adversarial: try to construct a `Balance<Shielded>` from a
  `Balance<Transparent>` without going through `BoundaryEvent` —
  should be impossible (no pub conversion exists).

Cross-fork (`evaporchain-state`):
- Snapshot apply / restore path serializes balances unchanged
  (verified by byte-comparing pre- vs post-migration snapshot
  files of an identical chain history).

---

## 6. Implementation order

1. **Types**: land `Compartment`, `Balance<C>`, `Delta<C>`,
   `BoundaryEvent`. All in `evaporchain-types`, fully tested in
   isolation. (~200 LOC + tests.)
2. **Account**: switch `Account::balance` and
   `Account::storage_deposit`. Mechanical replace; serde stays
   identical. Will likely break ~50 call sites across the
   workspace as the compiler tries to coerce `u64 → TransparentBalance`
   for arithmetic. Fix at each site by adding `TransparentBalance::new`
   wrappers OR by lifting the local var into the typed form.
   (~500 LOC across crates, mostly mechanical.)
3. **StateDB shielded-pool accessors**: switch `get_shielded_pool_balance`
   / `put_shielded_pool_balance` to `ShieldedBalance`. ~30 call
   sites. (~100 LOC.)
4. **Tx structs**: extend `TransferTx`, `ShieldTx`, `UnshieldTx`,
   `PrivateTransferTx`, `RefundTx`, etc. (~150 LOC.)
5. **Executor migration**: the privacy executor produces
   `BoundaryEvent`; the state-apply layer consumes it. Move the
   pool-delta math behind that interface. (~250 LOC.)
6. **Conservation audit**: re-express in the typed form. (~50 LOC.)
7. **Tests**: ~300 LOC of new compile-fail + regression tests.

Estimated total: ~1500 LOC. Larger than H7 Stage B, but each step
is independently shippable.

The right shape is a **stacked PR queue**:
- PR-1: types crate (no behavior change, just adds inert types).
- PR-2: Account migration only.
- PR-3: StateDB accessors only.
- PR-4: Tx struct migration.
- PR-5: Executor + audit migration.
Each merges to main in order; later PRs depend on earlier
through-types being landed.

---

## 7. Open questions

- (Q1) **How aggressive on legacy `u64` removal?** Some call
  sites (RPC dashboards, raw operator queries, ad-hoc scripts)
  read balances as `u64` via the serde-transparent shim. Do we
  want to leave `u64` interop at the boundary (RPC, RocksDB
  keys) and only enforce typed-balance inside the node, OR push
  the typing into every layer including telemetry?
- (Q2) **Newtype vs phantom-marker?** The proposal uses
  `Balance<C>` with a phantom `C: Compartment`. The alternative
  is two unrelated newtypes `TransparentBalance(u64)` and
  `ShieldedBalance(u64)` with no shared trait. Pro phantom-marker:
  generic helpers (a single fn that operates on any compartment)
  are possible. Pro newtype: zero indirection, simpler error
  messages. The shape matters for ergonomics, not correctness.
- (Q3) **Does this also cover energy?** Storage rent / decay is
  a separate Layer-0 unit accounted in `Energy`. Stage B
  proposal keeps `Energy` as a bare `u64` (matches today). If
  we want to apply the same discipline there (`Energy` vs
  `Balance<Transparent>` vs `Balance<Shielded>` as three
  disjoint compartments), the scope roughly doubles. Doable as
  Stage B.5.
- (Q4) **Migration sequencing under active testnets.** The
  whole refactor is internal-only (serde format unchanged), but
  it touches every Tx struct field type. Should we land it
  during a planned testnet downtime window, or fold it into
  the normal release cadence given the on-wire format is
  unchanged?
