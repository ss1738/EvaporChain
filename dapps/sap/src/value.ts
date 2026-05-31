// Client-side port of the on-chain AQ value formula. V2: matches
// sap.es's `>>`-based exact-exponential decay byte-for-byte
// (value halves every `halfLife` epochs via integer-truncated
// right-shift; reaches 0 by age = ~log2(initial) × halfLife).
//
// Use in the dApp's "your AQ is worth X right now" pill, the
// marketplace's bid preview, etc., to avoid a node round-trip on
// every render.

export interface AqState {
  bornEpochShifted: bigint;  // aq_born value from the chain (epoch_real + 1; 0 means never)
  redeemed: boolean;
  initialValue: bigint;
  halfLife: bigint;
}

/** Mirror of `current_value(who)` on-chain.
 *  V2: `initial_value >> (age / half_life)`, clamping the shift to <64
 *  to match the VM's runtime reject (avoids the silent-zero / panic
 *  ambiguity at shift=64). */
export function valueAtEpoch(s: AqState, atEpoch: bigint): bigint {
  if (s.bornEpochShifted === 0n) return 0n;
  if (s.redeemed) return 0n;
  if (atEpoch + 1n < s.bornEpochShifted) return s.initialValue;
  const shift = (atEpoch + 1n - s.bornEpochShifted) / s.halfLife;
  if (shift >= 64n) return 0n;
  return s.initialValue >> shift;
}

/** Mirror of `has_active_aq(who)` on-chain — true while value > 0. */
export function hasActiveAq(s: AqState, atEpoch: bigint): boolean {
  return valueAtEpoch(s, atEpoch) > 0n;
}

/** Mirror of `epochs_until_expiry(who)` on-chain.
 *  V2: over-approximate upper bound `64 × half_life − age` rather
 *  than the exact `log2(initial) × half_life − age` (computing log2
 *  in EvaporScript is awkward). dApps that need the exact lifetime
 *  should poll `valueAtEpoch` directly. */
export function epochsUntilExpiry(s: AqState, atEpoch: bigint): bigint {
  if (s.bornEpochShifted === 0n) return 0n;
  if (s.redeemed) return 0n;
  if (atEpoch + 1n < s.bornEpochShifted) return 64n * s.halfLife;
  const shift = (atEpoch + 1n - s.bornEpochShifted) / s.halfLife;
  if (shift >= 64n) return 0n;
  return s.bornEpochShifted + 64n * s.halfLife - atEpoch - 1n;
}
