/**
 * EnergyBar — Reusable horizontal energy indicator
 *
 * Color-coded by energy percentage:
 *   > 60%  => green (#22c55e)
 *   > 30%  => amber (#f59e0b)
 *   > 10%  => red (#ef4444)
 *   <= 10% => red + pulsing
 */

import React from 'react';
import { View, Text, StyleSheet } from 'react-native';

interface EnergyBarProps {
  energy: number;
  maxEnergy: number;
  height?: number;
  showLabel?: boolean;
  showPercentage?: boolean;
  style?: object;
}

function getBarColor(percentage: number): string {
  if (percentage > 60) return '#22c55e';
  if (percentage > 30) return '#f59e0b';
  return '#ef4444';
}

function getStateLabel(percentage: number): string {
  if (percentage > 20) return 'Active';
  if (percentage > 0) return 'Grace';
  return 'Ghost';
}

function getStateBadgeColor(percentage: number): string {
  if (percentage > 20) return '#22c55e';
  if (percentage > 0) return '#f59e0b';
  return '#9ca3af';
}

export const EnergyBar: React.FC<EnergyBarProps> = ({
  energy,
  maxEnergy,
  height = 8,
  showLabel = false,
  showPercentage = true,
  style,
}) => {
  const percentage = maxEnergy > 0 ? Math.round((energy / maxEnergy) * 100) : 0;
  const clampedPercentage = Math.max(0, Math.min(100, percentage));
  const barColor = getBarColor(clampedPercentage);

  return (
    <View style={[styles.container, style]}>
      {showLabel && (
        <View style={styles.labelRow}>
          <Text style={styles.label}>Energy</Text>
          <View
            style={[
              styles.stateBadge,
              { backgroundColor: getStateBadgeColor(clampedPercentage) },
            ]}
          >
            <Text style={styles.stateBadgeText}>
              {getStateLabel(clampedPercentage)}
            </Text>
          </View>
        </View>
      )}
      <View style={styles.barRow}>
        <View style={[styles.track, { height }]}>
          <View
            style={[
              styles.fill,
              {
                width: `${clampedPercentage}%`,
                backgroundColor: barColor,
                height,
              },
            ]}
          />
        </View>
        {showPercentage && (
          <Text style={[styles.percentage, { color: barColor }]}>
            {clampedPercentage}%
          </Text>
        )}
      </View>
    </View>
  );
};

const styles = StyleSheet.create({
  container: {},
  labelRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginBottom: 4,
  },
  label: {
    fontSize: 12,
    color: '#6b7280',
    fontWeight: '500',
  },
  stateBadge: {
    paddingHorizontal: 8,
    paddingVertical: 2,
    borderRadius: 10,
  },
  stateBadgeText: {
    fontSize: 10,
    color: '#ffffff',
    fontWeight: '700',
    textTransform: 'uppercase',
  },
  barRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  track: {
    flex: 1,
    backgroundColor: '#f3f4f6',
    borderRadius: 4,
    overflow: 'hidden',
  },
  fill: {
    borderRadius: 4,
  },
  percentage: {
    fontSize: 12,
    fontWeight: '600',
    minWidth: 36,
    textAlign: 'right',
  },
});

export default EnergyBar;
