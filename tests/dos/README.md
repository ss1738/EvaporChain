# T0.7 DoS Resistance — Cluster Acceptance Harness

This directory contains the operator acceptance harness for T0.7. The in-CI
regression suite lives in:

- `crates/evaporchain-consensus/tests/dos_resistance.rs` (V1-V4)
- `crates/evaporchain-consensus/tests/mcc_phase_d.rs` (V5 fork-spam)
- `crates/evaporchain-network/src/service.rs` tests (V6 ShardSample cap)

The scripts here drive the >= 1hr cluster-load acceptance tests that require
**T3.1 (Phase C cluster deploy)**. Do not run against a live mainnet node.

## Pre-flight

```bash
# All 3 Minis must be in lockstep
ssh satyawansingh@100.119.53.101 'curl -s localhost:8081/api/chain | jq .block_number'
ssh satyawan-mini-1@100.113.253.72 'curl -s localhost:8081/api/chain | jq .block_number'
ssh satyawan-mini-2@100.103.216.125 'curl -s localhost:8081/api/chain | jq .block_number'

# verify_signatures must be true on all nodes
ssh satyawansingh@100.119.53.101 'curl -s localhost:8081/api/governance/flags | jq .verify_signatures'
```

## Run all vectors (sequential, requires T3.1)

```bash
tests/dos/run-all.sh --target 100.119.53.101:8081 --duration 1h
```

## Individual vectors

See `scripts/dos-flood.sh` for V1/V2/V3.
See `docs/runbooks/dos-resistance.md` for full pass criteria and triage.

## Status

| Vector | CI test | Cluster harness | Notes |
|---|---|---|---|
| V1 tx flood | green | scripts/dos-flood.sh | Ready once T3.1 up |
| V2 sig storm | green | scripts/dos-flood.sh --garbage-sigs | Ready once T3.1 up |
| V3 single-sender | green | scripts/dos-flood.sh --single-sender | Ready once T3.1 up |
| V4 encrypted flood | green | scripts/dos-flood.sh --encrypted (TODO) | Needs dos-flood.sh V4 mode |
| V5 fork-spam | green | scripts/dos-flood.sh --forks (TODO) | Needs multi-validator orchestration |
| V6 ShardSample | green | shard-sample-flood binary (TODO) | Needs libp2p client binary |
