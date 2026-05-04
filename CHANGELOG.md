# EvaporChain Changelog

## 2026-05-03 / 2026-05-04 — Layer 3/4 substrate seams + Layer 0 closure + doctrine sweeps

This session lands the consensus abstraction seams (Layer 3), the
first concrete impls behind them (Layer 4), governance-flag wiring +
operator UX RPCs, four proptest mirrors locking trait invariants, and
two doctrine-doc sweeps reconciling `INVENTION_STACK.md` +
`DOCTRINE_PUNCH_LIST.md` with reality after the MERA gate FAILED on
real Ethereum data (VERKLE verdict).

Default behaviour is unchanged across all 27 commits — every new code
path is governance-gated `linear / fifo / observe` until an operator
explicitly opts in.

### Substrate (`evaporchain-consensus`)

| Lane | Trait / impl | Commit |
|---|---|---|
| G.1 | `pub trait BlockSource` + blanket impl on `Mempool` | `f78d965` |
| G.3 | `pub trait ForkChoice` + `LinearForkChoice` default | `61eb888` |
| G.4 | `pub trait MevPool` + blanket impl on `EncryptedMempool` | `150292c` |
| G.5 | `pub trait ValidatorSetSource` + impl on `ValidatorSet` | `118b19d` |
| I.1 | `TxAntichainMempool` — first non-default `BlockSource` impl | `842363f` |
| I.3 | `MccForkChoice` — first non-default `ForkChoice` impl | `c1a05bb` |
| I.5 | `mempool::antichain_project` — post-FIFO antichain helper | `2bdcdc2` |

### Hot-path consumers (governance-gated)

| Lane | What | Commit |
|---|---|---|
| I.4 | `parent_acceptance_mode = "mcc"` dispatches at `tendermint.rs:2643` | `ded1a73` |
| I.5 | `block_source_mode = "antichain"` filters at `tendermint.rs:3915` | `20d9fc8` |
| I.6 | MCC β derived from chain CFM (microbits/fee/epoch) | `a45588c` |
| F.1 | Singh-Lyapunov fee tick wired into `execute_block` | (sister `4d59b5d` + test fix `b14ed53`) |

### Operator UX

| Lane | Endpoint / API | Commit |
|---|---|---|
| J.0 | `GET /api/governance/flags` — inspect all soft-fork keys | `d694ce8` |
| K.1 | `POST /api/governance/param` — flip with allowlist | `2fa6362` |

`fork_choice_mode` retains its existing dedicated endpoint
(endorser-stake-validated). Other knobs (`parent_acceptance_mode`,
`block_source_mode`, `conservation_enforcement`) flip via the generic
allowlisted setter.

### Test rigor — proof-style coverage of trait contracts

256 randomised inputs each, ~1,536 randomised assertions per
`cargo test -p evaporchain-consensus`:

| Test | Properties locked |
|---|---|
| `tx_antichain_mempool::antichain_invariant_no_duplicate_senders` | 4 (Lane I.1) |
| `mempool::antichain_project_invariants` | 5 (Lane I.5 follow-up) |
| `fork_choice::mcc_proptest_invariants` | 3 (Lane I.3 follow-up) |
| `tx_antichain_mempool::block_source_contract_holds_for_both_impls` | cross-impl (Lane G.1 follow-up) |
| `fork_choice::fork_choice_contract_holds_for_both_impls` | cross-impl (Lane K.3) |
| `tendermint::tests::governance_set_param_proptest` | 4-bucket allowlist (Lane K.4) |

### Test rigor — integration tests for governance flag dispatch

| Test | Lane | Locks |
|---|---|---|
| `test_block_source_mode_antichain_dedups_same_sender_in_proposal` | J.1 | I.5 wire-path |
| `test_block_source_mode_default_admits_all_same_sender` | J.1 | typo-safety |
| `test_parent_acceptance_mode_mcc_diverges_from_linear_on_diverging_parent` | J.2 | I.4 + I.6 differential |
| `test_parent_acceptance_mode_typo_falls_through_to_linear` | J.2 | typo-safety |
| `test_governance_set_param_*` (4 tests) | K.2 | allowlist contract |

### Doctrine sweeps

| Lane | What | Commit |
|---|---|---|
| Layer 1 | INVENTION_STACK §A1.2 T1 + T2 wording fixes; MERA caveat; LightCone read-only note; CSLC endpoint label | `bfaa758` |
| H.1 | DOCTRINE_PUNCH_LIST Layer 0 #4 marked closed (verified `collect_demurrage` already wired) | `3f8d84b` |
| Layer 0 closure | DOCTRINE_PUNCH_LIST Layer 0 #3 + #5 marked closed | `f507434` |
| M.1 | DOCTRINE_PUNCH_LIST bullet sweep — 14 stale `[ ]` items closed with commit refs | `944879b` |
| M.2 | INVENTION_STACK MERA references swept post-VERKLE verdict | `66a84a4` |

### Final test counts (Mini 1)

- 415 evaporchain-consensus lib tests pass
- 0 regressions across the session
- ~1,536 proptest randomised assertions × 256 inputs = ~393k checks
  per `cargo test`

### What's still genuinely open

| Item | Effort |
|---|---|
| Layer 5 — Lambda-Fold real Nova IVC (`state_root_to_u64` truncation, `RealBlockCircuit` arity 6→7) | 3-6 weeks |
| Layer 6 — Crooks-MEV refund consensus integration (substrate exists, no consensus hot-path wiring) | multi-day |
| Layer 6 — Light-Cone full consensus rewrite (replaces tendermint.rs's 8.7K LOC) | months |
| Layer 7 — LLSA full theorem-grade governance (or descope to k-of-n auditor signatures) | 9-15 months OR 4-6 weeks |
| M2 — Coq build verification (manual) | 10 min |
| M3.1 — INVENTION_STACK §A1.2 T1 wording (Satyawan strategic call) | 30 min |
| M3.2 — INVENTION_STACK §A1.2 T2 wording (Satyawan strategic call) | 30 min |
| Layer 2 — CSSR (Shalizi-Klinkner ε-machine reconstruction) | 2-3 sessions |

### Cluster operations

3-Mini Tailscale cluster experienced a divergence event mid-session
(wipe-restart of Mini 1 with peers at h=771 produced a fork that the
current sync protocol couldn't reconcile — Mini 1 stuck at h=178, peers
halted at h=771 awaiting BFT 2/3+1). Cluster ops were de-prioritised in
favour of the building work above. Cluster-wide reset can be issued
later via `restart-tailscale-3node.sh` on all 3 Minis simultaneously.
