/**
 * RefreshPoolScreen — Protocol-owned pool funding object refreshes.
 *
 * Mirrors extension/src/components/RefreshPoolScreen.tsx.
 *
 * GET /api/refresh_pool returns api.rs §RefreshPoolResp:
 *   { total_accrued, credits[{ namespace_hex, accrued, last_touched_epoch }] }
 *
 * Read-only. Fetch on mount; pull-to-refresh.
 */

import React, { useCallback, useEffect, useMemo, useState } from 'react';
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
import type { ChainStatus, RefreshPoolStatus } from '../utils/api';

const RefreshPoolScreen: React.FC = () => {
  const [pool, setPool] = useState<RefreshPoolStatus | null>(null);
  const [chain, setChain] = useState<ChainStatus | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      const [p, c] = await Promise.allSettled([api.getRefreshPool(), api.getChainStatus()]);
      if (p.status === 'fulfilled') setPool(p.value);
      if (c.status === 'fulfilled') setChain(c.value);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const sortedCredits = useMemo(() => {
    if (!pool) return [];
    return [...pool.credits].sort((a, b) => b.accrued - a.accrued);
  }, [pool]);

  const latestEpoch = useMemo(() => {
    if (!pool || pool.credits.length === 0) return null;
    return pool.credits.reduce(
      (max, c) => (c.last_touched_epoch > max ? c.last_touched_epoch : max),
      0,
    );
  }, [pool]);

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
          <Text style={styles.label}>Total Pool Balance</Text>
          <View style={styles.bigRow}>
            <Text style={styles.bigVal}>{(pool?.total_accrued ?? 0).toLocaleString()}</Text>
            <Text style={styles.bigUnit}>EVAP</Text>
          </View>
          <View style={styles.statRow}>
            <View style={{ flex: 1 }}>
              <Text style={styles.muted}>Namespaces</Text>
              <Text style={styles.statLine}>{pool ? pool.credits.length : '—'}</Text>
            </View>
            <View style={{ flex: 1 }}>
              <Text style={styles.muted}>Last update</Text>
              <Text style={styles.statLine}>{latestEpoch != null ? `Epoch ${latestEpoch}` : '—'}</Text>
            </View>
          </View>
          {chain ? (
            <Text style={styles.subtle}>Current epoch: {chain.epoch.toLocaleString()}</Text>
          ) : null}
        </View>

        <Text style={styles.section}>Per-namespace credits</Text>
        <View style={styles.tableCard}>
          <View style={styles.tableHeader}>
            <Text style={[styles.colHead, { flex: 6 }]}>Namespace</Text>
            <Text style={[styles.colHead, { flex: 3, textAlign: 'right' }]}>Accrued</Text>
            <Text style={[styles.colHead, { flex: 3, textAlign: 'right' }]}>Epoch</Text>
          </View>
          {sortedCredits.length === 0 ? (
            <View style={styles.empty}>
              <Text style={styles.muted}>No credits accrued yet.</Text>
            </View>
          ) : (
            sortedCredits.map((c) => (
              <View key={c.namespace_hex} style={styles.tableRow}>
                <Text style={[styles.cellMono, { flex: 6 }]} numberOfLines={1}>
                  {c.namespace_hex.slice(0, 18)}…
                </Text>
                <Text style={[styles.cellNum, { flex: 3 }]}>{c.accrued.toLocaleString()}</Text>
                <Text style={[styles.cellMutedNum, { flex: 3 }]}>
                  {c.last_touched_epoch.toLocaleString()}
                </Text>
              </View>
            ))
          )}
        </View>

        <View style={styles.note}>
          <Text style={styles.noteText}>
            The refresh pool is funded by demurrage on idle balances and excess patronage donations.
            Object refreshes draw from this pool at the protocol level — you never pay refresh fees twice.
          </Text>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
};

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
  bigRow: { flexDirection: 'row', alignItems: 'baseline', gap: 6, marginBottom: 12 },
  bigVal: { fontSize: 28, fontWeight: '700', color: '#111827' },
  bigUnit: { fontSize: 12, color: '#9ca3af' },
  statRow: { flexDirection: 'row', gap: 16, marginBottom: 8 },
  muted: { fontSize: 10, color: '#9ca3af', textTransform: 'uppercase', letterSpacing: 0.4 },
  statLine: { fontSize: 14, fontWeight: '600', color: '#111827', marginTop: 2 },
  subtle: { fontSize: 11, color: '#9ca3af' },
  section: {
    fontSize: 11,
    color: '#6b7280',
    fontWeight: '600',
    marginBottom: 8,
    textTransform: 'uppercase',
    letterSpacing: 0.5,
  },
  tableCard: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    overflow: 'hidden',
    marginBottom: 12,
    borderWidth: 1,
    borderColor: '#e5e7eb',
  },
  tableHeader: {
    flexDirection: 'row',
    paddingHorizontal: 12,
    paddingVertical: 8,
    backgroundColor: '#f9fafb',
    borderBottomWidth: 1,
    borderBottomColor: '#e5e7eb',
  },
  colHead: { fontSize: 10, color: '#6b7280', fontWeight: '600', textTransform: 'uppercase', letterSpacing: 0.4 },
  tableRow: {
    flexDirection: 'row',
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderBottomWidth: 1,
    borderBottomColor: '#f3f4f6',
    alignItems: 'center',
  },
  cellMono: { fontSize: 11, fontFamily: 'monospace', color: '#374151' },
  cellNum: { fontSize: 12, fontWeight: '600', color: '#111827', textAlign: 'right' },
  cellMutedNum: { fontSize: 12, color: '#6b7280', textAlign: 'right' },
  empty: { padding: 20, alignItems: 'center' },
  note: {
    backgroundColor: '#ecfeff',
    borderColor: '#a5f3fc',
    borderWidth: 1,
    borderRadius: 10,
    padding: 12,
  },
  noteText: { fontSize: 11, color: '#0e7490', lineHeight: 16 },
});

export default RefreshPoolScreen;
