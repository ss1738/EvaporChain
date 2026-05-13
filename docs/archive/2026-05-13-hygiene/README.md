# Doc hygiene pass — 2026-05-13

This directory holds documents archived during the doc-hygiene pass on **2026-05-13**. None of them are deleted — they remain in git history. They live here so the live repo surface is smaller and clearer, while the archive stays accessible for any historical reference.

**See** the canonical-docs list at the root `README.md` for what is live. Everything in this directory has been moved out of the live surface.

## What's archived here and why

### Phase decision logs (4 files)

Snapshots of decision-moments from specific build phases. Now that the phases are shipped, the decisions live in code and `CHANGELOG.md`. The logs are historical reference only.

- `PHASE_3_DECISIONS.md` — `research/light_cone/` Phase 3
- `PHASE_4_DECISIONS.md` — `research/light_cone/` Phase 4
- `lambda_fold_PHASE_1_DECISIONS.md` — `research/lambda_fold/` Phase 1
- `crooks_mev_PHASE_2_DECISIONS.md` — `research/crooks_mev/` Phase 2

### Gate-result artifacts (3 files)

Historical records of specific research gates firing. The gates' outcomes are in code; these markdown logs are paper-trail only.

- `causal_chsh_GATE_RESULT.md` + `causal_chsh_GATE_RESULT_3K.md`
- `mera_gate_GATE_RESULT.md`

### Duplicate plan docs (2 files)

`MAINNET_READINESS.md` (still live at repo root) is the single canonical lane board. These two were prior planning attempts now superseded:

- `MAINNET_SPRINT_PLAN_2026_05_11.md` — date-stamped sprint plan, content merged into `MAINNET_READINESS.md` lane specs
- `DOCTRINE_PUNCH_LIST.md` — prior punch-list format, now covered by `MAINNET_READINESS.md` Tier 0/1/2/3 lane structure

### Genesis ceremony rehearsal (1 file)

- `GENESIS_CEREMONY_REHEARSAL.md` — single-use rehearsal log. The canonical ceremony procedure lives in `docs/GENESIS_CEREMONY.md`.

### Multi-token gas option exercise (2 files)

- `MULTI_TOKEN_GAS_OPTIONS.md` — was a decision exercise comparing 3 options
- `MULTI_TOKEN_GAS_VERIFICATION.md` — companion verification artifact

The chosen option is encoded in code + `docs/PARAMETERS.md`. The exercise itself is historical.

## What was NOT archived (also moved during this pass — to `docs/audits/`)

9 audit-related docs were consolidated into `docs/audits/` (not into this archive), since open audit findings are still actionable. See the canonical-docs list for current audit surface.

## How this pass was executed

Single PR: `pr/docs-hygiene-2026-05-13`. 21 files moved via `git mv` (preserves history). No content deleted, no edits made to file contents — purely a directory reorganisation.

## Going-forward rule

Per the strategy committed in `meta_strategic_question_flow.md` and reinforced in each project `CLAUDE.md`:

> Before writing any new strategy / plan / decision / audit doc, ask: "Does this go in `SESSION_PROGRESS.md`, `CHANGELOG.md`, `MAINNET_READINESS.md`, or one of the canonical files? If yes, write it there. If no, the answer is probably 'don't write the doc.'"

If this hygiene pass needs to be repeated within 6 months, that's a signal the going-forward rule isn't being followed.
