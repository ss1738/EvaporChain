// Sim — pure BigInt functions that mirror EvaporChain's on-chain
// decay formulas byte-for-byte. The browser demo (index.html)
// duplicates these inline (so it can run with zero install);
// this module exists so the math has tests + a single source of
// truth for the canonical formulas.
//
// Each contract type is a plain object; the per-type `*_value`
// functions return the decay state at a given epoch. The doctrine
// claim: every value is monotone-non-increasing in `atEpoch`
// unless `refresh()` is called.

/**
 * Canonical EvaporChain energy decay.
 *   energy_at_epoch(initial, half_life, epochs) =
 *     (initial >> fullHalvings) - fractional_within_halving
 *
 * Matches `evaporchain-types::energy_at_epoch` byte-for-byte (the
 * same formula every on-chain contract resolves through).
 */
export function energyAtEpoch(initial: bigint, halfLife: bigint, epochsElapsed: bigint): bigint {
  if (halfLife <= 0n) return 0n;
  if (epochsElapsed < 0n) return initial;
  const fullHalvings = epochsElapsed / halfLife;
  if (fullHalvings >= 64n) return 0n;
  const afterHalvings = initial >> fullHalvings;
  const remainder = epochsElapsed % halfLife;
  const fractionalDecay = (afterHalvings * remainder) / (2n * halfLife);
  const r = afterHalvings - fractionalDecay;
  return r < 0n ? 0n : r;
}

/* ──────────────────────────────────────────────────────────────────
 *  Mortal Message — sender writes once, recipient reads while alive.
 *  Contract's own energy decays; no logic beyond the energy curve.
 * ────────────────────────────────────────────────────────────────── */

export interface MortalMessage {
  kind: 'mortal-message';
  body: string;
  recipient: string;
  initialEnergy: bigint;
  halfLife: bigint;
  bornEpoch: bigint;
  refreshes: number;
}

export function mortalMessageEnergy(m: MortalMessage, atEpoch: bigint): bigint {
  return energyAtEpoch(m.initialEnergy, m.halfLife, atEpoch - m.bornEpoch);
}

/* ──────────────────────────────────────────────────────────────────
 *  Decay Access Pass — credential with a floor; valid iff
 *  energy >= floor. Below the floor, the pass is "expired but
 *  still technically alive."
 * ────────────────────────────────────────────────────────────────── */

export interface AccessPass {
  kind: 'access-pass';
  holder: string;
  initialEnergy: bigint;
  halfLife: bigint;
  floor: bigint;
  bornEpoch: bigint;
  revoked: boolean;
  refreshes: number;
}

export function accessPassEnergy(p: AccessPass, atEpoch: bigint): bigint {
  return energyAtEpoch(p.initialEnergy, p.halfLife, atEpoch - p.bornEpoch);
}

export function accessPassValid(p: AccessPass, atEpoch: bigint): boolean {
  if (p.revoked) return false;
  return accessPassEnergy(p, atEpoch) >= p.floor;
}

/* ──────────────────────────────────────────────────────────────────
 *  Mayfly — pure ephemeral NFT. The contract's own energy IS the
 *  NFT's lifespan; no logic beyond it. We track holder + metadata
 *  for display but the decay curve is the doctrine point.
 * ────────────────────────────────────────────────────────────────── */

export interface Mayfly {
  kind: 'mayfly';
  metadata: string;
  initialEnergy: bigint;
  halfLife: bigint;
  bornEpoch: bigint;
  refreshes: number;
}

export function mayflyEnergy(m: Mayfly, atEpoch: bigint): bigint {
  return energyAtEpoch(m.initialEnergy, m.halfLife, atEpoch - m.bornEpoch);
}

export function mayflyAlive(m: Mayfly, atEpoch: bigint): boolean {
  return mayflyEnergy(m, atEpoch) > 0n;
}

/* ──────────────────────────────────────────────────────────────────
 *  SAP Attention Quantum — linear-decay reference (matches sap.es).
 *  Value goes from initialValue at born_epoch to 0 over 2*half_life
 *  epochs. Redeem flag short-circuits to 0.
 * ────────────────────────────────────────────────────────────────── */

export interface AttentionQuantum {
  kind: 'attention-quantum';
  initialValue: bigint;
  halfLife: bigint;
  bornEpoch: bigint;
  redeemed: boolean;
}

export function attentionQuantumValue(a: AttentionQuantum, atEpoch: bigint): bigint {
  if (a.redeemed) return 0n;
  const lifespan = 2n * a.halfLife;
  const age = atEpoch - a.bornEpoch;
  if (age < 0n) return a.initialValue;
  if (age >= lifespan) return 0n;
  return (a.initialValue * (lifespan - age)) / lifespan;
}

/* ──────────────────────────────────────────────────────────────────
 *  WitnessFit Streak — current streak is decay-aware: it holds at
 *  `streakCount` while inside the half-life window; resets to 0
 *  (as a VIEW; on-chain it'd reset to 1 on the next check-in)
 *  once past the window without a check-in.
 * ────────────────────────────────────────────────────────────────── */

export interface Streak {
  kind: 'streak';
  streakCount: bigint;
  peak: bigint;
  lastCheckin: bigint;
  hasCheckedIn: boolean;
  halfLife: bigint;
  boostThresholdBp: bigint;
}

export function currentStreak(s: Streak, atEpoch: bigint): bigint {
  if (!s.hasCheckedIn) return 0n;
  if (atEpoch <= s.lastCheckin + s.halfLife) return s.streakCount;
  return 0n;
}

export function streakWindowRemaining(s: Streak, atEpoch: bigint): bigint {
  if (!s.hasCheckedIn) return 0n;
  if (atEpoch > s.lastCheckin + s.halfLife) return 0n;
  return s.lastCheckin + s.halfLife - atEpoch;
}

export function streakHasBoost(s: Streak, atEpoch: bigint): boolean {
  if (s.peak === 0n) return false;
  if (!s.hasCheckedIn) return false;
  if (atEpoch > s.lastCheckin + s.halfLife) return false;
  return s.streakCount * 10000n >= s.boostThresholdBp * s.peak;
}

/* ──────────────────────────────────────────────────────────────────
 *  MnemoChain Card — FSRS-lite retrievability. Linear from 10000bp
 *  at last_review to 0 over `stability` epochs.
 * ────────────────────────────────────────────────────────────────── */

export interface MnemoCard {
  kind: 'mnemo-card';
  question: string;       // display only
  stability: bigint;
  lastReview: bigint;
  hasReviewed: boolean;
}

export function cardRetrievabilityBp(c: MnemoCard, atEpoch: bigint): bigint {
  if (!c.hasReviewed) return 10000n;
  if (atEpoch >= c.lastReview + c.stability) return 0n;
  return (10000n * (c.lastReview + c.stability - atEpoch)) / c.stability;
}

export function cardIsDue(c: MnemoCard, atEpoch: bigint): boolean {
  if (!c.hasReviewed) return false;
  if (atEpoch >= c.lastReview + c.stability) return true;
  // 90% threshold: due when retrievability < 9000bp, i.e. age > stability/10.
  return 10n * atEpoch >= 10n * c.lastReview + c.stability;
}

/* ──────────────────────────────────────────────────────────────────
 *  Refresh actions — pure functions returning a new contract state.
 *  The demo wires its "refresh" buttons through these so a user can
 *  see the decay reverse in real time.
 * ────────────────────────────────────────────────────────────────── */

export function refreshMortalMessage(m: MortalMessage, atEpoch: bigint): MortalMessage {
  return { ...m, bornEpoch: atEpoch, refreshes: m.refreshes + 1 };
}

export function refreshAccessPass(p: AccessPass, atEpoch: bigint): AccessPass {
  return { ...p, bornEpoch: atEpoch, refreshes: p.refreshes + 1 };
}

export function refreshMayfly(m: Mayfly, atEpoch: bigint): Mayfly {
  return { ...m, bornEpoch: atEpoch, refreshes: m.refreshes + 1 };
}

/** Streak check-in: continues if inside the half-life window, resets
 *  to 1 otherwise. Mirrors witnessfit.es exactly. */
export function checkInStreak(s: Streak, atEpoch: bigint): Streak {
  if (!s.hasCheckedIn) {
    return {
      ...s,
      streakCount: 1n,
      lastCheckin: atEpoch,
      hasCheckedIn: true,
      peak: s.peak > 1n ? s.peak : 1n,
    };
  }
  if (atEpoch <= s.lastCheckin) return s; // same-or-prior epoch: no-op
  const next = atEpoch <= s.lastCheckin + s.halfLife ? s.streakCount + 1n : 1n;
  return {
    ...s,
    streakCount: next,
    lastCheckin: atEpoch,
    peak: next > s.peak ? next : s.peak,
  };
}

/** Card review: Again=1 halves, Hard=2 unchanged, Good=3 doubles,
 *  Easy=4 triples. Stability floor of 1. */
export function reviewCard(c: MnemoCard, rating: 1 | 2 | 3 | 4, atEpoch: bigint): MnemoCard {
  let nextStability = c.stability;
  if (rating === 1) {
    nextStability = c.stability / 2n;
    if (nextStability < 1n) nextStability = 1n;
  } else if (rating === 3) {
    nextStability = c.stability * 2n;
  } else if (rating === 4) {
    nextStability = c.stability * 3n;
  }
  return { ...c, stability: nextStability, lastReview: atEpoch, hasReviewed: true };
}

/* ──────────────────────────────────────────────────────────────────
 *  Convenience — bundle the whole demo state.
 * ────────────────────────────────────────────────────────────────── */

export interface VistaState {
  epoch: bigint;
  message: MortalMessage;
  pass: AccessPass;
  mayfly: Mayfly;
  aq: AttentionQuantum;
  streak: Streak;
  card: MnemoCard;
}

export function initialVista(): VistaState {
  return {
    epoch: 0n,
    message: {
      kind: 'mortal-message',
      body: 'happy birthday — burn the candle while you can',
      recipient: '0x21…',
      initialEnergy: 1000n,
      halfLife: 50n,
      bornEpoch: 0n,
      refreshes: 0,
    },
    pass: {
      kind: 'access-pass',
      holder: '0x22…',
      initialEnergy: 1000n,
      halfLife: 60n,
      floor: 250n,
      bornEpoch: 0n,
      revoked: false,
      refreshes: 0,
    },
    mayfly: {
      kind: 'mayfly',
      metadata: 'one day, one wing',
      initialEnergy: 1000n,
      halfLife: 10n,
      bornEpoch: 0n,
      refreshes: 0,
    },
    aq: {
      kind: 'attention-quantum',
      initialValue: 1000n,
      halfLife: 45n,
      bornEpoch: 0n,
      redeemed: false,
    },
    streak: {
      kind: 'streak',
      streakCount: 0n,
      peak: 0n,
      lastCheckin: 0n,
      hasCheckedIn: false,
      halfLife: 7n,
      boostThresholdBp: 5000n,
    },
    card: {
      kind: 'mnemo-card',
      question: 'capital of Mongolia',
      stability: 10n,
      lastReview: 0n,
      hasReviewed: false,
    },
  };
}
