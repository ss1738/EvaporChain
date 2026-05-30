import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  openPayload,
  addPiecePayload,
  removePiecePayload,
  closeEarlyPayload,
  isOpenPayload,
  isPieceActivePayload,
  pieceHashViewPayload,
  activePiecesPayload,
  galleryNamePayload,
  ageSinceOpenPayload,
  nextIdPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { GALLERY_FORGETS_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source", () => {
  const p = deployPayload({ deployer: 1, energy: 1000, halfLife: 100 });
  assert.equal(p.source_code, GALLERY_FORGETS_SOURCE);
});

test("openPayload carries the name string", () => {
  const p = openPayload({ caller: 1, contractId: 7, name: "Opening Night", epoch: 0 });
  assert.equal(p.method, "open");
  assert.deepEqual(p.args, ["Opening Night"]);
});

test("addPiecePayload carries the content_hash", () => {
  const p = addPiecePayload({ caller: 1, contractId: 7, contentHash: "ipfs://bafy...", epoch: 5 });
  assert.equal(p.method, "add_piece");
  assert.deepEqual(p.args, ["ipfs://bafy..."]);
});

test("removePiecePayload carries the piece_id", () => {
  const p = removePiecePayload({ caller: 1, contractId: 7, pieceId: 3, epoch: 10 });
  assert.equal(p.method, "remove_piece");
  assert.deepEqual(p.args, [3]);
});

test("closeEarly + view payloads are no-arg with correct names", () => {
  assert.equal(closeEarlyPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "close_early");
  assert.deepEqual(closeEarlyPayload({ caller: 1, contractId: 7, epoch: 0 }).args, []);

  for (const [fn, name] of [
    [isOpenPayload, "is_open"],
    [activePiecesPayload, "active_pieces"],
    [galleryNamePayload, "gallery_name_view"],
    [ageSinceOpenPayload, "age_since_open"],
    [nextIdPayload, "next_id"],
  ] as const) {
    assert.equal(fn({ caller: 1, contractId: 7, epoch: 0 }).method, name);
  }
});

test("piece-id view payloads carry the id", () => {
  const ipa = isPieceActivePayload({ caller: 1, contractId: 7, pieceId: 2, epoch: 0 });
  assert.equal(ipa.method, "is_piece_active");
  assert.deepEqual(ipa.args, [2]);

  const phv = pieceHashViewPayload({ caller: 1, contractId: 7, pieceId: 2, epoch: 0 });
  assert.equal(phv.method, "piece_hash_view");
  assert.deepEqual(phv.args, [2]);
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("GALLERY_FORGETS_SOURCE contains all methods + lifecycle hooks", () => {
  for (const name of [
    "open",
    "add_piece",
    "remove_piece",
    "close_early",
    "is_open",
    "is_piece_active",
    "piece_hash_view",
    "gallery_name_view",
    "age_since_open",
    "next_id",
    "on_grace",
    "on_refresh",
    "on_evaporate",
    "every piece is now memory", // doctrine-flagging event string
  ]) {
    assert.ok(GALLERY_FORGETS_SOURCE.includes(name), `GALLERY_FORGETS_SOURCE missing: ${name}`);
  }
});
