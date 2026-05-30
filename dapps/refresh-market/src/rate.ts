// Client-side port of refresh_market.es's rate formula:
//   rate = base_rent * (used + 1)^2 / capacity^2
//
// Used to render "what would my rate be?" UIs without a node round-trip
// or before deploying. Matches the on-chain integer-division semantics
// (u64 truncation) so client previews == on-chain values byte-for-byte.

/** Compute the per-epoch rate at a given utilisation level.
 *  Returns 0n if capacity is 0 (the contract returns 0 pre-arm).
 *  All arithmetic in BigInt to mirror u64 truncation. */
export function currentRate(used: bigint, capacity: bigint, baseRent: bigint): bigint {
  if (capacity <= 0n) return 0n;
  const factor = used + 1n;
  return (baseRent * factor * factor) / (capacity * capacity);
}

/** Convenience: the rate the holder will see right after claiming an
 *  N+1th slot (i.e., one more than current `used`). Useful for the
 *  "after-claim preview" pill in a UI. */
export function rateAfterOneMoreClaim(used: bigint, capacity: bigint, baseRent: bigint): bigint {
  return currentRate(used + 1n, capacity, baseRent);
}

/** First `used` for which the rate strictly exceeds `threshold`.
 *  Returns null if no `used <= capacity` clears the threshold (i.e.,
 *  rate at full saturation is still ≤ threshold). Useful for
 *  "when will this namespace get expensive?" timeline UIs. */
export function firstUsedAboveRate(
  baseRent: bigint,
  capacity: bigint,
  threshold: bigint,
): number | null {
  if (capacity <= 0n) return null;
  for (let used = 0n; used <= capacity; used += 1n) {
    if (currentRate(used, capacity, baseRent) > threshold) {
      return Number(used);
    }
  }
  return null;
}
