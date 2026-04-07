/**
 * Decay math utilities for EvaporChain energy forecasting.
 *
 * Core formula: E(t) = E0 * 2^(-t / half_life)
 */

import type { StateObject } from "./api";

/**
 * Calculate current energy after `elapsedEpochs` of decay.
 */
export function calculateEnergy(
  initialEnergy: number,
  halfLife: number,
  elapsedEpochs: number,
): number {
  if (halfLife <= 0) return 0;
  return initialEnergy * Math.pow(2, -elapsedEpochs / halfLife);
}

/**
 * How many epochs until energy drops below `threshold`.
 * Returns Infinity if halfLife <= 0 or currentEnergy <= 0.
 */
export function epochsUntilThreshold(
  currentEnergy: number,
  halfLife: number,
  threshold: number,
): number {
  if (halfLife <= 0 || currentEnergy <= 0) return 0;
  if (currentEnergy <= threshold) return 0;
  // threshold = currentEnergy * 2^(-t / halfLife)
  // t = -halfLife * log2(threshold / currentEnergy)
  return -halfLife * Math.log2(threshold / currentEnergy);
}

/**
 * Estimate the calendar date when an object evaporates (energy -> 0 threshold).
 * Uses a practical threshold of 0.01 (effectively zero).
 */
export function estimateEvaporationDate(
  currentEnergy: number,
  halfLife: number,
  epochDurationMs: number,
  threshold: number = 0.01,
): Date {
  const epochs = epochsUntilThreshold(currentEnergy, halfLife, threshold);
  const msUntil = epochs * epochDurationMs;
  return new Date(Date.now() + msUntil);
}

/**
 * Calculate days remaining until evaporation.
 */
export function daysUntilEvaporation(
  currentEnergy: number,
  halfLife: number,
  epochDurationMs: number,
  threshold: number = 0.01,
): number {
  const epochs = epochsUntilThreshold(currentEnergy, halfLife, threshold);
  const msUntil = epochs * epochDurationMs;
  return msUntil / (1000 * 60 * 60 * 24);
}

export interface RefreshRecommendation {
  objectId: string;
  objectName: string;
  energyToAdd: number;
  currentEnergy: number;
  daysRemaining: number;
  daysSavedAfterRefresh: number;
}

/**
 * Calculate optimal refresh strategy: sort by urgency, determine minimum energy
 * needed to keep each object alive for `targetDays` more days.
 */
export function optimalRefreshStrategy(
  objects: StateObject[],
  budget: number,
  targetDays: number,
  epochDurationMs: number,
): RefreshRecommendation[] {
  const targetEpochs = (targetDays * 24 * 60 * 60 * 1000) / epochDurationMs;
  const recommendations: RefreshRecommendation[] = [];

  // Sort by urgency (least days remaining first)
  const sorted = [...objects].sort((a, b) => {
    const daysA = daysUntilEvaporation(a.current_energy, a.half_life, epochDurationMs);
    const daysB = daysUntilEvaporation(b.current_energy, b.half_life, epochDurationMs);
    return daysA - daysB;
  });

  let remainingBudget = budget;

  for (const obj of sorted) {
    const daysRemaining = daysUntilEvaporation(obj.current_energy, obj.half_life, epochDurationMs);

    // Calculate energy needed to survive targetDays from now
    // We need: (current_energy + added) * 2^(-targetEpochs / halfLife) >= threshold
    // So: added >= threshold / 2^(-targetEpochs / halfLife) - current_energy
    const decayFactor = Math.pow(2, -targetEpochs / obj.half_life);
    const threshold = 0.01;
    const energyNeededTotal = threshold / decayFactor;
    const energyToAdd = Math.max(0, Math.ceil(energyNeededTotal - obj.current_energy));

    if (energyToAdd <= 0) {
      // Object already survives the target period
      recommendations.push({
        objectId: obj.id,
        objectName: obj.name,
        energyToAdd: 0,
        currentEnergy: obj.current_energy,
        daysRemaining,
        daysSavedAfterRefresh: daysRemaining,
      });
      continue;
    }

    const cost = Math.min(energyToAdd, remainingBudget);
    remainingBudget -= cost;

    const newEnergy = obj.current_energy + cost;
    const newDays = daysUntilEvaporation(newEnergy, obj.half_life, epochDurationMs);

    recommendations.push({
      objectId: obj.id,
      objectName: obj.name,
      energyToAdd: cost,
      currentEnergy: obj.current_energy,
      daysRemaining,
      daysSavedAfterRefresh: newDays,
    });
  }

  return recommendations;
}

/**
 * Calculate total budget needed to keep all objects alive for targetDays.
 */
export function totalBudgetForSurvival(
  objects: StateObject[],
  targetDays: number,
  epochDurationMs: number,
): number {
  const targetEpochs = (targetDays * 24 * 60 * 60 * 1000) / epochDurationMs;
  let total = 0;

  for (const obj of objects) {
    const decayFactor = Math.pow(2, -targetEpochs / obj.half_life);
    const threshold = 0.01;
    const energyNeededTotal = threshold / decayFactor;
    const energyToAdd = Math.max(0, Math.ceil(energyNeededTotal - obj.current_energy));
    total += energyToAdd;
  }

  return total;
}

/**
 * Project total portfolio energy across future epochs.
 * Returns an array of { epoch, totalEnergy, percentOfMax } for each epoch step.
 */
export function projectedPortfolioEnergy(
  objects: StateObject[],
  futureEpochs: number,
  stepSize: number = 1,
): Array<{ epoch: number; totalEnergy: number; percentOfMax: number }> {
  const maxTotal = objects.reduce((sum, obj) => sum + obj.max_energy, 0);
  const results: Array<{ epoch: number; totalEnergy: number; percentOfMax: number }> = [];

  for (let e = 0; e <= futureEpochs; e += stepSize) {
    let totalEnergy = 0;
    for (const obj of objects) {
      totalEnergy += calculateEnergy(obj.current_energy, obj.half_life, e);
    }
    results.push({
      epoch: e,
      totalEnergy: Math.max(0, totalEnergy),
      percentOfMax: maxTotal > 0 ? (totalEnergy / maxTotal) * 100 : 0,
    });
  }

  return results;
}

/**
 * Project a single object's energy over future epochs.
 */
export function projectedObjectEnergy(
  currentEnergy: number,
  halfLife: number,
  futureEpochs: number,
  stepSize: number = 1,
): Array<{ epoch: number; energy: number }> {
  const results: Array<{ epoch: number; energy: number }> = [];
  for (let e = 0; e <= futureEpochs; e += stepSize) {
    results.push({
      epoch: e,
      energy: Math.max(0, calculateEnergy(currentEnergy, halfLife, e)),
    });
  }
  return results;
}
