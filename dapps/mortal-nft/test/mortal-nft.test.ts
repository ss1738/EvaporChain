import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  setMetadataPayload,
  transferPayload,
  currentOwnerPayload,
  metadataUriPayload,
  transfersPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { MORTAL_NFT_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 7, energy: 100_000, halfLife: 365 });
  assert.equal(p.deployer, 7);
  assert.equal(p.source_code, MORTAL_NFT_SOURCE);
  assert.equal(p.energy, 100_000);
  assert.equal(p.half_life, 365);
});

test("set_metadata carries (name, collection, metadata, recipient) in canonical order", () => {
  const p = setMetadataPayload({
    caller: 7,
    contractId: 42,
    name: "Sunset No. 17",
    collection: "Mortal Skies",
    metadata: "ipfs://bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
    recipientHex: "0xab",
    epoch: 100,
  });
  assert.equal(p.method, "set_metadata");
  assert.deepEqual(p.args, [
    "Sunset No. 17",
    "Mortal Skies",
    "ipfs://bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
    "0xab",
  ]);
  assert.equal(p.caller, 7);
  assert.equal(p.contract_id, 42);
  assert.equal(p.epoch, 100);
});

test("transfer carries the `to` address", () => {
  const p = transferPayload({
    caller: 7,
    contractId: 42,
    toHex: "0xcd",
    epoch: 120,
  });
  assert.equal(p.method, "transfer");
  assert.deepEqual(p.args, ["0xcd"]);
});

test("no-arg view payloads have correct method names + zero args", () => {
  for (const [fn, name] of [
    [currentOwnerPayload, "current_owner"],
    [metadataUriPayload, "metadata_uri"],
    [transfersPayload, "transfers"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 2, epoch: 3 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("epoch + caller + contract_id thread through every call payload", () => {
  const setMeta = setMetadataPayload({
    caller: 99,
    contractId: 88,
    name: "x",
    collection: "y",
    metadata: "z",
    recipientHex: "0x1",
    epoch: 77,
  });
  assert.equal(setMeta.caller, 99);
  assert.equal(setMeta.contract_id, 88);
  assert.equal(setMeta.epoch, 77);

  const xfer = transferPayload({ caller: 99, contractId: 88, toHex: "0x2", epoch: 77 });
  assert.equal(xfer.caller, 99);
  assert.equal(xfer.contract_id, 88);
  assert.equal(xfer.epoch, 77);
});

test("set_metadata handles empty + long metadata URIs (no client-side validation)", () => {
  // The contract is opaque to the metadata format; the dApp may pass
  // empty (placeholder), short (IPFS CIDv0), or long (HTTP URL with
  // query string) values. Pin shape preservation through the payload.
  const empty = setMetadataPayload({
    caller: 1, contractId: 2, name: "x", collection: "y", metadata: "",
    recipientHex: "0xab", epoch: 0,
  });
  assert.equal(empty.args[2], "");

  const long = setMetadataPayload({
    caller: 1, contractId: 2, name: "x", collection: "y",
    metadata: "https://gateway.example.com/ipfs/Qm…?v=2&ext=png&w=1024",
    recipientHex: "0xab", epoch: 0,
  });
  assert.equal(long.args[2], "https://gateway.example.com/ipfs/Qm…?v=2&ext=png&w=1024");
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("MORTAL_NFT_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "name:",
    "collection:",
    "metadata:",
    "sealed:",
    "holder:",
    "transfer_count:",
    "last_transfer_epoch:",
    // mutators
    "fn set_metadata(",
    "fn transfer(",
    // views
    "fn current_owner()",
    "fn metadata_uri()",
    "fn transfers()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only minter can seal",
    "nft already minted",
    "only current owner can transfer",
    'emit("nft evaporated")',
  ]) {
    assert.ok(MORTAL_NFT_SOURCE.includes(name), `MORTAL_NFT_SOURCE missing: ${name}`);
  }
});

test("MORTAL_NFT_SOURCE: NFT-1 — state field is `holder` not `owner` (no builtin shadowing)", () => {
  // The pilot was originally written with state.owner, which shadowed
  // the EvaporScript builtin `caller == owner` and produced silently
  // wrong auth checks. Audit 2026-05-17 (NFT-1) renamed it to `holder`
  // and added a compile-time rejection for builtin-reserved names.
  // Pin the rename here so a future refactor can't reintroduce the
  // shadow.
  assert.ok(MORTAL_NFT_SOURCE.includes("holder: address"), "state field must be `holder`");
  // Specifically no `owner:` declaration in the state block (the
  // builtin is sufficient; redeclaring would shadow).
  const stateBlock = MORTAL_NFT_SOURCE.slice(
    MORTAL_NFT_SOURCE.indexOf("state {"),
    MORTAL_NFT_SOURCE.indexOf("fn set_metadata("),
  );
  assert.ok(
    !/^\s*owner:/m.test(stateBlock),
    "state block must not redeclare `owner` (shadows builtin)",
  );
});

test("MORTAL_NFT_SOURCE: transfer() gates on `self.holder` not `owner`", () => {
  // The transfer auth check is doctrinally important: only the
  // CURRENT holder can transfer, not the original minter. If this
  // gate uses `owner` instead, the minter would retain perpetual
  // claw-back ability — a critical departure from NFT norms. Pin
  // the literal.
  const xfer = MORTAL_NFT_SOURCE.slice(
    MORTAL_NFT_SOURCE.indexOf("fn transfer("),
    MORTAL_NFT_SOURCE.indexOf("fn current_owner()"),
  );
  assert.ok(
    xfer.includes("caller == self.holder"),
    "transfer() must gate on caller == self.holder, not owner",
  );
});
