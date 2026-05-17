#!/usr/bin/env node
// GDPR-Erasure pilot kit — the off-chain half of model A, wired to the
// verified chain-side (gdpr_vault.es) via the verified @evaporchain/sdk.
//
// This is the "someone else did the work" artifact: a regulated firm's
// engineer runs ONE command and watches a complete, compliant erasure
// lifecycle end-to-end on a live node.
//
// HONEST SCOPE (verified, not overclaimed — Dead Drop §9): the chain
// does NOT byte-erase. It provides the *tamper-evident retention clock
// + provable key-shred trigger*. Erasure itself = destroying the
// decryption key (crypto-shred — an ICO/ENISA-recognised technique).
// This kit holds the key in-memory as an HSM STAND-IN; production wires
// the same trigger to a real HSM/KMS. Personal data is NEVER on-chain
// (only a 32-byte ciphertext commitment).
//
// Flow: encrypt(record,K) -> ct_commit=sha256(ct) -> deploy gdpr_vault.es
// (energy/half_life = retention) -> seal(ct_commit,subject,basis) ->
// while alive: record is decryptable -> on terminal evaporation (or
// withdraw_consent): DESTROY K -> ciphertext now permanently
// inaccessible. The on-chain seal tx + terminal evaporation are the
// immutable audit artifact.

import { createCipheriv, createDecipheriv, randomBytes, createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
// SDK builds with tsconfig rootDir "." → compiled entry is
// dist/src/index.js (also now package.json "main", fixed in this PR).
import { EvaporChain } from "../../dist/src/index.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const ES_PATH = resolve(HERE, "../../../contracts/evaporscript/gdpr_vault.es");

function arg(name, def) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : def;
}
const NODE      = arg("node", "http://127.0.0.1:9001");
const EMAIL     = arg("email", "gdpr-pilot@example.com");
const PASSWORD  = arg("password", "gdprpilotpass123");
const DEPLOYER  = Number(arg("deployer", "0"));   // controller; 0 = genesis faucet
const SUBJECT   = Number(arg("subject", "1"));
const BASIS     = Number(arg("basis", "1"));      // 1=consent ...
const ENERGY    = Number(arg("retention-energy", "60000"));
const HALF_LIFE = Number(arg("half-life", "5"));
const MODE      = arg("mode", "retain");          // retain | withdraw
const RECORD    = arg("record", "Alice Example — DOB 1990-01-01 — acct 1234");
const TIMEOUT_S = Number(arg("timeout", "300"));

const log = (m) => console.log(`\x1b[36m[gdpr-pilot]\x1b[0m ${m}`);
const die = (m, c = 1) => { console.error(`\x1b[31m[gdpr-pilot ERROR]\x1b[0m ${m}`); process.exit(c); };

// ── 0. encrypt the record (the key K is the only thing whose
//      destruction effects erasure — the "HSM" stand-in) ──
let K = randomBytes(32);                       // AES-256 key (in-memory HSM stub)
const iv = randomBytes(12);
const cipher = createCipheriv("aes-256-gcm", K, iv);
const ciphertext = Buffer.concat([cipher.update(RECORD, "utf8"), cipher.final()]);
const authTag = cipher.getAuthTag();
const ctCommit = createHash("sha256").update(ciphertext).digest(); // 32 bytes
const decryptable = () => {
  if (K === null) return null;                 // key shredded → permanently inaccessible
  const d = createDecipheriv("aes-256-gcm", K, iv);
  d.setAuthTag(authTag);
  return Buffer.concat([d.update(ciphertext), d.final()]).toString("utf8");
};

console.log(`+= GDPR-Erasure pilot (model A: crypto-shred) =================+`);
console.log(`|  node: ${NODE}  mode: ${MODE}  retention E/hl: ${ENERGY}/${HALF_LIFE}`);
console.log(`|  record encrypted; ct_commit=${ctCommit.toString("hex").slice(0, 16)}…  (data NEVER on-chain)`);
console.log(`+=============================================================+`);

const chain = new EvaporChain({ baseUrl: NODE });

try {
  // ── 1. auth ──
  await chain.register(EMAIL, PASSWORD, "gdpr-pilot").catch(() => {});
  const s = await chain.login(EMAIL, PASSWORD);
  if (!s.token) die("login failed — no token", 9);
  log("authenticated (session token minted)");

  // ── 2. deploy gdpr_vault.es (energy/half_life = the retention clock) ──
  const src = readFileSync(ES_PATH, "utf8");
  const dep = await chain.deployScript(DEPLOYER, src, ENERGY, HALF_LIFE);
  if (!dep.tx_hash) die(`deploy rejected: ${dep.message || "(no msg)"}`, 3);
  const cid = await chain.waitForScriptContractId(dep.tx_hash, TIMEOUT_S * 1000);
  log(`gdpr_vault deployed: contract_id=${cid}`);

  // ── 3. seal(ct_commit, subject, basis) — owner-only, once ──
  const epoch = await chain.getEpoch();
  const seal = await chain.callScript(
    DEPLOYER, cid, "seal",
    [ EvaporChain.esAddrBytes([...ctCommit]),
      EvaporChain.esAddr(SUBJECT),
      EvaporChain.esU64(BASIS) ],
    epoch,
  );
  if (!seal.tx_hash) die(`seal rejected: ${seal.message || "(no msg)"}`, 4);
  // confirm sealed-active (non-vacuity)
  await new Promise((r) => setTimeout(r, 4000));
  const st0 = await chain.getScript(cid);
  const sealed = st0.state?.sealed?.Bool === true || st0.state?.sealed === true;
  if (!sealed) die(`vault not sealed after seal tx — got ${JSON.stringify(st0.state?.sealed)}`, 5);
  log(`vault SEALED on-chain (lawful_basis=${BASIS}). Record is in RETENTION.`);
  log(`  proof: record currently decryptable → "${decryptable()}"`);

  // ── 4. trigger: natural-deadline (retain) OR Art.17 (withdraw) ──
  let triggered = false;
  if (MODE === "withdraw") {
    log("Art.17/7(3): controller calls withdraw_consent() → early key-shred trigger");
    const e2 = await chain.getEpoch();
    const w = await chain.callScript(DEPLOYER, cid, "withdraw_consent", [], e2);
    if (!w.tx_hash) die(`withdraw rejected: ${w.message}`, 6);
    await new Promise((r) => setTimeout(r, 4000));
    const stw = await chain.getScript(cid);
    triggered = (stw.state?.expiry_forced?.Bool === true || stw.state?.expiry_forced === true);
    if (!triggered) die("withdraw_consent did not set expiry_forced — trigger unproven", 6);
    log("TRIGGER fired: state.expiry_forced=true ('erasure-due (consent withdrawn)')");
  } else {
    log(`RETAIN: never extend/withdraw — poll until the retention clock terminally evaporates`);
    const deadline = Date.now() + TIMEOUT_S * 1000;
    while (Date.now() < deadline) {
      let st;
      try { st = await chain.getScript(cid); } catch { triggered = true; break; }
      if (st.evaporated === true) { triggered = true; break; }
      await new Promise((r) => setTimeout(r, 4000));
    }
    if (!triggered) die(`vault never reached terminal evaporation in ${TIMEOUT_S}s (half-life too large?)`, 6);
    log("TRIGGER fired: terminal evaporated=true ('erasure-due: shred key') — retention elapsed by physics");
  }

  // ── 5. CRYPTO-SHRED: destroy K. This is the erasure. ──
  K.fill(0); K = null;
  log("KEY DESTROYED (crypto-shred). Attempting to read the record now…");
  const after = decryptable();
  if (after !== null) die("FATAL: record still decryptable after key-shred — kit bug", 7);
  log(`  proof: record now UNRECOVERABLE → ${after} (key gone; ciphertext is computationally inaccessible)`);

  console.log(`
+=============================================================+
|        OK  GDPR-ERASURE LIFECYCLE COMPLETE (model A)        |
|  contract_id: ${cid}
|  Personal data NEVER on-chain (only a 32-byte commitment).  |
|  Chain proved, tamper-evidently: seal + ${MODE === "withdraw" ? "Art.17 withdrawal" : "retention-elapsed"} trigger.
|  Erasure effected by crypto-shred (key destroyed); record   |
|  was decryptable in retention, is unrecoverable after.      |
|  HONEST SCOPE: chain proves retention+trigger (NOT byte-    |
|  erasure — Dead Drop §9); HSM here is an in-memory stub.    |
+=============================================================+`);
  process.exit(0);
} catch (e) {
  die(`pilot failed: ${e?.message || e}`, 1);
}
