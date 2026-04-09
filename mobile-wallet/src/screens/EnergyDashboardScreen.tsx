/**
 * EnergyDashboardScreen — Aggregate energy view across all assets
 *
 * This is EvaporChain's visual differentiator. No other wallet has this.
 * Shows: total energy health, urgency list, decay forecast, batch refresh.
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
  Alert,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { api } from '../utils/api';
import type { ChainObject, NFT } from '../utils/api';
import { keystore } from '../utils/keystore';
import { EnergyBar } from '../components/EnergyBar';

type Props = {
  navigation: NativeStackNavigationProp<RootStackParamList, 'EnergyDashboard'>;
};

interface AssetEnergy {
  id: string;
  name: string;
  type: 'object' | 'nft';
  energy: number;
  maxEnergy: number;
  state: string;
  estimatedGhostTime: number;
  percentage: number;
  hoursLeft: number;
}

const EnergyDashboardScreen: React.FC<Props> = ({ navigation }) => {
  const [assets, setAssets] = useState<AssetEnergy[]>([]);
  const [loading, setLoading] = useState(true);
  const [batchRefreshing, setBatchRefreshing] = useState(false);
  const [now, setNow] = useState(Date.now());

  const loadAssets = useCallback(async () => {
    try {
      const address = await keystore.getAddress();
      if (!address) return;

      const [objects, nfts] = await Promise.all([
        api.getObjects(address),
        api.getNFTs(address),
      ]);

      const combined: AssetEnergy[] = [
        ...objects.map((o: ChainObject) => ({
          id: o.id,
          name: o.name,
          type: 'object' as const,
          energy: o.energy,
          maxEnergy: o.maxEnergy,
          state: o.state,
          estimatedGhostTime: o.estimatedGhostTime,
          percentage: o.maxEnergy > 0 ? (o.energy / o.maxEnergy) * 100 : 0,
          hoursLeft: Math.max(0, (o.estimatedGhostTime - Date.now()) / 3600000),
        })),
        ...nfts.map((n: NFT) => ({
          id: n.id,
          name: n.name,
          type: 'nft' as const,
          energy: n.energy,
          maxEnergy: n.maxEnergy,
          state: n.state,
          estimatedGhostTime: n.estimatedGhostTime,
          percentage: n.maxEnergy > 0 ? (n.energy / n.maxEnergy) * 100 : 0,
          hoursLeft: Math.max(0, (n.estimatedGhostTime - Date.now()) / 3600000),
        })),
      ];

      combined.sort((a, b) => a.percentage - b.percentage);
      setAssets(combined);
    } catch {
      // Keep existing
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadAssets();
  }, [loadAssets]);

  useEffect(() => {
    const interval = setInterval(() => setNow(Date.now()), 5000);
    return () => clearInterval(interval);
  }, []);

  // Computed stats
  const totalAssets = assets.length;
  const criticalAssets = assets.filter((a) => a.percentage < 20 && a.state !== 'Ghost');
  const ghostAssets = assets.filter((a) => a.state === 'Ghost');
  const healthyAssets = assets.filter((a) => a.percentage >= 60);
  const avgEnergy = totalAssets > 0
    ? Math.round(assets.reduce((sum, a) => sum + a.percentage, 0) / totalAssets)
    : 0;

  const getHealthColor = (avg: number): string => {
    if (avg >= 60) return '#22c55e';
    if (avg >= 30) return '#f59e0b';
    return '#ef4444';
  };

  const getHealthLabel = (avg: number): string => {
    if (avg >= 60) return 'Healthy';
    if (avg >= 30) return 'At Risk';
    return 'Critical';
  };

  const handleBatchRefresh = async () => {
    const urgent = criticalAssets.slice(0, 5);
    if (urgent.length === 0) {
      Alert.alert('All Good', 'No assets need urgent refresh.');
      return;
    }

    Alert.alert(
      'Batch Refresh',
      `Refresh ${urgent.length} critical asset${urgent.length > 1 ? 's' : ''} with 1000 energy each?`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Refresh All',
          onPress: async () => {
            setBatchRefreshing(true);
            let success = 0;
            for (const asset of urgent) {
              try {
                const fn = asset.type === 'object' ? api.refreshObject : api.refreshNFT;
                const result = await fn(asset.id, 1000);
                if (result.success) success++;
              } catch {
                // Continue with others
              }
            }
            setBatchRefreshing(false);
            Alert.alert('Done', `Refreshed ${success}/${urgent.length} assets.`);
            await loadAssets();
          },
        },
      ]
    );
  };

  const formatTimeLeft = (hoursLeft: number): string => {
    if (hoursLeft <= 0) return 'Expired';
    if (hoursLeft < 1) return `${Math.round(hoursLeft * 60)}m`;
    if (hoursLeft < 24) return `${Math.round(hoursLeft)}h`;
    return `${Math.floor(hoursLeft / 24)}d ${Math.round(hoursLeft % 24)}h`;
  };

  const handleAssetPress = (asset: AssetEnergy) => {
    if (asset.type === 'object') {
      navigation.navigate('ObjectDetail', { objectId: asset.id });
    } else {
      navigation.navigate('NftDetail', { nftId: asset.id });
    }
  };

  const renderAsset = ({ item }: { item: AssetEnergy }) => (
    <TouchableOpacity
      style={styles.assetRow}
      onPress={() => handleAssetPress(item)}
      activeOpacity={0.7}
    >
      <View style={styles.assetLeft}>
        <View style={styles.assetNameRow}>
          <Text style={styles.assetTypeBadge}>
            {item.type === 'object' ? 'OBJ' : 'NFT'}
          </Text>
          <Text style={styles.assetName} numberOfLines={1}>{item.name}</Text>
        </View>
        <EnergyBar
          energy={item.energy}
          maxEnergy={item.maxEnergy}
          height={6}
          showPercentage={false}
          style={styles.assetEnergyBar}
        />
      </View>
      <View style={styles.assetRight}>
        <Text style={styles.assetPercentage}>{Math.round(item.percentage)}%</Text>
        <Text style={styles.assetTimeLeft}>{formatTimeLeft(item.hoursLeft)}</Text>
      </View>
    </TouchableOpacity>
  );

  if (loading) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator size="large" color="#06b6d4" />
      </View>
    );
  }

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <FlatList
        data={assets}
        keyExtractor={(item) => item.id}
        renderItem={renderAsset}
        contentContainerStyle={styles.list}
        refreshControl={
          <RefreshControl refreshing={false} onRefresh={loadAssets} tintColor="#06b6d4" />
        }
        ListHeaderComponent={
          <>
            {/* Health Ring */}
            <View style={styles.healthCard}>
              <View style={[styles.healthRing, { borderColor: getHealthColor(avgEnergy) }]}>
                <Text style={[styles.healthPercent, { color: getHealthColor(avgEnergy) }]}>
                  {avgEnergy}%
                </Text>
                <Text style={styles.healthLabel}>{getHealthLabel(avgEnergy)}</Text>
              </View>
            </View>

            {/* Stats Grid */}
            <View style={styles.statsGrid}>
              <View style={styles.statCard}>
                <Text style={styles.statValue}>{totalAssets}</Text>
                <Text style={styles.statLabel}>Total</Text>
              </View>
              <View style={styles.statCard}>
                <Text style={[styles.statValue, { color: '#22c55e' }]}>{healthyAssets.length}</Text>
                <Text style={styles.statLabel}>Healthy</Text>
              </View>
              <View style={styles.statCard}>
                <Text style={[styles.statValue, { color: '#ef4444' }]}>{criticalAssets.length}</Text>
                <Text style={styles.statLabel}>Critical</Text>
              </View>
              <View style={styles.statCard}>
                <Text style={[styles.statValue, { color: '#9ca3af' }]}>{ghostAssets.length}</Text>
                <Text style={styles.statLabel}>Ghost</Text>
              </View>
            </View>

            {/* Batch Refresh */}
            {criticalAssets.length > 0 && (
              <TouchableOpacity
                style={styles.batchButton}
                onPress={handleBatchRefresh}
                disabled={batchRefreshing}
                activeOpacity={0.7}
              >
                {batchRefreshing ? (
                  <ActivityIndicator color="#ffffff" />
                ) : (
                  <Text style={styles.batchButtonText}>
                    Batch Refresh {criticalAssets.length} Critical Asset{criticalAssets.length > 1 ? 's' : ''}
                  </Text>
                )}
              </TouchableOpacity>
            )}

            {/* List Header */}
            <Text style={styles.listTitle}>All Assets by Urgency</Text>
          </>
        }
        ListEmptyComponent={
          <View style={styles.emptyState}>
            <Text style={styles.emptyTitle}>No Assets</Text>
            <Text style={styles.emptySubtext}>
              Your objects and NFTs will appear here with live energy tracking.
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
  list: {
    padding: 16,
    paddingBottom: 32,
  },
  // Health Ring
  healthCard: {
    alignItems: 'center',
    paddingVertical: 24,
  },
  healthRing: {
    width: 140,
    height: 140,
    borderRadius: 70,
    borderWidth: 8,
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: '#ffffff',
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.05,
    shadowRadius: 8,
    elevation: 2,
  },
  healthPercent: {
    fontSize: 36,
    fontWeight: '800',
  },
  healthLabel: {
    fontSize: 13,
    color: '#6b7280',
    fontWeight: '500',
    marginTop: -2,
  },
  // Stats
  statsGrid: {
    flexDirection: 'row',
    gap: 8,
    marginBottom: 16,
  },
  statCard: {
    flex: 1,
    backgroundColor: '#ffffff',
    borderRadius: 12,
    paddingVertical: 14,
    alignItems: 'center',
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 1 },
    shadowOpacity: 0.03,
    shadowRadius: 3,
    elevation: 1,
  },
  statValue: {
    fontSize: 20,
    fontWeight: '700',
    color: '#111827',
  },
  statLabel: {
    fontSize: 11,
    color: '#9ca3af',
    marginTop: 2,
    fontWeight: '500',
  },
  // Batch
  batchButton: {
    backgroundColor: '#ef4444',
    borderRadius: 12,
    paddingVertical: 14,
    alignItems: 'center',
    marginBottom: 20,
    minHeight: 48,
    justifyContent: 'center',
  },
  batchButtonText: {
    color: '#ffffff',
    fontSize: 15,
    fontWeight: '700',
  },
  // List
  listTitle: {
    fontSize: 14,
    fontWeight: '600',
    color: '#6b7280',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginBottom: 10,
  },
  assetRow: {
    flexDirection: 'row',
    backgroundColor: '#ffffff',
    borderRadius: 12,
    padding: 14,
    marginBottom: 8,
    alignItems: 'center',
  },
  assetLeft: {
    flex: 1,
    marginRight: 12,
  },
  assetNameRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
    marginBottom: 8,
  },
  assetTypeBadge: {
    fontSize: 10,
    fontWeight: '700',
    color: '#6b7280',
    backgroundColor: '#f3f4f6',
    paddingHorizontal: 6,
    paddingVertical: 2,
    borderRadius: 4,
    overflow: 'hidden',
  },
  assetName: {
    fontSize: 15,
    fontWeight: '600',
    color: '#111827',
    flex: 1,
  },
  assetEnergyBar: {},
  assetRight: {
    alignItems: 'flex-end',
    minWidth: 60,
  },
  assetPercentage: {
    fontSize: 16,
    fontWeight: '700',
    color: '#111827',
  },
  assetTimeLeft: {
    fontSize: 12,
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

export default EnergyDashboardScreen;
