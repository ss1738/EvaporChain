/**
 * DsnScreen — Decay-Stamped Nullifiers privacy window.
 *
 * Mirrors extension/src/components/DsnBadge.tsx + DsnDetailsScreen.
 *
 *   GET /api/dsn/status — { status, total_count, aggregate_root_hex }
 *
 * Read-only; no signing. Polls every 10s while focused so the privacy
 * set count tracks chain progress.
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
import type { ChainStatus, DsnStatus } from '../utils/api';

const DsnScreen: React.FC = () => {
  const [dsn, setDsn] = useState<DsnStatus | null>(null);
  const [chain, setChain] = useState<ChainStatus | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      const [d, c] = await Promise.allSettled([api.getDsnStatus(), api.getChainStatus()]);
      if (d.status === 'fulfilled') setDsn(d.value);
      if (c.status === 'fulfilled') setChain(c.value);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
    const id = setInterval(load, 10000);
    return () => clearInterval(id);
  }, [load]);

  if (loading) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator size="large" color="#06b6d4" />
      </View>
    );
  }

  const total = dsn?.total_count ?? 0;
  const root = dsn?.aggregate_root_hex ?? '—';
  const shortRoot = root && root !== '—' ? `${root.slice(0, 12)}…${root.slice(-10)}` : '—';
  const isLargeSet = total > 100;

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView
        contentContainerStyle={styles.scroll}
        refreshControl={<RefreshControl refreshing={false} onRefresh={load} tintColor="#06b6d4" />}
      >
        <View style={styles.card}>
          <View style={styles.headerRow}>
            <View style={[styles.badge, isLargeSet && styles.badgePurple]}>
              <Text style={[styles.badgeText, isLargeSet && { color: '#8b5cf6' }]}>DSN</Text>
            </View>
            <Text style={styles.subtleHeader}>Decay-Stamped Nullifiers</Text>
          </View>
          <Text style={styles.label}>Anonymity Set</Text>
          <View style={styles.bigRow}>
            <Text style={styles.bigVal}>{total.toLocaleString()}</Text>
            <Text style={styles.bigUnit}>nullifiers</Text>
          </View>
          <Text style={styles.subtle}>
            Every shielded transfer in this window is indistinguishable from any other within the
            set.
          </Text>
        </View>

        <View style={styles.card}>
          <Text style={styles.label}>Aggregate Root (blake3)</Text>
          <Text style={styles.shortRoot}>{shortRoot}</Text>
          <Text style={styles.fullRoot} selectable>
            {root}
          </Text>
          <Text style={styles.subtle}>
            Two wallets seeing the same window will compute the same root. Use this to verify
            state agreement out-of-band.
          </Text>
        </View>

        <View style={styles.card}>
          <View style={styles.kvRow}>
            <Text style={styles.kvKey}>Chain epoch</Text>
            <Text style={styles.kvVal}>{chain?.epoch ?? 0}</Text>
          </View>
          <Text style={styles.subtle}>
            The window slides forward via /api/dsn/advance_window — advancing drops the oldest
            accumulator slot.
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
  headerRow: { flexDirection: 'row', alignItems: 'center', gap: 8, marginBottom: 8 },
  badge: {
    paddingHorizontal: 8,
    paddingVertical: 2,
    borderRadius: 999,
    backgroundColor: '#f3f4f6',
    borderWidth: 1,
    borderColor: '#e5e7eb',
  },
  badgePurple: { backgroundColor: '#f5f3ff', borderColor: '#c4b5fd' },
  badgeText: { fontSize: 10, fontWeight: '700', color: '#6b7280' },
  subtleHeader: { fontSize: 12, color: '#6b7280', fontWeight: '600' },
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
  subtle: { fontSize: 11, color: '#6b7280', marginTop: 6, lineHeight: 15 },
  shortRoot: { fontFamily: 'monospace', fontSize: 13, color: '#111827', fontWeight: '600' },
  fullRoot: {
    fontFamily: 'monospace',
    fontSize: 11,
    color: '#374151',
    marginTop: 6,
    lineHeight: 15,
  },
  kvRow: { flexDirection: 'row', justifyContent: 'space-between', paddingVertical: 4 },
  kvKey: { fontSize: 12, color: '#6b7280' },
  kvVal: { fontSize: 13, color: '#111827', fontWeight: '600', fontFamily: 'monospace' },
});

export default DsnScreen;
