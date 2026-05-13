# Genesis Ceremony Rehearsal

`evaporchain-rehearse` drives the cold-start ceremony end-to-end on a single
host so every operator has executed the flow at least once before mainnet.

## When to run it

- Before every real ceremony (validator onboarding, major chain-id reseat).
- As CI smoke for CLI changes touching `onboarding` or `testnet`.
- After upgrading the `evaporchain-node` binary, to confirm the cold path
  still produces blocks against a freshly-signed genesis.

## What it proves

1. `evaporchain onboarding generate-coordinator` writes a usable ML-DSA-65
   keypair.
2. A multi-validator manifest signs cleanly under the coordinator key.
3. Every operator's `verify` against that genesis returns OK.
4. `evaporchain testnet init/up` brings up a 3-node cluster against the
   signed config.
5. The cluster reaches finality (≥ N blocks) within `--timeout-seconds`.
6. `/api/validators` reports the validator set the manifest declared.

If steps 1–6 pass, the operator's local toolchain (CLI binary + node
binary) is ceremony-ready. If a step fails, the captured stderr from the
failing subprocess is printed verbatim, and the workdir is preserved for
inspection.

## Running it

```
evaporchain-rehearse \
  --operators 5 \
  --ceremony-blocks 10 \
  --timeout-seconds 120
```

`evaporchain` and `evaporchain-node` must be on `PATH` (or pass
`--cli-binary` / `--node-binary` explicitly). Default flags (5 operators,
10 blocks, 120 s timeout) match the public mainnet checklist.

To inspect the artefacts a rehearsal produces, pass `--keep-dir`:

```
evaporchain-rehearse --keep-dir ./rehearsal-out
```

This skips cleanup and writes everything (coordinator keypair, manifest,
signed `genesis.json`, per-validator data dirs, pid files) under the
named directory.

## Interpreting failures

| Step | Failing subprocess | Most common cause |
|-----:|---|---|
| 2 | `onboarding generate-coordinator` | output dir not writable |
| 4 | `onboarding build-genesis` | manifest violates `min_validator_stake` (rare; rehearse picks safe defaults) |
| 5 | `onboarding verify` | coordinator pk mismatch (file moved between steps) |
| 6 | `testnet init` / `testnet up` | `evaporchain-node` not next to `evaporchain` (the orchestrator looks alongside) |
| 7 | smoke poll | nodes alive but not making blocks — usually a port collision on 9100–9103 / 9201–9203 |

If step 6 fails with "evaporchain-node not found", install it next to
the CLI binary (`cp target/release/evaporchain-node $(dirname
$(which evaporchain))/`).

The exit code is `0` on full success, `1` on any failure.
