# GDPR-Erasure Pilot Kit

A regulated firm runs **one command** and watches a complete, compliant
data-erasure lifecycle end-to-end on a live EvaporChain node.

This is the off-chain half of **model A** (`research/GDPR_ERASURE_ARCHITECTURE.md`)
wired to the verified chain-side (`contracts/evaporscript/gdpr_vault.es`)
through the verified `@evaporchain/sdk`.

## What it proves

1. Personal data is encrypted; **only a 32-byte ciphertext commitment
   ever touches the chain** — never the data.
2. A `gdpr_vault.es` instance is deployed whose **own energy is the
   retention clock** (deploy `energy`/`half_life` = the retention
   period) and sealed with the commitment + subject + lawful basis.
3. During retention the record is decryptable (shown).
4. The key-shred trigger fires either by **physics** (retention elapses
   → terminal evaporation) or by **Art. 17 / Art. 7(3)**
   (`withdraw_consent`).
5. On the trigger the decryption key is **destroyed (crypto-shred)** —
   the record is then **permanently unrecoverable** (shown).

The immutable on-chain `seal` tx + the terminal evaporation (or the
`withdraw_consent` tx) are the tamper-evident audit artifact a
DPO/regulator needs.

## Honest scope (not overclaimed)

- The chain proves the **tamper-evident retention clock + key-shred
  trigger**. It does **not** byte-erase (verified, Dead Drop §9):
  `get_script` keeps returning the last state. Erasure is the
  **key destruction**, which is an ICO/ENISA-recognised technique
  (crypto-shredding).
- The key here is held **in-memory as an HSM stand-in**. Production
  wires the same `on_evaporate` / `expiry_forced` trigger to a real
  HSM/KMS — the integration boundary, not chain work.

## Run it

```bash
# 1. build the SDK once
cd ../..            # sdk/
npm ci && npm run build

# 2. run the pilot against a node
cd examples/gdpr-erasure-pilot
node pilot.mjs --node http://127.0.0.1:9001 \
  --record "Alice — DOB 1990-01-01 — acct 1234" \
  --retention-energy 60000 --half-life 5 --mode retain
```

Modes:
- `--mode retain` (default): never withdraw; the retention clock runs
  out by physics → natural-deadline key-shred.
- `--mode withdraw`: controller invokes `withdraw_consent()`
  (Art. 17 / 7(3)) → early key-shred.

Key flags: `--node`, `--email/--password` (auth), `--deployer` (u8;
0 = genesis faucet), `--subject` (u8), `--basis` (lawful-basis code),
`--retention-energy`, `--half-life`, `--timeout`.

Exit 0 = full compliant-erasure lifecycle proven end-to-end.
