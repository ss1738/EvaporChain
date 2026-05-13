# `docs/audits/` — audit findings + audit-prep artifacts

Consolidated 2026-05-13 (see `../archive/2026-05-13-hygiene/README.md` for the broader hygiene pass).

Everything related to audit work — current open findings, historical audit reports, and audit-prep artifacts — lives in this directory. Before the consolidation these were scattered across the root + `audit/` + various date-stamped files in `docs/`.

## Layout

```
docs/audits/
├── README.md                                  ← this file
├── AUDIT_2026_05_11.md                        ← most recent internal audit findings
├── audit_readiness_pack_2026_04_27.md         ← prep pack for external auditor outreach
├── cross_verification_2026_04_27.md           ← multi-agent cross-verification report
├── dependency_baseline_2026_04_27.md          ← dependency tree + cargo-deny baseline
├── end_to_end_audit_2026_04_27.md             ← first end-to-end audit findings
├── external_audit_rfp_2026_04_27.md           ← RFP draft for external auditor engagement
├── FULL_COMPARISON_REPORT.md                  ← cross-protocol comparison artifact
├── public_docs_drift_2026_04_27.md            ← drift findings between code + public docs
├── NFT_TRACK_AUDIT_2026_05_03.md              ← NFT-track-specific audit
└── firm_engagement_kit/                       ← engagement materials for external auditor selection
```

## Rule

Going forward, new audit findings + audit-related artifacts go here. Don't create new audit-related markdown at the repo root or scattered in `docs/`.

If you're adding a new audit report, use the date-stamped pattern: `AUDIT_<YYYY>_<MM>_<DD>.md` so chronological ordering is obvious.

## Status of external audit

Per `feedback_evaporchain_external_audit.md` (memory) the external audit is deferred during the May–Oct 2026 build sprint. The `audit_readiness_pack_2026_04_27.md` and `external_audit_rfp_2026_04_27.md` are ready to engage when the sprint window closes.
