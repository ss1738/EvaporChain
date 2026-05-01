/**
 * BellBeaconScreen — CHSH quantum-randomness certification.
 *
 * Mirrors extension/src/components/BellBeaconCard.tsx.
 *
 *   GET /api/bell/latest — most recent measured S, anchored to a
 *     block_height/epoch. Returns status:"no_data" until consensus
 *     persists per-block S.
 *
 * Read-only; no signing. Polls every 8s while focused so the S-value
 * tracks chain progress.
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
import type { BellBeaconLatestResp } from '../utils/api';

const BellBeaconScreen: React.FC = () => {
  const [latest, setLatest] = useState<BellBeaconLatestResp | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      const r = await api.getBellBeaconLatest();
      setLatest(r);
    } catch {
      /* keep stale */
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
    const id = setInterval(load, 8000);
    return () => clearInterval(id);
  }, [load]);

  if (loading) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator size="large" color="#06b6d4" />
      </View>
    );
  }

  const s = latest?.s_value_milli ?? 0;
  const threshold = latest?.threshold_milli ?? 2000;
  const certified = latest?.bell_certified === true;
  const noData = latest?.status === 'no_data';
  const sDecimal = (s / 1000).toFixed(3);
  const thresholdDecimal = (threshold / 1000).toFixed(2);
  // Bar covers 0 → 2.83 (Tsirelson). Threshold line at 2/2.83 ≈ 70.7%.
  const bandMax = 2828;
  const fillPct = Math.max(0, Math.min(100, (s / bandMax) * 100));
  const thresholdPct = (threshold / bandMax) * 100;

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView
        contentContainerStyle={styles.scroll}
        refreshControl={<RefreshControl refreshing={false} onRefresh={load} tintColor="#06b6d4" />}
      >
        <View style={styles.card}>
          <View style={styles.headerRow}>
            <View style={styles.bellPill}>
              <Text style={styles.bellPillText}>Bell</Text>
            </View>
            <Text style={styles.subtleHeader}>Beacon health</Text>
            {!noData ? (
              certified ? (
                <View style={styles.certPillEmerald}>
                  <Text style={styles.certPillEmeraldText}>Certified</Text>
                </View>
              ) : (
                <View style={styles.certPillAmber}>
                  <Text style={styles.certPillAmberText}>Local-realism</Text>
                </View>
              )
            ) : null}
          </View>

          <View style={styles.bigRow}>
            <Text style={styles.bigVal}>{noData ? '—' : sDecimal}</Text>
            <Text style={styles.bigUnit}>S-value</Text>
          </View>

          {/* Threshold bar */}
          <View style={styles.barWrap}>
            <View style={styles.barBg}>
              <View
                style={[
                  styles.barFill,
                  {
                    width: `${fillPct}%`,
                    backgroundColor: certified ? '#22c55e' : '#f59e0b',
                  },
                ]}
              />
              <View style={[styles.thresholdLine, { left: `${thresholdPct}%` }]} />
            </View>
            <View style={styles.barLabels}>
              <Text style={styles.barLabel}>0</Text>
              <Text style={[styles.barLabel, { color: '#8b5cf6' }]}>
                |S|=2 ({thresholdDecimal})
              </Text>
              <Text style={styles.barLabel}>2.83</Text>
            </View>
          </View>

          <Text style={styles.note}>
            CHSH bound: |S| ≤ 2 for any local-realist source. S {'>'} 2 certifies
            quantum-mechanical correlation in the beacon.
          </Text>
          {noData ? (
            <View style={styles.warnPill}>
              <Text style={styles.warnPillText}>
                No live measurement yet — consensus has not persisted per-block S.
              </Text>
            </View>
          ) : null}
        </View>

        {latest && !noData ? (
          <View style={styles.card}>
            <Text style={styles.label}>Anchored measurement</Text>
            <Row k="Block height" v={String(latest.block_height)} />
            <Row k="Epoch" v={String(latest.epoch)} />
            <Row k="S (milli)" v={latest.s_value_milli.toLocaleString()} />
            <Row k="Threshold (milli)" v={latest.threshold_milli.toLocaleString()} />
            <Row k="Bell certified" v={latest.bell_certified ? 'Yes' : 'No'} />
            {latest.detail ? <Text style={styles.detail}>{latest.detail}</Text> : null}
          </View>
        ) : null}
      </ScrollView>
    </SafeAreaView>
  );
};

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
  headerRow: { flexDirection: 'row', alignItems: 'center', gap: 8, marginBottom: 12 },
  bellPill: {
    backgroundColor: '#ecfeff',
    borderColor: '#a5f3fc',
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 8,
    paddingVertical: 2,
  },
  bellPillText: { color: '#0e7490', fontSize: 10, fontWeight: '700' },
  subtleHeader: { fontSize: 12, color: '#6b7280', fontWeight: '600', flex: 1 },
  certPillEmerald: {
    backgroundColor: '#ecfdf5',
    borderColor: '#a7f3d0',
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 8,
    paddingVertical: 2,
  },
  certPillEmeraldText: { color: '#22c55e', fontSize: 10, fontWeight: '700' },
  certPillAmber: {
    backgroundColor: '#fffbeb',
    borderColor: '#fde68a',
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 8,
    paddingVertical: 2,
  },
  certPillAmberText: { color: '#f59e0b', fontSize: 10, fontWeight: '700' },
  bigRow: { flexDirection: 'row', alignItems: 'baseline', gap: 6, marginBottom: 12 },
  bigVal: { fontSize: 32, fontWeight: '700', color: '#111827' },
  bigUnit: { fontSize: 12, color: '#9ca3af' },
  barWrap: { marginBottom: 12 },
  barBg: {
    height: 8,
    backgroundColor: '#f3f4f6',
    borderRadius: 999,
    overflow: 'hidden',
    position: 'relative',
  },
  barFill: { height: '100%', borderRadius: 999 },
  thresholdLine: {
    position: 'absolute',
    top: 0,
    bottom: 0,
    width: 2,
    backgroundColor: '#8b5cf6',
  },
  barLabels: { flexDirection: 'row', justifyContent: 'space-between', marginTop: 4 },
  barLabel: { fontSize: 10, color: '#9ca3af' },
  note: { fontSize: 11, color: '#6b7280', lineHeight: 15 },
  warnPill: {
    marginTop: 10,
    backgroundColor: '#fffbeb',
    borderColor: '#fde68a',
    borderWidth: 1,
    borderRadius: 8,
    paddingHorizontal: 10,
    paddingVertical: 6,
  },
  warnPillText: { color: '#a16207', fontSize: 11 },
  label: {
    fontSize: 11,
    color: '#6b7280',
    fontWeight: '600',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginBottom: 6,
  },
  kvRow: { flexDirection: 'row', justifyContent: 'space-between', paddingVertical: 4 },
  kvKey: { fontSize: 12, color: '#6b7280' },
  kvVal: { fontSize: 13, color: '#111827', fontWeight: '600', fontFamily: 'monospace' },
  detail: { fontSize: 11, color: '#9ca3af', marginTop: 8, lineHeight: 15 },
});

export default BellBeaconScreen;
