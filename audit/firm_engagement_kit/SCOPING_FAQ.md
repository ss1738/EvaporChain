# Audit Firm Scoping FAQ

Anticipated questions from auditors during the scoping phase, with prepared answers. Keeps the project responsive on the call (firms judge engagement quality on whether you can answer these without "let me get back to you").

This document is **internal**. The auditor never sees it. Once a firm is engaged, individual answers may be lifted into the deliverables they request.

---

## A. Codebase

**Q1. How big is the codebase?**
~66K LOC of Rust across 16 workspace crates plus a few prototype/test crates. See `docs/PARAMETERS.md` and `audit/audit_readiness_pack_2026_04_27.md §3` for per-crate LOC.

**Q2. What's the test count and coverage?**
4,486+ tests as of 2026-04-27. Test breakdown: ≥19 byzantine adversarial scenarios, proptest harnesses, fuzz harnesses in `fuzz/`. **Coverage report not yet generated** — committed to deliver before kickoff via `cargo-llvm-cov` on a Mini.

**Q3. What's the build environment?**
Rust stable; specific edition pinned to 2021 in workspace `Cargo.toml`. Workspace `Cargo.lock` is committed. Hermetic build via `cargo build --workspace`. Build runs only on the Apple-Silicon Macs in our test cluster; we do not develop on x86 currently.

**Q4. Are there architecture diagrams?**
Yes. `docs/architecture/diagrams/` has Mermaid diagrams for tx lifecycle, consensus state machine, DA flow, validator key lifecycle, cross-shard messaging. ASCII overview in `docs/ARCHITECTURE.md`.

**Q5. What's the scope you'd like audited?**
See `audit/external_audit_rfp_2026_04_27.md §3`. Headline: 13 in-scope crates covering consensus, execution, contracts, crypto, proving, DA, state, oracle, network, sharding, types, script, plus the node binary's key-load path. Out of scope: CLI, MCP stub, WASM bindings, application-layer wallets/dapps, tokenomics calibration.

## B. Project status

**Q6. Has the codebase been audited internally?**
Yes — internal multi-agent review on 2026-04-24 (`FULL_AUDIT_2026_04_24.md`), follow-up cross-verification on 2026-04-27 (`audit/cross_verification_2026_04_27.md`), and an end-to-end domain-by-domain review (`audit/end_to_end_audit_2026_04_27.md`). All three are supplied under NDA.

**Q7. Are there known issues?**
Yes. Listed in `audit/audit_readiness_pack_2026_04_27.md §5` and detailed in the cross-verification + end-to-end audit. Headline open items: oracle authentication broken, contract upgrade is a no-op, DA encoder not wired into block production, governance has no parameter bounds. We expect these closed before kickoff; auditor's day-1 job is to confirm closure quality, not re-discover.

**Q8. Is there a public testnet?**
Not yet. Currently a 3-node private Tailscale testnet on Mac Minis. Public testnet planned during the audit window. Auditors are welcome to spin up local devnets.

**Q9. When are you launching mainnet?**
Realistic: 8-14 months from 2026-04-27. The `genesis_time` in `genesis-mainnet.json` is a sprint placeholder, not a real launch target. Audit timeline drives the actual date. We are not optimising for fastest-possible launch.

**Q10. Who is the team?**
Solo founder + technical lead (Satyawan Singh). Some workstreams (frontend, marketing) handled by a separate cousin-led entity (Infonova Solutions Ltd) but EvaporChain protocol-level work is solo. This affects audit logistics: single point of contact, no separate "core team" call to schedule.

## C. Engagement model

**Q11. What's the budget envelope?**
`[FILL IN BEFORE SENDING]` — pick from these brackets when responding:
- "Tier 1 expectations" (TOB / Sigma Prime / Zellic): £150K-400K range
- "Tight": £100K-200K, scope reductions accepted
- "Open": willing to invest if the firm justifies the cost

**Q12. Fixed-price or time-and-materials?**
Strong preference for **fixed-price for the initial scope**, with a defined re-audit pass included. T&M acceptable for follow-on optional engagements (formal verification supplement, primitive review).

**Q13. How many findings rounds?**
One full audit + one re-audit pass after fixes is standard. Additional rounds at agreed T&M rate.

**Q14. Will you provide a public report?**
Yes — the final report is published with a 30-day embargo from delivery to allow for fixes. We expect the auditor to be public-by-default; tell us if you require redactions for active CRITICAL items (acceptable) or for any reason beyond that (we'd like to understand why).

**Q15. Timeline preference?**
RFP sent week 0. First-round scoping calls in weeks 1-3. Proposals due week 4. Selection weeks 5-6. Contract weeks 6-8. Kickoff weeks 8-12. We can flex on kickoff slot to align with your calendar.

## D. Logistics

**Q16. NDA?**
We have a UK-favourable mutual NDA template (`audit/firm_engagement_kit/NDA_TEMPLATE.md`). We're also fine to redline yours if it's no looser than ours on source-code handling and term. Governing law: England and Wales preferred.

**Q17. Repository access?**
See `audit/firm_engagement_kit/ACCESS_PLAN.md`. Default: GitHub Enterprise organisation invite to named individuals on the audit team, read-only access on a per-tag basis. Alternative: signed git bundle delivery if the firm prefers air-gapped handling.

**Q18. Communication cadence?**
Weekly written status updates from the firm; ad-hoc Slack / Signal channel for questions; one or two synchronous calls per week. We're in UK time zone; happy to flex calls into US/AU windows.

**Q19. Findings format?**
Markdown delivered as a private GitHub repo or PDF. We're flexible. We use `severity / file:line / one-line description / one-line fix` as the internal format and would appreciate consistency with that.

**Q20. Re-audit trigger?**
We'll mark each fix PR with the finding ID. Re-audit pass runs once all CRITICAL/HIGH have a marked PR merged. Estimated 2-4 weeks from PR-merge-complete to re-audit start.

## E. Technical specifics

**Q21. What's novel that we should be careful about?**
- **Energy decay** — every object has a decay curve; verify decay determinism across nodes near boundary timestamps.
- **Nova IVC folding** — recursive proof folding with HyperKZG; transcript binding to block state is a soundness anchor we'd like fresh eyes on.
- **Energy-Verkle Trie** — combined active-state Verkle + expired-state MMR. Single-root construction is in `evaporchain-crypto`; verify correctness against forged-proof attempts.
- **EvaporScript** — bespoke 91-opcode VM. Gas metering complete per opcode but bytecode validation at deploy time is loose.
- **Hybrid signatures** — ECDSA + ML-DSA. Verify the verifier rejects mismatched-tag combinations.
- **Custom Poseidon constants** — generated via BLAKE3-derived seeds, not a published RFC parameter set. **Externally unaudited**; we want this in scope.

**Q22. What's the current cluster topology?**
3 Mac Minis (M4) on Tailscale, ~9.5 blocks/sec, 224+ tx executed, state roots equal across nodes. No geographic distribution, no hostile-network topology yet.

**Q23. What dependencies should auditors flag?**
`pqc_dilithium` (upstream unaudited), `nova-snark` (research-grade pin), `blst` (verify subgroup checks), `libp2p` (broad surface). See `audit/dependency_baseline_2026_04_27.md`.

**Q24. Is there a formal specification?**
Whitepaper exists in `research/` (188 KB, 70 citations). Spec one-pager in `docs/SPEC.md`. Per-component specs in `docs/CRYPTO_SPEC.md` and `docs/EVAPORSCRIPT.md`. No machine-checked formal spec.

**Q25. Any active incidents?**
None.

## F. Things we'd like the auditor to do beyond the standard

- **Adversarial harness extension.** We have ~19 adversarial scenarios; we'd like the firm to add 5-10 more covering attack vectors they discover.
- **Formal-verification scoping.** We're open to a side engagement covering EvaporScript opcode semantics or finality monotonicity invariants — give us a quote.
- **DA layer dedicated review.** The DA path has a known wiring gap; we'd like the auditor to verify the fix and the encoded path together rather than treating DA as one bucket alongside consensus.

---

## Notes for the responder

When a firm asks something not on this list:
1. If you know the answer, give it directly.
2. If you don't, say so and ask for 24-48h to follow up.
3. **Don't fabricate a number.** Particularly on test counts, LOC, dependency versions — those are publicly verifiable from the repo, and inaccurate numbers immediately downgrade trust.
