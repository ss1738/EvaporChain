/**
 * Formatting utilities for the extension UI.
 */

/** Shorten an address: 0xabcd...ef12 */
export function shortAddress(addr: string): string {
  if (addr.length <= 14) return addr;
  return `${addr.slice(0, 6)}...${addr.slice(-4)}`;
}

/** Format EVAP balance with commas */
export function formatBalance(amount: number): string {
  return amount.toLocaleString("en-US");
}

/** Format energy as percentage */
export function energyPercent(current: number, max: number): number {
  if (max === 0) return 0;
  return Math.round((current / max) * 100);
}

/** Get energy bar color based on percentage */
export function energyColor(percent: number): string {
  if (percent > 50) return "#22c55e"; // green
  if (percent > 20) return "#f59e0b"; // amber
  if (percent > 5) return "#ef4444";  // red
  return "#6b7280"; // ghost gray
}

/** Get energy status label */
export function energyStatus(percent: number): string {
  if (percent > 50) return "Healthy";
  if (percent > 20) return "Declining";
  if (percent > 5) return "Critical";
  if (percent > 0) return "Near Death";
  return "Evaporated";
}

/** Time ago string from ISO date */
export function timeAgo(isoDate: string): string {
  const diff = Date.now() - new Date(isoDate).getTime();
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}
