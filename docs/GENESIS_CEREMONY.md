# EvaporChain Genesis Ceremony

The procedure for producing a real mainnet `genesis.json`, distinct from the placeholder `genesis-mainnet.json` currently in the repo. This document is normative for mainnet launch; deviations require a written governance vote and a SECURITY.md addendum.

**Status:** procedure defined, ceremony not executed. Date TBD — gated on closure of all mainnet blockers (currently tracked in `AUDIT_2026_05_17.md` + the #469 P0 launch-blocker pack, both closed 2026-05-28; per-finding closure trail in [`AUDIT_SCOPE.md`](AUDIT_SCOPE.md) §6.2 and §6.3). The earlier `audit/end_to_end_audit_2026_04_27.md` reference is superseded.

**See also:** [`MAINNET_LAUNCH.md`](MAINNET_LAUNCH.md) is the operator-facing companion to this doc. It walks through the `--mainnet` strict-mode binary boot path that consumes the `genesis.json` produced by this ceremony. The two docs together describe the full launch arc: produce the genesis file (here) and then run a node that boots from it (MAINNET_LAUNCH).

---

## Why a separate file

`genesis-mainnet.json` in the repo today contains placeholders (`/ip4/0.0.0.0/tcp/9000`, validator addresses `[1, 0, ..., 0]`, `genesis_time` set to a sprint deadline that is not the real launch time). It is a *development scaffold*, not the mainnet genesis. The real mainnet genesis is the output of this ceremony and lives in `genesis.json` at the repo root, signed and distributed separately.

`genesis-target.json` is a clean template with TODO markers for every value that must be filled in by the ceremony.

---

## Stakeholders

| Role | Responsibility | Count |
|---|---|---|
| Coordinator | Drives the timeline, chairs ceremony calls, custodies the final file | 1 |
| Genesis Validators | Run the first node binaries, contribute BLS + ML-DSA pubkeys, attest to genesis hash | ≥ 4 (recommend 7-10) |
| Token Custodians | Operate genesis treasury wallets (Foundation, Ecosystem, Core, Airdrop) | 1 each |
| Auditor | Reviews the final file, signs off on parameter values | 1 (the engaged audit firm) |
| Witnesses | Observe the ceremony, attest to its integrity in writing | ≥ 2 |

---

## Pre-ceremony checklist

All MUST be true before the ceremony begins.

### Code-side gates (✅ already closed as of 2026-06-01)

- [x] **AUDIT_2026_05_17 closure trail**: 9 CRITICAL + 14 HIGH + 25 MEDIUM + 13 LOW (CR-1/2/3 Verkle DST, H-1 VRF chain-id-scoping, H-2 address-derivation DST, H-3 MMR structural validation, H-4 BLS PoP at non-validator sites, Q1-Q8 DA-cert + Tendermint quorum + sampler binding + state-proof, GHOST-A and CONS-A explicitly tracked as Open paper-drift / governance items). Per-finding trail: `AUDIT_SCOPE.md` §6.2.
- [x] **#469 P0 launch-blocker pack** (PRIV-001/002 shielded-tx v1-gating, DA-001 `verify_signatures_bound`, VM-001 `DecayingToken::refresh_balance` checked_add, API-001 wallet master-key fail-closed, ECON-001 slash-redistribute conservation). Per-finding trail: `AUDIT_SCOPE.md` §6.3.
- [x] DA encoder wired into `produce_block` via `build_block_da_inputs(txs)`.
- [x] Governance parameter range bounds enforced at proposal application time.
- [x] Validator BLS key encryption (EVPL format: Argon2id + XChaCha20-Poly1305; magic-byte auto-detection for legacy migration).
- [x] `every_catalogue_default_binds` anti-regression gate green for all 30 catalogue templates.

### Operator-side gates (must be true at ceremony T-0)

- [ ] **External security audit** final report received; auditor sign-off on the genesis parameter set. Operator decision: T0.12 in `MAINNET_READINESS.md`; auditor selection still pending.
- [ ] **Tokenomics ceremony complete**: 28 open Q's in `docs/TOKENOMICS.md` resolved + signed off by the tokenomics advisory.
- [ ] **`MAINNET_COORDINATOR_PK_BYTES` baked in**. Edit `crates/evaporchain-node/src/main.rs:1418` from `Option<&[u8]> = None` to `Option<&[u8]> = Some(&[ /* 32 bytes */ ])`. Without this the `--mainnet` strict-mode binary refuses to boot. See [`MAINNET_LAUNCH.md`](MAINNET_LAUNCH.md) §0 item 1.
- [ ] **Bug-bounty program live for ≥ 30 days**, surfaced no unfixed CRITICALs. `docs/BUG_BOUNTY.md` is currently a scoping draft (§10 has the open operational questions); go-live is an operator decision.
- [ ] **Multi-validator soak** ran for ≥ 90 days under multi-validator topology with no chain-halt incidents. T0.6 in `MAINNET_READINESS.md`; currently blocked on T3.1 cluster bring-up OR co-located multi-validator on the permanent VPS (zero-cost interim path).
- [ ] `cargo-deny` clean on `Cargo.lock`.
- [ ] `cargo-llvm-cov` workspace coverage report supplied to validators.
- [ ] Operational runbooks in `docs/runbooks/` reviewed and acknowledged by all genesis validators.
- [ ] Architecture + audit-prep docs accurate to current `main` (refreshed 2026-06-01: README, SPEC, AUDIT_SCOPE, THREAT_MODEL, RUN_A_NODE, architecture, CRYPTO_SPEC — see the auditor septet).
- [ ] `docs/PARAMETERS.md` reflects current source constants.
- [ ] Bootstrap peer infrastructure (DNS or static) provisioned and reachable.
- [ ] Monitoring + alerting (Prometheus + alertmanager) deployed and tested for at least one validator.
- [ ] Disaster-recovery procedure exercised end-to-end on a test cluster.

---

## Ceremony procedure

### Step 1 — Parameter freeze (T-30 days)

Coordinator publishes the proposed `genesis-target.json` parameter set to all stakeholders. Two-week comment window. Auditor signs off on final parameter values.

Frozen items:
- **`chain_id`** — fixed at `evaporchain-mainnet-1` (the canonical `evaporchain_types::chain_ids::MAINNET` constant). A future hard-fork that breaks state compatibility increments the suffix (`evaporchain-mainnet-2`); the constant is added alongside, keeping `MAINNET` itself for archive readers. Chain-id is bound into the BLS signing message, the VRF leader-selection input, the paymaster sponsorship payload, and the gossipsub topic namespace — a one-character typo silently creates a partition, hence the explicit freeze here.
- `chain_params` (block interval, gas limits, unbonding period)
- `tokenomics` (supply, block reward, half-life, burn ratio) — must match the tokenomics-ceremony output per `docs/TOKENOMICS.md`
- Initial validator set composition (which validators, what stake)
- Initial token allocation by account label
- **Governance flag mainnet defaults** — per `MAINNET_LAUNCH.md` §5, four flags require explicit operator calls before launch: `block_source_mode` (fifo / antichain), `parent_acceptance_mode` (linear / mcc), `crooks_mev_settlement_mode` (observe / enforce), `lambda_fold_mode` (hash_chain / nova). Defaults can be flipped post-launch via governance, but the genesis must record the launch-time choice explicitly.

### Step 2 — Validator key collection (T-14 days)

Each genesis validator:
1. Generates a new BLS12-381 keypair using `evaporchain-cli genesis-keygen --type bls`. Secret stays local; public key + proof-of-possession submitted to coordinator.
2. Generates a new ML-DSA Dilithium3 keypair the same way.
3. Generates a self-signed TLS cert for the libp2p endpoint.
4. Provides:
   - BLS public key (compressed G1, 48 bytes hex)
   - BLS proof-of-possession (over the validator's identity + DST)
   - ML-DSA public key
   - libp2p PeerId derived from TLS cert
   - p2p multiaddr (real public IP, not 0.0.0.0)
   - Operator account address (32 bytes)
5. Signs the bundle with their ML-DSA wallet key and emails the signed JSON to the coordinator.

Coordinator verifies every PoP individually before accepting the validator into the genesis set.

### Step 3 — Treasury custody confirmation (T-14 days)

Each token custodian (Foundation, Ecosystem, Core, Airdrop) confirms the ML-DSA address that will hold their genesis allocation. Coordinator records the address + label + balance in the candidate file.

### Step 4 — Genesis time selection (T-7 days)

Coordinator proposes a `genesis_time` UTC. All validators confirm clock synchronization (NTP) and operator availability for the launch window. Genesis time is published in `genesis-target.json`.

### Step 5 — Final genesis file assembly (T-1 day)

Coordinator produces the final `genesis.json` from `genesis-target.json` by:
1. Inserting all collected validator records (no placeholders remain).
2. Inserting the four treasury accounts at confirmed addresses.
3. Inserting validator operator accounts (one per validator, label `Validator-{name} Operator`).
4. Setting `bootstrap_peers` to the validators' real multiaddrs.
5. Writing `genesis_time` as agreed.
6. Computing the genesis state root and embedding it in the file.

### Step 6 — Genesis hash attestation (T-0)

Coordinator publishes `blake3(genesis.json)` to:
- A signed git commit on `main`
- A signed Git tag `v1.0.0-mainnet-genesis`
- The project Twitter / Mastodon / blog
- Each genesis validator's mailing list

Each genesis validator independently:
1. Downloads `genesis.json`.
2. Computes its hash.
3. Signs the hash with their genesis BLS key.
4. Returns the signature to the coordinator.

Coordinator publishes the aggregate of validator signatures over the genesis hash. This is the *Genesis Attestation Bundle*.

### Step 7 — Cluster start (T+0)

At `genesis_time`, every validator runs the node binary in `--mainnet` strict mode against the signed genesis-config. Minimum-shape command:

```bash
export EVAPORCHAIN_KEY_MASTER="<32+ hex chars from /dev/urandom>"
export EVAPORCHAIN_BLS_PASSPHRASE="<this validator's own EVPL passphrase>"

cargo run -p evaporchain-node --release -- \
    --mainnet \
    --genesis-config /etc/evaporchain/mainnet-genesis.json \
    --genesis-attestation /etc/evaporchain/attestation.json \
    --data-dir /var/lib/evaporchain \
    --api --api-port 8080
```

The `--mainnet` strict-mode pre-flight (full 11-item list in [`MAINNET_LAUNCH.md`](MAINNET_LAUNCH.md) §3) refuses to boot unless:
1. The binary was compiled with `MAINNET_COORDINATOR_PK_BYTES = Some(...)` baked in.
2. The genesis-config's `coordinator_signature` verifies under the baked coordinator key.
3. The locally-computed `blake3(genesis.json)` matches the attestation.
4. `EVAPORCHAIN_KEY_MASTER` is set, ≥ 16 chars, not the dev default.
5. `EVAPORCHAIN_BLS_PASSPHRASE` is set, non-empty.
6. No plaintext `*-key.pem` files exist under the data-dir.
7. All `--mock-*` / `--demo` / `--no-da-enforcement` / `--faucet-rate-limit-disabled` are absent.

The node aborts at boot with a single aggregated error message listing *every* violated pre-flight check at once. Once the binary is up:

1. Validators connect to bootstrap peers from the genesis file.
2. Block 1 is produced when 2/3+1 stake comes online.

### Step 8 — Witness attestation (T+24 hours)

Witnesses sign a written attestation that the ceremony was conducted as documented. Attestations are committed to the repo at `audit/genesis_witness_attestations.md`.

---

## Failure recovery

| Failure | Recovery |
|---|---|
| A validator pubkey is found to be compromised between collection and launch | Coordinator removes that validator, re-issues `genesis-target.json` with reduced set. Restart from Step 4. |
| Genesis hash mismatch between validators at launch | Halt. Investigate which validator received a corrupt file. Re-publish from Step 6. |
| < 2/3+1 stake online at `genesis_time` | Wait up to 6 hours. If still insufficient, postpone launch to a coordinated new `genesis_time`. |
| A treasury custodian loses access to their key before launch | Replace the address in `genesis-target.json` with a fresh address. Restart from Step 5 (no new validator collection needed). |

---

## What the file does NOT include

- Smart-contract template deployments (deferred to post-genesis governance proposals; the 30 catalogue templates are deployable post-launch via `/api/tx/deploy-script` once the wallet UI is online)
- Initial validator commission rates (deferred to per-validator on-chain config — see [`docs/VALIDATOR_ONBOARDING.md`](VALIDATOR_ONBOARDING.md))
- DNS bootstrap configuration (handled out-of-band; not on-chain)
- Tokenomics ceremony output — see [`docs/TOKENOMICS.md`](TOKENOMICS.md) for the 28 Q's the ceremony resolves; the output of *that* ceremony populates the `tokenomics` block of `genesis-target.json` at Step 1 here
- The `--mainnet` strict-mode boot path — see [`docs/MAINNET_LAUNCH.md`](MAINNET_LAUNCH.md)

---

## Reference: `genesis-target.json`

See repo root. Every TODO marker in that file maps to a step above.
