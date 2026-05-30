// Client-side port of the on-chain AQ value formula. Matches
// sap.es's integer truncation byte-for-byte (linear decay from
// initial to 0 over 2 × half_life epochs). Use in the dApp's
// "your AQ is worth X right now" pill, the marketplace's bid
// preview, etc., to avoid a node round-trip on every render.

export interface AqState {
  bornEpochShifted: bigint;  // aq_born value from the chain (epoch_real + 1; 0 means never)
  redeemed: boolean;
  initialValue: bigint;
  halfLife: bigint;
}

/** Mirror of `current_value(who)` on-chain. */
export function valueAtEpoch(s: AqState, atEpoch: bigint): bigint {
  if (s.bornEpochShifted === 0n) return 0n;
  if (s.redeemed) return 0n;
  const lifespan = 2n * s.halfLife;
  if (atEpoch + 1n >= s.bornEpochShifted + lifespan) return 0n;
  const remaining = s.bornEpochShifted + lifespan - atEpoch - 1n;
  return (s.initialValue * remaining) / lifespan;
}

/** Mirror of `has_active_aq(who)` on-chain. */
export function hasActiveAq(s: AqState, atEpoch: bigint): boolean {
  if (s.bornEpochShifted === 0n) return false;
  if (s.redeemed) return false;
  const lifespan = 2n * s.halfLife;
  if (atEpoch + 1n >= s.bornEpochShifted + lifespan) return false;
  return true;
}

/** Mirror of `epochs_until_expiry(who)` on-chain. */
export function epochsUntilExpiry(s: AqState, atEpoch: bigint): bigint {
  if (s.bornEpochShifted === 0n) return 0n;
  if (s.redeemed) return 0n;
  const lifespan = 2n * s.halfLife;
  if (atEpoch + 1n >= s.bornEpochShifted + lifespan) return 0n;
  return s.bornEpochShifted + lifespan - atEpoch - 1n;
}
