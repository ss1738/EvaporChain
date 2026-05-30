// Client-side projection of the on-chain streak state — pure
// BigInt port of witnessfit.es so the UI can preview what the
// chain WOULD say without a round-trip. Mirrors the contract's
// integer arithmetic exactly.

export interface StreakState {
  streakCount: bigint;
  lastCheckinEpoch: bigint;  // only meaningful if hasCheckedIn is true
  hasCheckedIn: boolean;     // sentinel: distinguishes 'never' from 'at epoch 0'
  halfLife: bigint;          // decay window in epochs
  maxStreak: bigint;
  boostThresholdBp: bigint;  // basis points out of 10000
}

/** Decay-aware current streak — what `current_streak()` returns
 *  on-chain at the given epoch. Returns 0n if never checked in OR
 *  if the half-life window has elapsed since the last check-in. */
export function currentStreak(s: StreakState, atEpoch: bigint): bigint {
  if (!s.hasCheckedIn) return 0n;
  if (atEpoch <= s.lastCheckinEpoch + s.halfLife) return s.streakCount;
  return 0n;
}

/** Mirror of `has_boost()` on-chain. True iff current streak > 0 AND
 *  streakCount * 10000 >= boostThresholdBp * maxStreak. */
export function hasBoost(s: StreakState, atEpoch: bigint): boolean {
  if (s.maxStreak === 0n) return false;
  if (!s.hasCheckedIn) return false;
  if (atEpoch > s.lastCheckinEpoch + s.halfLife) return false;
  return s.streakCount * 10000n >= s.boostThresholdBp * s.maxStreak;
}

/** Mirror of `window_remaining()` on-chain. 0n if past the window. */
export function windowRemaining(s: StreakState, atEpoch: bigint): bigint {
  if (!s.hasCheckedIn) return 0n;
  if (atEpoch > s.lastCheckinEpoch + s.halfLife) return 0n;
  return s.lastCheckinEpoch + s.halfLife - atEpoch;
}

/** Predict the state AFTER a check_in at `atEpoch`, given the current
 *  state. Returns the would-be next state; pure (no mutation). */
export function checkInPreview(s: StreakState, atEpoch: bigint): StreakState {
  if (!s.hasCheckedIn) {
    // First-ever check-in: streak = 1, peak = max(prev peak, 1).
    return {
      ...s,
      streakCount: 1n,
      lastCheckinEpoch: atEpoch,
      hasCheckedIn: true,
      maxStreak: s.maxStreak > 1n ? s.maxStreak : 1n,
    };
  }
  // Reject double check-in same epoch by returning the unchanged state.
  if (atEpoch <= s.lastCheckinEpoch) return s;
  const nextStreak =
    atEpoch <= s.lastCheckinEpoch + s.halfLife ? s.streakCount + 1n : 1n;
  return {
    ...s,
    streakCount: nextStreak,
    lastCheckinEpoch: atEpoch,
    maxStreak: nextStreak > s.maxStreak ? nextStreak : s.maxStreak,
  };
}
