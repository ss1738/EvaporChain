// Client-side validity forecast for the Decay Access Pass.
//
// `energyAtEpoch` is a BYTE-EXACT TypeScript port of the chain's
// `evaporchain_types::energy_at_epoch` (BigInt to match u64/u128
// integer semantics), so the dApp can predict a pass's strength /
// validity / expiry WITHOUT a round-trip to the node. The on-chain
// `is_valid` (contracts/evaporscript/decay_access_pass.es) is the
// authority; this mirrors it for UI forecasting only.

/** Decayed energy of a pass: exact port of `energy_at_epoch`.
 *  `initial >> full_halvings`, minus linear within-halving interpolation. */
export function energyAtEpoch(initial: bigint, halfLife: bigint, epochsElapsed: bigint): bigint {
  if (halfLife <= 0n) return 0n;
  const fullHalvings = epochsElapsed / halfLife;
  if (fullHalvings >= 64n) return 0n;
  const afterHalvings = initial >> fullHalvings;
  const remainder = epochsElapsed % halfLife;
  const fractionalDecay = (afterHalvings * remainder) / (2n * halfLife);
  const r = afterHalvings - fractionalDecay;
  return r < 0n ? 0n : r; // mirrors saturating_sub
}

/** A pass's live strength at `epochsElapsed` after its last refresh. */
export function passStrengthAt(initial: bigint, halfLife: bigint, epochsElapsed: bigint): bigint {
  return energyAtEpoch(initial, halfLife, epochsElapsed);
}

export interface PassState {
  /** Strength baseline at last refresh (the contract's `energy`). */
  energy: bigint;
  halfLife: bigint;
  validityFloor: bigint;
  sealed: boolean;
  revoked: boolean;
}

/** Mirrors the contract's `is_valid`: sealed, not revoked, and strength
 *  still at or above the floor `epochsElapsed` after the last refresh. */
export function isValidAt(p: PassState, epochsElapsed: bigint): boolean {
  if (!p.sealed || p.revoked) return false;
  return passStrengthAt(p.energy, p.halfLife, epochsElapsed) >= p.validityFloor;
}

/** The epoch-offset at which a (sealed, un-revoked) pass first drops
 *  below its floor — the forecast "expires in N epochs" for the UI.
 *  `null` = never expires by the floor (floor ≤ 0). `0` = already below
 *  the floor (born invalid). Monotone decay ⇒ binary search. */
export function expiryEpochOffset(initial: bigint, halfLife: bigint, floor: bigint): number | null {
  if (floor <= 0n) return null;
  if (halfLife <= 0n) return 0; // energy is 0 → below any positive floor
  if (initial < floor) return 0;
  let lo = 0;
  let hi = Number(64n * halfLife); // strength is 0 here, < floor (≥1)
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (energyAtEpoch(initial, halfLife, BigInt(mid)) < floor) hi = mid;
    else lo = mid + 1;
  }
  return lo;
}
