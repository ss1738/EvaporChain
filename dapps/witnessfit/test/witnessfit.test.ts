import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  checkInPayload,
  resetPeakPayload,
  currentStreakPayload,
  hasBoostPayload,
  windowRemainingPayload,
  peakPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { WITNESSFIT_SOURCE } from "../src/contract.ts";
import {
  currentStreak,
  hasBoost,
  windowRemaining,
  checkInPreview,
  type StreakState,
} from "../src/streak.ts";

const initial: StreakState = {
  streakCount: 0n,
  lastCheckinEpoch: 0n,
  hasCheckedIn: false,
  halfLife: 7n,
  maxStreak: 0n,
  boostThresholdBp: 5000n,
};

test("deployPayload carries the contract source + params", () => {
  const p = deployPayload({ deployer: 1, energy: 1000, halfLife: 100 });
  assert.equal(p.source_code, WITNESSFIT_SOURCE);
  assert.equal(p.deployer, 1);
});

test("wearer-side payloads are no-arg with correct method names", () => {
  assert.equal(checkInPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "check_in");
  assert.deepEqual(checkInPayload({ caller: 1, contractId: 7, epoch: 0 }).args, []);

  assert.equal(resetPeakPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "reset_peak");

  assert.equal(currentStreakPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "current_streak");
  assert.equal(hasBoostPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "has_boost");
  assert.equal(
    windowRemainingPayload({ caller: 1, contractId: 7, epoch: 0 }).method,
    "window_remaining",
  );
  assert.equal(peakPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "peak");
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("currentStreak: pre-checkin → 0; inside window → live; past window → 0", () => {
  assert.equal(currentStreak(initial, 100n), 0n);
  const s: StreakState = { ...initial, streakCount: 3n, lastCheckinEpoch: 10n, hasCheckedIn: true, maxStreak: 3n };
  assert.equal(currentStreak(s, 10n), 3n);
  assert.equal(currentStreak(s, 17n), 3n); // 10+7 boundary inclusive
  assert.equal(currentStreak(s, 18n), 0n); // past
});

test("hasBoost: 50% threshold by default", () => {
  // Pre-checkin → false even if peak somehow set (defensive)
  const preCheckin: StreakState = { ...initial, streakCount: 2n, maxStreak: 4n };
  assert.equal(hasBoost(preCheckin, 0n), false);

  // Peak 4, streak 2 → 2 >= 0.5*4 ✓
  const live: StreakState = { ...initial, streakCount: 2n, lastCheckinEpoch: 10n, hasCheckedIn: true, maxStreak: 4n };
  assert.equal(hasBoost(live, 10n), true);

  // Streak 1, peak 4 → 1 < 0.5*4 → no boost
  const low: StreakState = { ...initial, streakCount: 1n, lastCheckinEpoch: 10n, hasCheckedIn: true, maxStreak: 4n };
  assert.equal(hasBoost(low, 10n), false);

  // Past window → no boost regardless
  assert.equal(hasBoost(live, 100n), false);
});

test("windowRemaining: counts down to 0 at boundary, 0 past it", () => {
  const s: StreakState = { ...initial, streakCount: 1n, lastCheckinEpoch: 10n, hasCheckedIn: true, maxStreak: 1n };
  assert.equal(windowRemaining(s, 10n), 7n);
  assert.equal(windowRemaining(s, 14n), 3n);
  assert.equal(windowRemaining(s, 17n), 0n);
  assert.equal(windowRemaining(s, 18n), 0n);
  // Pre-checkin: 0
  assert.equal(windowRemaining(initial, 0n), 0n);
});

test("checkInPreview: first-checkin seeds streak=1 + peak=1 + hasCheckedIn=true", () => {
  const next = checkInPreview(initial, 5n);
  assert.equal(next.streakCount, 1n);
  assert.equal(next.lastCheckinEpoch, 5n);
  assert.equal(next.hasCheckedIn, true);
  assert.equal(next.maxStreak, 1n);
});

test("checkInPreview: inside window grows streak; outside resets to 1 + peak preserved", () => {
  const s1: StreakState = { ...initial, streakCount: 3n, lastCheckinEpoch: 5n, hasCheckedIn: true, maxStreak: 3n };
  // Inside window (5 + 7 = 12, check-in at 10): streak → 4
  const grown = checkInPreview(s1, 10n);
  assert.equal(grown.streakCount, 4n);
  assert.equal(grown.maxStreak, 4n);
  // Outside window (5 + 7 = 12, check-in at 50): streak → 1, peak preserved
  const reset = checkInPreview(s1, 50n);
  assert.equal(reset.streakCount, 1n);
  assert.equal(reset.maxStreak, 3n);
});

test("checkInPreview: same-or-prior-epoch check-in is a no-op", () => {
  const s: StreakState = { ...initial, streakCount: 2n, lastCheckinEpoch: 10n, hasCheckedIn: true, maxStreak: 2n };
  assert.deepEqual(checkInPreview(s, 10n), s);
  assert.deepEqual(checkInPreview(s, 5n), s);
});

test("WITNESSFIT_SOURCE contains all method + lifecycle markers", () => {
  for (const name of [
    "check_in",
    "reset_peak",
    "current_streak",
    "has_boost",
    "window_remaining",
    "peak",
    "on_grace",
    "on_refresh",
    "on_evaporate",
  ]) {
    assert.ok(WITNESSFIT_SOURCE.includes(name), `WITNESSFIT_SOURCE missing: ${name}`);
  }
});
