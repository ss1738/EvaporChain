/**
 * NftDetailScreen — Full NFT view with image, energy, countdown, and refresh
 */

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  Alert,
  ActivityIndicator,
  Image,
  Dimensions,
  TextInput,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RouteProp } from '@react-navigation/native';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { api } from '../utils/api';
import type { NFT } from '../utils/api';
import { keystore } from '../utils/keystore';
import { EnergyBar } from '../components/EnergyBar';

type Props = {
  navigation: NativeStackNavigationProp<RootStackParamList, 'NftDetail'>;
  route: RouteProp<RootStackParamList, 'NftDetail'>;
};

const { width } = Dimensions.get('window');

const STATE_COLORS: Record<string, string> = {
  Active: '#22c55e',
  Grace: '#f59e0b',
  Ghost: '#9ca3af',
  Risen: '#8b5cf6',
};

const NftDetailScreen: React.FC<Props> = ({ route }) => {
  const { nftId } = route.params;
  const [nft, setNft] = useState<NFT | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [energyInput, setEnergyInput] = useState('1000');
  const [now, setNow] = useState(Date.now());

  const loadNft = useCallback(async () => {
    try {
      const address = await keystore.getAddress();
      if (!address) return;
      const nfts = await api.getNFTs(address);
      const found = nfts.find((n) => n.id === nftId);
      if (found) setNft(found);
    } catch {
      // Keep existing
    } finally {
      setLoading(false);
    }
  }, [nftId]);

  useEffect(() => {
    loadNft();
  }, [loadNft]);

  useEffect(() => {
    const interval = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(interval);
  }, []);

  const handleRefresh = async () => {
    const energy = parseInt(energyInput, 10);
    if (!energy || energy <= 0) {
      Alert.alert('Invalid', 'Enter a positive energy amount.');
      return;
    }
    setRefreshing(true);
    try {
      const result = await api.refreshNFT(nftId, energy);
      if (result.success) {
        Alert.alert('Refreshed', `Added ${energy} energy to NFT.`);
        await loadNft();
      } else {
        Alert.alert('Error', result.message);
      }
    } catch {
      Alert.alert('Error', 'Could not refresh NFT.');
    } finally {
      setRefreshing(false);
    }
  };

  const formatCountdown = (ghostTime: number): string => {
    const ms = ghostTime - now;
    if (ms <= 0) return 'Evaporated';
    const days = Math.floor(ms / 86400000);
    const hours = Math.floor((ms % 86400000) / 3600000);
    const minutes = Math.floor((ms % 3600000) / 60000);
    const seconds = Math.floor((ms % 60000) / 1000);
    if (days > 0) return `${days}d ${hours}h ${minutes}m`;
    if (hours > 0) return `${hours}h ${minutes}m ${seconds}s`;
    return `${minutes}m ${seconds}s`;
  };

  if (loading) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator size="large" color="#06b6d4" />
      </View>
    );
  }

  if (!nft) {
    return (
      <View style={styles.centered}>
        <Text style={styles.errorText}>NFT not found</Text>
      </View>
    );
  }

  const percentage = nft.maxEnergy > 0 ? Math.round((nft.energy / nft.maxEnergy) * 100) : 0;

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView contentContainerStyle={styles.scroll}>
        {/* Image */}
        <View style={styles.imageContainer}>
          {nft.imageUri ? (
            <Image source={{ uri: nft.imageUri }} style={styles.image} resizeMode="cover" />
          ) : (
            <View style={styles.imagePlaceholder}>
              <Text style={styles.placeholderText}>NFT</Text>
            </View>
          )}
          <View style={[styles.stateBadge, { backgroundColor: STATE_COLORS[nft.state] || '#9ca3af' }]}>
            <Text style={styles.stateBadgeText}>{nft.state}</Text>
          </View>
        </View>

        {/* Title */}
        <Text style={styles.name}>{nft.name}</Text>
        <Text style={styles.collection}>{nft.collectionName || nft.collection}</Text>

        {/* Energy */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Energy Status</Text>
          <EnergyBar
            energy={nft.energy}
            maxEnergy={nft.maxEnergy}
            showLabel
            showPercentage
            height={12}
            style={styles.energyBar}
          />
          <View style={styles.statsGrid}>
            <View style={styles.stat}>
              <Text style={styles.statValue}>{nft.energy.toLocaleString()}</Text>
              <Text style={styles.statLabel}>Current</Text>
            </View>
            <View style={styles.stat}>
              <Text style={styles.statValue}>{nft.maxEnergy.toLocaleString()}</Text>
              <Text style={styles.statLabel}>Maximum</Text>
            </View>
            <View style={styles.stat}>
              <Text style={styles.statValue}>{percentage}%</Text>
              <Text style={styles.statLabel}>Remaining</Text>
            </View>
          </View>
        </View>

        {/* Countdown */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Time Until Ghost</Text>
          <Text style={styles.countdown}>
            {formatCountdown(nft.estimatedGhostTime)}
          </Text>
          <View style={styles.metaRow}>
            <Text style={styles.metaLabel}>Decay Rate</Text>
            <Text style={styles.metaValue}>{nft.decayPercentage.toFixed(2)}% / epoch</Text>
          </View>
        </View>

        {/* Details */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Details</Text>
          <View style={styles.metaRow}>
            <Text style={styles.metaLabel}>Token ID</Text>
            <Text style={styles.metaValueMono} selectable numberOfLines={1}>
              {nft.id.slice(0, 20)}...
            </Text>
          </View>
          <View style={styles.metaRow}>
            <Text style={styles.metaLabel}>Collection</Text>
            <Text style={styles.metaValue}>{nft.collectionName || nft.collection}</Text>
          </View>
          <View style={styles.metaRow}>
            <Text style={styles.metaLabel}>Owner</Text>
            <Text style={styles.metaValue} numberOfLines={1}>
              {nft.owner.slice(0, 12)}...{nft.owner.slice(-6)}
            </Text>
          </View>
        </View>

        {/* Refresh */}
        {nft.state !== 'Ghost' && (
          <View style={styles.card}>
            <Text style={styles.cardTitle}>Refresh Energy</Text>
            <Text style={styles.refreshDescription}>
              Keep this NFT alive by adding energy before it reaches Ghost state.
            </Text>
            <View style={styles.refreshRow}>
              <TextInput
                style={styles.energyInput}
                value={energyInput}
                onChangeText={setEnergyInput}
                keyboardType="number-pad"
                placeholder="Energy amount"
                placeholderTextColor="#9ca3af"
              />
              <TouchableOpacity
                style={styles.refreshButton}
                onPress={handleRefresh}
                disabled={refreshing}
                activeOpacity={0.7}
              >
                {refreshing ? (
                  <ActivityIndicator size="small" color="#ffffff" />
                ) : (
                  <Text style={styles.refreshButtonText}>Refresh</Text>
                )}
              </TouchableOpacity>
            </View>
          </View>
        )}
      </ScrollView>
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
  errorText: {
    fontSize: 16,
    color: '#9ca3af',
  },
  scroll: {
    paddingBottom: 40,
  },
  imageContainer: {
    width: width,
    height: width * 0.8,
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
  placeholderText: {
    fontSize: 48,
    fontWeight: '700',
    color: '#d1d5db',
  },
  stateBadge: {
    position: 'absolute',
    top: 16,
    right: 16,
    paddingHorizontal: 12,
    paddingVertical: 5,
    borderRadius: 10,
  },
  stateBadgeText: {
    fontSize: 12,
    color: '#ffffff',
    fontWeight: '700',
    textTransform: 'uppercase',
  },
  name: {
    fontSize: 24,
    fontWeight: '700',
    color: '#111827',
    paddingHorizontal: 16,
    marginTop: 16,
  },
  collection: {
    fontSize: 15,
    color: '#6b7280',
    paddingHorizontal: 16,
    marginTop: 2,
    marginBottom: 16,
  },
  card: {
    backgroundColor: '#ffffff',
    borderRadius: 14,
    padding: 16,
    marginHorizontal: 16,
    marginBottom: 12,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 1 },
    shadowOpacity: 0.04,
    shadowRadius: 4,
    elevation: 1,
  },
  cardTitle: {
    fontSize: 14,
    fontWeight: '600',
    color: '#6b7280',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginBottom: 12,
  },
  energyBar: {
    marginBottom: 16,
  },
  statsGrid: {
    flexDirection: 'row',
    gap: 12,
  },
  stat: {
    flex: 1,
    backgroundColor: '#f9fafb',
    borderRadius: 10,
    padding: 12,
    alignItems: 'center',
  },
  statValue: {
    fontSize: 16,
    fontWeight: '700',
    color: '#111827',
  },
  statLabel: {
    fontSize: 11,
    color: '#9ca3af',
    marginTop: 2,
  },
  countdown: {
    fontSize: 32,
    fontWeight: '700',
    color: '#111827',
    textAlign: 'center',
    marginBottom: 16,
  },
  metaRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    paddingVertical: 8,
    borderTopWidth: 1,
    borderTopColor: '#f3f4f6',
  },
  metaLabel: {
    fontSize: 14,
    color: '#6b7280',
  },
  metaValue: {
    fontSize: 14,
    color: '#111827',
    fontWeight: '500',
    maxWidth: '60%',
  },
  metaValueMono: {
    fontSize: 13,
    color: '#111827',
    fontWeight: '500',
    fontFamily: 'monospace',
    maxWidth: '60%',
  },
  refreshDescription: {
    fontSize: 13,
    color: '#6b7280',
    lineHeight: 18,
    marginBottom: 12,
  },
  refreshRow: {
    flexDirection: 'row',
    gap: 10,
  },
  energyInput: {
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
  refreshButton: {
    backgroundColor: '#06b6d4',
    borderRadius: 12,
    paddingHorizontal: 20,
    alignItems: 'center',
    justifyContent: 'center',
    minWidth: 90,
    minHeight: 48,
  },
  refreshButtonText: {
    color: '#ffffff',
    fontSize: 15,
    fontWeight: '700',
  },
});

export default NftDetailScreen;
