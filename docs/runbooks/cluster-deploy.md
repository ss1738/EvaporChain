# Cluster Deploy Runbook

Operational procedure for rolling a new binary across the 5-node EvaporChain
WAN cluster (M1/M2/M3 Macs + H1/H2 Hetzners). Companion to
`network-upgrade.md` (which covers the upgrade-classification matrix);
this runbook is the **how to actually run the deploy without breaking the chain**
piece, captured from real-world failure modes hit during the 2026-05-08
death-is-final bundle deploy (commit `24920e6`).

## TL;DR

1. **Classify the change first.** Consensus-affecting? → stop-the-world. Pure
   API/observability? → rolling is safe.
2. **macOS launchd respawns within seconds of `pkill`.** You MUST
   `launchctl unload` first, swap binary, then `launchctl load`. `nohup`
   alongside launchd will race and fork the chain.
3. **systemd `Restart=on-failure` will pick up a pre-built binary on the
   same path** if it ever auto-restarts during the deploy window. Stage
   the new binary at `target/release/evaporchain-node.new` and only
   rename to the live path during the halt window.
4. **`light_cone_block_count` is NOT block height.** It's a windowed DAG
   metric. Use the canonical block-height endpoint for liveness probes
   or you will read misleading non-monotonic values during deploys.
5. **Recovery from a forked cluster:** wipe the data dirs (keep
   `bls_key.bin` + `network_key.bin`), restart, peers re-sync. State
   is lost. There is no clean roll-back from a partial-deploy fork
   without one side wiping.

---

## 1. Pre-flight (always)

Run on the operator workstation:

```bash
# (a) confirm git state — what we're deploying.
git log --oneline -5
git status -s    # should be clean; if not, commit before deploying

# (b) probe ALL 5 validators for liveness + height + skew.
for n in 100.119.53.101 100.113.253.72 100.103.216.125 100.66.208.20 100.91.235.22; do
  curl -s --max-time 10 "http://$n:8081/api/identity" \
    | python3 -c "import sys,json;d=json.load(sys.stdin);print(f'$n  blk={d[\"light_cone_block_count\"]} chain={d[\"chain_id\"]}')"
done
```

Acceptance: all 5 respond, all on the same chain_id, advancing in lockstep
within a 3-block sync window.

If a node is already DOWN before deploy: **stop**. Recover that node before
touching anything else — you cannot afford to lose a 2nd one mid-deploy.

## 2. Classify the change

| If the change touches… | Use… |
|---|---|
| Only HTTP/API surface, doc strings, bug fixes that don't alter execution semantics | Rolling (one node at a time) |
| `evaporchain-execution`, `evaporchain-consensus`, `evaporchain-state`, anything affecting `state_root` of the same block | **Stop-the-world** |
| Validator-set logic (jail/slash/elect) | **Stop-the-world** |
| Block-reward distribution paths | **Stop-the-world** |
| New `Account` / `Block` / serialised-state struct fields | **Stop-the-world + check on-disk schema compat** |

Rule of thumb: if old-binary-N and new-binary-N can compute different
`state_root` for the same block, you must NOT run them in the same cluster
for any block ≥ N. That's automatic stop-the-world.

The 2026-05-08 bundle (`24920e6`) was consensus-affecting (jail-on-tombstone
+ reward redirect). It needed stop-the-world. We hit divergence pain by
attempting partial rolls. Don't repeat.

## 3. macOS Mac procedure (M1, M2, M3)

Each Mini runs the validator under a launchd job at
`~/Library/LaunchAgents/com.evaporchain.validator-N.plist`. **launchd
will auto-respawn the process within ~1–5 seconds of any non-launchctl kill.**

### Build (non-destructive — running process unaffected)

On each Mini, in parallel, via SSH (per `~/CLAUDE.md` rule: never on the
MacBook):

```bash
ssh satyawansingh@100.119.53.101  'cd ~/EvaporChain && git fetch origin && git checkout <commit> && cargo build --release -p evaporchain-node' &
ssh satyawan-mini-1@100.113.253.72 'cd ~/EvaporChain && git fetch origin && git checkout <commit> && cargo build --release -p evaporchain-node' &
ssh satyawan-mini-2@100.103.216.125 'cd ~/EvaporChain && git fetch origin && git checkout <commit> && cargo build --release -p evaporchain-node' &
wait
```

Build is incremental; ~30s each on M4 hardware. Old binary still runs.

### Halt (stop launchd FIRST)

For each Mini, on the **same wall-clock countdown** as Hetzners (see §4):

```bash
ssh <user>@<minicheck> '
  launchctl unload ~/Library/LaunchAgents/com.evaporchain.validator-N.plist
  pkill -9 -f "target/release/evaporchain-node"
  sleep 2
  pgrep -f "target/release/evaporchain-node" && echo STILL_ALIVE || echo DEAD
'
```

**Do not skip `launchctl unload`.** If you `pkill` first, launchd respawns
the OLD binary while you're trying to start the NEW one. Result: split-brain.

### Restart with new binary

```bash
ssh <user>@<mini> '
  launchctl load ~/Library/LaunchAgents/com.evaporchain.validator-N.plist
  sleep 3
  pgrep -f "target/release/evaporchain-node" || echo FAILED_TO_START
'
```

The plist file's `ProgramArguments` should already point at
`./target/release/evaporchain-node` (relative to the working directory) —
the freshly-built binary is now what gets launched.

### Verify

```bash
curl -s --max-time 10 "http://<mini-tailscale-ip>:8081/api/four_act" | python3 -m json.tool
```

Look for the new fields you expect to be present (e.g.
`ghost_object_count`, `evaporation_mmr_size`, `evaporation_mmr_root` for
the 24920e6 bundle).

## 4. Linux Hetzner procedure (H1, H2)

Each Hetzner runs the validator under a systemd unit at
`/etc/systemd/system/evaporchain-validator-{4,5}.service`. systemd has
`Restart=on-failure` configured, which will auto-respawn the binary
**from whatever is at the `ExecStart` path** if the process ever fails.

### The `.new` path tactic (do this!)

Build into a separate path so a stray restart doesn't pick up an
incomplete binary:

```bash
ssh root@<hetzner> '
  cd ~/EvaporChain
  git fetch origin && git checkout <commit>
  cargo build --release -p evaporchain-node
  # binary now at target/release/evaporchain-node — but NOT yet active
  # because the systemd unit points at this same path. Two options:
  # (a) systemd unit points at evaporchain-node.live; build to .new; rename atomically.
  # (b) accept that a mid-deploy restart picks up the new binary
  #     prematurely. Risk: split-brain.
'
```

**Recommended:** patch the systemd unit `ExecStart` to point at
`target/release/evaporchain-node.live`, build to
`target/release/evaporchain-node` (the cargo default), then on the halt
moment do `mv target/release/evaporchain-node target/release/evaporchain-node.live`
and `systemctl restart`.

This isolates the live binary from the build target so cargo can rebuild
without affecting the running process or risking auto-restart-grabs-WIP.

### Halt + restart

```bash
ssh root@<hetzner> '
  systemctl stop evaporchain-validator-N.service
  # rename if using the .live tactic above
  mv ~/EvaporChain/target/release/evaporchain-node ~/EvaporChain/target/release/evaporchain-node.live
  systemctl start evaporchain-validator-N.service
'
```

`systemctl restart` does stop-then-start atomically. ~1–3 seconds of
process downtime.

## 5. The synchronized-halt countdown

For stop-the-world deploys: all 5 nodes must stop within a few seconds of
each other. Cluster downtime: ~2–5 minutes total.

**Two-operator pattern (parent + sister sessions):**

1. Parent and sister coordinate on a shared T+0 wall-clock moment via the
   human relay. Example: "fire at 13:30:00Z."
2. At T-30s: both confirm builds are complete + binaries staged.
3. At T+0: parent fires Mac halt commands in parallel; sister fires
   Hetzner systemctl-restart in parallel.
4. **It is OK if the two sides are 5–15 seconds apart.** Once 3+ nodes are
   down, BFT quorum is lost (need ≥4 of 5), the chain stalls, no blocks
   produced → no state divergence possible.
5. As long as "all 5 have new binary" is true before "any block is
   produced," the cluster will resume on the new binary cleanly.

**Don't deploy 3 of 5 first while the other 2 keep producing.** That's
the divergence trap that bit us 2026-05-08.

## 6. Post-deploy verification

Run these IN ORDER. If any fails, halt the deploy validation and
investigate before declaring success.

### (a) All 5 respond + advancing

```bash
for n in 100.119.53.101 100.113.253.72 100.103.216.125 100.66.208.20 100.91.235.22; do
  curl -s --max-time 10 "http://$n:8081/api/identity" \
    | python3 -c "import sys,json;d=json.load(sys.stdin);print(f'$n  blk={d[\"light_cone_block_count\"]}')"
done
sleep 30
# repeat the same probe — all 5 heights should have advanced
```

### (b) New binary identifiable on the wire

If your bundle adds new `/api/four_act` fields (e.g. `ghost_object_count`,
`dead_producer_redirect_total`), probe it on every node:

```bash
for n in 100.119.53.101 100.113.253.72 100.103.216.125 100.66.208.20 100.91.235.22; do
  curl -s "http://$n:8081/api/four_act" | python3 -c "
import sys,json
d=json.load(sys.stdin)
new=['ghost_object_count','evaporation_mmr_size','evaporation_mmr_root','dead_producer_redirect_total']
got=[k for k in new if k in d]
print(f'$n  new_fields_present={len(got)}/{len(new)}')"
done
```

All 5 should report the same field count. If any reports fewer, that node
is on the OLD binary — re-deploy it.

### (c) State convergence

```bash
# After ~30 seconds of advancing, all 5 should be on the same state_root.
for n in 100.119.53.101 100.113.253.72 100.103.216.125 100.66.208.20 100.91.235.22; do
  curl -s "http://$n:8081/api/blocks?limit=1" \
    | python3 -c "import sys,json;b=json.load(sys.stdin)[0];print(f'$n  blk={b[\"number\"]} sr={b[\"state_root\"][:16]}')"
done
```

All 5 should agree on the latest committed block's `state_root`. If they
diverge: chain forked, halt and recover (§7).

## 7. Recovery from a forked cluster

If post-deploy probe shows different `state_root` across nodes, or heights
are wildly different, you have a fork. There is no clean rollback. The
only reliable recovery: **pick a canonical chain head, wipe everyone else's
state, let them sync from peers.**

### (a) Halt all 5 nodes

Same procedure as §3 / §4 halt steps. All processes stopped.

### (b) Identify canonical head

The "winning" chain is whichever fork most validators are on. Check by
reading each node's RocksDB-persisted height post-halt (one of the
data-dir files records it; or just compare what each node reported as
its height at halt-time).

### (c) Wipe data dirs on the LOSING-side nodes

```bash
# On each losing node:
ssh <user>@<host> '
  rm -rf ~/.evaporchain-tailscale-5node-data/chain
  rm -rf ~/.evaporchain-tailscale-5node-data/state
  rm -rf ~/.evaporchain-tailscale-5node-data/snapshots
  # KEEP these — they are validator identity, not chain state:
  ls ~/.evaporchain-tailscale-5node-data/bls_key.bin
  ls ~/.evaporchain-tailscale-5node-data/network_key.bin
'
```

### (d) Restart all 5

Losing-side nodes start from genesis on the new binary; they peer with
canonical-side nodes and replay the chain forward. Sync time is
proportional to chain depth; on a fresh testnet ~1k blocks this is
under a minute.

### (e) Re-verify per §6

## 8. Common pitfalls

### `light_cone_block_count` is NOT block height

`/api/identity::light_cone_block_count` is the size of the in-memory
Light-Cone DAG, which is sliding-window-pruned via `prune_before_epoch`
on every epoch boundary. It can DECREASE between probes. **Do not use
it for liveness checks.** Use `/api/blocks?limit=1` to get the actual
current block number.

### `scp` with tilde-path can silently fail

```bash
# This sometimes lands, sometimes doesn't, depending on remote shell config:
scp local satyawan-mini-1@host:~/EvaporChain/...

# This always works:
scp local satyawan-mini-1@host:/Users/satyawan-mini-1/EvaporChain/...
```

If you're seeing build errors after a `scp`, verify with `wc -l` on both
sides that the file actually transferred.

### Background `nohup`-restart races launchd auto-respawn

Don't write deploy scripts that do `pkill && nohup ./binary &`. The
nohup'd process and the launchd-spawned process will both try to grab
RocksDB's LOCK file; one will panic; you end up with whichever launchd
spawned (the OLD binary). Use `launchctl unload` / `launchctl load`
exclusively on macOS.

### "It's just a doc-string change, surely rolling is safe"

Yes. But verify. Sometimes changing a doc field requires the field to
exist in the deserialised type, which can break old binaries reading
state written by new ones. Spot-check schemas before assuming rolling
safety.

### `last_conservation_audit_ok: false` is normal

Under inflationary block-reward emission, the §1.2 conservation audit
reports `false` every block — it measures `sum(accounts + stake +
refresh_pool + slashed)` which mints upward each block while doctrine
says the total should monotonically decrease. As of commit `a421321`,
`last_conservation_violation_type` exposes which variant fired. If it's
`DecayIncreasedTotal`, that's the known doctrine-vs-emission gap and
not actionable. Other variants (`RedirectChangedTotal`,
`DecayExceededLambda`) are real invariant breaches — investigate.

## 9. References

- `~/CLAUDE.md` — never run `cargo build/test/check` on the MacBook;
  always SSH to a Mini.
- `~/mac-mini-cluster-access.md` — full SSH list for the Mac Minis.
- `docs/runbooks/network-upgrade.md` — pre-existing rolling vs
  hard-fork classification matrix.
- `docs/runbooks/disaster-recovery.md` — recovery scenarios beyond
  partial-deploy fork.
- `AUDIT_2026_05_08_DECAY_LOOP.md` — full session arc that surfaced
  these lessons, including the empirical failure modes.
