import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  setMetadataPayload,
  setPhasePayload,
  commitPayload,
  revealPayload,
  recordWinnerPayload,
  currentPhasePayload,
  nominalBidOfPayload,
  effectiveBidOfPayload,
  committedHashOfPayload,
  winnerOfPayload,
  isSettledPayload,
  commitsReceivedPayload,
  revealsReceivedPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { SEALED_BID_AUCTION_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 5, energy: 100_000, halfLife: 365 });
  assert.equal(p.deployer, 5);
  assert.equal(p.source_code, SEALED_BID_AUCTION_SOURCE);
});

test("set_metadata carries (item, reserve) in canonical order", () => {
  const p = setMetadataPayload({
    caller: 5, contractId: 42, itemLabel: "Genesis NFT #1", reserve: 5000, epoch: 100,
  });
  assert.equal(p.method, "set_metadata");
  assert.deepEqual(p.args, ["Genesis NFT #1", 5000]);
});

test("set_phase carries the next phase value", () => {
  const p = setPhasePayload({ caller: 5, contractId: 42, next: 1, epoch: 150 });
  assert.equal(p.method, "set_phase");
  assert.deepEqual(p.args, [1]);
});

test("commit carries the commit hash", () => {
  const p = commitPayload({
    caller: 9, contractId: 42, commitHash: "0xabcd1234deadbeef", epoch: 110,
  });
  assert.equal(p.method, "commit");
  assert.deepEqual(p.args, ["0xabcd1234deadbeef"]);
});

test("reveal carries (nominal, effective, commitment_hash) in canonical order", () => {
  const p = revealPayload({
    caller: 9, contractId: 42,
    nominalBid: 10_000,
    effectiveBid: 9_500,
    commitmentHash: "0xabcd1234deadbeef",
    epoch: 160,
  });
  assert.equal(p.method, "reveal");
  assert.deepEqual(p.args, [10_000, 9_500, "0xabcd1234deadbeef"]);
});

test("record_winner carries (winner, winning_effective_bid)", () => {
  const p = recordWinnerPayload({
    caller: 5, contractId: 42,
    winnerHex: "0xfeed", winningEffectiveBid: 9_500,
    epoch: 200,
  });
  assert.equal(p.method, "record_winner");
  assert.deepEqual(p.args, ["0xfeed", 9_500]);
});

test("address-arg views carry the queried address", () => {
  for (const [fn, name] of [
    [nominalBidOfPayload, "nominal_bid_of"],
    [effectiveBidOfPayload, "effective_bid_of"],
    [committedHashOfPayload, "committed_hash_of"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 42, whoHex: "0xcd", epoch: 0 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, ["0xcd"]);
  }
});

test("no-arg views have correct method names + zero args", () => {
  for (const [fn, name] of [
    [currentPhasePayload, "current_phase"],
    [winnerOfPayload, "winner_of"],
    [isSettledPayload, "is_settled"],
    [commitsReceivedPayload, "commits_received"],
    [revealsReceivedPayload, "reveals_received"],
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

test("SEALED_BID_AUCTION_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "seller:",
    "item:",
    "reserve_price:",
    "sealed:",
    "phase:",
    "committed_hashes:",
    "committed:",
    "revealed:",
    "nominal:",
    "effective:",
    "winner:",
    "winning_effective:",
    "settled:",
    // mutators
    "fn set_metadata(",
    "fn set_phase(",
    "fn commit(",
    "fn reveal(",
    "fn record_winner(",
    // views
    "fn current_phase()",
    "fn nominal_bid_of(",
    "fn effective_bid_of(",
    "fn committed_hash_of(",
    "fn winner_of()",
    "fn is_settled()",
    "fn commits_received()",
    "fn reveals_received()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only seller can set metadata",
    "only seller can advance phase",
    "phase only advances forward",
    "max phase is 3",
    "not in COMMIT phase",
    "not in REVEAL phase",
    "not in SETTLE phase",
    "commitment hash mismatch",
    "nominal below reserve",
    "effective cannot exceed nominal",
    "void (bidders refund)",
  ]) {
    assert.ok(
      SEALED_BID_AUCTION_SOURCE.includes(name),
      `SEALED_BID_AUCTION_SOURCE missing: ${name}`,
    );
  }
});

test("SEALED_BID_AUCTION_SOURCE: SBA-1 commit-reveal binding enforced", () => {
  // The commit_hash stored at commit time must match the
  // commitment_hash supplied at reveal time. Without this check,
  // a bidder could commit one hash and reveal an arbitrary
  // nominal/effective pair, defeating the sealed-bid property.
  const reveal = SEALED_BID_AUCTION_SOURCE.slice(
    SEALED_BID_AUCTION_SOURCE.indexOf("fn reveal("),
    SEALED_BID_AUCTION_SOURCE.indexOf("fn record_winner("),
  );
  assert.ok(
    reveal.includes(
      "require(self.committed_hashes[caller] == commitment_hash, \"commitment hash mismatch\")",
    ),
    "reveal() must re-verify the stored commit_hash matches the supplied commitment_hash",
  );
});

test("SEALED_BID_AUCTION_SOURCE: set_phase strict monotone forward only", () => {
  // The phase machine must never rewind — a seller couldn't reopen
  // COMMIT after REVEAL, for example, since that would let new
  // bidders submit commits they could decode against revealed
  // bids. Pin the strict-greater-than guard.
  const setPhase = SEALED_BID_AUCTION_SOURCE.slice(
    SEALED_BID_AUCTION_SOURCE.indexOf("fn set_phase("),
    SEALED_BID_AUCTION_SOURCE.indexOf("fn commit("),
  );
  assert.ok(
    setPhase.includes("require(next > self.phase, \"phase only advances forward\")"),
    "set_phase must require next > current (strict, not >=)",
  );
});

test("SEALED_BID_AUCTION_SOURCE: record_winner verifies winner-revealed + effective matches", () => {
  // Two sanity checks at settle time: (a) the winner address must
  // have actually revealed in phase 1; (b) the seller's claimed
  // winning_effective_bid must match the on-chain reveal. Pin both
  // — without (a) the seller could declare an arbitrary winner;
  // without (b) the seller could declare a lower effective than
  // the bidder revealed.
  const record = SEALED_BID_AUCTION_SOURCE.slice(
    SEALED_BID_AUCTION_SOURCE.indexOf("fn record_winner("),
    SEALED_BID_AUCTION_SOURCE.indexOf("fn current_phase()"),
  );
  assert.ok(
    record.includes("require(self.revealed[winner_addr] > 0, \"winner did not reveal\")"),
    "record_winner must verify winner_addr revealed",
  );
  assert.ok(
    record.includes("require(on_chain == winning_effective_bid, \"effective bid mismatch\")"),
    "record_winner must verify claimed effective matches on-chain reveal",
  );
});

test("SEALED_BID_AUCTION_SOURCE: on_evaporate emits void event only when unsettled", () => {
  // A settled auction discharged its purpose; on_evaporate should
  // not emit void on it. An unsettled auction's bidders need to be
  // refunded — coordinator watches for the void emit. Pin the gate.
  const evap = SEALED_BID_AUCTION_SOURCE.slice(
    SEALED_BID_AUCTION_SOURCE.indexOf("on_evaporate()"),
    SEALED_BID_AUCTION_SOURCE.length,
  );
  assert.ok(
    evap.includes("if self.settled == false"),
    "on_evaporate must gate void emit on settled == false",
  );
});
