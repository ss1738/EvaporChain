import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  setEventPayload,
  enterPayload,
  drawPayload,
  claimPrizePayload,
  entriesTotalPayload,
  isEnteredPayload,
  winnerOfPayload,
  isDrawnPayload,
  isVoidedPayload,
  prizeSizePayload,
  stakePerEntryPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { LOTTERY_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 5, energy: 100_000, halfLife: 365 });
  assert.equal(p.deployer, 5);
  assert.equal(p.source_code, LOTTERY_SOURCE);
  assert.equal(p.energy, 100_000);
  assert.equal(p.half_life, 365);
});

test("set_event carries (prize, stake) in canonical order", () => {
  const p = setEventPayload({
    caller: 5, contractId: 42, prizeAmount: 1_000_000, stakeAmount: 5_000, epoch: 100,
  });
  assert.equal(p.method, "set_event");
  assert.deepEqual(p.args, [1_000_000, 5_000]);
});

test("enter is a no-arg open call", () => {
  const p = enterPayload({ caller: 9, contractId: 42, epoch: 110 });
  assert.equal(p.method, "enter");
  assert.deepEqual(p.args, []);
});

test("draw is a no-arg operator call", () => {
  const p = drawPayload({ caller: 5, contractId: 42, epoch: 200 });
  assert.equal(p.method, "draw");
  assert.deepEqual(p.args, []);
});

test("claim_prize is a no-arg winner-only call", () => {
  const p = claimPrizePayload({ caller: 9, contractId: 42, epoch: 210 });
  assert.equal(p.method, "claim_prize");
  assert.deepEqual(p.args, []);
});

test("address-arg views carry the queried address", () => {
  const p = isEnteredPayload({ caller: 1, contractId: 42, whoHex: "0xcd", epoch: 0 });
  assert.equal(p.method, "is_entered");
  assert.deepEqual(p.args, ["0xcd"]);
});

test("no-arg views have correct method names + zero args", () => {
  for (const [fn, name] of [
    [entriesTotalPayload, "entries_total"],
    [winnerOfPayload, "winner_of"],
    [isDrawnPayload, "is_drawn"],
    [isVoidedPayload, "is_voided"],
    [prizeSizePayload, "prize_size"],
    [stakePerEntryPayload, "stake_per_entry"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 2, epoch: 3 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("LOTTERY_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "operator:",
    "prize:",
    "stake:",
    "sealed:",
    "entered:",
    "entry_by_index:",
    "entry_count:",
    "drawn:",
    "winner:",
    "claimed:",
    "voided:",
    // mutators
    "fn set_event(",
    "fn enter()",
    "fn draw()",
    "fn claim_prize()",
    // views
    "fn entries_total()",
    "fn is_entered(",
    "fn winner_of()",
    "fn is_drawn()",
    "fn is_voided()",
    "fn prize_size()",
    "fn stake_per_entry()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only operator can configure",
    "lottery already configured",
    "lottery not configured",
    "draw already happened",
    "already entered",
    "only operator can trigger draw",
    "lottery already drawn",
    "no entries to draw from",
    "no draw yet",
    "only winner can claim",
    "prize already claimed",
    "lottery evaporated — entries refunded",
  ]) {
    assert.ok(
      LOTTERY_SOURCE.includes(name),
      `LOTTERY_SOURCE missing: ${name}`,
    );
  }
});

test("LOTTERY_SOURCE: LOTTERY-1 chain-VRF draw — random_range pinned in draw()", () => {
  // The whole influence-asymmetry claim ("operator picks WHEN, not
  // WHO") rests on `random_range(self.entry_count)`. If the draw is
  // ever changed to take an operator-supplied index, the security
  // posture collapses. Pin the call shape.
  const draw = LOTTERY_SOURCE.slice(
    LOTTERY_SOURCE.indexOf("fn draw()"),
    LOTTERY_SOURCE.indexOf("fn claim_prize()"),
  );
  assert.ok(
    draw.includes("let winner_index = random_range(self.entry_count)"),
    "draw() must derive winner_index from random_range(self.entry_count) — never an operator arg",
  );
  assert.ok(
    draw.includes("self.winner = self.entry_by_index[winner_index]"),
    "draw() must assign winner via entry_by_index[winner_index] — never an operator-named address",
  );
});

test("LOTTERY_SOURCE: enter() stamps entry_by_index BEFORE incrementing entry_count", () => {
  // LOTTERY-1's index-keyed parallel map only works if the index
  // recorded BEFORE the counter is incremented. If the increment
  // happens first, `entry_by_index[entry_count]` skips a slot and
  // `entry_by_index[0]` stays empty — draw() pulls the zero address.
  // Pin the ordering.
  const enter = LOTTERY_SOURCE.slice(
    LOTTERY_SOURCE.indexOf("fn enter()"),
    LOTTERY_SOURCE.indexOf("fn draw()"),
  );
  const idxIndex = enter.indexOf("self.entry_by_index[self.entry_count] = caller");
  const incIndex = enter.indexOf("self.entry_count += 1");
  assert.ok(idxIndex >= 0, "enter() must stamp entry_by_index[entry_count] = caller");
  assert.ok(incIndex >= 0, "enter() must increment entry_count");
  assert.ok(idxIndex < incIndex, "enter() must stamp the index BEFORE incrementing the counter");
});

test("LOTTERY_SOURCE: set_event is one-shot (sealed-flag gate)", () => {
  // set_event seals `prize` + `stake` for the entire contract
  // lifetime. Re-calling would reset both — a clear rug-pull vector
  // if reopened. Pin the sealed-flag gate.
  const setEvent = LOTTERY_SOURCE.slice(
    LOTTERY_SOURCE.indexOf("fn set_event("),
    LOTTERY_SOURCE.indexOf("fn enter()"),
  );
  assert.ok(
    setEvent.includes("require(self.sealed == false, \"lottery already configured\")"),
    "set_event must reject re-configuration once sealed",
  );
  assert.ok(
    setEvent.includes("self.sealed = true"),
    "set_event must set sealed = true after successful configuration",
  );
});

test("LOTTERY_SOURCE: on_evaporate sets voided=true only when not drawn", () => {
  // A drawn lottery has already paid out (or is awaiting claim);
  // setting voided=true on a drawn lottery would confuse the
  // coordinator into refunding entries that already won. Pin the
  // `drawn == false` gate.
  const evap = LOTTERY_SOURCE.slice(
    LOTTERY_SOURCE.indexOf("on_evaporate()"),
    LOTTERY_SOURCE.length,
  );
  assert.ok(
    evap.includes("if self.drawn == false"),
    "on_evaporate must gate void on drawn == false",
  );
  assert.ok(
    evap.includes("self.voided = true"),
    "on_evaporate must set voided = true in the unresolved branch",
  );
});
