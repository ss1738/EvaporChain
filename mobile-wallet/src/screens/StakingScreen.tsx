/**
 * StakingScreen — Validator staking, rewards, and delegation
 *
 * Shows staked balance, pending rewards, stake/unstake actions,
 * and top validators list.
 */

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  TextInput,
  TouchableOpacity,
  ScrollView,
  FlatList,
  StyleSheet,
  Alert,
  ActivityIndicator,
  RefreshControl,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as LocalAuthentication from 'expo-local-authentication';
import { api } from '../utils/api';
import type { StakingInfo, Validator } from '../utils/api';
import { keystore } from '../utils/keystore';

type Tab = 'overview' | 'validators';

const StakingScreen: React.FC = () => {
  const [tab, setTab] = useState<Tab>('overview');
  const [stakingInfo, setStakingInfo] = useState<StakingInfo | null>(null);
  const [balance, setBalance] = useState<number>(0);
  const [validators, setValidators] = useState<Validator[]>([]);
  const [loading, setLoading] = useState(true);
  const [stakeAmount, setStakeAmount] = useState('');
  const [unstakeAmount, setUnstakeAmount] = useState('');
  const [processing, setProcessing] = useState(false);

  const loadData = useCallback(async () => {
    try {
      const address = await keystore.getAddress();
      if (!address) return;

      const [info, bal, vals] = await Promise.allSettled([
        api.getStakingInfo(address),
        api.getBalance(address),
        api.getValidators(),
      ]);

      if (info.status === 'fulfilled') setStakingInfo(info.value);
      if (bal.status === 'fulfilled') setBalance(bal.value.balance);
      if (vals.status === 'fulfilled') {
        const sorted = vals.value.sort((a, b) => b.stake - a.stake);
        setValidators(sorted);
      }
    } catch {
      // Keep existing
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const authenticateAndProcess = async (
    action: string,
    fn: () => Promise<void>
  ) => {
    const auth = await LocalAuthentication.authenticateAsync({
      promptMessage: `Confirm ${action}`,
      cancelLabel: 'Cancel',
    });
    if (!auth.success) return;

    setProcessing(true);
    try {
      await fn();
    } catch {
      Alert.alert('Error', `${action} failed. Please try again.`);
    } finally {
      setProcessing(false);
    }
  };

  const handleStake = () => {
    const amount = parseFloat(stakeAmount);
    if (!amount || amount <= 0) {
      Alert.alert('Invalid', 'Enter a positive amount to stake.');
      return;
    }
    if (amount > balance) {
      Alert.alert('Insufficient', 'Amount exceeds available balance.');
      return;
    }

    authenticateAndProcess('Stake', async () => {
      const address = await keystore.getAddress();
      if (!address) return;
      const bal = await api.getBalance(address);
      const result = await api.stake(address, amount, bal.nonce);
      if (result.success) {
        Alert.alert('Staked', `${amount} EVAP staked successfully.`);
        setStakeAmount('');
        await loadData();
      } else {
        Alert.alert('Failed', result.message);
      }
    });
  };

  const handleUnstake = () => {
    const amount = parseFloat(unstakeAmount);
    if (!amount || amount <= 0 || !stakingInfo) {
      Alert.alert('Invalid', 'Enter a positive amount to unstake.');
      return;
    }
    if (amount > stakingInfo.staked) {
      Alert.alert('Insufficient', 'Amount exceeds staked balance.');
      return;
    }

    authenticateAndProcess('Unstake', async () => {
      const address = await keystore.getAddress();
      if (!address) return;
      const bal = await api.getBalance(address);
      const result = await api.unstake(address, amount, bal.nonce);
      if (result.success) {
        Alert.alert('Unstaked', `${amount} EVAP unstake initiated. Subject to unbonding period.`);
        setUnstakeAmount('');
        await loadData();
      } else {
        Alert.alert('Failed', result.message);
      }
    });
  };

  const handleClaimRewards = () => {
    if (!stakingInfo || stakingInfo.rewards <= 0) {
      Alert.alert('No Rewards', 'No pending rewards to claim.');
      return;
    }

    authenticateAndProcess('Claim Rewards', async () => {
      const address = await keystore.getAddress();
      if (!address) return;
      const bal = await api.getBalance(address);
      const result = await api.claimRewards(address, bal.nonce);
      if (result.success) {
        Alert.alert('Claimed', `${stakingInfo.rewards} EVAP rewards claimed.`);
        await loadData();
      } else {
        Alert.alert('Failed', result.message);
      }
    });
  };

  const getStatusColor = (status: string): string => {
    if (status === 'active') return '#22c55e';
    if (status === 'jailed') return '#ef4444';
    return '#9ca3af';
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
      {/* Tabs */}
      <View style={styles.tabRow}>
        <TouchableOpacity
          style={[styles.tab, tab === 'overview' && styles.tabActive]}
          onPress={() => setTab('overview')}
          activeOpacity={0.7}
        >
          <Text style={[styles.tabText, tab === 'overview' && styles.tabTextActive]}>
            My Staking
          </Text>
        </TouchableOpacity>
        <TouchableOpacity
          style={[styles.tab, tab === 'validators' && styles.tabActive]}
          onPress={() => setTab('validators')}
          activeOpacity={0.7}
        >
          <Text style={[styles.tabText, tab === 'validators' && styles.tabTextActive]}>
            Validators
          </Text>
        </TouchableOpacity>
      </View>

      {tab === 'overview' ? (
        <ScrollView
          contentContainerStyle={styles.scroll}
          refreshControl={<RefreshControl refreshing={false} onRefresh={loadData} tintColor="#06b6d4" />}
        >
          {/* Staking Overview Card */}
          <View style={styles.overviewCard}>
            <View style={styles.overviewRow}>
              <View style={styles.overviewStat}>
                <Text style={styles.overviewValue}>
                  {stakingInfo ? stakingInfo.staked.toLocaleString() : '0'}
                </Text>
                <Text style={styles.overviewLabel}>Staked EVAP</Text>
              </View>
              <View style={styles.overviewDivider} />
              <View style={styles.overviewStat}>
                <Text style={[styles.overviewValue, { color: '#22c55e' }]}>
                  {stakingInfo ? stakingInfo.rewards.toFixed(4) : '0'}
                </Text>
                <Text style={styles.overviewLabel}>Rewards</Text>
              </View>
            </View>
            {stakingInfo?.isValidator && (
              <View style={styles.validatorBadge}>
                <Text style={styles.validatorBadgeText}>Active Validator</Text>
              </View>
            )}
          </View>

          {/* Claim Rewards */}
          {stakingInfo && stakingInfo.rewards > 0 && (
            <TouchableOpacity
              style={styles.claimButton}
              onPress={handleClaimRewards}
              disabled={processing}
              activeOpacity={0.7}
            >
              {processing ? (
                <ActivityIndicator color="#ffffff" />
              ) : (
                <Text style={styles.claimButtonText}>
                  Claim {stakingInfo.rewards.toFixed(4)} EVAP Rewards
                </Text>
              )}
            </TouchableOpacity>
          )}

          {/* Unbonding */}
          {stakingInfo?.unbondingAmount && stakingInfo.unbondingAmount > 0 && (
            <View style={styles.unbondingCard}>
              <Text style={styles.unbondingTitle}>Unbonding</Text>
              <Text style={styles.unbondingAmount}>
                {stakingInfo.unbondingAmount.toLocaleString()} EVAP
              </Text>
              <Text style={styles.unbondingEpoch}>
                Completes at epoch {stakingInfo.unbondingCompleteEpoch}
              </Text>
            </View>
          )}

          {/* Stake */}
          <View style={styles.actionCard}>
            <Text style={styles.actionTitle}>Stake EVAP</Text>
            <Text style={styles.availableText}>
              Available: {balance.toLocaleString()} EVAP
            </Text>
            <View style={styles.actionRow}>
              <TextInput
                style={styles.actionInput}
                value={stakeAmount}
                onChangeText={setStakeAmount}
                keyboardType="decimal-pad"
                placeholder="Amount"
                placeholderTextColor="#9ca3af"
              />
              <TouchableOpacity
                style={styles.actionButton}
                onPress={handleStake}
                disabled={processing}
                activeOpacity={0.7}
              >
                <Text style={styles.actionButtonText}>Stake</Text>
              </TouchableOpacity>
            </View>
          </View>

          {/* Unstake */}
          {stakingInfo && stakingInfo.staked > 0 && (
            <View style={styles.actionCard}>
              <Text style={styles.actionTitle}>Unstake EVAP</Text>
              <Text style={styles.availableText}>
                Staked: {stakingInfo.staked.toLocaleString()} EVAP
              </Text>
              <View style={styles.actionRow}>
                <TextInput
                  style={styles.actionInput}
                  value={unstakeAmount}
                  onChangeText={setUnstakeAmount}
                  keyboardType="decimal-pad"
                  placeholder="Amount"
                  placeholderTextColor="#9ca3af"
                />
                <TouchableOpacity
                  style={[styles.actionButton, { backgroundColor: '#f59e0b' }]}
                  onPress={handleUnstake}
                  disabled={processing}
                  activeOpacity={0.7}
                >
                  <Text style={styles.actionButtonText}>Unstake</Text>
                </TouchableOpacity>
              </View>
              <Text style={styles.warningText}>
                Unstaking has a 7-epoch unbonding period.
              </Text>
            </View>
          )}
        </ScrollView>
      ) : (
        <FlatList
          data={validators}
          keyExtractor={(item) => item.address}
          contentContainerStyle={styles.scroll}
          refreshControl={<RefreshControl refreshing={false} onRefresh={loadData} tintColor="#06b6d4" />}
          renderItem={({ item, index }) => (
            <View style={styles.validatorCard}>
              <View style={styles.validatorHeader}>
                <View style={styles.validatorRank}>
                  <Text style={styles.rankText}>#{index + 1}</Text>
                </View>
                <View style={styles.validatorInfo}>
                  <Text style={styles.validatorName}>{item.name}</Text>
                  <Text style={styles.validatorAddress}>
                    {item.address.slice(0, 10)}...{item.address.slice(-6)}
                  </Text>
                </View>
                <View style={[styles.statusDot, { backgroundColor: getStatusColor(item.status) }]} />
              </View>
              <View style={styles.validatorStats}>
                <View style={styles.validatorStatItem}>
                  <Text style={styles.validatorStatValue}>
                    {(item.stake / 1000).toFixed(0)}K
                  </Text>
                  <Text style={styles.validatorStatLabel}>Stake</Text>
                </View>
                <View style={styles.validatorStatItem}>
                  <Text style={styles.validatorStatValue}>{item.commission}%</Text>
                  <Text style={styles.validatorStatLabel}>Commission</Text>
                </View>
                <View style={styles.validatorStatItem}>
                  <Text style={styles.validatorStatValue}>{item.uptime}%</Text>
                  <Text style={styles.validatorStatLabel}>Uptime</Text>
                </View>
              </View>
            </View>
          )}
          ListEmptyComponent={
            <View style={styles.emptyState}>
              <Text style={styles.emptyTitle}>No Validators</Text>
              <Text style={styles.emptySubtext}>
                Validator information will appear here once the network launches.
              </Text>
            </View>
          }
        />
      )}
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
  tabRow: {
    flexDirection: 'row',
    paddingHorizontal: 16,
    paddingVertical: 12,
    gap: 8,
  },
  tab: {
    flex: 1,
    paddingVertical: 10,
    borderRadius: 10,
    backgroundColor: '#ffffff',
    alignItems: 'center',
    borderWidth: 1,
    borderColor: '#e5e7eb',
  },
  tabActive: {
    backgroundColor: '#06b6d4',
    borderColor: '#06b6d4',
  },
  tabText: {
    fontSize: 14,
    fontWeight: '600',
    color: '#6b7280',
  },
  tabTextActive: {
    color: '#ffffff',
  },
  scroll: {
    padding: 16,
    paddingBottom: 40,
  },
  // Overview
  overviewCard: {
    backgroundColor: '#ffffff',
    borderRadius: 14,
    padding: 20,
    marginBottom: 12,
    alignItems: 'center',
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.05,
    shadowRadius: 8,
    elevation: 2,
  },
  overviewRow: {
    flexDirection: 'row',
    width: '100%',
  },
  overviewStat: {
    flex: 1,
    alignItems: 'center',
  },
  overviewDivider: {
    width: 1,
    backgroundColor: '#e5e7eb',
    marginHorizontal: 16,
  },
  overviewValue: {
    fontSize: 24,
    fontWeight: '700',
    color: '#111827',
  },
  overviewLabel: {
    fontSize: 13,
    color: '#9ca3af',
    marginTop: 4,
    fontWeight: '500',
  },
  validatorBadge: {
    backgroundColor: '#ecfdf5',
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 6,
    marginTop: 14,
  },
  validatorBadgeText: {
    fontSize: 13,
    fontWeight: '600',
    color: '#22c55e',
  },
  // Claim
  claimButton: {
    backgroundColor: '#22c55e',
    borderRadius: 12,
    paddingVertical: 14,
    alignItems: 'center',
    marginBottom: 12,
    minHeight: 48,
    justifyContent: 'center',
  },
  claimButtonText: {
    color: '#ffffff',
    fontSize: 15,
    fontWeight: '700',
  },
  // Unbonding
  unbondingCard: {
    backgroundColor: '#fffbeb',
    borderWidth: 1,
    borderColor: '#fde68a',
    borderRadius: 12,
    padding: 14,
    marginBottom: 12,
  },
  unbondingTitle: {
    fontSize: 13,
    fontWeight: '600',
    color: '#92400e',
  },
  unbondingAmount: {
    fontSize: 18,
    fontWeight: '700',
    color: '#111827',
    marginTop: 4,
  },
  unbondingEpoch: {
    fontSize: 12,
    color: '#a16207',
    marginTop: 4,
  },
  // Action Cards
  actionCard: {
    backgroundColor: '#ffffff',
    borderRadius: 14,
    padding: 16,
    marginBottom: 12,
  },
  actionTitle: {
    fontSize: 16,
    fontWeight: '600',
    color: '#111827',
    marginBottom: 4,
  },
  availableText: {
    fontSize: 13,
    color: '#9ca3af',
    marginBottom: 12,
  },
  actionRow: {
    flexDirection: 'row',
    gap: 10,
  },
  actionInput: {
    flex: 1,
    backgroundColor: '#f9fafb',
    borderWidth: 1,
    borderColor: '#e5e7eb',
    borderRadius: 12,
    paddingHorizontal: 14,
    paddingVertical: 12,
    fontSize: 16,
    color: '#111827',
  },
  actionButton: {
    backgroundColor: '#06b6d4',
    borderRadius: 12,
    paddingHorizontal: 20,
    alignItems: 'center',
    justifyContent: 'center',
    minWidth: 80,
    minHeight: 48,
  },
  actionButtonText: {
    color: '#ffffff',
    fontSize: 15,
    fontWeight: '700',
  },
  warningText: {
    fontSize: 12,
    color: '#f59e0b',
    marginTop: 10,
  },
  // Validators
  validatorCard: {
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 14,
    marginBottom: 8,
  },
  validatorHeader: {
    flexDirection: 'row',
    alignItems: 'center',
    marginBottom: 12,
  },
  validatorRank: {
    width: 32,
    height: 32,
    borderRadius: 10,
    backgroundColor: '#f3f4f6',
    alignItems: 'center',
    justifyContent: 'center',
    marginRight: 12,
  },
  rankText: {
    fontSize: 13,
    fontWeight: '700',
    color: '#6b7280',
  },
  validatorInfo: {
    flex: 1,
  },
  validatorName: {
    fontSize: 15,
    fontWeight: '600',
    color: '#111827',
  },
  validatorAddress: {
    fontSize: 12,
    color: '#9ca3af',
    fontFamily: 'monospace',
    marginTop: 1,
  },
  statusDot: {
    width: 10,
    height: 10,
    borderRadius: 5,
    marginLeft: 8,
  },
  validatorStats: {
    flexDirection: 'row',
    gap: 8,
  },
  validatorStatItem: {
    flex: 1,
    backgroundColor: '#f9fafb',
    borderRadius: 8,
    paddingVertical: 8,
    alignItems: 'center',
  },
  validatorStatValue: {
    fontSize: 14,
    fontWeight: '700',
    color: '#111827',
  },
  validatorStatLabel: {
    fontSize: 10,
    color: '#9ca3af',
    marginTop: 2,
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

export default StakingScreen;
