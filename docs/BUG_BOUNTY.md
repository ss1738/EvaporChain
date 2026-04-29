# EvaporChain Bug Bounty Program — Scoping Document

Companion to `SECURITY.md` at the repo root, which is normative for disclosure process and severity. This document defines the **scope, reward structure, and operational policy** of the formal bug-bounty program planned to launch ahead of mainnet.

**Status:** scoping draft, not yet active. Gap A code items are merged as of 2026-04-29. Program goes live when operational infrastructure (triage queue, escrow, payment rails) is in place — see §10 open questions.

---

## 1. Why have a bug bounty

Three concrete reasons:
1. Independent eyes catch what insiders miss. The formal external audit (RFP issued separately — see `audit/external_audit_rfp_2026_04_27.md`) covers a fixed scope at one point in time. A bounty is continuous coverage.
2. Mainnet validators expect one. Most institutional validators won't run a chain without a documented, funded bounty program.
3. It deters black-market disclosure. Cheaper to pay a researcher than to recover from an exploit.

A bounty is a complement to, not a substitute for, the external audit.

---

## 2. Scope

### In scope (highest tier)

- Anything in `crates/evaporchain-{consensus,execution,contracts,crypto,proving,da,state,oracle,sharding,types,network}` that produces:
  - Consensus break (two valid commits at the same height)
  - Loss-of-funds (unauthorized state mutation, balance forgery, double-spend, privacy-layer break)
  - Censorship (a small subset of validators can permanently halt the chain)
  - Validator compromise (key extraction from a running honest node, given only network access)
  - Privilege escalation in a deployed contract (any account upgrades a contract they don't own)

### In scope (medium tier)

- DoS that takes a single honest node down faster than honest network load
- Mempool / gossip / p2p bypass of admission rules
- Arithmetic over/underflow producing economic gain
- Slashing-condition false positive (honest validator gets slashed)
- Slashing-condition false negative (provable equivocation that doesn't slash)

### In scope (low tier)

- Information disclosure beyond what the protocol commits to publish
- Robustness debt that translates to crashes (panic, unwrap on attacker-controlled bytes)
- Operational hardening gaps (logged secrets, weak file permissions, missing TLS verification)

### Out of scope

- Issues in the user-facing dashboard / explorer / SDK that don't affect chain state
- Spam, social engineering, phishing
- DoS via load below the parameters in `docs/PARAMETERS.md`
- Issues in third-party dependencies (report upstream; we accept a finding if a working PoC against EvaporChain is included)
- Theoretical attacks without a runnable PoC against `main`
- Issues in `evaporchain-mcp` (currently a stub)
- Issues in `mobile-wallet` / `extension` / `dapps` directories during testnet phase

---

## 3. Reward tiers

Indicative ranges, USD. Final award is set by the program committee based on:
- Severity (consensus break > theft > censorship > DoS)
- Quality of disclosure (PoC, suggested fix, severity self-assessment)
- Novelty (truly novel finding > duplicate of internal audit ticket)

| Tier | Range | Examples |
|---|---|---|
| **Critical** | $25,000 – $100,000 | Consensus break, undetectable double-spend, mass key extraction |
| **High** | $5,000 – $25,000 | Single-validator key compromise via network, contract upgrade bypass, DA forgery |
| **Medium** | $1,000 – $5,000 | DoS taking down a node, slashing-condition false positive |
| **Low** | $250 – $1,000 | Robustness debt with reachable crash, weak operational hygiene |
| **Informational** | $0 – $250 | Hardening suggestions, documentation issues, theoretical attacks |

Reward currency: USD or USDC at researcher's choice. Payment within 30 days of the report being marked Resolved.

**Bonus criteria:** double the tier maximum if the report includes a working fix that's mergeable as-is and passes CI.

---

## 4. Disclosure process

Mirrors `SECURITY.md`. Email `security@evaporchain.io` with:
1. Description of the vulnerability
2. PoC against `main` (preferred) or a tagged release
3. Suggested severity
4. Suggested fix (optional, eligible for bonus)

Acknowledgment within 48 hours. Triage decision within 7 business days. Public disclosure happens **after** the fix is merged AND deployed to ≥ 2/3 of mainnet stake (or testnet if pre-launch). Researchers may publish their own write-up at that point.

Coordinated disclosure window is up to 90 days for Critical, 60 for High, 30 for Medium/Low. Faster if the issue is already actively exploited.

---

## 5. Safe harbor

We will not pursue legal action against researchers who:
- Act in good faith and follow this disclosure policy
- Avoid privacy violations, data destruction, or production-scale service disruption
- Provide sufficient information to reproduce
- Do not access data, accounts, or systems beyond what's necessary to demonstrate the issue
- Do not exfiltrate user data
- Do not exploit the issue for profit

This safe harbor extends to UK and EU researchers under the GDPR Article 32 / NIS2 good-faith defense and to US researchers under DOJ CFAA guidance for security research.

---

## 6. Eligibility

- **Open to:** anyone, anywhere, except as restricted below.
- **Not eligible:** current EvaporChain employees, contractors actively engaged on the codebase, immediate family of either, residents of jurisdictions under comprehensive UK / US sanctions (DPRK, Iran, Syria, Crimea, Donetsk, Luhansk).
- **Anonymous reports:** accepted, but the researcher must accept payment in USDC to a self-custody address; KYC may be required for fiat payouts above $10,000 per UK MLR / FATF rules.

---

## 7. Program governance

A three-person committee triages and decides on rewards:
- Chair: Project security lead (currently the founder)
- Operator representative: Rotating, drawn from genesis validator set
- External: Auditor of record (the firm engaged via `audit/external_audit_rfp_2026_04_27.md`)

Decisions by majority. Researcher may appeal once per finding. Final decisions are public-by-default after coordinated disclosure.

---

## 8. What changes after mainnet

- Critical-tier maximum increases to align with TVL (target: 1% of locked value, capped at $1M)
- Live tracker of resolved findings, anonymized, with hash + severity + reward
- Quarterly bounty leaderboard
- Hall of fame / acknowledgements page

---

## 9. Pre-launch testnet bounty

Until mainnet, all valid reports earn:
- Public credit in `SECURITY.md` Audits section
- An NFT on the testnet
- Priority eligibility (no waiting list) for the mainnet program

We reserve the right to backpay testnet researchers in cash at mainnet launch, depending on the severity and timing of the finding.

---

## 10. Open questions for committee

These need decisions before the program goes live:

- [ ] Final critical-tier ceiling (current draft: $100,000; institutional reviewers may push for $250,000)
- [ ] Escrow custodian (multisig vs custodial provider)
- [ ] PoC environment (private testnet, local devnet, or main)
- [ ] Minimum payout threshold (eliminate trivial reports?)
- [ ] Duplicate policy when two researchers find the same issue within 24h
