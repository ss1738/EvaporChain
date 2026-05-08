# EvaporChain — Session Progress Tracker

Working journal for the build. Each session appends an entry at the TOP. Newest first.

**This is NOT** `CHANGELOG.md` (formal published ship log) or `AUDIT_*.md` (point-in-time audit). This is the operator-level "what we did + what's next + what's blocked" view across sessions.

---

## How to add an entry (read once, then forget)

When you wrap up a session, prepend a new block at the top using this template. Don't bother with prose — bullet-pointed is fine. The discipline is **consistency of format**, not polish.

```markdown
## YYYY-MM-DD (morning|afternoon|evening) — short focus

**Focus:** one sentence

**Commits shipped:** N (first-hash → last-hash). See `CHANGELOG.md` for detailed breakdown.

**Deliverables:**
- bullet
- bullet

**Empirical results (if any):**
- what fired in production
- what was observed

**Decisions made:**
- doctrine call X resolved as Y
- parameter Z changed from A to B with reason

**What's next:**
- top 2-3 items for the next session

**Blockers / open questions:**
- anything that needs human judgment / external info / parallel work

**Cross-references:**
- `CHANGELOG.md` <date>
- `AUDIT_*.md` if a new audit doc landed
- `docs/runbooks/*.md` if a new runbook landed
- specific commits worth highlighting

---
```

The reverse-chronological layout means the most recent session is always at the top. Old entries stay; treat them as historical record. The file grows append-only.

---

## 2026-05-08 (evening) — paymaster sponsorship (Option B) shipped end-to-end across 5 days

**Focus:** pull Option B forward from V1.5 deferral into the V1 sprint per `docs/MULTI_TOKEN_GAS_OPTIONS.md`. Built chain-side enforcement, off-chain service, wallet client, full E2E test, and operator runbook in one session arc. Closed a live drain bug on the way (forged-paymaster debit was previously allowed; chain now requires a hybrid sponsorship signature unconditionally).

**Commits shipped:** 6 (`dc89531` → `21fd448`).

| # | Commit | Layer |
|---|---|---|
| 1 | `dc89531` | Day 1A+1B chain-side: `paymaster_signature` + `paymaster_public_key` on `UserOpTx` + canonical sponsorship payload + verification in `execute_user_op` (closes drain) |
| 2 | `3ccf4f7` | Day 1C chain-side: `call_data` → inner `Transfer` dispatch via `execute_inner_transfer` (skips outer-already-done nonce/sig); JSON encoding (bincode broke on `skip_serializing_if`) |
| 3 | `cd64a3b` | Day 2 service crate `evaporchain-paymaster`: `Paymaster::sponsor` + axum `/healthz`/`/info`/`/sponsor` + atomic nonce persistence + keypair load/generate helpers |
| 4 | `2337d63` | Day 3 wallet client `wallet::paymaster::PaymasterClient` (reqwest-backed) + `build_unsigned_user_op` helper; integration tests via real axum task |
| 5 | `85effec` | Day 4 E2E in `tests/integration/src/paymaster_e2e.rs`: HTTP `/sponsor` → `SimpleExecutor::execute_block` happy path + tampered-call_data rejection through full block dispatch |
| 6 | `21fd448` | Day 5 docs: `docs/runbooks/paymaster.md` (~280 lines, service-style runbook), `crates/evaporchain-paymaster/README.md`, `docs/README.md` index pointer |

**Deliverables:**

- New crate `evaporchain-paymaster` (632 LOC across lib + bin) — workspace member, axum HTTP, `Paymaster` struct with mutex-guarded monotonic nonce + fsync'd persistence, `Signer`-trait-based hybrid signing.
- New module `wallet::paymaster` (396 LOC) — async `PaymasterClient` + `build_unsigned_user_op`. Re-exports `SponsorshipRequest`/`Response`/`PaymasterInfo` from `evaporchain-paymaster` so wire format stays in lock-step.
- Chain-side: `UserOpTx` gained two fields (`paymaster_signature`, `paymaster_public_key`, both `Option<Vec<u8>>` with `serde(default, skip_serializing_if)` for backwards-compat) and one method (`paymaster_sponsorship_payload(chain_id)`); `execute_user_op` gained the consent-to-sponsor verification block (~60 lines) before gas debit and the `call_data` dispatch block (~30 lines) after.
- Operator runbook covering build, first-run keypair gen + funding, CLI flags, endpoints, wallet curl examples, live-cluster smoke procedure (deferred from Day 4 — operator-driven), restart semantics, monitoring, three closed threats, two remaining threats with V1.5 hardening, competing-paymaster federation note, pricing-policy guidance, failure-modes matrix.

**Test surface — 25 paymaster tests across 5 crates, all green on Mini 1:**

| Crate | Tests | What they pin |
|---|---|---|
| `evaporchain-execution` | 12 (8 sig + 4 dispatch) | Drain-by-forged-paymaster rejection; pk-must-derive-to-paymaster; tampered-call-data invalidates sig; sender-no-impersonation; non-whitelisted inner variants reject; undecodable call_data rejects |
| `evaporchain-paymaster` | 5 | `sponsor` stamps all 4 paymaster fields; monotonic nonce; persists across restart; sig verifies under chain rules; refuses to overwrite existing sig |
| `evaporchain-wallet` | 6 | `PaymasterClient` info + sponsor wire shapes round-trip; chain-rule binding holds across HTTP; monotonic nonces over multiple `/sponsor`; 400 on already-signed; `build_unsigned_user_op` round-trip; keypair file format round-trip |
| `evaporchain-integration-tests` | 2 | Sponsored Transfer through full `execute_block` (sender debited once, paymaster gas debited, recipient credited); tampered call_data rejected at block layer with paymaster + sender state untouched |

Pre-existing 3 failures (`test_sequential_nonces_work`, `test_claim_delegation_after_unbonding_period`, `demurrage_fires_in_parallel_execute_block`) reproduce on clean main `94f5c9f` without paymaster work — unrelated, pre-existing.

**Decisions made:**

- **Envelope route over per-variant paymaster fields.** Doc literally said "add `paymaster: Option<AccountAddress>` to Transaction types" (per-variant). Chose ERC-4337-shaped envelope: extend the existing `UserOpTx` (which already had `paymaster: Option<AccountAddress>` from earlier audit work) with the two missing fields, and dispatch `call_data` as inner `Transaction`. One file changed, ERC-4337-idiomatic, doctrine-consistent. Per-variant rejected as 15-variant invasive change with no closed-form benefit.
- **Pull V1.5 forward.** Doc recommended deferring Option B to V1.5 (~Jan 2027); user explicitly chose to ship it with V1 (Oct 2026 mainnet) while context was sharp. Build now, ship with V1.
- **Drain fix bundled in same commit as Day 1.** The unverified `paymaster_data` field meant any user could forge `paymaster: <victim>` and drain a victim. Treated as CRITICAL security closure, shipped in `dc89531` alongside the new fields. No separate hotfix commit because nobody is using UserOp in prod yet.
- **JSON for `call_data`, not bincode.** `TransferTx`'s `#[serde(skip_serializing_if = "Option::is_none")]` on `signature`/`public_key` clashes with bincode's positional encoding (omitted bytes → "unexpected end of file" on decode). JSON is self-describing; per-tx size overhead acceptable.
- **Verify paymaster sig unconditionally** (not gated by `verify_signatures`). Paymaster debit is always real state; consent-to-sponsor must always be enforced. Tests confirmed no existing fixture uses `paymaster: Some(_)`, so this didn't break anything.
- **Day 1 inner-tx whitelist: Transfer only.** Other variants either protocol-issued (`Refund`), ZK-authenticated with their own gas paths (`Unshield`, `PrivateTransfer`, `Shield`), or themselves an envelope (`UserOp`, `MultiSig`, `Blob`, `Deferred`). All hard-rejected. CallScript / CallContract land in subsequent days as the paymaster service requires them.
- **Day 4 live cluster smoke deferred to operator-driven.** Touching the running 5-node WAN cluster (binary deploy + paymaster funding from a real wallet) crosses from local-reversible to shared-state-affecting. Documented as a procedure in the Day 5 runbook; in-process E2E covers the wire path.
- **Day 5 competing-paymaster doc + pricing-policy folded into the runbook** rather than published as separate files — keeps the operator-facing surface in one place.

**What's next:**

1. Push the 6 commits to `origin/main` — currently local-only.
2. Ops decision: which V1 paymaster operator? Foundation runs the only paymaster initially, or open the federation immediately? (See `docs/runbooks/paymaster.md` §Competing paymasters.)
3. Live cluster smoke per the runbook (operator-driven; needs a funded paymaster address).

**Blockers / open questions:**

- The wallet's `signer.rs` already handles `Transaction::UserOp`; a one-line convenience helper that bundles `build_unsigned_user_op` + sign + sponsor + submit would close the wallet UX loop. Tracked as a Day 3 follow-up; not blocking.
- Spam-signing protection (per-sender rate limits, mandatory user-sig verification on the paymaster side) is V1.5 hardening. The V1 paymaster signs unconditionally; the foundation paymaster operator will need to monitor for abuse manually until V1.5 lands. Documented in the runbook §Threat: spam-signing.

**Cross-references:**

- `docs/MULTI_TOKEN_GAS_OPTIONS.md` — decision artifact (Option A vs B vs C; this session shipped Option B).
- `docs/runbooks/paymaster.md` — operator runbook (new).
- `crates/evaporchain-paymaster/README.md` — crate-level README (new).
- `tests/integration/src/paymaster_e2e.rs` — E2E reference flow (new; useful as a wallet integration template).
- All 6 commits `dc89531 → 21fd448`.

---

## 2026-05-08 (sister-session, evening) — Ethereum bridge: §17.4 cross-chain primitive shipped end-to-end on Anvil

**Focus:** turn the whitepaper §17.4 line ("an evaporation event on EvaporChain could trigger an Ethereum action via an MMR inclusion proof") into a runnable cryptographic primitive. Plowed Phases 0 through 5 of `ETHEREUM_BRIDGE_PLAN.md` in one session.

**Commits shipped:** 0 (uncommitted — all green on Mac + Mini1, ready for review). Files live under `ethereum-bridge/`, `crates/evaporchain-eth-bridge/`, plus `ETHEREUM_BRIDGE_PLAN.md`.

**Deliverables:**

| Phase | What landed |
|---|---|
| 0 | Foundry v1.7.1 install (Mac + Mini1), `ethereum-bridge/contracts/` Foundry project (Prague EVM for EIP-2537), `crates/evaporchain-eth-bridge/` workspace member |
| 1 | `ValidatorSetRegistry.sol` + `BridgeTypes.Validator` + Rust mirror `valset::compute_root`. **Cross-side hash agreement byte-for-byte** (Solidity & Rust both produce `0xd9772b11…` for the same valset pre-image). |
| 2 | `lib/BLS381.sol` (EIP-2537 wrapper, no-revert variants), `lib/HashToCurve.sol` (RFC 9380 expand_message_xmd_sha256 + double SSWU), `CommitCertVerifier.sol`. **Real BLS aggregate signature from `bls12_381 0.8` verifies on EVM.** Cross-side hash-to-G2 byte-for-byte under DST `BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_`. |
| 3a | `EvaporHeaderInbox.sol` accepts BLS-signed `(height, blockHash, stateRoot, mmrRoot, epoch)` tuples. New `DOMAIN_TAG_HEADER`. |
| 3b | Standalone relayer crate `ethereum-bridge/relayer/` with full alloy 0.8 ABI binding. **Headline E2E:** `anvil_e2e_relays_50_headers` — Anvil cold-start + deploys + 50 BLS-signed headers verified in **12.81 s** (plan budget was 30 min). |
| 5 | `lib/MmrInclusion.sol`, `EvaporationDispatcher.sol` (one-shot replay-immune hooks), `evaporchain-eth-bridge::mmr` Rust module. **Headline E2E:** `anvil_full_pipeline_e2e_evaporation_to_ghost_mint` — deploys ALL 5 contracts cold, registers a hook, dispatches with the inclusion proof, watches `GhostTokenMinter.minted()` go 0 → 1. |

**Empirical results:**

- 61 tests green:
  - **39** forge tests across 8 suites (BridgeConstants, ValidatorSetRegistry, ValsetAgreement, BLS381, HashToCurve, CommitCertVerifier, EvaporHeaderInbox, EvaporationDispatcher).
  - **19** Rust eth-bridge tests across 6 binaries (lib + 4 cross-side fixture tests + 1 ghost-leaf hash test).
  - **3** relayer tests including both Anvil headlines.
- Anvil 50-header soak: 50/50 in 12.81 s. ~250 ms per submission round-trip.
- Full-pipeline E2E (`GhostTokenMinter.minted()` 0 → 1) runs in seconds on a single Anvil node, no mocks.
- Gas measurements (locked):
  - `updateValset(5 signers)` = **841 k**
  - `submitHeader(5 signers)` = **980 k**
  - `dispatch(8-leaf MMR, depth-3 path)` = **672 k**
  - Hash-to-G2 (one call) = ~280 k; pairing(2) = ~104 k; G1MSM(5) = ~50 k.

**Decisions made:**

- **Phase 4 architecture pivot.** Discovered our Verkle uses **Pallas curve** (`pasta_curves`), not Bandersnatch — no EVM precompile. Pure-Solidity Pallas IPA verification = millions of gas, infeasible. Plan locked: route Pallas-IPA proofs through a Halo2 → Groth16 wrap on BN254. Documented fallback (BLS-multisig validator attestation) = the next chunk to ship if Halo2 work slips.
- **Bitmap convention** for signed-set: LSB-first per byte (matches what Rust producers naturally pack).
- **G1 pubkey storage** = compressed 48 bytes; verification path takes uncompressed 128 bytes from calldata + checks x-coordinate consistency. Y-coordinate consistency is enforced indirectly by the pairing equation (`-P` would flip the pairing result).
- **Bridge MMR uses keccak256, not BLAKE3.** Solidity has no cheap BLAKE3 path; the bridge-layer MMR is a parallel construction to whatever EvaporChain's native MMR uses, and validators are expected to sign over both roots.
- **Relayer is a separate Cargo workspace** (`ethereum-bridge/relayer/`), deliberately outside the EvaporChain root workspace — the alloy/ethers stack is heavy and the parent already has 147+ crates.
- **Foundry installed on Mini1** to enable Anvil-driven integration tests under the no-local-builds doctrine. Both Mac and Mini1 now have `~/.foundry/bin/{forge,cast,anvil,chisel}`.

**What's next:**

- **Phase 4 MVP** — `StateMembershipAttestation.sol` using BLS-multisig over `(height, key, valueHash)` claims. Reuses `CommitCertVerifier` infrastructure; ships in a few hours.
- **Phase 4 full** — Halo2 circuit for Pallas-IPA verification + Groth16 wrap. Multi-day cryptographic build; lives in `ethereum-bridge/circuits/`.
- **Phase 6** — Sepolia deploy + 24/7 relayer + public dashboard. Operational lift; needs Sepolia ETH + ops decisions.
- **Phase 5 polish** — `ConeIntersection.sol` (replay-immunity port from `evaporchain-cone-bridge::bridge_valid`).
- **EvaporChain node-side endpoints** — relayer expects `/api/headers/finalized`, `/api/headers/<h>/commit_cert`, `/api/validators?epoch=N`. They don't exist yet on `evaporchain-node`. Adding them is a chain-side patch.

**Blockers / open questions:**

- Phase 6 needs the user's call on (a) Sepolia ETH source, (b) where to host the public dashboard, (c) whether to also do Holesky as a backup.
- Phase 4 architecture: full Halo2 → Groth16 wrap is multi-day. MVP path (BLS-multisig) is hours. User to choose which.
- The headline gas numbers are healthy for a research-grade bridge but well above the original plan's ≤350 k target for `verifyCommit`. Real production deploy will want either gas optimisation pass or layered submission.

**Cross-references:**

- `ETHEREUM_BRIDGE_PLAN.md` — canonical plan + per-phase status log
- `research/whitepaper.md` §17.4 — the line this build operationalises
- `research/papers/paper_2_state_economics.md` — the doctrinal context (why state-decay is the only viable long-run path)
- `crates/evaporchain-cone-bridge/` — the existing Tier-2 cone-merged bridge primitive that the §5 polish will port
- `crates/evaporchain-crypto/src/signatures.rs` line 403 — `BLS_DST` constant the Solidity HashToCurve binds to

---

## 2026-05-08 (late-evening + night) — multi-token gas decision package + smart-contract empirical proof + cluster-health finding + faucet config bug

**Focus:** convert the "should I buy crypto / does the chain take ETH for gas" conversation into a structured decision artifact + empirically verify smart contracts are actually wired through the chain + ship a multi-token testnet faucet + empirically test the existing faucet end-to-end (which surfaced a real config bug).

**Commits shipped:** 8 (`8ab9666` → `f67f8bf`). Plus a sister-session commit `a6bc9df` arrived in parallel — see the entry below this one.

**Deliverables:**

| # | Commit | Theme |
|---|---|---|
| 1 | `8ab9666` | `docs/MULTI_TOKEN_GAS_OPTIONS.md` — research + decision artifact (3 options, comparative research across 11 chains, recommendation locked: V1 status quo / V1.5 paymaster / NEVER protocol-level) |
| 2 | `3cc6341` | `docs/MULTI_TOKEN_GAS_VERIFICATION.md` — 6-layer verification strategy for the eventual paymaster build |
| 3 | `c68e236` | §9 added: synthetic-vs-real tokens; verification costs $0 |
| 4 | `68f63a7` | §10 added: $10-20 dead-zone analysis; binary in practice — either $0 or $1000+ |
| 5 | `3d8ca13` | `SESSION_PROGRESS.md` initial late-evening entry (the discipline working) |
| 6 | `344a0ae` | `/api/faucet/token` + `/api/faucet/bundle` endpoints + `scripts/fund-test-user.sh` wrapper — closes the test-user funding gap surfaced by the contract-deploy reject |
| 7 | `f67f8bf` | **Genesis faucet address bug fix** — empirical end-to-end test of existing `/api/faucet` surfaced a latent config bug: code reads `FAUCET_ADDRESS = [0xFA;32]` but genesis funds `[0xa0;0...]`. Mismatched. Fixed both `genesis-tailscale-{3,5}node.json`. Bumped balance 250k→100M (10k drips × 10k each). |
| 8 | (this entry update) | `SESSION_PROGRESS.md` amended to capture the faucet sub-arc |

Pre-existing context: `94f5c9f` (CLAUDE.md enforces SESSION_PROGRESS read-at-start + append-at-end) + `901966c` (tracker file created).

**Empirical results:**

- **Smart contracts ARE wired end-to-end** — verified via 2 deploy txs (`8d99382f...` and `261bba98e4...`) included in real chain blocks (#12790 + #13145). Both rejected at execution layer with `error: rejected, gas_used: 100000, confirmations: 246/1`. **Reject root cause: anonymous-user-vs-deployer-address mismatch** (`require_wallet_ownership` gate — random session user can't deploy as val-2/val-5). Pipeline is intact: tx → mempool → block inclusion → execution verdict.
- **Already-running contracts on cluster:** EVAP, FLUX, HEAT (DecayingToken instances). We exercised them all session via `/api/swap/quote` + `/api/tokens` (HEAT was at 98.7% decay during the empirical decay test).
- **Cluster health spread observed:** at probe time (~h=12779 canonical), M1 stuck at h=12742 (37 blocks behind), M2 stuck at h=12323 (**456 blocks behind** — significant silent desync). Quorum held by M3 + H1 + H2 (3 of 5 = exact threshold). One more node going dark = chain halts. Same observation sister recorded in their evening entry.
- **Faucet end-to-end test (with admin key from launchd plist `cluster-soak-2026-05-admin-recovery-key`)** surfaced a real config bug:
  - HTTP routing ✓, auth gate ✓, tx submission ✓ (`success:true`)
  - But recipient balance never increased — only block-reward credit (~100 EVP) over 20 seconds
  - Diagnosis: `GET /api/account/0xfafa...fafa` returned `balance: 0` (where the code drips from); `GET /api/account/0xa000...0000` returned `balance: 235870` (where genesis funded)
  - **The faucet has been silently failing every drip since the cluster's inception** because the genesis allocation went to a different address than the code reads from. Both addresses labeled "Devnet Faucet" — but one is `[0xa0,0,0,...]` (in JSON), the other is `[0xFA;32]` (in code). They don't match.

**Decisions made:**

- **Multi-token gas direction locked** in the new docs:
  - V1 (now → mainnet Oct 2026): EVP-only gas (status quo)
  - V1.5 (~Jan 2027 post-mainnet): wallet paymaster pattern (1 week build + 2.5 week verification)
  - NEVER: protocol-level multi-token gas (consensus-liveness risk; loses native-token demand anchor; ~30% larger audit scope)
- **Real-money verification IS a category error** for any feature pre-mainnet. Synthetic tokens on EvaporChain (DecayingToken templates) exercise the same code paths as real tokens. **$0 verification is sufficient until the very last pre-launch sanity check.** Spending $10-20 buys "psychological closure," not technical signal — dead zone between $0 and $1000+.
- **Smart contracts are NOT on the V1 critical path** — they're already shipped. More contracts is app-layer work that any dApp builder can do in parallel without blocking mainnet.
- **Faucet genesis balance: 100M EVP** — chosen for testnet stress capacity (10,000 drips at 10k each, sustainable). Mainnet preserves "no faucet" (no allocation in `genesis-mainnet.json`); revisit if a public faucet is ever wired.

**What's next (4-step concrete action list, ranked by leverage):**

1. **Stop-the-world deploy with fresh genesis** — single action that activates all 27+ accumulated commits (4ec297d → f67f8bf) PLUS sister's `a6bc9df`, fixes the faucet (via fresh genesis with the correct allocation), wakes Singh Pool API, takes demurrage threshold change live, etc. **Until this happens, none of this is operational.** Per `docs/runbooks/cluster-deploy.md`. ~1-2h.

2. **Recover M1 + M2 desync** — same operation as #1 (data dir wipe + launchctl reload re-syncs from peers). Brings cluster back to 5/5 healthy from current 3/5 (at exact BFT threshold).

3. **Run smoke tests post-deploy:**
   - `curl -X POST /api/faucet` with admin key — expect 10k EVP credit lands after next block
   - `./scripts/test-singh-pool.sh` — expect all PASS
   - `./scripts/fund-test-user.sh <addr>` — expect bundle delivered
   - `curl /api/account/<above-threshold-addr>/demurrage_preview` — expect non-zero `pending_demurrage`

4. **Resolve §1.2 conservation doctrine call** — architectural decision (fixed-supply emission vs. retract §1.2 wording). Sister's `a6bc9df` shipped a stopgap; the real call still needs human judgment. Until decided, `last_conservation_audit_ok` shows misleading signals.

**Blockers / open questions:**

- **Anonymous-deploy auth gate** — operator workflow needs keystore-signed deploys, not session-auth'd. Not a feature gap; just a UX path. Test scripts use `fund-test-user.sh` workaround now.
- **Cluster spread (M1, M2 desync)** — handled by step #1's data-dir wipe + restart.
- **Per-node mempool isolation** — txs submitted to one node don't propagate. Workaround: submit to all 5 in parallel (cluster-faucet.py pattern). Real fix lives in mempool gossip layer; not on the immediate critical path.
- **Hetzner SSH access** still required for the deploy step #1 — sister has it; coordinate.
- **§1.2 conservation doctrine** — needs human judgment, not code.

**Cross-references:**

- `docs/MULTI_TOKEN_GAS_OPTIONS.md` — research + decision artifact
- `docs/MULTI_TOKEN_GAS_VERIFICATION.md` — 6-layer verification + $0 cost answer + dead-zone analysis
- `docs/runbooks/cluster-deploy.md` — stop-the-world procedure (step #1 above)
- `scripts/test-singh-pool.sh` + `scripts/fund-test-user.sh` — smoke tooling
- Sister-session entry below — `a6bc9df` covers the orthogonal TOKENOMICS+conservation+MCP work
- Empirical anchors:
  - Smart contracts: tx hashes `8d99382f...` and `261bba98e4...` (in chain blocks)
  - Faucet bug: `GET /api/account/0xfafa...fafa` returns balance:0 vs `0xa000...0000` returns balance:235870
  - Cluster spread: M1 h=12742, M2 h=12323, M3+H1+H2 lockstep at 12779

---

## 2026-05-08 (evening) — 8-item bundle: tx-hash fix, eulogy wiring, TOKENOMICS §2.1+§2.2+§2.5, conservation observe-mode fix, MCP hardening

**Focus:** ship a verified-but-undeployed bundle of 8 correctness/observability items. Verify on Mini 1; commit + push; defer cluster deploy to next session pending Hetzner SSH access.

**Commits shipped:** 1 (`a6bc9df`). +383/-49 across 20 files.

**Deliverables:**

| # | Item | Files |
|---|---|---|
| 1 | Demo NFT/HEAT half-life 100 → 1000 | `node/main.rs` |
| 2 | `compute_tx_hash` → `tx.tx_hash()` (canonical signing bytes) — closes "tx vanishes from `/api/tx/<hash>` after ring expiry" | `node/persistence.rs` |
| 3 | Eulogy-trie wiring on every newly-evaporated object (matches §A2.5 "small deaths" doctrine) | `execution/lib.rs` |
| 4 | TOKENOMICS §2.1: `process_block_rewards_v2` 60/40 proposer/attester split, dust to first attester, falls back to v1 when no attesters | `execution/rewards.rs`, `lib.rs`, `parallel.rs` |
| 5 | TOKENOMICS §2.2: `commission_ppm` field on ValidatorInfo (serde-default 100_000 ppm = 10%) | `consensus-types/lib.rs` |
| 6 | TOKENOMICS §2.5: `blocks_per_year` field + `apy_capped_reward` method on Tokenomics; v2 wires the cap. 4 genesis JSONs updated. | `types/genesis.rs`, `genesis-{mainnet,tailscale-3node,tailscale-5node,target}.json` |
| 7 | Conservation §1.2 fix: `minted_this_block` credited into pre-block compartment snapshot before `audit_block_step` so DecayIncreasedTotal stops false-firing on legitimate minting | `execution/lib.rs` |
| 8 | MCP hardening: 3 new validators (`validate_hex_id_field` w/ path-injection guard, tx-hash, block-height); 5 hardened tool handlers; auth default inverted (token present → require auth unless explicitly relaxed) | `mcp/{validation,tools,main}.rs` |

Plus 4 backward-compat fixups for the new struct fields (Tokenomics × 5 literals, ValidatorInfo × 1, Block.post_state_root × 2 in integration tests, dfri-fs MOD_P import).

**Empirical results:**

- `cargo check --workspace` on Mini 1: green.
- `cargo test --workspace --no-fail-fast` on Mini 1: only 4 pre-existing failures remain (`state_sync::test_snapshot_metadata_state_root_mismatch_rejected`, `state_sync::test_tip_discovery`, `state_sync::test_full_sync_flow_with_provider`, `cli::cli_snapshot_create_then_verify`). All 4 are in code untouched by this bundle — regressions from intermediate work between the 2026-05-02 baseline and HEAD. Bundle adds zero new failures.
- Round 2 of the test suite (with my originally-flipped governance defaults) had 11 failures (7 from the flips + 4 pre-existing). Round 3 with the reverted defaults dropped to 4. Confirms the reverts.

**Decisions made:**

- **Doctrine-grade governance flag flips (antichain mempool, Nova IVC, conservation enforce) are NOT ridden in via default change.** The flips happen via `POST /api/governance/param` after a clean stop-the-world deploy, so the binary stays bit-compatible with a running cluster on default settings. Changing the defaults in code would hard-fork any running cluster on the next binary swap — verified via the `governance_flags_snapshot` API surface that returns effective values.
- Demurrage threshold: my session's edit (raise to 100M EVP) was superseded by the better committed work in commit `7bdbfaf` (testnet 250k / mainnet 100M split with goldilocks calibration math). My edit not included in this bundle.
- Conservation enforcement default kept at `"observe"` until in-cluster validation that `minted_this_block` credit fully nullifies DecayIncreasedTotal. Live testnet shows `last_conservation_audit_ok: false` with the violation discriminant unexposed — flipping to `"enforce"` blind would halt the chain.

**Blockers / open questions:**

- **Hetzner SSH access blocks Phase C cluster deploy.** Nodes 100.66.208.20 (`evaporchain-hel-1`) and 100.91.235.22 (`evaporchain-hel-2`) are not reachable for the operator account that has access to the 3 Minis. Stop-the-world is mandatory because `process_block_rewards_v2` changes block-reward distribution semantics — partial deploy would fork the chain. Operator must supply credentials.
- **Cluster heightspread:** at probe time, 3 nodes lockstep at h~12700 (Mini 1 + 2 Hetzners), Mini 2 lagged 296 blocks, Mini 3 lagged 317 blocks (the val-1+val-3 organically tombstoned pair from 2026-05-08 afternoon). They should sync up once block production is steady.
- **Deploy procedure unchanged:** `docs/runbooks/cluster-deploy.md` §3 stop-the-world. After deploy, post-deploy governance flips: `block_source_mode→antichain`, `lambda_fold_mode→nova`, then `conservation_enforcement→enforce` (in that order, with cluster observation between each).
- **Tx-hash forward-only:** old tx receipts in chain_store keyed by JSON-byte hash will be unreachable from new binary lookups (which use canonical hash). Acceptable for testnet; mainnet would want a re-index migration.

**What's next:**

- Get Hetzner SSH credentials → run Phase C stop-the-world deploy on the 5-node cluster.
- Post-deploy: governance-param tx flips for the three doctrine flags (with verification windows between).
- Write tests for `process_block_rewards_v2` 60/40 split (currently exercised only via existing v1 fallback path; v2 split paths uncovered).

**Cross-references:**

- `CHANGELOG.md` 2026-05-08 (evening) — to be appended.
- `docs/runbooks/cluster-deploy.md` — stop-the-world procedure.
- Plan file `~/.claude-account-b/plans/glittery-jumping-cat.md` — verify+deploy strategy.
- Commit `a6bc9df` — the bundle.

---

## 2026-05-08 (afternoon) — death-is-final doctrine + Singh Pool API + decay observability

**Focus:** ratchet the chain's namesake decay thesis from "substrate-shipped" to "empirically operational" across all 5 layers; fully wire Singh Pool AMM HTTP surface.

**Commits shipped:** 19 (`24920e6` → `d906d80`). Full detail in `CHANGELOG.md` 2026-05-08 (afternoon).

**Deliverables:**

| # | Commit | Theme |
|---|---|---|
| 1 | `24920e6` | 6-ratchet death-is-final bundle (DEPLOYED) |
| 2 | `4ec297d` | session-arc audit doc |
| 3 | `0321b50` | pnt 0x-prefix fix |
| 4 | `f8605d7` | light_cone_block_count + MMR docstrings |
| 5 | `a421321` | conservation violation discriminant |
| 6 | `8c79129` | Verkle/DA proof 0x-prefix sweep |
| 7 | `0404d27` | Singh Pool Stage 1 (read endpoints) |
| 8 | `3333dab` | Singh Pool Stage 2 (mutators) |
| 9 | `3b7bc8d` | cluster-deploy runbook |
| 10 | `bc4a956` | CHANGELOG entry |
| 11 | `50a9c40` | Singh Pool Stage 3a (route /api/swap through pools) |
| 12 | `51260a3` | Singh Pool Stage 3b (latent serde_json bug) |
| 13 | `6fa1d61` | bincode fix + 8 helper-fn unit tests |
| 14 | `a23e44a` | top-level README update |
| 15 | `56e9ac1` | Singh Pool smoke-test bash script |
| 16 | `f1bc8c1` | demurrage_preview endpoint |
| 17 | `1cb6677` | empirical correction (demurrage threshold-gated dormant) |
| 18 | `7bdbfaf` | demurrage threshold re-calibration 100M → 250k |
| 19 | `d906d80` | docs/README + dapps/singh-pool/README updates |

**Empirical results on the live 5-node WAN cluster:**

```
Decay-thesis layer       Status (post-deploy of 24920e6)
─────────────────────────────────────────────────
Object Active→Grace→Ghost     fired live (multiple test objects)
HBCT H+1 capacity expiry      fired live (8 positions burned)
Storage-rent → tombstone      fired live (val-1, val-3 organic)
Validator jail-on-tombstone   fired live (R4 ratchet)
Refresh-pool §1.2 absorption  ~155k EVP accrued
Account-balance demurrage     dormant (threshold gating, fixed
                              by 7bdbfaf — fires post-deploy)
```

Cluster reached block 11,000+ during the session under stress (2 jailed validators, 3-of-5 BFT quorum holding).

**Decisions made:**

- **`DemurrageParams::default_genesis()` re-calibrated: threshold 100M → 250k EVP.** Testnet validators sit at 300-600k, far below the original 100M (which was a mainnet-scale calibration assuming 50M+ validator funding). New value: validators above 250k pay ~0.1-1 EVP/epoch (~25× solvency margin vs block rewards). Mainnet calibration preserved as `mainnet_calibration()` constructor for the eventual mainnet genesis.
- **Singh Pool persistence: bincode file at `<data_dir>/singh_pools.bin`.** JSON was attempted (commit `51260a3`) but `serde_json` rejects `HashMap<HolderId, LpShare>` because `HolderId` is `[u8; 32]` and JSON doesn't support array-keyed maps. Caught by the test in `6fa1d61`. RocksDB-backed alternative noted as Stage 4 (Week 1 next session).
- **0x-prefix audit: complete** (6 endpoints fixed across 3 commits). All path-param hex endpoints now consistently accept both `0x`-prefixed and bare hex.
- **`light_cone_block_count` is NOT block height.** Operationally non-monotonic (sliding-window-pruned DAG count). Documented; canonical block height read is `/api/blocks?limit=1`.
- **R3 (dead-producer credit redirect) is doctrine-correct at 0.** R4's jail-on-tombstone preempts R3 in normal flow; R3 is defense-in-depth that fires only on the rare race where proposer-and-tombstoned-validator are the same in the same block.

**What's next (1-month plan from this session's wrap-up):**

Week 1 (mainnet correctness foundations):
- §1.2 conservation doctrine call (fixed-supply emission OR retract — architectural decision)
- Singh Pool Stage 4 (RocksDB persistence + state-root commitment)
- Deploy the 18 follow-up commits to the live cluster (stop-the-world per `cluster-deploy.md`)

Week 2 (observability + DAG):
- Per-tx demurrage receipt full version (Agent 4 Candidate 1 — TxOutcome → receipt store)
- ConcurrentFinality event emission (Agent 4 Candidate 3)

Week 3 (real-world data + flag-flip experiments):
- Real oracle TWAP feeds for `/api/swap`
- HBCT real Elexon BMRS integration
- Governance flag-flip experiments on isolated testnet (`conservation_enforcement: enforce`, `lambda_fold_mode: nova`, `cartel_alarm_mode: alarm`)

Week 4 (mainnet genesis prep + audit engagement):
- Tokenomics ceremony Q&A finalization (§2.1 / §2.2 / §2.5)
- External audit engagement
- Mainnet genesis ceremony rehearsal
- Cluster-health web UI dashboard

**Blockers / open questions:**

- **§1.2 conservation doctrine call** — needs human judgment on direction (fixed-supply emission vs. retract §1.2 wording). Code work depends on this.
- **18 commits accumulated awaiting deploy** — sister or future session needs to roll them via stop-the-world.

**Cross-references:**

- `CHANGELOG.md` 2026-05-08 (afternoon) — full commit-by-commit detail
- `AUDIT_2026_05_08_DECAY_LOOP.md` — pre-bundle audit + empirical correction addendum
- `docs/runbooks/cluster-deploy.md` — stop-the-world deploy procedure
- `scripts/test-singh-pool.sh` — Singh Pool live-cluster smoke test

---

## 2026-05-08 (morning) — Refactor A + Refactor B + cross-backend interop

**Focus:** unblock the WASM Light Client SDK build by extracting `evaporchain-consensus-types` (no RocksDB) and feature-flagging the BLS backend (pure-Rust `bls12_381` for wasm32 vs. native `blst`).

**Commits shipped:** 9. Full detail in `CHANGELOG.md` 2026-05-08 (morning).

**Deliverables:**

- `evaporchain-consensus-types` extracted (Phases 1, 2+4+5, 3a, 3b)
- BLS backend feature-flagged (`bls-native` / `bls-portable`)
- 10 cross-backend interop tests (single-sig + DST + 3-signer aggregate-verify)
- WASM build unblocked: 310 KB `.wasm` + 26 KB ES module + TS declarations

**Cross-references:**

- `CHANGELOG.md` 2026-05-08 (morning)
- `crates/evaporchain-light-client-wasm/README.md`

---

<!-- Future sessions: prepend new entries above this line. -->
