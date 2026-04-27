# EvaporChain Genesis Ceremony

The procedure for producing a real mainnet `genesis.json`, distinct from the placeholder `genesis-mainnet.json` currently in the repo. This document is normative for mainnet launch; deviations require a written governance vote and a SECURITY.md addendum.

**Status:** procedure defined, ceremony not executed. Date TBD — gated on closure of all mainnet blockers in `audit/end_to_end_audit_2026_04_27.md`.

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

- [ ] All `audit/end_to_end_audit_2026_04_27.md` Gap A items are resolved and merged.
- [ ] External audit final report received and CRITICAL/HIGH items addressed.
- [ ] DA encoder wired into `produce_block`.
- [ ] Governance parameter range bounds added.
- [ ] Validator BLS key encryption implemented.
- [ ] Bug-bounty program live for ≥ 30 days, surfaced no unfixed CRITICALs.
- [ ] Public testnet ran for ≥ 90 days under multi-validator topology.
- [ ] `cargo-deny` clean on `Cargo.lock`.
- [ ] `cargo-llvm-cov` workspace coverage report supplied to validators.
- [ ] Operational runbooks in `docs/runbooks/` reviewed and acknowledged by all genesis validators.
- [ ] Architecture diagrams in `docs/architecture/diagrams/` accurate to current `main`.
- [ ] `docs/PARAMETERS.md` reflects current source constants.
- [ ] Bootstrap peer infrastructure (DNS or static) provisioned and reachable.
- [ ] Monitoring + alerting (Prometheus + alertmanager) deployed and tested for at least one validator.
- [ ] Disaster-recovery procedure exercised end-to-end on a test cluster.

---

## Ceremony procedure

### Step 1 — Parameter freeze (T-30 days)

Coordinator publishes the proposed `genesis-target.json` parameter set to all stakeholders. Two-week comment window. Auditor signs off on final parameter values.

Frozen items:
- `chain_params` (block interval, gas limits, unbonding period)
- `tokenomics` (supply, block reward, half-life, burn ratio)
- Initial validator set composition (which validators, what stake)
- Initial token allocation by account label

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

At `genesis_time`:
1. Every validator runs `evaporchain-node --tendermint-mode --genesis-file genesis.json --genesis-attestation attestation.json`.
2. The node refuses to start if its locally-computed genesis hash doesn't match the attestation.
3. Validators connect to bootstrap peers from the genesis file.
4. Block 1 is produced when 2/3+1 stake comes online.

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

- Smart-contract template deployments (deferred to post-genesis governance proposals)
- Initial validator commission rates (deferred to per-validator on-chain config)
- DNS bootstrap configuration (handled out-of-band; not on-chain)

---

## Reference: `genesis-target.json`

See repo root. Every TODO marker in that file maps to a step above.
