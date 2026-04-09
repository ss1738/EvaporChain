/**
 * NftScreen — NFT gallery with energy bars and evaporation countdown
 */

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  FlatList,
  StyleSheet,
  RefreshControl,
  Alert,
  ActivityIndicator,
  Image,
  Dimensions,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { api } from '../utils/api';
import type { NFT } from '../utils/api';
import { keystore } from '../utils/keystore';
import { EnergyBar } from '../components/EnergyBar';

type Props = {
  navigation: NativeStackNavigationProp<RootStackParamList, 'NFTs'>;
};

const { width } = Dimensions.get('window');
const CARD_WIDTH = (width - 48) / 2; // 2 columns with gaps

const STATE_COLORS: Record<string, string> = {
  Active: '#22c55e',
  Grace: '#f59e0b',
  Ghost: '#9ca3af',
};

const NftScreen: React.FC<Props> = ({ navigation }) => {
  const [nfts, setNfts] = useState<NFT[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshingId, setRefreshingId] = useState<string | null>(null);

  const loadNfts = useCallback(async () => {
    try {
      const address = await keystore.getAddress();
      if (!address) return;
      const result = await api.getNFTs(address);
      result.sort((a, b) => {
        const pctA = a.maxEnergy > 0 ? a.energy / a.maxEnergy : 0;
        const pctB = b.maxEnergy > 0 ? b.energy / b.maxEnergy : 0;
        return pctA - pctB;
      });
      setNfts(result);
    } catch {
      // Keep existing data
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadNfts();
  }, [loadNfts]);

  const handleRefreshNft = async (nftId: string) => {
    setRefreshingId(nftId);
    try {
      const result = await api.refreshNFT(nftId, 1000);
      if (result.success) {
        Alert.alert('Refreshed', 'NFT energy has been restored.');
        await loadNfts();
      } else {
        Alert.alert('Error', result.message);
      }
    } catch {
      Alert.alert('Error', 'Could not refresh NFT.');
    } finally {
      setRefreshingId(null);
    }
  };

  const formatCountdown = (estimatedGhostTime: number): string => {
    const ms = estimatedGhostTime - Date.now();
    if (ms <= 0) return 'Evaporated';
    const hours = Math.floor(ms / 3600000);
    const days = Math.floor(hours / 24);
    if (days > 0) return `${days}d ${hours % 24}h`;
    if (hours > 0) return `${hours}h ${Math.floor((ms % 3600000) / 60000)}m`;
    return `${Math.round(ms / 60000)}m`;
  };

  const renderNft = ({ item }: { item: NFT }) => {
    const isRefreshing = refreshingId === item.id;

    return (
      <TouchableOpacity
        style={styles.card}
        onPress={() => navigation.navigate('NftDetail', { nftId: item.id })}
        activeOpacity={0.7}
      >
        {/* NFT Image */}
        <View style={styles.imageContainer}>
          {item.imageUri ? (
            <Image source={{ uri: item.imageUri }} style={styles.image} resizeMode="cover" />
          ) : (
            <View style={styles.imagePlaceholder}>
              <Text style={styles.imagePlaceholderText}>NFT</Text>
            </View>
          )}
          <View
            style={[
              styles.stateBadge,
              { backgroundColor: STATE_COLORS[item.state] || '#9ca3af' },
            ]}
          >
            <Text style={styles.stateBadgeText}>{item.state}</Text>
          </View>
        </View>

        {/* Info */}
        <View style={styles.cardBody}>
          <Text style={styles.nftName} numberOfLines={1}>
            {item.name}
          </Text>
          <Text style={styles.collectionName} numberOfLines={1}>
            {item.collectionName}
          </Text>

          <EnergyBar
            energy={item.energy}
            maxEnergy={item.maxEnergy}
            height={6}
            showPercentage={false}
            style={styles.energyBar}
          />

          <View style={styles.countdownRow}>
            <Text style={styles.countdownText}>
              {formatCountdown(item.estimatedGhostTime)}
            </Text>
          </View>

          {item.state !== 'Ghost' && (
            <TouchableOpacity
              style={styles.refreshButton}
              onPress={() => handleRefreshNft(item.id)}
              disabled={isRefreshing}
              activeOpacity={0.7}
            >
              {isRefreshing ? (
                <ActivityIndicator size="small" color="#06b6d4" />
              ) : (
                <Text style={styles.refreshButtonText}>Refresh</Text>
              )}
            </TouchableOpacity>
          )}
        </View>
      </TouchableOpacity>
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
      <FlatList
        data={nfts}
        keyExtractor={(item) => item.id}
        renderItem={renderNft}
        numColumns={2}
        columnWrapperStyle={styles.row}
        contentContainerStyle={styles.list}
        refreshControl={
          <RefreshControl refreshing={false} onRefresh={loadNfts} tintColor="#06b6d4" />
        }
        ListEmptyComponent={
          <View style={styles.emptyState}>
            <Text style={styles.emptyTitle}>No NFTs</Text>
            <Text style={styles.emptySubtext}>
              Your EvaporChain NFTs will appear here. Remember — they need energy to survive!
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
  row: {
    justifyContent: 'space-between',
    marginBottom: 12,
  },
  card: {
    width: CARD_WIDTH,
    backgroundColor: '#ffffff',
    borderRadius: 14,
    overflow: 'hidden',
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 1 },
    shadowOpacity: 0.04,
    shadowRadius: 4,
    elevation: 1,
  },
  imageContainer: {
    width: '100%',
    height: CARD_WIDTH,
    position: 'relative',
  },
  image: {
    width: '100%',
    height: '100%',
  },
  imagePlaceholder: {
    width: '100%',
    height: '100%',
    backgroundColor: '#f3f4f6',
    alignItems: 'center',
    justifyContent: 'center',
  },
  imagePlaceholderText: {
    fontSize: 24,
    fontWeight: '700',
    color: '#d1d5db',
  },
  stateBadge: {
    position: 'absolute',
    top: 8,
    right: 8,
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 8,
  },
  stateBadgeText: {
    fontSize: 10,
    color: '#ffffff',
    fontWeight: '700',
    textTransform: 'uppercase',
  },
  cardBody: {
    padding: 10,
  },
  nftName: {
    fontSize: 14,
    fontWeight: '600',
    color: '#111827',
  },
  collectionName: {
    fontSize: 11,
    color: '#9ca3af',
    marginTop: 2,
    marginBottom: 8,
  },
  energyBar: {
    marginBottom: 6,
  },
  countdownRow: {
    marginBottom: 8,
  },
  countdownText: {
    fontSize: 11,
    color: '#6b7280',
    fontWeight: '500',
  },
  refreshButton: {
    backgroundColor: '#ecfeff',
    borderWidth: 1,
    borderColor: '#06b6d4',
    borderRadius: 8,
    paddingVertical: 6,
    alignItems: 'center',
    minHeight: 32,
    justifyContent: 'center',
  },
  refreshButtonText: {
    color: '#06b6d4',
    fontSize: 12,
    fontWeight: '600',
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

export default NftScreen;
