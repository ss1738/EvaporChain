/**
 * ShardScreen — sharding visibility dashboard.
 *
 * Mirrors `extension/src/components/ShardScreen.tsx`:
 *   - GET /api/shards          → num_shards + pending_cross_shard_messages
 *   - GET /api/shards/health   → per-shard health rows + compaction count
 *
 * Shows a summary card (total / healthy / unhealthy / pending),
 * "Your account is on shard N" callout, and a per-shard list with the
 * "you" chip on the user's home shard.
 *
 * Address → shard is computed locally because the node has no
 * /api/address/:addr/shard endpoint; we mirror
 * `evaporchain_sharding::shard_for_object` in api.computeShardForAddress.
 */

import React, { useCallback, useEffect, useState } from 'react';
import {
  View,
  Text,
  StyleSheet,
  ScrollView,
  TouchableOpacity,
  RefreshControl,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { api } from '../utils/api';
import type { ShardsHealth } from '../utils/api';
import { keystore } from '../utils/keystore';

const ShardScreen: React.FC = () => {
  const [snapshot, setSnapshot] = useState<ShardsHealth | null>(null);
  const [address, setAddress] = useState<string>('');
  const [addressShard, setAddressShard] = useState<number | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const load = useCallback(async () => {
    try {
      const addr = await keystore.getAddress();
      setAddress(addr ?? '');
      const snap = await api.getShardsHealth();
      setSnapshot(snap);
      if (snap.active && addr) {
        const a = api.computeShardForAddress(addr, snap.total_shards);
        setAddressShard(a?.shard_id ?? null);
      } else {
        setAddressShard(null);
      }
    } catch {
      // Endpoints unavailable — keep previous snapshot.
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const onRefresh = useCallback(async () => {
    setRefreshing(true);
    await load();
    setRefreshing(false);
  }, [load]);

  const noSharding = snapshot != null && snapshot.active === false;
  const sortedShards = snapshot
    ? [...snapshot.shards].sort((a, b) => a.shard_id - b.shard_id)
    : [];

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView
        contentContainerStyle={styles.scroll}
        refreshControl={
          <RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor="#06b6d4" />
        }
      >
        <Text style={styles.intro}>
          Per-shard health and your account's home shard.
        </Text>

        {/* Summary */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Sharding Status</Text>
          {noSharding ? (
            <Text style={styles.muted}>
              Sharding is disabled on this node. Single-shard chain.
            </Text>
          ) : !snapshot ? (
            <Text style={styles.muted}>Loading shard status…</Text>
          ) : (
            <>
              <View style={styles.totalRow}>
                <Text style={styles.totalNumber}>{snapshot.total_shards}</Text>
                <Text style={styles.totalLabel}>total shards</Text>
              </View>
              <View style={styles.statRow}>
                <Stat label="Healthy" value={String(snapshot.healthy)} tone="emerald" />
                <Stat
                  label="Unhealthy"
                  value={String(snapshot.unhealthy)}
                  tone={snapshot.unhealthy > 0 ? 'red' : 'default'}
                />
                <Stat
                  label="Pending msgs"
                  value={String(snapshot.pending_cross_shard_messages)}
                  tone="cyan"
                />
              </View>
              {snapshot.compaction_candidates > 0 && (
                <Text style={styles.amber}>
                  {snapshot.compaction_candidates} shard
                  {snapshot.compaction_candidates === 1 ? '' : 's'} flagged for compaction
                </Text>
              )}
            </>
          )}
        </View>

        {/* Your shard */}
        {snapshot?.active && addressShard != null && address.length > 0 && (
          <View style={styles.youCard}>
            <Text style={styles.cardTitle}>Your Account</Text>
            <Text style={styles.youText}>
              Your account is on{' '}
              <Text style={styles.youHome}>shard {addressShard}</Text>.
            </Text>
            <Text style={styles.youAddr} numberOfLines={1}>
              {address}
            </Text>
          </View>
        )}

        {/* Per-shard list */}
        <Text style={styles.sectionLabel}>Per-shard health</Text>
        {!snapshot ? (
          <Text style={styles.muted}>Loading…</Text>
        ) : sortedShards.length === 0 ? (
          <Text style={styles.muted}>
            {noSharding ? 'Single-shard chain — no per-shard rows.' : 'No per-shard data reported yet.'}
          </Text>
        ) : (
          sortedShards.map((s) => {
            const isMine = addressShard === s.shard_id;
            const livenessPct = Math.round(s.liveness_ratio * 100);
            const tone = s.is_dead
              ? { borderColor: '#fca5a5', backgroundColor: '#fef2f2' }
              : isMine
              ? { borderColor: '#67e8f9', backgroundColor: '#ecfeff' }
              : { borderColor: '#e5e7eb', backgroundColor: '#ffffff' };
            const dotColor = s.is_dead
              ? '#dc2626'
              : livenessPct > 50
              ? '#16a34a'
              : '#f59e0b';

            return (
              <View key={s.shard_id} style={[styles.shardRow, tone]}>
                <View style={styles.shardHeader}>
                  <View style={styles.shardLeft}>
                    <View style={[styles.dot, { backgroundColor: dotColor }]} />
                    <Text style={styles.shardName}>Shard {s.shard_id}</Text>
                    {isMine && (
                      <View style={styles.youChip}>
                        <Text style={styles.youChipText}>you</Text>
                      </View>
                    )}
                    {s.is_dead && (
                      <View style={styles.deadChip}>
                        <Text style={styles.deadChipText}>dead</Text>
                      </View>
                    )}
                  </View>
                  <Text style={styles.shardLive}>{livenessPct}% live</Text>
                </View>
                <View style={styles.shardStats}>
                  <ShardStat label="Objects" value={`${s.live_objects}/${s.total_objects}`} />
                  <ShardStat label="Energy" value={s.total_energy.toLocaleString()} />
                  <ShardStat label="State" value={s.is_dead ? 'compaction' : 'active'} />
                </View>
              </View>
            );
          })
        )}

        <View style={styles.footerNote}>
          <Text style={styles.footerNoteText}>
            Shard assignment is deterministic: each 20-byte object id (and account address)
            maps to one shard via its first two bytes. Cross-shard transfers are routed by
            the shard bridge and may take longer to finalise than same-shard ones.
          </Text>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
};

const Stat: React.FC<{
  label: string;
  value: string;
  tone?: 'default' | 'emerald' | 'red' | 'cyan';
}> = ({ label, value, tone = 'default' }) => {
  const color =
    tone === 'emerald'
      ? '#16a34a'
      : tone === 'red'
      ? '#dc2626'
      : tone === 'cyan'
      ? '#06b6d4'
      : '#111827';
  return (
    <View style={styles.statBlock}>
      <Text style={styles.statLabel}>{label}</Text>
      <Text style={[styles.statValue, { color }]}>{value}</Text>
    </View>
  );
};

const ShardStat: React.FC<{ label: string; value: string }> = ({ label, value }) => (
  <View style={styles.statBlock}>
    <Text style={styles.statLabelSmall}>{label}</Text>
    <Text style={styles.statValueSmall} numberOfLines={1}>
      {value}
    </Text>
  </View>
);

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#f9fafb' },
  scroll: { padding: 16, paddingBottom: 40 },
  intro: { fontSize: 12, color: '#6b7280', marginBottom: 12, lineHeight: 16 },
  card: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    borderWidth: 1,
    borderColor: '#e5e7eb',
    padding: 14,
    marginBottom: 12,
  },
  youCard: {
    backgroundColor: '#ecfeff',
    borderRadius: 12,
    borderWidth: 1,
    borderColor: '#a5f3fc',
    padding: 14,
    marginBottom: 12,
  },
  cardTitle: {
    fontSize: 11,
    fontWeight: '700',
    color: '#6b7280',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginBottom: 8,
  },
  totalRow: { flexDirection: 'row', alignItems: 'baseline', gap: 8 },
  totalNumber: { fontSize: 28, fontWeight: '700', color: '#111827' },
  totalLabel: { fontSize: 13, color: '#6b7280' },
  statRow: { flexDirection: 'row', marginTop: 12, gap: 8 },
  statBlock: { flex: 1 },
  statLabel: {
    fontSize: 10,
    color: '#9ca3af',
    textTransform: 'uppercase',
    letterSpacing: 0.4,
  },
  statValue: { fontSize: 16, fontWeight: '700', marginTop: 2 },
  statLabelSmall: {
    fontSize: 10,
    color: '#9ca3af',
    textTransform: 'uppercase',
    letterSpacing: 0.4,
  },
  statValueSmall: {
    fontSize: 12,
    fontWeight: '500',
    color: '#374151',
    marginTop: 2,
  },
  amber: { fontSize: 11, color: '#b45309', marginTop: 8 },
  muted: { fontSize: 12, color: '#9ca3af', textAlign: 'center', paddingVertical: 12 },
  youText: { fontSize: 13, color: '#0e7490' },
  youHome: { fontWeight: '700', color: '#0891b2' },
  youAddr: { fontSize: 11, fontFamily: 'monospace', color: '#9ca3af', marginTop: 4 },
  sectionLabel: {
    fontSize: 11,
    fontWeight: '700',
    color: '#6b7280',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginBottom: 8,
    marginTop: 4,
  },
  shardRow: {
    borderRadius: 12,
    borderWidth: 1,
    padding: 12,
    marginBottom: 8,
  },
  shardHeader: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    marginBottom: 8,
  },
  shardLeft: { flexDirection: 'row', alignItems: 'center', gap: 8 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  shardName: { fontSize: 13, fontWeight: '700', color: '#111827' },
  shardLive: { fontSize: 11, color: '#6b7280' },
  shardStats: { flexDirection: 'row', gap: 8 },
  youChip: {
    paddingHorizontal: 8,
    paddingVertical: 2,
    borderRadius: 999,
    backgroundColor: '#cffafe',
    borderWidth: 1,
    borderColor: '#67e8f9',
  },
  youChipText: { fontSize: 10, color: '#0891b2', fontWeight: '700' },
  deadChip: {
    paddingHorizontal: 8,
    paddingVertical: 2,
    borderRadius: 999,
    backgroundColor: '#fee2e2',
    borderWidth: 1,
    borderColor: '#fca5a5',
  },
  deadChipText: { fontSize: 10, color: '#dc2626', fontWeight: '700' },
  footerNote: {
    marginTop: 12,
    backgroundColor: '#ecfeff',
    borderRadius: 12,
    borderWidth: 1,
    borderColor: '#a5f3fc',
    padding: 12,
  },
  footerNoteText: { fontSize: 11, color: '#0e7490', lineHeight: 16 },
});

export default ShardScreen;
