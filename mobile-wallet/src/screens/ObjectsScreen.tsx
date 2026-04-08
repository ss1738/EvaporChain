/**
 * ObjectsScreen — View and manage owned chain objects
 *
 * Shows objects with energy bars, state badges, sort-by-urgency,
 * and one-tap refresh buttons.
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
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { api } from '../utils/api';
import type { ChainObject } from '../utils/api';
import { keystore } from '../utils/keystore';
import { EnergyBar } from '../components/EnergyBar';

const STATE_COLORS: Record<string, string> = {
  Active: '#22c55e',
  Grace: '#f59e0b',
  Ghost: '#9ca3af',
};

const ObjectsScreen: React.FC = () => {
  const [objects, setObjects] = useState<ChainObject[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshingId, setRefreshingId] = useState<string | null>(null);

  const loadObjects = useCallback(async () => {
    try {
      const address = await keystore.getAddress();
      if (!address) return;
      const result = await api.getObjects(address);
      // Sort by energy percentage ascending (most urgent first)
      result.sort((a, b) => {
        const pctA = a.maxEnergy > 0 ? a.energy / a.maxEnergy : 0;
        const pctB = b.maxEnergy > 0 ? b.energy / b.maxEnergy : 0;
        return pctA - pctB;
      });
      setObjects(result);
    } catch {
      // Keep existing data
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadObjects();
  }, [loadObjects]);

  const handleRefreshObject = async (objectId: string) => {
    setRefreshingId(objectId);
    try {
      const result = await api.refreshObject(objectId, 1000);
      if (result.success) {
        Alert.alert('Refreshed', 'Object energy has been restored.');
        await loadObjects();
      } else {
        Alert.alert('Error', result.message);
      }
    } catch {
      Alert.alert('Error', 'Could not refresh object.');
    } finally {
      setRefreshingId(null);
    }
  };

  const formatTimeLeft = (estimatedGhostTime: number): string => {
    const ms = estimatedGhostTime - Date.now();
    if (ms <= 0) return 'Expired';
    const hours = Math.floor(ms / 3600000);
    const days = Math.floor(hours / 24);
    if (days > 0) return `${days}d ${hours % 24}h left`;
    if (hours > 0) return `${hours}h left`;
    return `${Math.round(ms / 60000)}m left`;
  };

  const renderObject = ({ item }: { item: ChainObject }) => {
    const isRefreshing = refreshingId === item.id;

    return (
      <View style={styles.card}>
        <View style={styles.cardHeader}>
          <View style={styles.cardTitleRow}>
            <Text style={styles.cardName}>{item.name}</Text>
            <View
              style={[
                styles.stateBadge,
                { backgroundColor: STATE_COLORS[item.state] || '#9ca3af' },
              ]}
            >
              <Text style={styles.stateBadgeText}>{item.state}</Text>
            </View>
          </View>
          <Text style={styles.cardId}>{item.id.slice(0, 16)}...</Text>
        </View>

        <EnergyBar
          energy={item.energy}
          maxEnergy={item.maxEnergy}
          showLabel
          showPercentage
          style={styles.energyBar}
        />

        <View style={styles.cardFooter}>
          <Text style={styles.timeLeft}>{formatTimeLeft(item.estimatedGhostTime)}</Text>
          {item.state !== 'Ghost' && (
            <TouchableOpacity
              style={styles.refreshButton}
              onPress={() => handleRefreshObject(item.id)}
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
      <FlatList
        data={objects}
        keyExtractor={(item) => item.id}
        renderItem={renderObject}
        contentContainerStyle={styles.list}
        refreshControl={
          <RefreshControl
            refreshing={false}
            onRefresh={loadObjects}
            tintColor="#06b6d4"
          />
        }
        ListEmptyComponent={
          <View style={styles.emptyState}>
            <Text style={styles.emptyTitle}>No Objects</Text>
            <Text style={styles.emptySubtext}>
              Chain objects you own will appear here with their energy status.
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
  cardHeader: {
    marginBottom: 12,
  },
  cardTitleRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
  },
  cardName: {
    fontSize: 16,
    fontWeight: '600',
    color: '#111827',
    flex: 1,
  },
  stateBadge: {
    paddingHorizontal: 10,
    paddingVertical: 3,
    borderRadius: 10,
    marginLeft: 8,
  },
  stateBadgeText: {
    fontSize: 11,
    color: '#ffffff',
    fontWeight: '700',
    textTransform: 'uppercase',
  },
  cardId: {
    fontSize: 12,
    color: '#9ca3af',
    marginTop: 4,
    fontFamily: 'monospace',
  },
  energyBar: {
    marginBottom: 12,
  },
  cardFooter: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
  },
  timeLeft: {
    fontSize: 13,
    color: '#6b7280',
  },
  refreshButton: {
    backgroundColor: '#ecfeff',
    borderWidth: 1,
    borderColor: '#06b6d4',
    borderRadius: 8,
    paddingHorizontal: 16,
    paddingVertical: 8,
    minWidth: 80,
    alignItems: 'center',
    minHeight: 36,
    justifyContent: 'center',
  },
  refreshButtonText: {
    color: '#06b6d4',
    fontSize: 13,
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

export default ObjectsScreen;
