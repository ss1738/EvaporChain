/**
 * HistoryScreen — Full transaction history with filtering
 */

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  FlatList,
  StyleSheet,
  RefreshControl,
  ActivityIndicator,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { api } from '../utils/api';
import type { Transaction } from '../utils/api';
import { keystore } from '../utils/keystore';

type FilterType = 'all' | 'sent' | 'received';

const FILTERS: { label: string; value: FilterType }[] = [
  { label: 'All', value: 'all' },
  { label: 'Sent', value: 'sent' },
  { label: 'Received', value: 'received' },
];

const HistoryScreen: React.FC = () => {
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [address, setAddress] = useState('');
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<FilterType>('all');

  const loadTransactions = useCallback(async () => {
    try {
      const addr = await keystore.getAddress();
      if (!addr) return;
      setAddress(addr);
      const txns = await api.getTransactions(addr, 50);
      setTransactions(txns);
    } catch {
      // Keep existing
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadTransactions();
  }, [loadTransactions]);

  const filtered = transactions.filter((tx) => {
    if (filter === 'all') return true;
    const isSent = tx.from.toLowerCase() === address.toLowerCase();
    return filter === 'sent' ? isSent : !isSent;
  });

  const formatAddress = (addr: string): string =>
    `${addr.slice(0, 8)}...${addr.slice(-6)}`;

  const formatTime = (timestamp: number): string => {
    const date = new Date(timestamp);
    const now = Date.now();
    const diff = now - timestamp;
    if (diff < 3600000) return `${Math.round(diff / 60000)}m ago`;
    if (diff < 86400000) return `${Math.round(diff / 3600000)}h ago`;
    return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  };

  const formatAmount = (amount: string): string => {
    const num = parseFloat(amount);
    if (isNaN(num)) return '0.00';
    return num.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 4 });
  };

  const renderTransaction = ({ item }: { item: Transaction }) => {
    const isSent = item.from.toLowerCase() === address.toLowerCase();
    return (
      <View style={styles.txRow}>
        <View style={styles.txIcon}>
          <Text style={[styles.txIconText, { color: isSent ? '#ef4444' : '#22c55e' }]}>
            {isSent ? 'S' : 'R'}
          </Text>
        </View>
        <View style={styles.txContent}>
          <View style={styles.txTopRow}>
            <Text style={styles.txType}>{isSent ? 'Sent' : 'Received'}</Text>
            <Text style={[styles.txAmount, { color: isSent ? '#ef4444' : '#22c55e' }]}>
              {isSent ? '-' : '+'}{formatAmount(item.amount)} EVAP
            </Text>
          </View>
          <View style={styles.txBottomRow}>
            <Text style={styles.txAddress}>
              {isSent ? 'To ' : 'From '}
              {formatAddress(isSent ? item.to : item.from)}
            </Text>
            <Text style={styles.txTime}>{formatTime(item.timestamp)}</Text>
          </View>
          {item.type !== 'Transfer' && (
            <Text style={styles.txTypeBadge}>{item.type}</Text>
          )}
        </View>
      </View>
    );
  };

  if (loading) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator size="large" color="#06b6d4" />
      </View>
    );
  }

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      {/* Filter Tabs */}
      <View style={styles.filterRow}>
        {FILTERS.map((f) => (
          <TouchableOpacity
            key={f.value}
            style={[styles.filterTab, filter === f.value && styles.filterTabActive]}
            onPress={() => setFilter(f.value)}
            activeOpacity={0.7}
          >
            <Text style={[styles.filterText, filter === f.value && styles.filterTextActive]}>
              {f.label}
            </Text>
          </TouchableOpacity>
        ))}
      </View>

      <FlatList
        data={filtered}
        keyExtractor={(item) => item.hash}
        renderItem={renderTransaction}
        contentContainerStyle={styles.list}
        refreshControl={
          <RefreshControl refreshing={false} onRefresh={loadTransactions} tintColor="#06b6d4" />
        }
        ListEmptyComponent={
          <View style={styles.emptyState}>
            <Text style={styles.emptyTitle}>No Transactions</Text>
            <Text style={styles.emptySubtext}>
              {filter === 'all'
                ? 'Your transaction history will appear here.'
                : `No ${filter} transactions found.`}
            </Text>
          </View>
        }
      />
    </SafeAreaView>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#f9fafb',
  },
  centered: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: '#f9fafb',
  },
  filterRow: {
    flexDirection: 'row',
    paddingHorizontal: 16,
    paddingVertical: 12,
    gap: 8,
  },
  filterTab: {
    flex: 1,
    paddingVertical: 10,
    borderRadius: 10,
    backgroundColor: '#ffffff',
    alignItems: 'center',
    borderWidth: 1,
    borderColor: '#e5e7eb',
  },
  filterTabActive: {
    backgroundColor: '#06b6d4',
    borderColor: '#06b6d4',
  },
  filterText: {
    fontSize: 14,
    fontWeight: '600',
    color: '#6b7280',
  },
  filterTextActive: {
    color: '#ffffff',
  },
  list: {
    paddingHorizontal: 16,
    paddingBottom: 32,
  },
  txRow: {
    flexDirection: 'row',
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 14,
    marginBottom: 8,
    alignItems: 'center',
  },
  txIcon: {
    width: 40,
    height: 40,
    borderRadius: 12,
    backgroundColor: '#f3f4f6',
    alignItems: 'center',
    justifyContent: 'center',
    marginRight: 12,
  },
  txIconText: {
    fontSize: 16,
    fontWeight: '700',
  },
  txContent: {
    flex: 1,
  },
  txTopRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
  },
  txType: {
    fontSize: 15,
    fontWeight: '600',
    color: '#111827',
  },
  txAmount: {
    fontSize: 15,
    fontWeight: '600',
  },
  txBottomRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginTop: 4,
  },
  txAddress: {
    fontSize: 12,
    color: '#9ca3af',
  },
  txTime: {
    fontSize: 12,
    color: '#9ca3af',
  },
  txTypeBadge: {
    fontSize: 11,
    color: '#6b7280',
    backgroundColor: '#f3f4f6',
    paddingHorizontal: 8,
    paddingVertical: 2,
    borderRadius: 6,
    alignSelf: 'flex-start',
    marginTop: 6,
    fontWeight: '500',
    overflow: 'hidden',
  },
  emptyState: {
    alignItems: 'center',
    paddingVertical: 60,
  },
  emptyTitle: {
    fontSize: 18,
    fontWeight: '600',
    color: '#374151',
    marginBottom: 8,
  },
  emptySubtext: {
    fontSize: 14,
    color: '#9ca3af',
    textAlign: 'center',
    lineHeight: 20,
    paddingHorizontal: 32,
  },
});

export default HistoryScreen;
