import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  setPayloadPayload,
  readPayload,
  recordBoostPayload,
  inspectPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { MORTAL_MESSAGE_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 5, energy: 10_000, halfLife: 50 });
  assert.equal(p.deployer, 5);
  assert.equal(p.source_code, MORTAL_MESSAGE_SOURCE);
  assert.equal(p.energy, 10_000);
  assert.equal(p.half_life, 50);
});

test("set_payload carries (body, recipient) in canonical order", () => {
  const p = setPayloadPayload({
    caller: 5,
    contractId: 42,
    body: "if you're reading this, the deal went through",
    recipientHex: "0xfeed",
    epoch: 100,
  });
  assert.equal(p.method, "set_payload");
  assert.deepEqual(p.args, ["if you're reading this, the deal went through", "0xfeed"]);
  assert.equal(p.caller, 5);
  assert.equal(p.contract_id, 42);
  assert.equal(p.epoch, 100);
});

test("read + record_boost + inspect have correct method names + zero args", () => {
  for (const [fn, name] of [
    [readPayload, "read"],
    [recordBoostPayload, "record_boost"],
    [inspectPayload, "inspect"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 2, epoch: 3 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("epoch + caller + contract_id thread through every call payload", () => {
  const setp = setPayloadPayload({
    caller: 99,
    contractId: 88,
    body: "x",
    recipientHex: "0x1",
    epoch: 77,
  });
  assert.equal(setp.caller, 99);
  assert.equal(setp.contract_id, 88);
  assert.equal(setp.epoch, 77);

  const read = readPayload({ caller: 99, contractId: 88, epoch: 77 });
  assert.equal(read.caller, 99);
  assert.equal(read.contract_id, 88);
  assert.equal(read.epoch, 77);
});

test("set_payload handles empty body + multiline body shapes", () => {
  // The contract doesn't restrict body shape; clients sometimes
  // pass empty (e.g., delete-by-overwrite) or multiline. Pin both
  // to catch any future arg-encoding regression.
  const empty = setPayloadPayload({
    caller: 1, contractId: 2, body: "", recipientHex: "0xab", epoch: 0,
  });
  assert.deepEqual(empty.args, ["", "0xab"]);

  const multiline = setPayloadPayload({
    caller: 1, contractId: 2, body: "line one\nline two\nline three", recipientHex: "0xab", epoch: 0,
  });
  assert.deepEqual(multiline.args, ["line one\nline two\nline three", "0xab"]);
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("MORTAL_MESSAGE_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "body:",
    "recipient:",
    "sender:",
    "sealed:",
    "boost_count:",
    "last_boost_epoch:",
    // mutators
    "fn set_payload(",
    "fn read()",
    "fn record_boost()",
    // views
    "fn inspect()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only sender can seal",
    "message already sealed",
    "not authorized",
    // chain-as-keeper claim — the runtime, not the contract, drives decay
    'emit("message evaporated")',
  ]) {
    assert.ok(MORTAL_MESSAGE_SOURCE.includes(name), `MORTAL_MESSAGE_SOURCE missing: ${name}`);
  }
});

test("MORTAL_MESSAGE_SOURCE: read() gates on sender OR recipient (not 'and')", () => {
  // The privacy surface depends on this being OR. An accidental
  // && would lock everyone out (no caller is both sender AND
  // recipient simultaneously). Pin the literal.
  const readBlock = MORTAL_MESSAGE_SOURCE.slice(
    MORTAL_MESSAGE_SOURCE.indexOf("fn read()"),
    MORTAL_MESSAGE_SOURCE.indexOf("fn record_boost()"),
  );
  assert.ok(
    readBlock.includes("caller == self.recipient || caller == owner"),
    "read() must allow recipient OR owner (sender)",
  );
});

test("MORTAL_MESSAGE_SOURCE: on_refresh bumps boost_count + records epoch", () => {
  // The runtime hook fires automatically on every refresh action,
  // BEFORE any user code runs. The body must bump boost_count and
  // record last_boost_epoch — clients rely on this counter for
  // 'message has been refreshed N times' UX.
  const refresh = MORTAL_MESSAGE_SOURCE.slice(
    MORTAL_MESSAGE_SOURCE.indexOf("on_refresh()"),
    MORTAL_MESSAGE_SOURCE.indexOf("on_evaporate()"),
  );
  assert.ok(refresh.includes("self.boost_count += 1"), "on_refresh must bump boost_count");
  assert.ok(refresh.includes("self.last_boost_epoch = epoch"), "on_refresh must record epoch");
});
