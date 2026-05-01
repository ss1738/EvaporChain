/**
 * FeeControllerScreen — Lyapunov-stable base-fee controller.
 *
 * Mirrors extension/src/components/FeeControllerWidget.tsx (full screen
 * variant for mobile).
 *
 * GET /api/fee_controller/status — api.rs §get_fee_controller_status:
 *   { status, energy, base_fee, target_energy, target_gas, fee_response_ppm }
 *
 * Read-only. Pressure pill + drift state derived locally from the
 * energy/target_energy ratio. Sparkline omitted on mobile (no native
 * sparkline lib in dependency budget).
 *
 * Polled every 12s while focused so the pressure pill animates with
 * chain activity.
 */

import React, { useCallback, useEffect, useState } from 'react';
import {
  View,
  Text,
  ScrollView,
  StyleSheet,
  ActivityIndicator,
  RefreshControl,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { api } from '../utils/api';
import type { FeeControllerStatus } from '../utils/api';

type Band = 'low' | 'med' | 'high';

const FeeControllerScreen: React.FC = () => {
  const [status, setStatus] = useState<FeeControllerStatus | null>(null);
  const [history, setHistory] = useState<number[]>([]);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      const s = await api.getFeeControllerStatus();
      setStatus(s);
      setHistory((prev) => {
        const next = [...prev, s.base_fee];
        return next.length > 12 ? next.slice(-12) : next;
      });
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
    const id = setInterval(load, 12000);
    return () => clearInterval(id);
  }, [load]);

  const pressure =
    status && status.target_energy > 0 ? status.energy / status.target_energy : null;
  const band: Band = pressure == null ? 'low' : pressure < 0.66 ? 'low' : pressure < 1.33 ? 'med' : 'high';

  // Drift trace from base_fee deltas (sum of last 8 deltas; negative =
  // converging back to target).
  const deltas: number[] = [];
  for (let i = 1; i < history.length; i++) {
    deltas.push(history[i]! - history[i - 1]!);
  }
  const recentDrift = deltas.slice(-8).reduce((a, b) => a + b, 0);
  const converging = recentDrift <= 0;

  if (loading) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator size="large" color="#06b6d4" />
      </View>
    );
  }

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView
        contentContainerStyle={styles.scroll}
        refreshControl={<RefreshControl refreshing={false} onRefresh={load} tintColor="#06b6d4" />}
      >
        <View style={styles.card}>
          <Text style={styles.label}>Base Fee</Text>
          <View style={styles.bigRow}>
            <Text style={styles.bigVal}>
              {status ? status.base_fee.toLocaleString() : '—'}
            </Text>
            <Text style={styles.bigUnit}>wei/gas</Text>
          </View>

          <View style={styles.row}>
            <PressurePill band={band} />
            <DriftIndicator converging={converging} delta={recentDrift} />
          </View>

          {status ? (
            <View style={styles.gridFooter}>
              <Text style={styles.subtle}>
                E {status.energy.toLocaleString()} / target {status.target_energy.toLocaleString()}
              </Text>
              <Text style={styles.subtle}>
                ρ {status.fee_response_ppm.toLocaleString()} ppm
              </Text>
            </View>
          ) : null}
        </View>

        {status ? (
          <View style={styles.card}>
            <Text style={styles.label}>Target & Response</Text>
            <Row k="Target gas" v={status.target_gas.toLocaleString()} />
            <Row k="Target energy" v={status.target_energy.toLocaleString()} />
            <Row k="Current energy" v={status.energy.toLocaleString()} />
            <Row
              k="Pressure"
              v={pressure == null ? '—' : `${(pressure * 100).toFixed(1)}%`}
            />
            <Row k="Response (ppm)" v={status.fee_response_ppm.toLocaleString()} />
          </View>
        ) : null}

        <View style={styles.note}>
          <Text style={styles.noteText}>
            The fee controller minimises a Lyapunov V-function: gas pressure pulls fees back toward
            the target every block. Negative drift = converging to target_gas; positive drift =
            persistent congestion.
          </Text>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
};

const PressurePill: React.FC<{ band: Band }> = ({ band }) => {
  const palette: Record<Band, { bg: string; bd: string; fg: string; label: string }> = {
    low: { bg: '#ecfdf5', bd: '#a7f3d0', fg: '#22c55e', label: 'Low pressure' },
    med: { bg: '#fffbeb', bd: '#fde68a', fg: '#f59e0b', label: 'Med pressure' },
    high: { bg: '#fef2f2', bd: '#fecaca', fg: '#ef4444', label: 'High pressure' },
  };
  const p = palette[band];
  return (
    <View style={[styles.pill, { backgroundColor: p.bg, borderColor: p.bd }]}>
      <Text style={[styles.pillText, { color: p.fg }]}>{p.label}</Text>
    </View>
  );
};

const DriftIndicator: React.FC<{ converging: boolean; delta: number }> = ({
  converging,
  delta,
}) => (
  <View style={styles.driftRow}>
    <Text style={[styles.driftArrow, { color: converging ? '#22c55e' : '#f59e0b' }]}>
      {converging ? '↘' : '↗'}
    </Text>
    <Text style={[styles.driftText, { color: converging ? '#22c55e' : '#f59e0b' }]}>
      {converging ? 'Converging' : 'Diverging'} (Δ{delta.toFixed(0)})
    </Text>
  </View>
);

const Row: React.FC<{ k: string; v: string }> = ({ k, v }) => (
  <View style={styles.kvRow}>
    <Text style={styles.kvKey}>{k}</Text>
    <Text style={styles.kvVal}>{v}</Text>
  </View>
);

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#f9fafb' },
  centered: { flex: 1, alignItems: 'center', justifyContent: 'center', backgroundColor: '#f9fafb' },
  scroll: { padding: 16, paddingBottom: 40 },
  card: {
    backgroundColor: '#ffffff',
    borderRadius: 14,
    padding: 16,
    marginBottom: 12,
    borderWidth: 1,
    borderColor: '#e5e7eb',
  },
  label: {
    fontSize: 11,
    color: '#6b7280',
    fontWeight: '600',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginBottom: 6,
  },
  bigRow: { flexDirection: 'row', alignItems: 'baseline', gap: 6 },
  bigVal: { fontSize: 28, fontWeight: '700', color: '#111827' },
  bigUnit: { fontSize: 12, color: '#9ca3af' },
  row: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginTop: 12 },
  pill: { paddingHorizontal: 10, paddingVertical: 4, borderRadius: 999, borderWidth: 1 },
  pillText: { fontSize: 11, fontWeight: '600' },
  driftRow: { flexDirection: 'row', alignItems: 'center', gap: 4 },
  driftArrow: { fontSize: 16, fontWeight: '700' },
  driftText: { fontSize: 11, fontWeight: '600' },
  gridFooter: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    marginTop: 14,
    paddingTop: 10,
    borderTopWidth: 1,
    borderTopColor: '#f3f4f6',
  },
  subtle: { fontSize: 11, color: '#6b7280' },
  kvRow: { flexDirection: 'row', justifyContent: 'space-between', paddingVertical: 8 },
  kvKey: { fontSize: 12, color: '#6b7280' },
  kvVal: { fontSize: 13, color: '#111827', fontWeight: '600' },
  note: {
    backgroundColor: '#ecfeff',
    borderColor: '#a5f3fc',
    borderWidth: 1,
    borderRadius: 10,
    padding: 12,
  },
  noteText: { fontSize: 11, color: '#0e7490', lineHeight: 16 },
});

export default FeeControllerScreen;
