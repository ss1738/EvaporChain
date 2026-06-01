# EvaporChain mainnet launch playbook

End-to-end walkthrough of the `--mainnet` strict-mode launch path.
Mirrors what the node binary actually enforces — every step here corresponds
to a check the binary will fail at boot if it's wrong.

This file is the operator-facing companion to `docs/GENESIS_CEREMONY.md`
(which covers the protocol-level ceremony — committee selection, validator-
set composition, key derivation) and `docs/VALIDATOR_ONBOARDING.md` (which
covers post-genesis joining). Read those alongside this one.

> **Read-this-first.** The node refuses to boot in `--mainnet` mode unless
> every pre-flight check below passes. The error message at startup lists
> every violated requirement at once — there's no "boot first and discover
> issues over time" path.

## 0. Pre-launch checklist (operator decisions)

These are NOT code changes; they're decisions you make once.

- [ ] **Coordinator key generated and the public key bytes baked in.**
      Edit `crates/evaporchain-node/src/main.rs:1418`:
      `pub const MAINNET_COORDINATOR_PK_BYTES: Option<&[u8]> = Some(&[ /* 32 bytes */ ]);`
      The coordinator key signs every genesis-config submitted to a mainnet
      node. Keep the private key offline.
- [ ] **`EVAPORCHAIN_KEY_MASTER` chosen** — high-entropy random string, at
      least 16 chars. Never the dev default. Goes in the validator's
      systemd unit's `Environment=` line; not in source.
- [ ] **`EVAPORCHAIN_BLS_PASSPHRASE` chosen** — non-empty, per-operator.
      Encrypts the BLS key on disk so a stolen disk image doesn't yield
      a signing key.
- [ ] **External audit complete.** See `docs/AUDIT_SCOPE.md`. T0.12 in
      `MAINNET_READINESS.md` is operator-gated on auditor selection.
- [ ] **Tokenomics ceremony complete.** Q1-Q28 in `docs/TOKENOMICS.md`
      have a documented answer signed off by the tokenomics advisory.
- [ ] **Multi-validator soak complete.** T0.6 in `MAINNET_READINESS.md`.
      Either the 3-Mini Tailscale cluster or a co-located cluster on the
      permanent Hetzner VPS, run for ≥ 1 week without incident.

## 1. Chain-id naming

The canonical chain-ids live in `evaporchain_types::chain_ids`:

| Constant | String | When to use |
|---|---|---|
| `chain_ids::MAINNET` | `evaporchain-mainnet-1` | Production. Bound into BLS signing message, VRF leader input, paymaster sponsorship payload, gossipsub topic. |
| `chain_ids::TESTNET` | `evaporchain-testnet-1` | Public testnet. Same surface, distinct namespace. |
| `chain_ids::DEVNET` | `evaporchain-devnet-1` | Local development. Wallet tokens won't accidentally cross-replay against the public testnet. |

The trailing `-1` is the chain-id **version**. A future hard fork that
breaks state compatibility increments the suffix (`-2`); the new constant
is added alongside, keeping `MAINNET` itself for archive readers.

## 2. The genesis-config file

A JSON document containing the ceremony output. The minimum mainnet shape:

```json
{
  "chain_params": {
    "chain_id": "evaporchain-mainnet-1",
    "block_interval_ms": 2000,
    "grace_period": 5,
    "block_gas_limit": 500000,
    "max_tx_size": 1048576,
    "max_txs_per_block": 10000,
    "min_validator_stake": 100000,
    "unbonding_period": 100
  },
  "tokenomics": { /* finalised per TOKENOMICS.md */ },
  "validators": [ /* one entry per founding validator */ ],
  "initial_allocations": [ /* Foundation + ecosystem allocations */ ],
  "coordinator_signature": "<hex bytes>"
}
```

The `coordinator_signature` is the BLS signature of the canonical-JSON
serialisation of the genesis-config (excluding the signature field itself)
under the coordinator key. The node verifies it against the baked-in
`MAINNET_COORDINATOR_PK_BYTES` at boot.

> **The coordinator key signs once.** After signing the launch genesis
> config, the operational rule is that the coordinator private key is
> destroyed or moved to deep cold storage. The signature is a launch
> artefact, not a recurring authority.

## 3. The `--mainnet` strict-mode pre-flight

The node aborts at boot with a single aggregated error message listing
*every* violated requirement. The current list:

| Pre-flight | Failure mode |
|---|---|
| `--mock-consensus` is set | Mainnet requires Tendermint BFT; reject. |
| `--mock-prove` is set | Proofs would not be cryptographically verified; reject. |
| `--demo` is set | Synthetic txs in mainnet state; reject. |
| `--no-da-enforcement` is set | DA attestation bypass; reject. |
| `--faucet-rate-limit-disabled` is set | Faucet cooldown disabled; reject. |
| `--validators=N > 1` without `--genesis-config <path>` | Per-node implicit genesis splits the cluster; reject (audit K-07/K-08). |
| `EVAPORCHAIN_KEY_MASTER` unset / dev-default / < 16 chars | Reject. |
| `EVAPORCHAIN_BLS_PASSPHRASE` unset | BLS key would be on disk in plaintext; reject. |
| Any `*-key.pem` under data-dir is not EVKV-encrypted | TLS path's static keys must be encrypted at rest; reject. |
| `MAINNET_COORDINATOR_PK_BYTES = None` (compile-time) | Binary not built for mainnet; reject (`--mainnet refuses to start: MAINNET_COORDINATOR_PK_BYTES is None`). |
| `coordinator_pk` in genesis-config ≠ baked-in coordinator key | Reject (`--mainnet rejects genesis: coordinator_pk does not match baked-in`). |
| Genesis-config `coordinator_signature` invalid under coordinator key | Reject. |

If the boot output is silent on a check it means that check passed.

## 4. Launching a founding validator

Once the pre-flight passes:

```bash
export EVAPORCHAIN_KEY_MASTER="<32+ hex chars from /dev/urandom>"
export EVAPORCHAIN_BLS_PASSPHRASE="<the validator's own EVPL passphrase>"

cargo run -p evaporchain-node --release -- \
  --mainnet \
  --genesis-config /etc/evaporchain/mainnet-genesis.json \
  --data-dir /var/lib/evaporchain \
  --api --api-port 8080
```

The node binary
1. Validates every `--mainnet` pre-flight check.
2. Loads + verifies the genesis-config under the baked coordinator key.
3. Pulls the validator's own BLS / ML-DSA / VRF keys from the EVKV store
   (gated by `EVAPORCHAIN_KEY_MASTER` + `EVAPORCHAIN_BLS_PASSPHRASE`).
4. Joins the network on the gossipsub topic scoped to
   `chain_ids::MAINNET`.
5. Begins Tendermint BFT consensus.

## 5. Governance-flag defaults at launch

The chain ships with these flag defaults (resolved through
`tendermint.rs::governance_defaults_for_chain(chain_id)`, called by
`governance_flags_snapshot`):

| Flag | Default | Mainnet decision needed before launch? |
|---|---|---|
| `conservation_enforcement` | `enforce` | No — default is the mainnet posture. |
| `block_source_mode` | `fifo` | **Maybe.** `antichain` enables the antichain mempool drain. Both are mainnet-safe; `antichain` is closer to the doctrine but isn't the default. Operator call. |
| `parent_acceptance_mode` | `linear` | **Maybe.** `mcc` enables Boltzmann fork-choice. Same story as above. |
| `crooks_mev_settlement_mode` | `observe` | **Yes.** `enforce` settles MEV refunds on-chain. Observe-mode keeps the chain bit-compatible with pre-flag history but means no actual MEV refunds. Operator call before launch. |
| `light_cone_state_branches_enabled` | `false` | No — keep `false` at launch; flip later via governance once soak data is in. |
| `lambda_fold_mode` | `hash_chain` | **Yes.** `nova` switches to real Nova IVC accumulator. Operationally heavier; the conservative launch is `hash_chain`. Operator call. |
| `cartel_alarm_mode` | `observe` | Operator confirms with the doctrine team. |
| `cross_epoch_churn_mode` | `observe` | Legacy count-only churn cap; flip later via governance once D7-Part2 lifecycle stabilises. |
| `post_state_verify_mode` | `warn` | Per `POST_EXEC_STATE_VERIFICATION_PLAN.md` Phase 4 (lane T0.3). Flip to `enforce` post-soak. |

### 5.1 How to land a mainnet-specific default (Phase B)

The dispatcher infrastructure is in place as of `8731ff36`. To diverge a
flag's default for mainnet — without touching testnet/devnet defaults —
edit one match arm:

```rust
// crates/evaporchain-consensus/src/tendermint.rs
pub fn governance_defaults_for_chain(chain_id: &str) -> &'static [(&'static str, &'static str)] {
    const UNIVERSAL: &[(&str, &str)] = &[ /* current 8 defaults */ ];

    // After Phase B operator decisions land, the mainnet arm forks:
    const MAINNET_DEFAULTS: &[(&str, &str)] = &[
        ("fork_choice_mode", "mcc"),
        ("parent_acceptance_mode", "linear"),     // ← change to "mcc" if decided
        ("block_source_mode", "fifo"),            // ← change to "antichain" if decided
        ("conservation_enforcement", "enforce"),
        ("lambda_fold_mode", "hash_chain"),       // ← change to "nova" if decided
        // ...
    ];

    match chain_id {
        _ if chain_id == evaporchain_types::chain_ids::MAINNET => MAINNET_DEFAULTS,
        _ => UNIVERSAL,
    }
}
```

The five tests in `governance_defaults_per_chain_tests` (same file) pin
the current state. The `mainnet_matches_universal_today` test
intentionally fails the moment mainnet diverges — its inline comments
instruct the operator to UPDATE the assertion rather than delete the
test, so the divergence is visible in the diff.

### 5.2 Post-launch flag flips

Defaults can be flipped post-launch via the governance surface
(`POST /api/governance/param` + the doctrine's stake-quorum amendment
process — see `docs/PARAMETERS.md` and the Mortal-DAO catalogue entry).
The dispatcher above provides the *initial* state; governance overrides
take precedence at runtime via the `governance_params` map carried in
state.

## 6. What this playbook does NOT cover

Out of scope; see the linked docs:

- **The genesis ceremony itself** — `docs/GENESIS_CEREMONY.md`
- **Validator onboarding post-launch** — `docs/VALIDATOR_ONBOARDING.md`
- **Tokenomics ceremony** — `docs/TOKENOMICS.md`
- **Audit scope** — `docs/AUDIT_SCOPE.md`
- **Bug-bounty go-live** — `docs/BUG_BOUNTY.md`
- **Disaster-recovery drills** — TBD (future doc; presently lives in
  validator-onboarding.md's "recovery" section)
- **ETH bridge production config** — `ETHEREUM_BRIDGE_PLAN.md`
- **Multi-token-gas policy** — `docs/MULTI_TOKEN_GAS_OPTIONS.md`

## 7. Update protocol

Edit this file whenever:

- A new `--mainnet` pre-flight check lands in `validate_mainnet_strict`.
- The governance-flag default list changes.
- A new chain-id version (`-2`, `-3`, ...) is cut.
- The genesis-config shape gains a required field.

Cross-link the relevant commit / lane / audit entry. This file is the
single source of truth for the operator's view of mainnet launch.
