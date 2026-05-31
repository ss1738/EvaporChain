import { test } from "node:test";
import assert from "node:assert/strict";
import {
  LS_KEYS,
  DEFAULT_NODE,
  getToken,
  getNode,
  isAuthed,
  getProfile,
  setSession,
  clearSession,
  __resetForTests,
} from "../auth.ts";

test("getToken: null when nothing stored, returns value after setSession", () => {
  __resetForTests();
  assert.equal(getToken(), null);
  setSession({ node: "http://x:1", token: "abc", email: "a@b.c", userId: 7, displayName: "n" });
  assert.equal(getToken(), "abc");
});

test("getNode: falls back to DEFAULT_NODE when not set, strips trailing slash", () => {
  __resetForTests();
  assert.equal(getNode(), DEFAULT_NODE);
  setSession({ node: "http://node:8080/", token: "t", email: "a@b.c", userId: 1, displayName: "n" });
  assert.equal(getNode(), "http://node:8080");
});

test("isAuthed: tracks getToken presence", () => {
  __resetForTests();
  assert.equal(isAuthed(), false);
  setSession({ node: "http://x:1", token: "t", email: "a@b.c", userId: 1, displayName: "n" });
  assert.equal(isAuthed(), true);
});

test("getProfile: reads back the fields setSession wrote", () => {
  __resetForTests();
  assert.deepEqual(getProfile(), { email: null, userId: null, displayName: null });
  setSession({ node: "http://x:1", token: "tok", email: "user@example.com", userId: 42, displayName: "Alice" });
  assert.deepEqual(getProfile(), { email: "user@example.com", userId: 42, displayName: "Alice" });
});

test("clearSession: nukes token + profile but leaves node URL intact", () => {
  __resetForTests();
  setSession({ node: "http://x:1", token: "tok", email: "a@b.c", userId: 1, displayName: "Alice" });
  clearSession();
  assert.equal(getToken(), null);
  assert.equal(isAuthed(), false);
  assert.deepEqual(getProfile(), { email: null, userId: null, displayName: null });
  // node URL preserved — operators expect "logout" to keep the chosen node.
  assert.equal(getNode(), "http://x:1");
});

test("LS_KEYS shape matches the wallet's contract", () => {
  // The wallet writes these exact keys; if either side drifts, both
  // break. Pin the names here so a rename gets caught at test time.
  assert.equal(LS_KEYS.node,    "evaporchain_wallet_node");
  assert.equal(LS_KEYS.token,   "evaporchain_wallet_token");
  assert.equal(LS_KEYS.email,   "evaporchain_wallet_email");
  assert.equal(LS_KEYS.userId,  "evaporchain_wallet_user_id");
  assert.equal(LS_KEYS.display, "evaporchain_wallet_display_name");
});

test("DEFAULT_NODE is the public devnet", () => {
  assert.equal(DEFAULT_NODE, "http://89.167.52.40:8099");
});

test("setSession is idempotent — second write replaces first", () => {
  __resetForTests();
  setSession({ node: "http://x:1", token: "first", email: "a@b.c", userId: 1, displayName: "n" });
  setSession({ node: "http://y:2", token: "second", email: "c@d.e", userId: 2, displayName: "m" });
  assert.equal(getToken(), "second");
  assert.equal(getNode(), "http://y:2");
  assert.deepEqual(getProfile(), { email: "c@d.e", userId: 2, displayName: "m" });
});
