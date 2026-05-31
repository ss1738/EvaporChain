import { test } from "node:test";
import assert from "node:assert/strict";
import {
  energyAtEpoch,
  mortalMessageEnergy,
  accessPassEnergy,
  accessPassValid,
  mayflyEnergy,
  mayflyAlive,
  attentionQuantumValue,
  currentStreak,
  streakWindowRemaining,
  streakHasBoost,
  cardRetrievabilityBp,
  cardIsDue,
  refreshMortalMessage,
  refreshAccessPass,
  refreshMayfly,
  checkInStreak,
  reviewCard,
  initialVista,
  type MnemoCard,
  type Streak,
} from "../src/sim.ts";

test("energyAtEpoch matches the canonical halving math at boundary points", () => {
  // Same numbers as decay-access-pass/test/decay-access-pass.test.ts
  assert.equal(energyAtEpoch(1_000_000n, 100n, 0n), 1_000_000n);
  assert.equal(energyAtEpoch(1_000_000n, 100n, 100n), 500_000n);
  assert.equal(energyAtEpoch(1_000_000n, 100n, 200n), 250_000n);
  assert.equal(energyAtEpoch(1_000_000n, 100n, 260n), 175_000n);
  assert.equal(energyAtEpoch(1_000_000n, 100n, 10_000n), 0n);
  assert.equal(energyAtEpoch(1_000_000n, 0n, 5n), 0n);
});

test("mortalMessage / accessPass / mayfly energy decay monotonically", () => {
  const v = initialVista();
  let prev = mortalMessageEnergy(v.message, 0n);
  for (let e = 1n; e <= 500n; e++) {
    const cur = mortalMessageEnergy(v.message, e);
    assert.ok(cur <= prev, `non-monotone at epoch ${e}: ${cur} > ${prev}`);
    prev = cur;
  }
  assert.ok(mayflyEnergy(v.mayfly, 200n) < mayflyEnergy(v.mayfly, 50n));
  assert.ok(accessPassEnergy(v.pass, 100n) > accessPassEnergy(v.pass, 300n));
});

test("accessPassValid flips false below the floor", () => {
  const v = initialVista();
  // initialEnergy=1000, halfLife=60, floor=250.
  // energy crosses 250 around epoch 120 (2 half-lives → 250).
  assert.ok(accessPassValid(v.pass, 0n));
  assert.ok(accessPassValid(v.pass, 100n));
  assert.ok(accessPassValid(v.pass, 120n)); // exactly at floor, still valid
  assert.ok(!accessPassValid(v.pass, 130n)); // below floor → invalid
});

test("accessPassValid false after revoke even at full energy", () => {
  const v = initialVista();
  const revoked = { ...v.pass, revoked: true };
  assert.ok(!accessPassValid(revoked, 0n));
});

test("mayflyAlive flips false when energy reaches 0", () => {
  const v = initialVista();
  // initialEnergy=1000, halfLife=10 → effectively dead by ~epoch 100.
  assert.ok(mayflyAlive(v.mayfly, 0n));
  assert.ok(mayflyAlive(v.mayfly, 50n));
  assert.ok(!mayflyAlive(v.mayfly, 1000n));
});

test("attentionQuantum decays linearly + redeem zeroes it", () => {
  const v = initialVista();
  // initialValue=1000, halfLife=45 → lifespan=90.
  assert.equal(attentionQuantumValue(v.aq, 0n), 1000n);
  assert.equal(attentionQuantumValue(v.aq, 45n), 500n); // half lifespan
  assert.equal(attentionQuantumValue(v.aq, 90n), 0n);   // expiry
  assert.equal(attentionQuantumValue({ ...v.aq, redeemed: true }, 10n), 0n);
});

test("checkInStreak: first check-in seeds + within window grows + outside resets", () => {
  const v = initialVista();
  const s1 = checkInStreak(v.streak, 5n);
  assert.equal(s1.streakCount, 1n);
  assert.equal(s1.hasCheckedIn, true);
  // Inside window (5 + 7 = 12, check-in at 10) — grows.
  const s2 = checkInStreak(s1, 10n);
  assert.equal(s2.streakCount, 2n);
  // Outside window (10 + 7 = 17, check-in at 50) — resets.
  const s3 = checkInStreak(s2, 50n);
  assert.equal(s3.streakCount, 1n);
  assert.equal(s3.peak, 2n);
});

test("currentStreak / streakWindowRemaining: decay-aware views", () => {
  const s: Streak = {
    kind: "streak",
    streakCount: 3n,
    peak: 3n,
    lastCheckin: 10n,
    hasCheckedIn: true,
    halfLife: 7n,
    boostThresholdBp: 5000n,
  };
  assert.equal(currentStreak(s, 17n), 3n); // boundary inclusive
  assert.equal(currentStreak(s, 18n), 0n); // past
  assert.equal(streakWindowRemaining(s, 17n), 0n);
  assert.equal(streakWindowRemaining(s, 14n), 3n);
  assert.equal(streakHasBoost(s, 17n), true); // 3*10000 >= 5000*3
  assert.equal(streakHasBoost(s, 18n), false); // past window
});

test("reviewCard: Again halves with floor, Good doubles, Easy triples, Hard unchanged", () => {
  let c: MnemoCard = initialVista().card;
  c = reviewCard(c, 3, 0n); // Good 10 → 20
  assert.equal(c.stability, 20n);
  c = reviewCard(c, 4, 1n); // Easy 20 → 60
  assert.equal(c.stability, 60n);
  c = reviewCard(c, 2, 2n); // Hard unchanged
  assert.equal(c.stability, 60n);
  c = reviewCard(c, 1, 3n); // Again halves → 30
  assert.equal(c.stability, 30n);
  // Floor at 1.
  for (let i = 0; i < 10; i++) c = reviewCard(c, 1, BigInt(4 + i));
  assert.equal(c.stability, 1n);
});

test("cardRetrievabilityBp: pre-first-review full; linear post-review", () => {
  const v = initialVista();
  assert.equal(cardRetrievabilityBp(v.card, 100n), 10000n);
  const c2 = reviewCard(v.card, 2, 0n); // Hard keeps stability at 10
  assert.equal(cardRetrievabilityBp(c2, 0n), 10000n);
  assert.equal(cardRetrievabilityBp(c2, 5n), 5000n);
  assert.equal(cardRetrievabilityBp(c2, 10n), 0n);
});

test("cardIsDue: fires at 90% retrievability (stability/10)", () => {
  const v = initialVista();
  const c2 = reviewCard(v.card, 2, 0n); // stability=10
  assert.equal(cardIsDue(c2, 0n), false);
  assert.equal(cardIsDue(c2, 1n), true); // 10*1 >= 10*0 + 10
});

test("refresh* functions reset bornEpoch + bump counter", () => {
  const v = initialVista();
  const m = refreshMortalMessage(v.message, 50n);
  assert.equal(m.bornEpoch, 50n);
  assert.equal(m.refreshes, v.message.refreshes + 1);
  const p = refreshAccessPass(v.pass, 50n);
  assert.equal(p.bornEpoch, 50n);
  const f = refreshMayfly(v.mayfly, 50n);
  assert.equal(f.bornEpoch, 50n);
});

test("initialVista returns a usable starting state", () => {
  const v = initialVista();
  assert.equal(v.epoch, 0n);
  assert.equal(v.message.kind, "mortal-message");
  assert.equal(v.pass.kind, "access-pass");
  assert.equal(v.mayfly.kind, "mayfly");
  assert.equal(v.aq.kind, "attention-quantum");
  assert.equal(v.streak.kind, "streak");
  assert.equal(v.card.kind, "mnemo-card");
});
