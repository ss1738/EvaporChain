/**
 * Helper utilities for the EvaporChain wallet SDK.
 * Zero-dependency — pure functions only.
 */

/**
 * Check whether the EvaporChain wallet extension is installed.
 * Returns true if `window.evaporchain` is present and identifies
 * itself as the EvaporChain provider.
 */
export function isEvaporChainInstalled(): boolean {
  return (
    typeof window !== "undefined" &&
    window.evaporchain !== undefined &&
    window.evaporchain.isEvaporChain === true
  );
}

/**
 * Format a raw balance (integer) into a human-readable string.
 *
 * @param amount - Raw balance amount (smallest unit)
 * @param decimals - Number of decimal places (default: 9, matching EVAP's native precision)
 * @returns Formatted string, e.g. "1,234.567"
 *
 * @example
 * formatBalance(1_500_000_000) // "1.5"
 * formatBalance(123456, 4)     // "12.3456"
 */
export function formatBalance(amount: number, decimals: number = 9): string {
  const divisor = Math.pow(10, decimals);
  const value = amount / divisor;

  // Use Intl for locale-aware grouping, then trim trailing zeros
  const formatted = new Intl.NumberFormat("en-US", {
    minimumFractionDigits: 0,
    maximumFractionDigits: decimals,
  }).format(value);

  return formatted;
}

/**
 * Shorten an address for display purposes.
 *
 * @param address - Full hex address
 * @returns Shortened address, e.g. "0x1a2b...9f0e"
 *
 * @example
 * shortenAddress("0x1a2b3c4d5e6f7890abcdef1234567890abcd9f0e")
 * // "0x1a2b...9f0e"
 */
export function shortenAddress(address: string): string {
  if (address.length <= 12) return address;

  const prefix = address.startsWith("0x") ? address.slice(0, 6) : address.slice(0, 4);
  const suffix = address.slice(-4);
  return `${prefix}...${suffix}`;
}

/**
 * Calculate the remaining energy of a decaying object after a given
 * number of epochs, using EvaporChain's exponential decay formula:
 *
 *   energy(t) = initialEnergy * 2^(-t / halfLife)
 *
 * @param energy - Current energy level
 * @param halfLife - Half-life in epochs
 * @param epochs - Number of epochs elapsed
 * @returns Remaining energy (floored to integer)
 *
 * @example
 * calculateDecay(1000, 10, 10) // 500 (one half-life elapsed)
 * calculateDecay(1000, 10, 20) // 250 (two half-lives elapsed)
 */
export function calculateDecay(energy: number, halfLife: number, epochs: number): number {
  if (halfLife <= 0 || energy <= 0) return 0;
  return Math.floor(energy * Math.pow(2, -epochs / halfLife));
}

/**
 * Estimate how many epochs remain before an object's energy effectively
 * reaches zero (drops below 1).
 *
 * Uses the inverse of the decay formula:
 *   epochs = halfLife * log2(energy)
 *
 * @param energy - Current energy level
 * @param halfLife - Half-life in epochs
 * @returns Estimated epochs remaining until evaporation
 *
 * @example
 * estimateEvaporation(1000, 10) // ~100 (log2(1000) * 10 ≈ 99.66)
 */
export function estimateEvaporation(energy: number, halfLife: number): number {
  if (halfLife <= 0 || energy <= 1) return 0;
  return Math.ceil(halfLife * Math.log2(energy));
}
