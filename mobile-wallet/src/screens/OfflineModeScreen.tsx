/**
 * OfflineModeScreen — Offline transaction queueing and sync
 *
 * When the device has no network:
 *   1. User builds transactions normally.
 *   2. Transactions are serialised and queued in AsyncStorage.
 *   3. On reconnection, the queue drains in submission order.
 *
 * This screen shows the queue, lets the user remove items, and
 * manually trigger a sync attempt.
 */

import React, { useState, useCallback, useEffect } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  Alert,
  ActivityIndicator,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import AsyncStorage from '@react-native-async-storage/async-storage';
import NetInfo from '@react-native-community/netinfo';
import { api } from '../utils/api';
import type { TxResult } from '../utils/api';

const QUEUE_KEY = 'evaporchain_offline_queue';

export type OfflineTxType = 'transfer' | 'refresh' | 'stake' | 'unstake';

export interface QueuedTransaction {
  id: string;
  type: OfflineTxType;
  params: Record<string, unknown>;
  createdAt: number;
  status: 'pending' | 'submitting' | 'success' | 'failed';
  error?: string;
}

// ── Queue persistence helpers ──

async function loadQueue(): Promise<QueuedTransaction[]> {
  try {
    const raw = await AsyncStorage.getItem(QUEUE_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

async function saveQueue(queue: QueuedTransaction[]): Promise<void> {
  await AsyncStorage.setItem(QUEUE_KEY, JSON.stringify(queue));
}

export async function enqueueTransaction(tx: Omit<QueuedTransaction, 'id' | 'status' | 'createdAt'>): Promise<void> {
  const queue = await loadQueue();
  queue.push({
    ...tx,
    id: `qtx-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
    createdAt: Date.now(),
    status: 'pending',
  });
  await saveQueue(queue);
}

async function submitQueuedTx(tx: QueuedTransaction): Promise<TxResult> {
  switch (tx.type) {
    case 'transfer': {
      const p = tx.params as { from: string; to: string; amount: number; nonce: number };
      return api.transfer(p.from, p.to, p.amount, p.nonce);
    }
    case 'refresh': {
      const p = tx.params as { objectId: string; energyDeposit: number };
      return api.refreshObject(p.objectId, p.energyDeposit);
    }
    case 'stake': {
      const p = tx.params as { from: string; amount: number; nonce: number };
      return api.stake(p.from, p.amount, p.nonce);
    }
    case 'unstake': {
      const p = tx.params as { from: string; amount: number; nonce: number };
      return api.unstake(p.from, p.amount, p.nonce);
    }
    default:
      throw new Error(`Unknown queued tx type: ${tx.type}`);
  }
}

// ── Screen ──

const OfflineModeScreen: React.FC = () => {
  const [queue, setQueue] = useState<QueuedTransaction[]>([]);
  const [isOnline, setIsOnline] = useState<boolean | null>(null);
  const [syncing, setSyncing] = useState(false);

  const refresh = useCallback(async () => {
    const q = await loadQueue();
    setQueue(q);
  }, []);

  useEffect(() => {
    refresh();

    const unsub = NetInfo.addEventListener((state) => {
      setIsOnline(state.isConnected ?? false);
    });
    NetInfo.fetch().then((state) => setIsOnline(state.isConnected ?? false));

    return () => unsub();
  }, [refresh]);

  const handleSync = async () => {
    if (!isOnline) {
      Alert.alert('Offline', 'Cannot sync while offline. Please restore your network connection.');
      return;
    }

    setSyncing(true);
    const q = await loadQueue();
    const updated = [...q];

    for (let i = 0; i < updated.length; i++) {
      if (updated[i].status !== 'pending') continue;
      updated[i] = { ...updated[i], status: 'submitting' };
      setQueue([...updated]);

      try {
        const result = await submitQueuedTx(updated[i]);
        updated[i] = {
          ...updated[i],
          status: result.success ? 'success' : 'failed',
          error: result.success ? undefined : result.message,
        };
      } catch (err: any) {
        updated[i] = { ...updated[i], status: 'failed', error: err.message ?? 'Submission failed' };
      }

      setQueue([...updated]);
      await saveQueue(updated);
    }

    setSyncing(false);
  };

  const handleRemove = (id: string) => {
    Alert.alert(
      'Remove Transaction',
      'Remove this transaction from the queue?',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Remove', style: 'destructive',
          onPress: async () => {
            const updated = queue.filter((tx) => tx.id !== id);
            setQueue(updated);
            await saveQueue(updated);
          },
        },
      ]
    );
  };

  const handleClearDone = async () => {
    const updated = queue.filter((tx) => tx.status === 'pending' || tx.status === 'submitting');
    setQueue(updated);
    await saveQueue(updated);
  };

  const pendingCount = queue.filter((tx) => tx.status === 'pending').length;
  const doneCount = queue.filter((tx) => tx.status === 'success' || tx.status === 'failed').length;

  const formatTime = (ts: number) => {
    const d = new Date(ts);
    return d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  };

  const txTypeLabel: Record<OfflineTxType, string> = {
    transfer: 'Transfer',
    refresh: 'Refresh Object',
    stake: 'Stake',
    unstake: 'Unstake',
  };

  const statusColor: Record<QueuedTransaction['status'], string> = {
    pending: '#f59e0b',
    submitting: '#3b82f6',
    success: '#22c55e',
    failed: '#ef4444',
  };

  return (
    <SafeAreaView style={styles.safe} edges={['bottom']}>
      {/* Network banner */}
      <View style={[styles.netBanner, isOnline ? styles.netOnline : styles.netOffline]}>
        <View style={[styles.netDot, { backgroundColor: isOnline ? '#22c55e' : '#ef4444' }]} />
        <Text style={[styles.netLabel, { color: isOnline ? '#166534' : '#991b1b' }]}>
          {isOnline === null ? 'Checking connection…' : isOnline ? 'Online' : 'Offline — transactions will be queued'}
        </Text>
      </View>

      {/* Summary row */}
      <View style={styles.summaryRow}>
        <View style={styles.summaryItem}>
          <Text style={styles.summaryValue}>{queue.length}</Text>
          <Text style={styles.summaryLabel}>Total</Text>
        </View>
        <View style={styles.summaryItem}>
          <Text style={[styles.summaryValue, { color: '#f59e0b' }]}>{pendingCount}</Text>
          <Text style={styles.summaryLabel}>Pending</Text>
        </View>
        <View style={styles.summaryItem}>
          <Text style={[styles.summaryValue, { color: '#22c55e' }]}>
            {queue.filter((t) => t.status === 'success').length}
          </Text>
          <Text style={styles.summaryLabel}>Sent</Text>
        </View>
        <View style={styles.summaryItem}>
          <Text style={[styles.summaryValue, { color: '#ef4444' }]}>
            {queue.filter((t) => t.status === 'failed').length}
          </Text>
          <Text style={styles.summaryLabel}>Failed</Text>
        </View>
      </View>

      {/* Action buttons */}
      <View style={styles.actionRow}>
        <TouchableOpacity
          style={[styles.syncBtn, (!isOnline || pendingCount === 0 || syncing) && styles.btnDisabled]}
          onPress={handleSync}
          disabled={!isOnline || pendingCount === 0 || syncing}
        >
          {syncing
            ? <ActivityIndicator color="#fff" size="small" />
            : <Text style={styles.syncBtnText}>Sync Now ({pendingCount})</Text>
          }
        </TouchableOpacity>
        {doneCount > 0 && (
          <TouchableOpacity style={styles.clearBtn} onPress={handleClearDone}>
            <Text style={styles.clearBtnText}>Clear Done</Text>
          </TouchableOpacity>
        )}
      </View>

      {/* Queue list */}
      <ScrollView style={styles.list} contentContainerStyle={styles.listContent}>
        {queue.length === 0 ? (
          <View style={styles.emptyBlock}>
            <Text style={styles.emptyIcon}>📭</Text>
            <Text style={styles.emptyTitle}>Queue is Empty</Text>
            <Text style={styles.emptyText}>
              Transactions made while offline will appear here and sync automatically when you reconnect.
            </Text>
          </View>
        ) : (
          queue.map((tx) => (
            <View key={tx.id} style={styles.txCard}>
              <View style={styles.txHeader}>
                <View style={[styles.txTypeBadge, { backgroundColor: statusColor[tx.status] + '20' }]}>
                  <Text style={[styles.txTypeText, { color: statusColor[tx.status] }]}>
                    {txTypeLabel[tx.type]}
                  </Text>
                </View>
                <View style={[styles.statusDot, { backgroundColor: statusColor[tx.status] }]} />
                <Text style={[styles.statusLabel, { color: statusColor[tx.status] }]}>
                  {tx.status === 'submitting' ? 'Submitting…' : tx.status.charAt(0).toUpperCase() + tx.status.slice(1)}
                </Text>
                {tx.status === 'pending' && (
                  <TouchableOpacity style={styles.removeBtn} onPress={() => handleRemove(tx.id)}>
                    <Text style={styles.removeBtnText}>✕</Text>
                  </TouchableOpacity>
                )}
              </View>

              <Text style={styles.txTime}>{formatTime(tx.createdAt)}</Text>

              {/* Params preview */}
              {tx.type === 'transfer' && (
                <Text style={styles.txDetail}>
                  → {String(tx.params.to).slice(0, 12)}… · {String(tx.params.amount)} EVAP
                </Text>
              )}
              {tx.type === 'refresh' && (
                <Text style={styles.txDetail}>
                  Object {String(tx.params.objectId).slice(0, 10)}… · +{String(tx.params.energyDeposit)} energy
                </Text>
              )}
              {(tx.type === 'stake' || tx.type === 'unstake') && (
                <Text style={styles.txDetail}>{String(tx.params.amount)} EVAP</Text>
              )}

              {tx.error && (
                <Text style={styles.txError}>{tx.error}</Text>
              )}

              {tx.status === 'submitting' && (
                <View style={styles.submittingRow}>
                  <ActivityIndicator size="small" color="#3b82f6" />
                  <Text style={styles.submittingText}>Submitting to network…</Text>
                </View>
              )}
            </View>
          ))
        )}
      </ScrollView>
    </SafeAreaView>
  );
};

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: '#f9fafb' },
  netBanner: {
    flexDirection: 'row', alignItems: 'center', gap: 8,
    paddingHorizontal: 14, paddingVertical: 10,
    borderBottomWidth: 1,
  },
  netOnline: { backgroundColor: '#f0fdf4', borderBottomColor: '#bbf7d0' },
  netOffline: { backgroundColor: '#fef2f2', borderBottomColor: '#fecaca' },
  netDot: { width: 8, height: 8, borderRadius: 4 },
  netLabel: { fontSize: 13, fontWeight: '600' },
  summaryRow: {
    flexDirection: 'row',
    backgroundColor: '#fff',
    borderBottomWidth: 1, borderBottomColor: '#e5e7eb',
    paddingVertical: 12,
  },
  summaryItem: { flex: 1, alignItems: 'center' },
  summaryValue: { fontSize: 20, fontWeight: '800', color: '#111827' },
  summaryLabel: { fontSize: 11, color: '#9ca3af', marginTop: 2 },
  actionRow: {
    flexDirection: 'row', gap: 10,
    padding: 12, backgroundColor: '#fff',
    borderBottomWidth: 1, borderBottomColor: '#e5e7eb',
  },
  syncBtn: {
    flex: 1, backgroundColor: '#06b6d4',
    borderRadius: 10, paddingVertical: 11, alignItems: 'center',
  },
  syncBtnText: { color: '#fff', fontSize: 14, fontWeight: '700' },
  clearBtn: {
    paddingVertical: 11, paddingHorizontal: 16,
    borderRadius: 10, borderWidth: 1, borderColor: '#e5e7eb',
    alignItems: 'center',
  },
  clearBtnText: { fontSize: 13, color: '#6b7280', fontWeight: '600' },
  btnDisabled: { opacity: 0.4 },
  list: { flex: 1 },
  listContent: { padding: 12, gap: 10, paddingBottom: 32 },
  emptyBlock: { alignItems: 'center', paddingTop: 48, gap: 8 },
  emptyIcon: { fontSize: 40 },
  emptyTitle: { fontSize: 15, fontWeight: '700', color: '#374151' },
  emptyText: { fontSize: 13, color: '#9ca3af', textAlign: 'center', maxWidth: 260, lineHeight: 20 },
  txCard: {
    backgroundColor: '#fff', borderWidth: 1, borderColor: '#e5e7eb',
    borderRadius: 14, padding: 14, gap: 6,
  },
  txHeader: { flexDirection: 'row', alignItems: 'center', gap: 8 },
  txTypeBadge: { paddingHorizontal: 8, paddingVertical: 3, borderRadius: 6 },
  txTypeText: { fontSize: 12, fontWeight: '700' },
  statusDot: { width: 6, height: 6, borderRadius: 3 },
  statusLabel: { fontSize: 12, fontWeight: '600', flex: 1 },
  removeBtn: {
    width: 24, height: 24, borderRadius: 12,
    backgroundColor: '#f3f4f6', alignItems: 'center', justifyContent: 'center',
  },
  removeBtnText: { fontSize: 10, color: '#6b7280', fontWeight: '700' },
  txTime: { fontSize: 11, color: '#9ca3af' },
  txDetail: { fontSize: 13, color: '#374151', fontFamily: 'monospace' },
  txError: { fontSize: 12, color: '#ef4444', backgroundColor: '#fef2f2', padding: 6, borderRadius: 6 },
  submittingRow: { flexDirection: 'row', alignItems: 'center', gap: 8 },
  submittingText: { fontSize: 12, color: '#3b82f6' },
});

export default OfflineModeScreen;
