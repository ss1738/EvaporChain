# Conservation Invariant — Proof Notes

**Crate:** `evaporchain-energy-kernel`
**Spec:** `research/INVENTION_STACK.md §1.2`
**TLA+:** `research/tla/ConservationInvariant.tla`
**Coq companion:** `research/coq/EnergyDecayMonotonicity.v`
**LLSA gate:** `research/proofs/LLSAInvariantPreservation.v`

---

## 1. The Invariant

**Informal statement:**

> The quantity `TotalEnergy = accounts + stake + refresh_pool + slashed_pool`
> is monotone non-increasing across every protocol transition. The only
> legal source of decrease is the global decay function `energy_at_epoch`
> parameterised by the single chain constant λ (half-life in epochs).

**Formal statement (Rust, `conservation.rs`):**

For every block step with `epochs_elapsed` epochs and accumulators `before`, `after`:

```
after.total() <= before.total()
after.total() >= energy_at_epoch(before.total(), λ.half_life(), epochs_elapsed)
```

The first line rules out energy creation. The second rules out energy destruction
faster than λ permits.

---

## 2. Why the Invariant Matters

Every other L1 has implicit energy conservation as a social contract enforced only by
the sum of individual transaction validity checks. EvaporChain makes it a single,
auditable accumulator-level assertion — `ConservationCheck` — that every block producer
must pass before a block is accepted.

This closes two attack classes:

1. **Silent destruction bugs** — a coding error that debits one compartment without
   crediting another is caught immediately at `ConservationCheck::redirect`, not
   discovered by reconciling wallet balances after the fact.

2. **Over-decay bugs** — a misconfigured or adversarially chosen λ that burns energy
   faster than the governance-approved rate is caught by the `DecayExceededLambda` variant.

---

## 3. The Four Compartments

The conservation domain (§1.2, `compartment.rs`) is the closed set:

| Compartment   | What it holds                               | Enum variant              |
|---------------|---------------------------------------------|---------------------------|
| `Accounts`    | Aggregate user balances                     | `Compartment::Accounts`   |
| `Stake`       | Validator-bonded stake (active + delegated) | `Compartment::Stake`      |
| `RefreshPool` | Protocol-owned keep-alive sink              | `Compartment::RefreshPool`|
| `SlashedPool` | Slashed-but-not-yet-redirected holding      | `Compartment::SlashedPool`|

No energy can appear or disappear outside this set. Transfers between compartments are
exclusively via `EnergyRedirect::apply`, which hard-codes a `(from, to)` pair per
`RedirectKind`.

---

## 4. Why Each Transition Preserves or Legitimately Weakens the Invariant

### 4.1 Redirect-class transitions (preserve total exactly)

Each calls `EnergyRedirect::apply`, debiting `amount` from `from` and crediting
`amount` to `to`. Total is preserved by arithmetic. `ConservationCheck::redirect`
asserts `after.total() == before.total()`.

| Transition      | `from` → `to`               | RedirectKind     |
|-----------------|-----------------------------|------------------|
| Slash           | `Stake` → `SlashedPool`     | `Slash`          |
| SlashSettle     | `SlashedPool` → `RefreshPool` | `SlashSettle`  |
| MevBurn         | `Accounts` → `RefreshPool`  | `MevBurn`        |
| Demurrage       | `Accounts` → `RefreshPool`  | `Demurrage`      |
| Refresh payout  | `RefreshPool` → `Accounts`  | `RefreshPayout`  |
| Transfer        | `Accounts` → `Accounts`     | (same compartment)|

Why this holds: `EnergyRedirect::apply` debits before it credits, and returns `Err`
(leaving the accumulator unchanged) if the source is insufficient. Debit-first ordering
means a saturation in the credit can never mask a missing debit.

### 4.2 Decay transition (total may drop, within λ bound)

An epoch advance applies `energy_at_epoch(total, λ, epochs_elapsed)` to compute the
minimum retained total. The chain may retain more, but may not retain less.

`ConservationCheck::decay_step` asserts:

```
after.total() <= before.total()                                         (no creation)
after.total() >= energy_at_epoch(before.total(), λ, epochs_elapsed)    (no over-destruction)
```

The Coq proof that `energy_at_epoch` is monotone-non-increasing in `epochs_elapsed`
lives in `research/coq/EnergyDecayMonotonicity.v`.

### 4.3 BlockProduce (composite — redirects then decay)

`ConservationCheck::block_step` collapses to `decay_step`. Redirects are
total-preserving, so running zero or more redirects before the decay step cannot
increase the total; the decay floor bound still applies to the original `before.total()`.

---

## 5. What Would Falsify the Invariant

1. **A redirect that changes total.** → `ConservationViolation::RedirectChangedTotal`
   Root cause: compartment debited without equal credit, or vice versa.

2. **Total increasing across a decay step.** → `ConservationViolation::DecayIncreasedTotal`
   Root cause: non-redirect transition created energy (e.g. mint bug bypassing `EnergyRedirect`).

3. **Total dropping below the lambda floor.** → `ConservationViolation::DecayExceededLambda`
   Root cause: λ applied more than once per epoch, or redirect disguised as decay step.

4. **A new compartment outside `Compartment::ALL`.** Not a runtime violation today (enum
   is closed), but would require governance amendment gated by `LLSAInvariantPreservation.v`.

5. **Negative energy in any compartment.** `EnergyAccumulator::debit` returns `Err(())`
   rather than underflowing. Any caller ignoring this error would produce a
   negative-masked `u64`; the conservation check would catch it as a total mismatch.

---

## 6. Relationship to Coq and TLA+ Work

- `research/coq/EnergyDecayMonotonicity.v` — proves the base lemma that
  `energy_at_epoch(e, λ, n+1) <= energy_at_epoch(e, λ, n)`.

- `research/tla/ConservationInvariant.tla` — TLC-checkable model of all transition
  classes. The `EnergyNonIncreasing` action property is the machine-checked form of §1.2.

- `research/proofs/LLSAInvariantPreservation.v` — any governance amendment touching the
  energy kernel must supply a Coq term of type `forall s, Inv(s) → Inv(step_new(s))`
  before `apply_amendment` will accept it.
