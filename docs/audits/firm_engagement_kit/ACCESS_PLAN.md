# Audit Firm Repository Access Plan

How an external audit firm gets the EvaporChain source code under engagement. Designed to balance auditor productivity (fast access, no friction) against source-code custody (no leaks, traceable).

This document is shared with the auditor after NDA signature, as part of kickoff materials.

---

## 1. Default access path — GitHub repository invite

Preferred when the audit firm has a corporate GitHub presence and individual auditors with verified work emails.

### Setup

1. The firm provides:
   - A list of named auditor GitHub usernames (no shared accounts)
   - A list of corresponding work email addresses
   - The firm's GitHub Enterprise organisation name (for cross-org auditing if applicable)

2. Project creates a dedicated GitHub team `audit-{firm-name}-{quarter}` and grants:
   - **Read access** to `ss1738/EvaporChain` (primary monorepo)
   - **Read access** to dependent repos: `ss1738/evaporchain-website`, any private SDK repos
   - **No write access**, no fork permission, no actions trigger
   - Tag-pinned access if available (the firm reviews a specific commit / release tag, not rolling main)

3. Each auditor enables:
   - GitHub two-factor (TOTP or hardware key, not SMS)
   - Verified email matching the firm's domain
   - SSH key registered for clone access

4. The project enforces:
   - Audit-log review of access events (`who pulled, when, from what IP`)
   - SAML enforcement on the EvaporChain organisation (mandatory for the audit team if firm has a SAML provider)
   - Suspension of access at engagement end (auto-removal from team)

### Why GitHub default

- Auditors already use it; no new tooling friction.
- GitHub audit log is sufficient for our forensic needs.
- Tag-pinned access prevents the auditor from seeing in-flight uncommitted hot-fix work that may be sensitive.

---

## 2. Alternative path — signed git bundle delivery

Used when the firm requires air-gapped handling (e.g., for a formal-verification engagement, or if the firm's policy disallows third-party Git hosting access).

### Setup

1. Project produces a signed bundle:

```sh
cd ~/EvaporChain
git bundle create /tmp/evaporchain-audit-{tag}.bundle --all --tags
gpg --detach-sign --armor /tmp/evaporchain-audit-{tag}.bundle
sha256sum /tmp/evaporchain-audit-{tag}.bundle > /tmp/evaporchain-audit-{tag}.bundle.sha256
```

2. Bundle is delivered via:
   - SCP push to a firm-controlled host (firm provides credentials over a secure channel — Signal / encrypted email)
   - Or: SecureDrop / Magic Wormhole / similar over a one-time channel
   - **Never** plain email, Dropbox, Google Drive, or any web upload that's not E2E encrypted

3. Firm verifies:
   - GPG signature against project's published key (in repo `KEYS.txt` and on keys.openpgp.org)
   - SHA-256 hash matches the value published in the engagement contract

4. Firm clones from the bundle locally:

```sh
git clone /path/to/evaporchain-audit-{tag}.bundle EvaporChain
```

5. At engagement end, firm certifies destruction in writing per NDA §4(c).

### Updates during engagement

If the firm requires updates (e.g., to verify fixes in re-audit), repeat the process with a new bundle and tag. **Do not** ask the firm to `git pull` from a remote during an air-gapped engagement.

---

## 3. Access scope by engagement phase

| Phase | What the firm accesses |
|---|---|
| Pre-engagement (NDA signed, contract not yet) | Public repo only, plus the audit-readiness pack and RFP delivered separately |
| Active audit | Full primary repo at agreed audit-baseline tag (e.g., `audit-baseline-2026-XX-XX`) |
| Re-audit pass | Audit-baseline tag + diff to fix-PR commits, identified by finding ID labels |
| Post-engagement (final report delivered, embargo period) | None. Final report archived locally per NDA §4(c) |

The audit-baseline tag is created by the project at kickoff. Subsequent in-flight work on `main` is not in scope unless the firm explicitly asks (e.g., to verify a fix landed cleanly).

---

## 4. Information NOT in the audit repo

The following are NEVER shared via the standard access mechanism, even with the engaged firm:

- Production validator BLS / TLS / ML-DSA secret keys (not in repo)
- Node operator passwords, KMS / HSM credentials
- Treasury wallet keys
- Personal email backups / DMs / Slack
- Other Satyawan-Singh-owned project source (FINGAURD, ZovoNotes, CardioSafe, Vayu, etc.) — separate engagement if relevant
- Customer / validator personal data

If any of these are needed for a specific finding, they'll be shared one-off via the same air-gapped channel as §2 with explicit per-item written authorisation.

---

## 5. Firm offboarding checklist

Triggered when:
- Engagement ends per contract
- A specific auditor leaves the firm (notification within 5 business days expected)
- Material breach of NDA (immediate)

Steps:

1. Project removes the firm's GitHub team from the EvaporChain organisation. Confirms no orphaned individual collaborators.
2. Project rotates any deployment-side secrets that may have been mentioned in shared materials (CI tokens, monitoring API keys, etc.) — even if not directly disclosed.
3. Firm certifies in writing (per NDA §4(c)):
   - All source-code copies deleted from firm systems within 30 days
   - Working notes archived only as required by professional indemnity policy
   - Final audit report retained per legal requirement
4. Project files the certification with the engagement record.

---

## 6. Audit-log retention

Project retains:
- GitHub access logs for the engagement period + 12 months
- All written communications about scope, findings, and fixes
- Final deliverables
- NDA + contract + insurance certificate

For a period of **6 years from engagement end** (or longer if applicable law requires).

---

## 7. Things the firm should ask for, and our default answers

| Firm asks | Default answer |
|---|---|
| "Can we have write access to push fix-PRs?" | No. We accept fix suggestions inline in findings; we author the PRs. |
| "Can we run the test suite on our own infrastructure?" | Yes — build locally per `Cargo.toml`. We do not provide a remote test rig. |
| "Can we deploy a node and observe live behaviour?" | Yes for local dev networks. Production-validator node access requires per-item written authorisation. |
| "Can we use AI tools (e.g., Cursor, GitHub Copilot) on the code?" | Yes for code review assistance, **provided** the AI tool's data-handling policy meets the NDA confidentiality bar. Disclose which tools are used in the kickoff. |
| "Can we publish blog posts about the engagement before the embargo expires?" | No. Public commentary (including teasers) is embargoed until 30 days post-final-report. |
| "Can we credit ourselves on a public client list?" | Yes after the embargo, with our written approval of the listing wording. |

---

## 8. Open items to confirm before first firm engagement

- [ ] GitHub Enterprise upgrade (currently on free tier; SAML enforcement requires Enterprise)
- [ ] Project GPG signing key generated and published (for §2 bundle signing)
- [ ] Firm-name placeholder in NDA template replaced
- [ ] Insurance certificate received from firm
- [ ] Audit-baseline tag procedure documented in the engineering handbook (so the team knows to cut a clean tag at kickoff)
