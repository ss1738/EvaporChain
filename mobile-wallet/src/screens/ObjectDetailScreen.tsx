/**
 * ObjectDetailScreen — Full object view with energy status, decay forecast, and refresh
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
  TextInput,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RouteProp } from '@react-navigation/native';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { api } from '../utils/api';
import type { ChainObject } from '../utils/api';
import { keystore } from '../utils/keystore';
import { EnergyBar } from '../components/EnergyBar';

type Props = {
  navigation: NativeStackNavigationProp<RootStackParamList, 'ObjectDetail'>;
  route: RouteProp<RootStackParamList, 'ObjectDetail'>;
};

const STATE_COLORS: Record<string, string> = {
  Active: '#22c55e',
  Grace: '#f59e0b',
  Ghost: '#9ca3af',
  Risen: '#8b5cf6',
};

const ObjectDetailScreen: React.FC<Props> = ({ route }) => {
  const { objectId } = route.params;
  const [object, setObject] = useState<ChainObject | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [energyInput, setEnergyInput] = useState('1000');
  const [now, setNow] = useState(Date.now());

  const loadObject = useCallback(async () => {
    try {
      const address = await keystore.getAddress();
      if (!address) return;
      const objects = await api.getObjects(address);
      const found = objects.find((o) => o.id === objectId);
      if (found) setObject(found);
    } catch {
      // Keep existing
    } finally {
      setLoading(false);
    }
  }, [objectId]);

  useEffect(() => {
    loadObject();
  }, [loadObject]);

  // Live countdown
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
      const result = await api.refreshObject(objectId, energy);
      if (result.success) {
        Alert.alert('Refreshed', `Added ${energy} energy to object.`);
        await loadObject();
      } else {
        Alert.alert('Error', result.message);
      }
    } catch {
      Alert.alert('Error', 'Could not refresh object.');
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

  const formatHalfLife = (seconds: number): string => {
    if (seconds >= 86400) return `${(seconds / 86400).toFixed(1)} days`;
    if (seconds >= 3600) return `${(seconds / 3600).toFixed(1)} hours`;
    return `${(seconds / 60).toFixed(0)} minutes`;
  };

  if (loading) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator size="large" color="#06b6d4" />
      </View>
    );
  }

  if (!object) {
    return (
      <View style={styles.centered}>
        <Text style={styles.errorText}>Object not found</Text>
      </View>
    );
  }

  const percentage = object.maxEnergy > 0 ? Math.round((object.energy / object.maxEnergy) * 100) : 0;

  return (
    <SafeAreaView style={styles.container} edges={['bottom']}>
      <ScrollView contentContainerStyle={styles.scroll}>
        {/* Header */}
        <View style={styles.header}>
          <Text style={styles.name}>{object.name}</Text>
          <View style={[styles.stateBadge, { backgroundColor: STATE_COLORS[object.state] || '#9ca3af' }]}>
            <Text style={styles.stateBadgeText}>{object.state}</Text>
          </View>
        </View>
        <Text style={styles.objectId} selectable>{object.id}</Text>

        {/* Energy Section */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Energy Status</Text>
          <EnergyBar
            energy={object.energy}
            maxEnergy={object.maxEnergy}
            showLabel
            showPercentage
            height={12}
            style={styles.energyBar}
          />
          <View style={styles.statsGrid}>
            <View style={styles.stat}>
              <Text style={styles.statValue}>{object.energy.toLocaleString()}</Text>
              <Text style={styles.statLabel}>Current</Text>
            </View>
            <View style={styles.stat}>
              <Text style={styles.statValue}>{object.maxEnergy.toLocaleString()}</Text>
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
            {formatCountdown(object.estimatedGhostTime)}
          </Text>
          <View style={styles.metaRow}>
            <Text style={styles.metaLabel}>Half-Life</Text>
            <Text style={styles.metaValue}>{formatHalfLife(object.halfLife)}</Text>
          </View>
          <View style={styles.metaRow}>
            <Text style={styles.metaLabel}>Decay Rate</Text>
            <Text style={styles.metaValue}>{object.decayPercentage.toFixed(2)}% / epoch</Text>
          </View>
        </View>

        {/* Owner */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Details</Text>
          <View style={styles.metaRow}>
            <Text style={styles.metaLabel}>Owner</Text>
            <Text style={styles.metaValue} numberOfLines={1}>
              {object.owner.slice(0, 12)}...{object.owner.slice(-6)}
            </Text>
          </View>
        </View>

        {/* Refresh Section */}
        {object.state !== 'Ghost' && (
          <View style={styles.card}>
            <Text style={styles.cardTitle}>Refresh Energy</Text>
            <Text style={styles.refreshDescription}>
              Add energy to extend the object's lifetime and prevent evaporation.
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
    padding: 16,
    paddingBottom: 40,
  },
  header: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginBottom: 4,
  },
  name: {
    fontSize: 24,
    fontWeight: '700',
    color: '#111827',
    flex: 1,
  },
  stateBadge: {
    paddingHorizontal: 12,
    paddingVertical: 4,
    borderRadius: 10,
    marginLeft: 12,
  },
  stateBadgeText: {
    fontSize: 12,
    color: '#ffffff',
    fontWeight: '700',
    textTransform: 'uppercase',
  },
  objectId: {
    fontSize: 12,
    color: '#9ca3af',
    fontFamily: 'monospace',
    marginBottom: 20,
  },
  card: {
    backgroundColor: '#ffffff',
    borderRadius: 14,
    padding: 16,
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

export default ObjectDetailScreen;
