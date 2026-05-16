# evaporchain-sfsv

**Singh Future-Self Vault** — EvaporChain's reference dApp for the energy-decay primitive.

A vault locks an energy-denominated deposit on behalf of the creator's *future self*. When a decay-state predicate trips (an epoch is reached, or the vault contract's own physical energy decays below a threshold), the deposit is released to the current holder. A SDDC-cleared secondary market lets third parties bid for the future claim at a Dutch-clearing discount.

> First launch-dApp candidate per `INVENTION_STACK.md §A5.2`.
>
> *"You can now sell your future self's money — and your future self can't sue."*

---

## Status

| Surface | Coverage | Status |
|---|---|---|
| `src/predicate.rs` | model (a): `EnergyDecaysBelow { threshold }` over engine-supplied live `contract_energy` (refresh-aware) | reconciled · VERIFIED GREEN (Mini 1) |
| `src/payout.rs` | 3-arg `payout(vault, epoch_now, contract_energy)` | reconciled · VERIFIED GREEN |
| `src/vault.rs` | vault FSM + on-chain listing state machine (`list_for_sale`/`cancel_listing`/`record_sale`) | VERIFIED GREEN |
| `src/market.rs` | SDDC shim; settle routes through the on-chain `record_sale` guard (7 tests) | VERIFIED GREEN |
| `tests/adversarial.rs` | 22 (§8 threat model + refresh-aware + §8.6 listing adversaries) | VERIFIED GREEN |
| `tests/predicate_inlining_parity.rs` | 9 (`.es`↔Rust predicate drift detector) | VERIFIED GREEN |
| `tests/listing_parity.rs` | NEW — `.es`↔Rust listing-guard parity (`§5-A`) | VERIFIED GREEN |
| `scripts/deploy-sfsv.sh` | full lifecycle **live-e2e PASS** on a node (deploy → set_terms → predicate-gated try_payout → directly-observed `released`) | live-verified |

Source of truth for business logic: `contracts/evaporscript/future_self_vault.es` (the on-chain contract; this crate is its substrate-side mirror — used by the execution layer, test harness, and SDDC marketplace). `.es`↔Rust parity is machine-enforced by the two `*_parity.rs` suites. Full record: `VERIFICATION_2026_05_16.md`.

---

## Quick start

```bash
# Run the substrate-crate test suite
cargo test -p evaporchain-sfsv

# Validate the deploy runbook without touching the network
./scripts/deploy-sfsv.sh --dry-run \
    --deployer 0 --future-self 2 \
    --predicate 0 --release-param 200

# Execute against a live node. deployer/caller are u8 devnet account
# indices (node maps i -> addr_from_byte(i)); index 0 is the
# genesis-funded faucet account. Auth: register+login mints a session
# token (testnet auto-verifies) -> pass it as --token.
./scripts/deploy-sfsv.sh \
    --node http://127.0.0.1:9001 \
    --token "$EVAPORCHAIN_TX_TOKEN" \
    --deployer 0 --future-self 2 \
    --predicate 0 --release-param 200 \
    --energy 1000000 --half-life 1000000 --deposit 1000
```

---

## Architecture

Full spec at [`research/SFSV_ARCHITECTURE.md`](../../research/SFSV_ARCHITECTURE.md) (571 lines).

Reading order for new contributors:
1. **`research/SFSV_ARCHITECTURE.md` §1–4** — mission, doctrine anchor, state machine, math
2. **`contracts/evaporscript/future_self_vault.es`** — the `.es` contract (source of truth for business logic)
3. **`src/vault.rs`** — Rust API surface
4. **`tests/adversarial.rs`** — what we defend against (`§8` threat model in arch doc)
5. **`research/SFSV_ARCHITECTURE.md` §5–7** — frontier predicate variants + cryptographic stack
6. **`research/SFSV_ARCHITECTURE.md` §10–13** — roadmap + open problems + forkability

---

## API surface

### Predicate

```rust
use evaporchain_sfsv::predicate::{Predicate, PredicateContext, evaluate};

// EpochReached: releases when chain epoch ≥ release_epoch.
let p = Predicate::EpochReached { release_epoch: 1_000 };
assert!(evaluate(&p, PredicateContext { epoch_now: 1_000, contract_energy: 0 }));

// EnergyDecaysBelow (model (a)): a pure comparison over the
// engine-supplied LIVE contract energy — no frozen formula. The chain
// decays the vault's own energy (refresh-aware); the predicate just
// compares it. Restores the invariant that the predicate reads the
// same physical energy the evaporation engine maintains.
let p = Predicate::EnergyDecaysBelow { threshold: 100 };
assert!(evaluate(&p, PredicateContext { epoch_now: 0, contract_energy: 99 }));
```

### Vault

```rust
use evaporchain_sfsv::vault::{Vault, VaultError};

let v = Vault::create(
    /* id          */ [0xAB; 32],
    /* creator     */ creator_addr,
    /* future_self */ future_self_addr,
    /* deposit     */ 1_000,
    /* predicate   */ Predicate::EpochReached { release_epoch: 100 },
    /* created_at  */ 0,
)?;
assert!(v.is_locked());
assert_eq!(v.current_holder(), Some(future_self_addr));
```

### Payout

```rust
use evaporchain_sfsv::payout::{payout, PayoutError};

// payout takes the engine-supplied live contract energy as the 3rd
// arg (model (a)) — the same value the predicate compares against.
let result = payout(&mut v, /* epoch_now */ 100, /* contract_energy */ 0)?;
assert_eq!(result.paid_to, future_self_addr);
assert_eq!(result.amount, 1_000);

// Double-payout is rejected.
let err = payout(&mut v, 101, 0).unwrap_err();
assert_eq!(err, PayoutError::AlreadyReleased);
```

### Secondary market (SDDC)

```rust
use evaporchain_sfsv::market::{list_for_sale, settle_secondary};
// list_for_sale opens an SDDC auction AND mirrors the on-chain
// listing (vault.list_for_sale guards). settle_secondary runs
// Dutch clearing and routes the claim transfer through the on-chain
// record_sale guard. See src/market.rs for the full surface.
```

---

## Contract surface (`.es`)

```
fn set_terms(future_self: address, predicate: u64, release_param: u64, deposit_amount: u64)
fn list_for_sale(ceiling: u64, floor: u64, duration: u64)
fn cancel_listing()
fn record_sale(winner_addr: address)
fn try_payout()

# Read-only queries
fn current_holder()                -> address
fn is_released()                   -> bool
fn is_listed()                     -> bool
fn deposit_amount()                -> u64
fn release_target()                -> u64
fn predicate_satisfied()           -> u64
fn listing_ceiling()               -> u64
fn listing_floor()                 -> u64
fn epochs_until_listing_expires()  -> u64

# Lifecycle hooks
on_grace()      -> emit("vault energy low")
on_refresh()    -> emit("vault boosted")
on_evaporate()  -> emit("vault evaporated") if !released
```

`predicate_satisfied` and `try_payout` inline the SAME predicate logic byte-for-byte (EvaporScript has no internal dispatch). Drift between them is caught at PR time by `tests/predicate_inlining_parity.rs`.

---

## Fork recipe — build your own decay-dApp in 50 lines

The viral-demo purpose (`SFSV_ARCHITECTURE.md §1.3`) is satisfied when a third-party developer can read this crate and produce a different decay-dApp without rebuilding the primitive. The recipe:

1. **Pick your decay use-case.** Mortal credential? Decaying NFT? Rental? Demurrage stablecoin? See `research/APPLICATION_UNIVERSE.md` for the 12 categories that pass the "wouldn't work on Ethereum" filter.

2. **Identify the predicate.** What condition triggers your dApp's "release"? It must be expressible as a pure comparison over chain state (`epoch` or the contract's own `energy`, or both).

3. **Fork `contracts/evaporscript/future_self_vault.es`.** Rename the contract, change the state fields to match your domain. For a decaying NFT, swap `deposit: u64` for `metadata_uri: bytes`. For a rental, add `start_epoch` + `monthly_rent`.

4. **Inline your predicate in `try_payout` and `predicate_satisfied`.** Both must be byte-identical. Adapt `tests/predicate_inlining_parity.rs` to your contract path; the drift detector ports verbatim.

5. **Wire to SDDC** if your dApp has a secondary market. Otherwise drop the listing fields and the `record_sale` entry point.

6. **Adversarial-test the §8 threat model.** Copy `tests/adversarial.rs`, rename the adversaries to your domain, and re-target each row. Most carry over (replay, transfer-claim race, key-loss); §8.1 (Present-Self Reneger) usually needs a domain-specific rename.

**Fork distance for a typical decay-dApp: 30–80 lines of EvaporScript + ~200 LOC of substrate-crate test scaffolding.** The chain primitives (energy-decay, SDDC, MMR, EvaporScript VM) are unchanged.

---

## Roadmap

| Version | Surface | Status |
|---|---|---|
| v1.0 | Reference impl: model-(a) crate + 22 adversarial + 9 predicate-parity + listing-parity + deploy runbook (live-e2e PASS) + TS view (#359) | §10.2 gaps: #1–#4 closed + verified; #5 (this README) corrected — in-browser UX pass + main-fold remain |
| v1.1 | VDF-anchored EpochReached predicate (§5.3) + Lambda-Fold batch release (§5.7) | not started |
| v1.5 | Threshold m-of-n future-self (§5.4) + forward-secure rotating claim (§5.6) | not started |
| v2.0 | Witness-encrypted beneficiary (§5.5) + lattice-based threshold migration | research |

§-prefixed references resolve in `research/SFSV_ARCHITECTURE.md`.

---

## Related

- **Architecture spec:** `research/SFSV_ARCHITECTURE.md`
- **Doctrine anchor:** `research/INVENTION_STACK.md §A5.2`
- **Application taxonomy:** `research/APPLICATION_UNIVERSE.md` (cat 7: DeFi with native decay)
- **Substrate base:** `crates/evaporchain-sddc` (Dutch-clearing pattern shared with SHLM)
- **EvaporScript VM:** `crates/evaporchain-script` (44-opcode VM, gas-metered, pure predicate evaluation)
- **Chain decay primitive:** `evaporchain_types::energy_at_epoch` (Coq-proven)
