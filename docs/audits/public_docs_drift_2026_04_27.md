# Public-Docs Drift Report — 2026-04-27 (re-audit 2026-05-02)

## Re-audit 2026-05-02 — canonical numbers verified against the live codebase

| Item | Value | Source of truth |
|------|------:|------------------|
| EvaporScript opcode count | **44** | `crates/evaporchain-script/src/compiler.rs:9 enum Op` |
| Contract templates | **8** (was claimed 6/7) | `crates/evaporchain-contracts/src/lib.rs ContractTemplate` enum |
| Workspace `crates/` entries | **78** | `Cargo.toml` |
| Core crates (long-standing) | **18** | types, consensus, node, network, state, execution, contracts, crypto, proving, da, cli, mcp, script, script-lad, wsbf, ib-validators, sentinel, sddc |
| Substrate primitives | **60** | The remaining `crates/` members |
| Workspace tests | **6,046 / 6,051 pass** | `cargo test --workspace --release` on satyawan 2026-05-02. 0 failures + 5 ignored. (An earlier truncated count of 2,781 in this report's first draft was a parser artifact from a `tail -50` on the per-binary result lines — the un-truncated log shows 173 result lines summing to 6,046 passes.) |

In-source patches applied this re-audit (commits to follow):
- `crates/evaporchain-contracts/src/lib.rs` — top doc-comment 6→8 templates; `ContractTemplate` doc 6→8.
- `README.md` — "7 templates" → "8 templates".
- `REMAINING_WORK.md` — Phase 4 line 7→8 templates.
- `FULL_AUDIT_2026_04_24.md` — "91 opcodes / ~95" → "44 opcodes (cross-checked)".
- `audit/firm_engagement_kit/SCOPING_FAQ.md` — "91-opcode VM" → "44-opcode VM".

Items still open (not patched here): whitepaper §4 rewrite (Tendermint not Health-Score), pitch-deck Mysticeti/RSA-accumulator/founder-plurality drift, EF + Sui grant text. Those need narrative editing, not number swaps; the original 2026-04-27 report below has the full action list.

**Note on test count**: 6,046/6,051 (`cargo test --workspace --release` on satyawan 2026-05-02 evening). Up from the prior memory of 5,801/5,807 by ~245 tests (the regression repros for nova `state_root_to_u64` plus FoldQueue tests + others added during this sprint). Single-source-of-truth: this number is from the un-truncated test-result-line sum.

No failing tests in the latest run.

---

# Public-Docs Drift Report — 2026-04-27 (original)

The whitepaper, pitch deck, grant applications, and arXiv submission notes contain claims that do not match the 2026-04-27 codebase reality. Sending any of these out — to arXiv, to Ethereum Foundation, to Sui Foundation, to investors — risks misrepresentation and credibility damage.

This document lists every drift item found, with `claim → reality → action`. Per the project rule "Don't generate CVs, decks, or legal copy without explicit source — fabricated metrics have caused real damage," do **not** ship any of these documents externally without first applying the fixes below.

---

## Severity legend

- 🟥 **Wrong technical claim** — could mislead a reader / damage credibility / disqualify a grant
- 🟧 **Stale metric** — was true at some point, no longer current
- 🟨 **Soft claim** — defensible but should be tightened
- 🟩 **Internal naming inconsistency** — confusing across docs but not externally fatal

---

## 1. Whitepaper (`research/whitepaper.md`, 1026 lines, dated "March 2026 v1.0")

### 1.1 🟥 Consensus chapter is fundamentally wrong

**Claim (§4):** "Consensus: Rotating Leader with Health Scores" — 23 pages of content describing a custom rotating-leader scheme with `HEALTH_PER_EVAPORATION = 0.05`, `HEALTH_DECAY_RATE = 0.01`, deterministic stake-weighted leader election.

**Reality:** Code is **Tendermint BFT** with stake-weighted 2/3 quorum, BLS12-381 aggregate signatures, prevote/precommit phases, view-change with exponential timeout, equivocation slashing. See `crates/evaporchain-consensus/src/tendermint.rs`. Health-score consensus does not exist in the current codebase.

**Action:** Rewrite §4 entirely against the Tendermint implementation. Cite `tendermint.rs:65` for `MAX_ROUNDS_PER_HEIGHT = 10`, `bridge.rs:79` for the 2/3 quorum check, `validator_set.rs:341-393` for slashing. Use the `docs/architecture/diagrams/consensus_state_machine.mmd` Mermaid diagram as the figure source.

### 1.2 🟧 Sections to verify against current code

The whitepaper covers fee market, mempool, EvaporScript, Nova proving, networking, economic model. Each needs a single read-pass against current source. Likely-stale areas:

- **§8 (Encrypted mempool):** Whitepaper may describe an older scheme. Code has AES-256-GCM encrypted mempool per pitch slide 12 — confirm the chapter matches.
- **§9 (Fee market):** PID controller exists; verify exponents/parameters match the chapter.
- **§10 (EvaporScript VM):** confirm opcode count and gas table match (current source: **44 opcodes** in `compiler.rs:11 enum Op`).
- **§11 (Nova proving):** verify the engine and parameters match `evaporchain-proving` (Bn256/Grumpkin + HyperKZG per the benchmark report).
- **§14 (Economic model):** must reflect current `genesis-mainnet.json` tokenomics: total_supply 1B, block_reward 100, half_life 1M, fee_burn 50%, staker_share 50%, target_apy 5%.
- **§16 (Comparison with existing chains):** dates / numbers against competitor chains may be stale.

**Action:** §1.1 rewrite is the heavy lift. The other sections need a single verification pass — most chapters are likely accurate, but each one needs at least one ground-truth check.

### 1.3 🟧 Author affiliation

**Claim:** Author line reads "Satyawan Singh, Infonova Solutions, Leicester, United Kingdom"

**Reality (per `~/.claude/CLAUDE.md`):** "Infonova Solutions Ltd ≠ FINGAURD Ltd. They are separate companies with separate owners." Madhu Dasari is the director of Infonova, not Satyawan. EvaporChain is Satyawan's solo project.

**Action:** Change to "Satyawan Singh, Independent Researcher, Leicester, United Kingdom" or "University of Leicester (student)" if the affiliation must include an institution. Do not list Infonova as the author affiliation for EvaporChain.

---

## 2. Pitch Deck (`pitch/PITCH_DECK_CONTENT.md`, 196 lines, uncommitted change)

### 2.1 🟥 Slide 5 — Wrong accumulator primitive

**Claim:** "Evaporated objects leave a **cryptographic nullifier** (RSA accumulator membership proof)"

**Reality:** Code uses **MMR (Merkle Mountain Range)**, not RSA accumulator. README, whitepaper §7, and `crates/evaporchain-crypto` all confirm MMR.

**Action:** Replace "RSA accumulator" with "MMR (Merkle Mountain Range) nullifier accumulator." Update slide 8 too — same error.

### 2.2 🟥 Slide 8 — Wrong consensus

**Claim:** "Consensus: Mysticeti DAG-BFT — sub-second finality, proven at scale on Sui"

**Reality:** Tendermint BFT, not Mysticeti. Mysticeti is a Sui DAG-BFT design. EvaporChain has never implemented Mysticeti.

**Action:** Replace with "Tendermint BFT — stake-weighted 2/3 quorum + BLS12-381 aggregate signatures + checked-arithmetic execution."

### 2.3 🟥 Slide 13 — Roadmap repeats Mysticeti error

**Claim:** "Public testnet with Mysticeti consensus — Q2 2026 — Planned"

**Reality:** Same drift. Also Q2 2026 is now (April 2026), and there's no public Mysticeti testnet.

**Action:** Replace with "Public testnet with Tendermint BFT consensus — Q3 2026 — Planned" (after the 8-12 weeks of Gap A code fixes per `audit/end_to_end_audit_2026_04_27.md`).

### 2.4 🟧 Slide 5 — Grace period unit mismatch

**Claim:** "7-day grace period"

**Reality:** Code constant `GRACE_PERIOD = 5` (`crates/evaporchain-node/src/main.rs:163`). The unit is **epochs**, not days. With block_interval_ms=2000, 5 epochs ≠ 7 days.

**Action:** Replace with the actual number tied to current parameters: "5-epoch grace period" or "~10-second grace at 2-second block time" (or whatever the per-protocol epoch length resolves to — see `docs/PARAMETERS.md` once epoch length is documented).

### 2.5 🟧 Slide 7 — Benchmark numbers from prototype

**Claim:** "1,000 blocks in 6.2 seconds with 6.2ms per block"

**Reality (per `pitch/BENCHMARK_REPORT.md`):** This is from the `prototypes/fold-a-block` Nova IVC prototype, not the live testnet. The 3-Mini live cluster runs ~9.5 blocks/sec total throughput, which is a different metric. The 6.2ms/block proving speed is real but specific to the Bn256/Grumpkin + HyperKZG proving prototype.

**Action:** Either (a) cite as "Nova IVC proving prototype: 6.2ms amortized per block" with explicit prototype scope, or (b) replace with current testnet metrics. Avoid letting a reader assume live-chain throughput is 6.2ms/block.

### 2.6 🟥 Slide 14 — "Founded by engineers"

**Claim:** "Founded by engineers with ML/systems background"

**Reality:** Solo founder per CLAUDE.md ("Solo builder across every project listed below").

**Action:** "Founded by Satyawan Singh — ML Engineer and University of Leicester student." Or singular "engineer." Plural misrepresents the team.

### 2.7 🟥 Title slide — Company affiliation

**Claim:** Title slide and elsewhere lists "Infonova Solutions Ltd"

**Reality:** Same as §1.3 above. Infonova is a separate company owned by Satyawan's cousin (Madhu Dasari). EvaporChain is solo.

**Action:** Remove Infonova from the deck. Use "Independent" or "Satyawan Singh" or, if a Limited company is being formed for EvaporChain, use that name once registered. Do not borrow Infonova's name.

### 2.8 🟧 Slide 9 — Template contract count

**Claim:** "7 template contracts shipped: DecayingToken, MortalNFT, ThermodynamicEscrow, DecayingAuction, StakingPool, DAOVote, TemporalContract"

**Reality:** Audit-readiness pack and `audit/audit_readiness_pack_2026_04_27.md` say "6 template contracts + rule engine." The README crate map says "7 templates." Pitch lists 7 by name. Need to verify the canonical count.

**Action:** Read `crates/evaporchain-contracts/src/lib.rs` and count actual `impl` blocks for each template. Use that number in all docs (README, whitepaper, pitch, grants).

---

## 3. Ethereum Foundation Grant (`grants/ethereum_foundation.md`)

### 3.1 🟧 Test count stale

**Claim:** "4,159 passing tests across 13 Rust crates"

**Reality:** README claims 4,668+; audit-readiness pack 4,486+. Workspace has 16 members, not 13.

**Action:** Use the README number after `cargo test --workspace` is run on a Mini and confirmed. Crate count: 16.

### 3.2 🟥 Public testnet doesn't exist

**Claim:** "Live testnet: https://testnet.evaporchain.com"

**Reality:** No public testnet. 3-Mini Tailscale private cluster only. README also says "Coming soon" for the public testnet.

**Action:** Remove the URL. Replace with "Internal multi-node testnet running on Apple Silicon Mac Minis; public testnet planned Q3 2026 (post-audit)."

### 3.3 🟧 Whitepaper size

**Claim:** "188KB whitepaper with 70 academic citations"

**Reality:** `research/whitepaper.md` is 40KB. The "188KB" may be a PDF rendering or an earlier version. Citation count not verified — `grep -oE "\[[0-9]+\]"` returned 0 (citations may use a different format).

**Action:** Run `wc -c research/whitepaper.md` and any rendered PDF; quote the actual size. Count citations by reading the bibliography section. Use real numbers.

### 3.4 🟧 Founder credentials

**Claim:** "Computer science graduate (2026)"

**Reality (per CLAUDE.md):** "Currently studying at the University of Leicester."

**Action:** Either "expected graduation 2026" or "BSc Computer Science, University of Leicester (in progress)." Do not claim a degree not yet received.

### 3.5 🟨 Performance claims

**Claim:** "PID controller … 3-5x lower fee volatility"; "Nova IVC folding at 6.2ms per block on commodity hardware."

**Reality:** Nova benchmark is real (`pitch/BENCHMARK_REPORT.md` confirms 6.2ms with explicit methodology). PID volatility claim — find the source backtest or remove.

**Action:** Cite the benchmark report for Nova; either find the PID backtest evidence or soften to "PID controller designed for lower fee volatility than EIP-1559's exponential adjustment."

---

## 4. Sui Foundation Grant (`grants/sui_foundation.md`)

### 4.1 🟥 Move-compatibility claim

**Claim:** "EvaporChain extends the Move language with thermodynamic state decay"; "Move-compatible execution engine with decay semantics"

**Reality:** EvaporScript is **a bespoke 44-opcode stack VM**, not Move. There is no Move parser, no Move compiler, no shared Move object model. The only Move-adjacent thing is "EvaporScript demonstrates how Move COULD support temporal types" — but that's a research observation, not an implementation.

**Action:** Either (a) **build actual Move-extension prototype before applying**, or (b) reframe the application honestly: "EvaporScript: a temporal-type scripting language that demonstrates language-design patterns Move could adopt, with reference implementation in 44 opcodes." If (b), the grant ask should be lower (research/comparison work, not engine work).

**Risk:** Submitting the current text could be read by Sui Foundation reviewers as misrepresentation, disqualifying the application and damaging future relations. Don't ship as-is.

### 4.2 🟧 Test count stale

Same as §3.1.

---

## 5. arXiv Submission (`announcement/arxiv_submission.md`)

### 5.1 🟧 Filename mismatch

**Claim:** "Convert EVAPORCHAIN_WHITEPAPER.md to PDF first"

**Reality:** File is `research/whitepaper.md`.

**Action:** Update the pandoc command path.

### 5.2 🟥 Don't submit until truth-pass complete

The whitepaper has the consensus drift in §1.1. Submitting to arXiv before the rewrite would publish a paper that contradicts the implementation. arXiv submissions can be replaced/updated, but the v1 listing is permanent.

**Action:** Block arXiv submission until §1.1, §1.2, §1.3 of this drift report are resolved.

---

## 6. Benchmark Report (`pitch/BENCHMARK_REPORT.md`)

### 6.1 🟧 Grace-period unit

**Claim:** "objects reaching zero energy enter a 7-day grace period before evaporation"

**Reality:** Same as §2.4. Code constant is 5 epochs.

**Action:** Replace "7-day" with "configurable" or the real epoch count.

### 6.2 🟩 Otherwise, the report appears accurate

The methodology, engine choice (Bn256/Grumpkin + HyperKZG), batching strategy (5 blocks per fold step → 200 fold steps for 1,000 blocks), and verdict (PASS) are internally consistent. The Mina/SP1/Polygon/StarkNet comparisons are accurate as written.

---

## 7. Internal cross-doc inconsistencies

These don't affect external credibility individually but cause confusion across the project:

- **Test count drift:** README 4,668+, audit-readiness 4,486+, grants 4,159, audit 2026-04-24 said 4,375. Single source of truth needed: run `cargo test --workspace` once on a Mini, record the number, and update all four files in the same commit.
- **Crate count drift:** grants say 13, audit says 16, README crate map shows 14 names but Cargo.toml has 16 members (15 named + integration tests + wallet + prototype). Use **16 workspace members** as canonical, with breakdown.
- **Opcode count drift:** my own `audit/audit_readiness_pack_2026_04_27.md` and `audit/end_to_end_audit_2026_04_27.md` cite "91 opcodes" — **this was wrong**, I'll fix it. Source of truth: 44 opcodes (`compiler.rs:11 enum Op`).
- **Template contract count:** 6 vs 7 (see §2.8 above).

---

## 8. Recommended action sequence

In order of dependency:

1. **Fix the 91→44 opcode error in my own audit docs** — five-minute edit, removes cascading error.
2. **Verify template contract count** in `evaporchain-contracts/src/lib.rs` — single source of truth.
3. **Run `cargo test --workspace` on a Mini** — get the real test count.
4. **Whitepaper §4 rewrite** (Tendermint not Health-Score consensus) — biggest single fix.
5. **Whitepaper §1.3, §8-§14 verification pass** against current code.
6. **Pitch deck fixes** for slides 5, 8, 9, 13, 14 + title.
7. **EF grant rewrite** with correct testnet language, founder credentials, citation count.
8. **Sui grant decision:** either soften the Move claim significantly or do not submit.
9. **arXiv submission package** — only after whitepaper rewrite is complete.

The numbered fixes are independent of the code-fix session. They need accurate data, not new code.

---

## 9. What doesn't drift (good news)

- Core thesis: thermodynamic state decay, energy-based half-life, MMR ghost records, post-quantum signatures via ML-DSA. All real and accurate as concept.
- Benchmark report's Nova proving numbers (6.2ms / block, 11.3KB proof, 15ms verify) are real prototype results.
- Slide 12 economic model (refresh fees, fee burn, target staking APY 5%) matches `genesis-mainnet.json`.
- Slide 11 competitive landscape (vs Mina, Celestia, Sui, Ethereum) is reasonable.

The drift is concentrated in: consensus naming (Mysticeti / Health-Score → Tendermint), accumulator naming (RSA → MMR), test/crate counts, and unsupportable claims (Move-compat, public testnet URL, "founded by engineers" plural).

---

## 10. After fixes — what to do with the corrected docs

- Whitepaper → arXiv submission once §4 rewrite is verified by the auditor.
- Pitch deck → ready for investor conversations only after fact-fixes; the £8-12M raise is a separate decision.
- EF grant → submit after fixes; £50K addresses ~half the audit budget.
- Sui grant → decision required: build Move-extension prototype or pull the application.
- Benchmark report → minor edits, can ship near-immediately.

This drift report itself is a deliverable for the engaged auditor — it shows the level of internal-consistency review the project applies.
