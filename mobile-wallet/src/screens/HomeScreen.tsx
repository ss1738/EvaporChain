/**
 * HomeScreen — Main wallet dashboard
 *
 * Shows balance, quick actions, recent transactions, chain status,
 * and a decay alert banner when assets are at risk.
 */

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  RefreshControl,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { api } from '../utils/api';
import type { Balance, ChainStatus, Transaction, ChainObject, NFT, ShardsHealth } from '../utils/api';
import { keystore } from '../utils/keystore';
import { DecayAlert } from '../components/DecayAlert';
import { usePendingTxs } from '../state/txStore';

type Props = {
  navigation: NativeStackNavigationProp<RootStackParamList, 'Home'>;
};

const QUICK_ACTIONS = [
  { label: 'Send', screen: 'Send' as const, color: '#06b6d4', icon: 'S' },
  { label: 'Receive', screen: 'Receive' as const, color: '#8b5cf6', icon: 'R' },
  { label: 'Objects', screen: 'Objects' as const, color: '#f59e0b', icon: 'O' },
  { label: 'NFTs', screen: 'NFTs' as const, color: '#ef4444', icon: 'N' },
  { label: 'Stake', screen: 'Staking' as const, color: '#22c55e', icon: 'V' },
  { label: 'Substrate', screen: 'SubstrateHub' as const, color: '#0e7490', icon: 'X' },
];

const HomeScreen: React.FC<Props> = ({ navigation }) => {
  const [balance, setBalance] = useState<Balance | null>(null);
  const [chainStatus, setChainStatus] = useState<ChainStatus | null>(null);
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [objects, setObjects] = useState<ChainObject[]>([]);
  const [nfts, setNfts] = useState<NFT[]>([]);
  const [refreshing, setRefreshing] = useState(false);
  const [shardsHealth, setShardsHealth] = useState<ShardsHealth | null>(null);
  const [addressShard, setAddressShard] = useState<number | null>(null);
  const pendingTxs = usePendingTxs();
  const inflight = pendingTxs.filter(
    (t) => t.status !== 'finalised' && t.status !== 'rejected',
  ).length;

  const loadData = useCallback(async () => {
    try {
      const address = await keystore.getAddress();
      if (!address) return;

      const [bal, status, txns, objs, nftList, shards] = await Promise.allSettled([
        api.getBalance(address),
        api.getChainStatus(),
        api.getTransactions(address, 5),
        api.getObjects(address),
        api.getNFTs(address),
        api.getShardsHealth(),
      ]);

      if (bal.status === 'fulfilled') setBalance(bal.value);
      if (status.status === 'fulfilled') setChainStatus(status.value);
      if (txns.status === 'fulfilled') setTransactions(txns.value);
      if (objs.status === 'fulfilled') setObjects(objs.value);
      if (nftList.status === 'fulfilled') setNfts(nftList.value);
      if (shards.status === 'fulfilled') {
        setShardsHealth(shards.value);
        if (shards.value.active) {
          const a = api.computeShardForAddress(address, shards.value.total_shards);
          setAddressShard(a?.shard_id ?? null);
        } else {
          setAddressShard(null);
        }
      }
    } catch {
      // Silently handle — user sees stale data
    }
  }, []);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const onRefresh = useCallback(async () => {
    setRefreshing(true);
    await loadData();
    setRefreshing(false);
  }, [loadData]);

  const formatAmount = (amount: string): string => {
    const num = parseFloat(amount);
    if (isNaN(num)) return '0.00';
    return num.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 4 });
  };

  const formatAddress = (addr: string): string =>
    `${addr.slice(0, 8)}...${addr.slice(-6)}`;

  const formatTime = (timestamp: number): string => {
    const date = new Date(timestamp);
    const now = Date.now();
    const diff = now - timestamp;
    if (diff < 3600000) return `${Math.round(diff / 60000)}m ago`;
    if (diff < 86400000) return `${Math.round(diff / 3600000)}h ago`;
    return date.toLocaleDateString();
  };

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView
        contentContainerStyle={styles.scroll}
        refreshControl={
          <RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor="#06b6d4" />
        }
      >
        {/* Balance Card */}
        <View style={styles.balanceCard} testID="home-balance-card">
          <Text style={styles.balanceLabel}>Total Balance</Text>
          <Text style={styles.balanceAmount} testID="home-balance-amount">
            {balance ? formatAmount(String(balance.balance)) : '---'} EVAP
          </Text>
          <Text style={styles.balanceUsd}>
            {balance ? `~ $${(balance.balance * 0.042).toFixed(2)} USD` : ''}
          </Text>
          {/* Status pills: pending-tx counter + shard pill. Hidden when
              there's nothing to show (no inflight + single-shard chain). */}
          {(inflight > 0 ||
            (shardsHealth?.active &&
              shardsHealth.total_shards > 1 &&
              addressShard != null)) && (
            <View style={styles.statusRow}>
              {inflight > 0 && (
                <TouchableOpacity
                  testID="home-pending-pill"
                  style={styles.pendingPill}
                  onPress={() => navigation.navigate('History')}
                  activeOpacity={0.7}
                >
                  <View style={styles.pendingDot} />
                  <Text style={styles.pendingText}>
                    {inflight} pending
                  </Text>
                </TouchableOpacity>
              )}
              {shardsHealth?.active &&
                shardsHealth.total_shards > 1 &&
                addressShard != null && (
                  <TouchableOpacity
                    style={styles.shardPill}
                    onPress={() => navigation.navigate('Shards')}
                    activeOpacity={0.7}
                  >
                    <Text style={styles.shardPillText}>Shard {addressShard}</Text>
                  </TouchableOpacity>
                )}
            </View>
          )}
        </View>

        {/* Decay Alert */}
        <DecayAlert
          objects={objects}
          nfts={nfts}
          onPressObject={() => navigation.navigate('Objects')}
          onPressNft={() => navigation.navigate('NFTs')}
        />

        {/* Quick Actions */}
        <View style={styles.quickActions}>
          {QUICK_ACTIONS.map((action) => (
            <TouchableOpacity
              key={action.label}
              testID={`home-quick-action-${action.label.toLowerCase()}`}
              style={styles.quickAction}
              onPress={() => navigation.navigate(action.screen)}
              activeOpacity={0.7}
            >
              <View style={[styles.quickActionIcon, { backgroundColor: action.color }]}>
                <Text style={styles.quickActionIconText}>{action.icon}</Text>
              </View>
              <Text style={styles.quickActionLabel}>{action.label}</Text>
            </TouchableOpacity>
          ))}
        </View>

        {/* Chain Status Bar */}
        {chainStatus && (
          <View style={styles.chainStatus}>
            <View style={styles.chainStatusItem}>
              <Text style={styles.chainStatusLabel}>Block</Text>
              <Text style={styles.chainStatusValue}>
                #{chainStatus.blockHeight.toLocaleString()}
              </Text>
            </View>
            <View style={styles.chainStatusDivider} />
            <View style={styles.chainStatusItem}>
              <Text style={styles.chainStatusLabel}>Epoch</Text>
              <Text style={styles.chainStatusValue}>{chainStatus.epoch}</Text>
            </View>
            <View style={styles.chainStatusDivider} />
            <View style={styles.chainStatusItem}>
              <Text style={styles.chainStatusLabel}>Peers</Text>
              <Text style={styles.chainStatusValue}>{chainStatus.peerCount}</Text>
            </View>
          </View>
        )}

        {/* Recent Transactions */}
        <View style={styles.section}>
          <View style={styles.sectionHeader}>
            <Text style={styles.sectionTitle}>Recent Transactions</Text>
            <TouchableOpacity onPress={() => navigation.navigate('History')} activeOpacity={0.7}>
              <Text style={styles.viewAllText}>View All</Text>
            </TouchableOpacity>
          </View>
          {transactions.length === 0 ? (
            <View style={styles.emptyState}>
              <Text style={styles.emptyText}>No transactions yet</Text>
            </View>
          ) : (
            transactions.map((tx) => {
              const address = balance?.address || '';
              const isSent = tx.from.toLowerCase() === address.toLowerCase();
              return (
                <View key={tx.hash} style={styles.txRow}>
                  <View style={styles.txLeft}>
                    <Text style={styles.txType}>
                      {isSent ? 'Sent' : 'Received'}
                    </Text>
                    <Text style={styles.txAddress}>
                      {formatAddress(isSent ? tx.to : tx.from)}
                    </Text>
                  </View>
                  <View style={styles.txRight}>
                    <Text
                      style={[
                        styles.txAmount,
                        { color: isSent ? '#ef4444' : '#22c55e' },
                      ]}
                    >
                      {isSent ? '-' : '+'}{formatAmount(tx.amount)} EVAP
                    </Text>
                    <Text style={styles.txTime}>{formatTime(tx.timestamp)}</Text>
                  </View>
                </View>
              );
            })
          )}
        </View>

        {/* Faucet link */}
        <TouchableOpacity
          testID="home-faucet-link"
          style={styles.faucetLink}
          onPress={() => navigation.navigate('Faucet' as any)}
          activeOpacity={0.7}
        >
          <Text style={styles.faucetLinkText}>Claim Testnet Tokens</Text>
        </TouchableOpacity>
      </ScrollView>
    </SafeAreaView>
  );
};

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#f9fafb',
  },
  scroll: {
    paddingBottom: 32,
  },
  balanceCard: {
    backgroundColor: '#ffffff',
    marginHorizontal: 16,
    marginTop: 16,
    padding: 24,
    borderRadius: 16,
    alignItems: 'center',
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.05,
    shadowRadius: 8,
    elevation: 2,
  },
  balanceLabel: {
    fontSize: 13,
    color: '#6b7280',
    fontWeight: '500',
    marginBottom: 4,
  },
  balanceAmount: {
    fontSize: 36,
    fontWeight: '700',
    color: '#111827',
    letterSpacing: -1,
  },
  balanceUsd: {
    fontSize: 14,
    color: '#9ca3af',
    marginTop: 4,
  },
  quickActions: {
    flexDirection: 'row',
    justifyContent: 'space-around',
    paddingHorizontal: 16,
    paddingVertical: 20,
  },
  quickAction: {
    alignItems: 'center',
    minWidth: 56,
  },
  quickActionIcon: {
    width: 48,
    height: 48,
    borderRadius: 14,
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: 6,
  },
  quickActionIconText: {
    color: '#ffffff',
    fontSize: 18,
    fontWeight: '700',
  },
  quickActionLabel: {
    fontSize: 12,
    color: '#4b5563',
    fontWeight: '500',
  },
  chainStatus: {
    flexDirection: 'row',
    backgroundColor: '#ffffff',
    marginHorizontal: 16,
    borderRadius: 12,
    padding: 12,
    alignItems: 'center',
    justifyContent: 'space-around',
  },
  chainStatusItem: {
    alignItems: 'center',
    flex: 1,
  },
  chainStatusLabel: {
    fontSize: 10,
    color: '#9ca3af',
    fontWeight: '500',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
  },
  chainStatusValue: {
    fontSize: 14,
    color: '#111827',
    fontWeight: '600',
    marginTop: 2,
  },
  chainStatusDivider: {
    width: 1,
    height: 24,
    backgroundColor: '#e5e7eb',
  },
  section: {
    marginTop: 20,
    marginHorizontal: 16,
  },
  sectionHeader: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginBottom: 12,
  },
  sectionTitle: {
    fontSize: 16,
    fontWeight: '600',
    color: '#111827',
  },
  viewAllText: {
    fontSize: 14,
    fontWeight: '500',
    color: '#06b6d4',
  },
  emptyState: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 24,
    alignItems: 'center',
  },
  emptyText: {
    fontSize: 14,
    color: '#9ca3af',
  },
  txRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 14,
    marginBottom: 8,
  },
  txLeft: {},
  txType: {
    fontSize: 14,
    fontWeight: '600',
    color: '#111827',
  },
  txAddress: {
    fontSize: 12,
    color: '#9ca3af',
    marginTop: 2,
  },
  txRight: {
    alignItems: 'flex-end',
  },
  txAmount: {
    fontSize: 14,
    fontWeight: '600',
  },
  txTime: {
    fontSize: 11,
    color: '#9ca3af',
    marginTop: 2,
  },
  faucetLink: {
    marginTop: 16,
    marginHorizontal: 16,
    backgroundColor: '#ecfeff',
    borderWidth: 1,
    borderColor: '#06b6d4',
    borderRadius: 12,
    paddingVertical: 14,
    alignItems: 'center',
  },
  faucetLinkText: {
    fontSize: 14,
    color: '#06b6d4',
    fontWeight: '600',
  },
  statusRow: {
    flexDirection: 'row',
    gap: 8,
    marginTop: 12,
    flexWrap: 'wrap',
    justifyContent: 'center',
  },
  pendingPill: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    paddingHorizontal: 10,
    paddingVertical: 4,
    borderRadius: 999,
    backgroundColor: '#ecfeff',
    borderWidth: 1,
    borderColor: '#67e8f9',
  },
  pendingDot: {
    width: 6,
    height: 6,
    borderRadius: 3,
    backgroundColor: '#06b6d4',
  },
  pendingText: {
    fontSize: 11,
    color: '#0891b2',
    fontWeight: '600',
  },
  shardPill: {
    paddingHorizontal: 10,
    paddingVertical: 4,
    borderRadius: 999,
    backgroundColor: '#ecfeff',
    borderWidth: 1,
    borderColor: '#67e8f9',
  },
  shardPillText: {
    fontSize: 11,
    color: '#0891b2',
    fontWeight: '700',
  },
});

export default HomeScreen;
