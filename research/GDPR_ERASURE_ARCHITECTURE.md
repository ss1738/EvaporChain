# GDPR-Erasure-as-a-Service — Architecture (model A: crypto-shred on verified chain behavior)

> Build-queue **slot 5** — the commercial wedge
> (`evaporchain_application_universe.md` cat 1, "largest commercial
> wedge"). Sibling of `SFSV_ARCHITECTURE.md` /
> `EVAPORCASH_ARCHITECTURE.md` / `DEAD_DROP_ARCHITECTURE.md`; same
> doctrine, same substrate, same verified node API. **Spec only — not
> the build.** Contract pilot: `contracts/evaporscript/gdpr_vault.es`
> (to build, single-instance mortal pattern).

---

## ⚠️ FOUNDING CONSTRAINT (read first — this is why model A exists)

A **directly-verified** chain behaviour bounds this entire product.
The Dead Drop §9 rigor probe (2026-05-17, contract_id 17, ~300s):
**after a contract reaches terminal `evaporated:true`, `GET
/api/script/:id` still returns `.state` — the bytes are NOT purged and
the endpoint does NOT 404.** The evaporation engine performs
*terminal liveness-death* (contract dead, unrefreshable, out of the
active set), **not byte-erasure**.

Therefore EvaporChain **cannot** truthfully offer "we deleted the
personal data from the chain." Selling that to a regulated buyer
would be the Dead Drop overclaim escalated into a **paid legal
compliance guarantee** — unacceptable. Model A is the design that is
*honest on verified behaviour*:

> **Erasure = crypto-shredding.** Personal data is never stored in
> chain state in plaintext. It is encrypted; the on-chain artifact is
> a tamper-evident **consent + retention lifecycle** contract whose
> own energy is the retention clock. GDPR Art. 17 erasure is effected
> by **destroying the decryption key** (off-chain), after which the
> ciphertext is permanently inaccessible. The chain's job is to be
> the *provable, tamper-evident clock and trigger*, not the eraser.

Crypto-shredding is an established, regulator-recognised erasure
technique (ICO / ENISA guidance: rendering data permanently
inaccessible by destroying keys constitutes erasure). The novelty
EvaporChain adds is a **protocol-physics retention clock + provable
trigger** that no immutable chain can offer.

---

## 0. TL;DR for cold readers

A data controller (e.g. a UK bank) must, under UK GDPR Art. 5(1)(e)
storage-limitation + Art. 17 right-to-erasure, delete personal data
when its retention basis ends — and *prove* it did. Today that is
brittle off-chain cron + audit-log-you-must-trust.

GDPR-Erasure-as-a-Service: per record (or per data-subject), deploy a
mortal `gdpr_vault.es` instance. Its `energy`/`half_life` encode the
**retention period** (the proven SFSV/mortal_message single-instance
pattern — no maps, no in-script decay math). The plaintext lives
encrypted off the chain; only a ciphertext commitment + consent state
live on-chain. When the retention period elapses the contract reaches
**terminal evaporation by physics** (verified-real) and emits the
key-shred trigger; the off-chain key-custody service destroys the key
(crypto-shred). The immutable, tamper-evident record that the
retention clock ran out and the trigger fired **is the audit
artifact** a DPO/regulator needs.

UVP filter (`APPLICATION_UNIVERSE.md` cat 1) — passes honestly: on
Ethereum the retention clock is an off-chain keeper and the data is
immutable forever (the opposite of erasure). Here the clock is
protocol physics and the trigger is protocol-guaranteed.

---

## 1. Mission & Doctrine Anchor

### 1.1 What it is

A **provable retention-and-erasure lifecycle** primitive: encrypt →
commit on-chain → physics-driven retention countdown → terminal
evaporation → key-shred trigger → tamper-evident proof. Sold as a
service to data controllers with hard storage-limitation obligations.

### 1.2 What it is **not**

- **Not** "the chain deletes your data." It does not (verified). The
  chain holds *no plaintext, ever* — only ciphertext commitments +
  lifecycle state. Erasure is key-destruction, off-chain.
- **Not** a KMS. It triggers and proves; the key-custody/HSM is the
  customer's or a partner's (integration boundary, §6.6).
- **Not** legal advice / a compliance certification. It is the
  tamper-evident technical control + evidence; sign-off is the
  controller's DPO + their legal review (Open Question).
- **Not** a token play. Satoshi-pattern; the service is the revenue,
  not a token.

### 1.3 Why slot 5 / why now

`evaporchain_application_universe.md`: cat 1 is the *largest
commercial wedge*; build-queue slot 5 is Satyawan's personal
commercial build (£50–200k contracts realistic per the doc). It is
the wedge precisely because storage-limitation is a *hard, audited,
universal* obligation and every controller's current control is
weak. Unblocked: the chain + the single-instance mortal pattern +
the verified node API all exist and were live-verified this session.

---

## 2. The Primitives at Play

### 2.1 Energy-Decay = the retention clock (Layer 0)

`gdpr_vault.es` is one contract instance per retained record. Deploy
`energy` + `half_life` are sized so the instance reaches terminal
evaporation at the retention deadline — the **exact SFSV /
mortal_message / evaporcash_note pattern**, proven this session. The
contract reads only its own `energy` builtin; **there is no in-script
decay primitive** (verified — see `EVAPORSCRIPT.md`; this is the
EvaporCash lesson, designed-in not discovered-late).

### 2.2 Terminal evaporation = the trigger (verified semantic)

When the instance hits `evaporated:true` (terminal, unrefreshable —
verified to occur), `on_evaporate` emits the key-shred trigger. The
controller's key-custody service subscribes and destroys the key.
The chain guarantees the *trigger fires by physics on schedule and is
tamper-evident*; it does not and need not touch the data.

### 2.3 EvaporScript VM (Layer 3)

`.es` is source of truth (invariant #2). Single-instance, the four
real builtins only (`caller|owner|epoch|energy`), no maps, tagged
`Value` args — every constraint verified this session and baked in
here so the contract is implementable as written (unlike the
pre-finding EvaporCash map model).

---

## 3. State Machine & Lifecycle

```
controller: encrypt(record) off-chain → key K in HSM; ct_commit = H(ciphertext)
                      │
        deploy gdpr_vault.es  (energy/half_life = retention period)
                      │
            seal(ct_commit, subject_ref, lawful_basis)   (owner-only, once)
                      ▼
   ┌──────────── Active (within retention) ───────────┐
   │  consent state mutable: withdraw_consent() can    │  read-only:
   │  force early expiry (Art. 7(3) / Art. 17(1)(b))   │  status(), basis(),
   │  on_refresh: lawful retention extension (logged)  │  expires_at()
   └───────────────┬───────────────────────────────────┘
                    │ energy decays out  OR  consent withdrawn
                    ▼
   ┌──────── terminal evaporated:true ────────────────┐
   │  on_evaporate → emit("erasure-due: shred K for    │
   │  ct_commit") — IMMUTABLE, tamper-evident trigger  │
   └───────────────┬───────────────────────────────────┘
                    │ off-chain key-custody subscriber
                    ▼
   destroy K (crypto-shred) → ciphertext permanently inaccessible.
   Audit artifact = the on-chain finalised set_terms + the terminal
   evaporation at/after the retention epoch (both immutable).
```

`withdraw_consent()` is the Art. 17/7(3) early-erasure path: it marks
the vault for immediate expiry so the trigger fires now, not at the
natural deadline.

---

## 4. Mathematical Foundation

### 4.1 Retention clock

```
alive(e) = energy_at_epoch(E, e − deploy_epoch) above the evaporation floor
```

`E` = deploy `energy`, `τ` = deploy `half_life`. Controller sizes
(E, τ) so terminal evaporation occurs at `deploy_epoch +
retention_epochs`. Same canonical decay as SFSV §4.1; the contract
contains **no** decay arithmetic (invariant #1).

### 4.2 The honest guarantee (what is and is NOT proven)

```
PROVEN on-chain (immutable, tamper-evident):
  ∀ e ≥ evaporate_epoch:  evaporated(e)=true  ∧  trigger emitted  ∧  ¬revive
NOT claimed:
  that ciphertext/commitment bytes leave chain state  (verified false)
ERASURE proven OFF-chain:
  key K destroyed ⇒ ciphertext computationally inaccessible (crypto-shred)
```

The compliance argument is the **composition**: chain proves the
retention period ran and the shred was triggered tamper-evidently;
the HSM attests K was destroyed. Neither half alone is "erasure"; the
composition is GDPR-defensible (subject to the controller's legal
review — Open Question).

---

## 5. Contract Surface (`.es` — `gdpr_vault.es`, single-instance)

```
fn seal(ct_commit: address, subject_ref: address, lawful_basis: u64)
                          # owner-only, once (caller == owner)
fn withdraw_consent()     # subject/controller: force early expiry (Art.17/7(3))
fn extend_retention()     # logged lawful extension (on_refresh telemetry)

# read-only (audit)
fn status()        -> u64       # 0 active, 1 expiry-forced, (evaporated = engine)
fn lawful_basis()  -> u64
fn subject()       -> address
fn ct_commitment() -> address
fn expires_at()    -> u64

on_grace()      -> emit("retention ending — prepare key-shred")
on_refresh()    -> emit("retention lawfully extended")
on_evaporate()  -> emit("erasure-due: shred key for this ct_commit")  # the trigger
```

Bounded state (one commitment, one subject ref, two scalars, one
flag) → inside EvaporScript's structural totality. No maps. The
ciphertext commitment is a 32-byte hash carried as `address` (the
proven type-faithful trick — EvaporScript has `address`/`u64`/`bool`,
no `bytes`).

---

## 6. Connect with mainnet

Every interface below was **live-verified this session**
(`VERIFICATION_2026_05_16.md`, the SFSV/Dead Drop/EvaporCash e2es).
No chain change required.

- **Deploy** `POST /api/tx/deploy-script` `{deployer:u8,
  source_code, energy, half_life}`; `deployer 0` = genesis-funded
  faucet. Poll `GET /api/tx/:hash` → `.contract_id`.
- **Call** `POST /api/tx/call-script` `{caller:u8, contract_id:u64,
  method, args:Vec<Value>, epoch:u64}`. `args` externally-tagged
  (`{"Address":[…32]}`, `{"U64":n}`); `epoch` required; session-token
  auth (register→login→bearer).
- **Audit read** `GET /api/script/:id` (the script store — **not**
  `/api/contract/:id`). Post-evaporation it still returns the record
  with `evaporated:true` — which is *exactly* the desired audit
  artifact here (the immutable proof the retention clock terminated),
  the same behaviour that was wrong for Dead Drop's claim but is
  *correct* for this one.
- **Runbook**: `scripts/deploy-gdpr-vault.sh` (to build) — fork of
  the verified `deploy-evaporcash.sh`/`deploy-dead-drop.sh` shape:
  deploy → seal → confirm-active (non-vacuity) → terminal evaporation
  → assert the `on_evaporate` trigger event recorded.

### 6.6 Integration boundary (not chain work)

Key-custody/HSM + the off-chain encryptor + the shred-subscriber are
**the customer's or a partner's**, behind a defined interface (the
emitted trigger event + `ct_commit`). EvaporChain ships the
lifecycle/trigger/proof; it never holds K or plaintext.

---

## 7. Threat Model

| # | Adversary | Defence |
|---|---|---|
| 7.1 | Controller claims erasure but didn't shred | The chain proves the *trigger* fired immutably; non-repudiation of the obligation. Actual shred is attested by the HSM (out of chain scope — stated, not overclaimed). |
| 7.2 | Tamper with the retention clock to keep data | Energy decay is protocol physics; `extend_retention` is logged + emits. A silent extension is impossible — every extension is on-chain evidence. |
| 7.3 | Re-org around the evaporation | Finality (T0.1/T0.4) bounds re-org depth; the trigger is re-emitted deterministically post-finality. |
| 7.4 | Plaintext leak via chain | Impossible by construction — no plaintext or key ever on-chain; only `H(ciphertext)`. |
| 7.5 | "You didn't really delete it (bytes persist)" | **Honest answer, designed-in:** correct — the chain never claimed to. Erasure = key-shred; the persisting on-chain bytes are a *hash commitment*, not personal data. This is the founding constraint, not a vulnerability. |
| 7.6 | Quantum | Commitment is BLAKE3; ciphertext encryption is the controller's (recommend PQ-ready); chain signatures are ML-DSA. |

---

## 8. Doctrine Mapping

- **Thesis (`INEVITABILITY_STRATEGY`):** "data without a half-life is
  a bug" — applied to the most regulated, most monetisable surface:
  legally-mandated retention limits. The retention clock as protocol
  physics is the inevitability argument made commercial.
- **APPLICATION_UNIVERSE cat 1** (regulatory-compliant data infra) —
  the explicit "largest commercial wedge."
- **Honest-scope lineage:** this spec is *designed around* the Dead
  Drop §9 finding from line 1 — the overclaim that took three
  correction passes for the demos is structurally impossible here
  because erasure is defined as key-shred, never byte-erasure.

---

## 9. Reference Implementation Status

| Surface | State |
|---|---|
| this spec | ✅ written (model A; honest-scope founded on the verified #9 behaviour) |
| `contracts/evaporscript/gdpr_vault.es` | to build (single-instance mortal pattern; ~SFSV-class) |
| `scripts/deploy-gdpr-vault.sh` | to build (fork verified deploy-evaporcash.sh) |
| off-chain encryptor + HSM key-custody + shred-subscriber | **partner/customer integration** — NOT chain work (§6.6) |
| live e2e on a node | pending (deploy → seal → terminal-evap → trigger-event assert) |
| Node API | already verified live — no chain change |

**Honest status:** the *chain-side* is SFSV-class and unblocked. The
*product* is larger than a demo because its value is the
crypto-shred + HSM + legal-defensibility composition (§4.2, §6.6) —
materially the customer/partner integration + a DPO/legal review
(Open Question), not more chain code. This spec deliberately scopes
slot 5 so that decision can be made on honest, verified ground —
**not a commitment to build the multi-week product**, which remains
the operator's call.
